//! What VizX remembers between runs, outside any project: the recently
//! opened projects, and autosaved copies of the open project so that work
//! survives a crash or a power cut.
//!
//! Everything lives in a small folder of its own (`%APPDATA%\VizX` on
//! Windows, `~/.config/vizx` elsewhere). The files are a few kilobytes.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::EditorSession;

/// How many recent projects are remembered.
const RECENT_PROJECT_COUNT: usize = 10;
/// How often the open project is autosaved while it has unsaved changes.
pub const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(60);

/// The folder for VizX's own files, if the system has one.
pub fn app_data_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(|folder| PathBuf::from(folder).join("VizX"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|folder| folder.join("vizx"))
    }
}

/// Settings of VizX itself (not of a project).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// The projects opened or saved last, the newest first.
    pub recent_projects: Vec<PathBuf>,
    /// Whether the welcome window opens when VizX starts.
    pub show_welcome: bool,
    /// How the panels were arranged when VizX closed, as the user interface
    /// stores it (`None`: the default layout).
    pub panel_layout: Option<serde_json::Value>,
    /// Playback volume, 0 to 1.
    pub volume: f32,
    /// Whether clips on one track may cover each other.
    pub allow_clip_overlap: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            recent_projects: Vec::new(),
            show_welcome: true,
            panel_layout: None,
            volume: 1.0,
            allow_clip_overlap: false,
        }
    }
}

impl AppSettings {
    /// The settings stored in `folder`; defaults if there are none yet.
    pub fn load(folder: &Path) -> Self {
        fs::read_to_string(folder.join("settings.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, folder: &Path) -> Result<(), String> {
        fs::create_dir_all(folder).map_err(|error| error.to_string())?;
        let text = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        fs::write(folder.join("settings.json"), text).map_err(|error| error.to_string())
    }

    /// Puts `path` at the top of the recent projects.
    pub fn remember_project(&mut self, path: &Path) {
        self.recent_projects.retain(|recent| recent != path);
        self.recent_projects.insert(0, path.to_owned());
        self.recent_projects.truncate(RECENT_PROJECT_COUNT);
    }
}

/// Where an autosave belongs, stored next to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct AutosaveNote {
    /// The project file it is a copy of; `None` if it was never saved.
    project_path: Option<PathBuf>,
    /// Shown to the user: the project's name.
    project_name: String,
    /// The VizX that wrote it.
    process_id: u32,
}

/// Keeps an autosaved copy of the open project while it has unsaved
/// changes. Call [`Autosaver::tick`] once per frame; it writes at most once
/// per [`AUTOSAVE_INTERVAL`], and only when something changed.
pub struct Autosaver {
    folder: PathBuf,
    last_check: Instant,
    /// What was written last, to skip writing the same again.
    last_written: Option<String>,
}

impl Autosaver {
    pub fn new(app_data: &Path) -> Self {
        Self {
            folder: app_data.join("autosave"),
            last_check: Instant::now(),
            last_written: None,
        }
    }

    fn files(&self) -> (PathBuf, PathBuf) {
        let stem = format!("autosave-{}", std::process::id());
        (
            self.folder.join(format!("{stem}.vzx")),
            self.folder.join(format!("{stem}.json")),
        )
    }

    /// Autosaves `session` if it is time and it has unsaved changes.
    /// Returns `Ok(true)` when it wrote.
    pub fn tick(&mut self, session: &EditorSession, now: Instant) -> Result<bool, String> {
        if now.duration_since(self.last_check) < AUTOSAVE_INTERVAL {
            return Ok(false);
        }
        self.last_check = now;
        if !session.has_unsaved_changes() {
            // Saved by the user: the copy is not needed any more.
            self.clear();
            return Ok(false);
        }
        self.write(session)
    }

    /// Writes the copy now (if it differs from the last one).
    pub fn write(&mut self, session: &EditorSession) -> Result<bool, String> {
        let text = session
            .project()
            .to_vzx_string()
            .map_err(|error| error.to_string())?;
        if self.last_written.as_ref() == Some(&text) {
            return Ok(false);
        }
        fs::create_dir_all(&self.folder).map_err(|error| error.to_string())?;
        let (project_file, note_file) = self.files();
        let note = AutosaveNote {
            project_path: session.file_path().map(Path::to_path_buf),
            project_name: session.project_name(),
            process_id: std::process::id(),
        };
        let note_text = serde_json::to_string_pretty(&note).map_err(|error| error.to_string())?;
        // The project first: a note is only ever next to a whole copy.
        fs::write(&project_file, &text).map_err(|error| error.to_string())?;
        fs::write(&note_file, note_text).map_err(|error| error.to_string())?;
        self.last_written = Some(text);
        Ok(true)
    }

    /// Removes the copy: after saving, after a deliberate close, or when
    /// the project was replaced.
    pub fn clear(&mut self) {
        let (project_file, note_file) = self.files();
        let _ = fs::remove_file(note_file);
        let _ = fs::remove_file(project_file);
        self.last_written = None;
    }
}

/// An autosaved project left behind by a VizX that did not close properly.
#[derive(Debug, Clone, PartialEq)]
pub struct Recovery {
    /// The autosaved copy.
    pub autosave: PathBuf,
    note: PathBuf,
    /// The file it is a copy of, if the project had been saved before.
    pub project_path: Option<PathBuf>,
    pub project_name: String,
    /// When the copy was written.
    pub saved_at: Option<SystemTime>,
}

impl Recovery {
    /// Deletes the copy (after it was recovered or the user declined it).
    pub fn discard(&self) {
        let _ = fs::remove_file(&self.note);
        let _ = fs::remove_file(&self.autosave);
    }
}

/// The autosaves in `app_data` whose VizX is no longer running, newest
/// first.
pub fn find_recoveries(app_data: &Path) -> Vec<Recovery> {
    let Ok(entries) = fs::read_dir(app_data.join("autosave")) else {
        return Vec::new();
    };
    let mut found: Vec<Recovery> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .filter_map(|note_path| {
            let note: AutosaveNote =
                serde_json::from_str(&fs::read_to_string(&note_path).ok()?).ok()?;
            let autosave = note_path.with_extension("vzx");
            let is_left_behind =
                note.process_id != std::process::id() && !is_process_running(note.process_id);
            (is_left_behind && autosave.is_file()).then(|| Recovery {
                saved_at: fs::metadata(&autosave)
                    .and_then(|data| data.modified())
                    .ok(),
                autosave,
                note: note_path,
                project_path: note.project_path,
                project_name: note.project_name,
            })
        })
        .collect();
    found.sort_by_key(|recovery| std::cmp::Reverse(recovery.saved_at));
    found
}

/// Whether a process with this id is running.
#[cfg(windows)]
fn is_process_running(process_id: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: plain Windows calls on a handle that is checked and closed.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id);
        if handle.is_null() {
            return false;
        }
        let mut exit_code = 0;
        let is_running =
            GetExitCodeProcess(handle, &mut exit_code) != 0 && exit_code == STILL_ACTIVE as u32;
        CloseHandle(handle);
        is_running
    }
}

#[cfg(not(windows))]
fn is_process_running(process_id: u32) -> bool {
    Path::new(&format!("/proc/{process_id}")).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_folder(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("vizx-app-data-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        folder
    }

    #[test]
    fn recent_projects_are_kept_newest_first_without_repeats() {
        let folder = temporary_folder("recent");
        let mut settings = AppSettings::load(&folder);
        for index in 0..12 {
            settings.remember_project(Path::new(&format!("song{index}.vzx")));
        }
        settings.remember_project(Path::new("song5.vzx"));
        settings.save(&folder).unwrap();

        let loaded = AppSettings::load(&folder);
        let _ = fs::remove_dir_all(&folder);

        assert_eq!(loaded.recent_projects.len(), RECENT_PROJECT_COUNT);
        assert_eq!(loaded.recent_projects[0], Path::new("song5.vzx"));
        assert_eq!(loaded.recent_projects[1], Path::new("song11.vzx"));
        assert_eq!(
            loaded
                .recent_projects
                .iter()
                .filter(|path| *path == Path::new("song5.vzx"))
                .count(),
            1
        );
    }

    #[test]
    fn an_autosave_is_written_only_with_unsaved_changes_and_found_after_a_crash() {
        let folder = temporary_folder("autosave");
        let mut session = EditorSession::new_untitled();
        let mut autosaver = Autosaver::new(&folder);
        let later = Instant::now() + AUTOSAVE_INTERVAL * 2;

        // Nothing changed: nothing written.
        assert_eq!(autosaver.tick(&session, later), Ok(false));
        session.add_text().unwrap();
        // Too soon after the last check.
        assert_eq!(autosaver.tick(&session, later), Ok(false));
        assert_eq!(
            autosaver.tick(&session, later + AUTOSAVE_INTERVAL),
            Ok(true)
        );
        // The same project again: not written twice.
        assert_eq!(autosaver.write(&session), Ok(false));

        // This VizX is running, so its own autosave is no recovery...
        assert!(find_recoveries(&folder).is_empty());
        // ...but one written by a VizX that is gone is.
        let (project_file, note_file) = autosaver.files();
        let note = fs::read_to_string(&note_file).unwrap();
        let gone = note.replace(
            &format!("\"process_id\": {}", std::process::id()),
            "\"process_id\": 4000000000",
        );
        fs::write(folder.join("autosave/autosave-1.json"), gone).unwrap();
        fs::copy(&project_file, folder.join("autosave/autosave-1.vzx")).unwrap();

        let recoveries = find_recoveries(&folder);
        assert_eq!(recoveries.len(), 1);
        assert_eq!(recoveries[0].project_path, None);
        let recovered = EditorSession::recover(&recoveries[0].autosave, None).unwrap();
        assert!(recovered.has_unsaved_changes());
        assert_eq!(recovered.project(), session.project());

        recoveries[0].discard();
        autosaver.clear();
        let leftovers = fs::read_dir(folder.join("autosave")).unwrap().count();
        let _ = fs::remove_dir_all(&folder);
        assert_eq!(leftovers, 0);
    }
}
