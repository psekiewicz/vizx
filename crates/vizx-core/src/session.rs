use std::path::{Path, PathBuf};

use vizx_audio::AudioFileError;
use vizx_project::{
    AudioProperties, ImageProperties, Project, ProjectFileError, Track, TrackId, TrackKind,
};

use crate::{PresetEntry, PresetLibrary, Selection};

/// Name shown for a project that has not been saved to a file yet.
pub const UNTITLED_PROJECT_NAME: &str = "Untitled";

/// The state of one open project while it is being edited.
///
/// There is exactly one session in the application and every UI panel is a
/// view of it. Panels never keep their own copy of project data, which is
/// what keeps the editor unified instead of a set of separate tools.
#[derive(Debug, Clone)]
pub struct EditorSession {
    project: Project,
    /// Where the project is stored on disk. `None` until it is first saved.
    file_path: Option<PathBuf>,
    has_unsaved_changes: bool,
    selection: Selection,
    /// Current position on the project timeline, in seconds.
    playhead_seconds: f64,
}

impl EditorSession {
    /// Starts a session with a new, never-saved project.
    pub fn new_untitled() -> Self {
        Self {
            project: Project::with_starter_tracks(),
            file_path: None,
            has_unsaved_changes: false,
            selection: Selection::Nothing,
            playhead_seconds: 0.0,
        }
    }

    /// Starts a session by loading a project from a `.vzx` file.
    pub fn open(path: &Path) -> Result<Self, ProjectFileError> {
        Ok(Self {
            project: Project::load_from_file(path)?,
            file_path: Some(path.to_owned()),
            has_unsaved_changes: false,
            selection: Selection::Nothing,
            playhead_seconds: 0.0,
        })
    }

    /// Where the playhead is on the project timeline, in seconds.
    ///
    /// Every panel that shows "now" (the timeline, the time display, later
    /// the preview) reads this one value. Like the selection, it is editing
    /// state: moving it is not a change to the project and it is not saved.
    pub fn playhead_seconds(&self) -> f64 {
        self.playhead_seconds
    }

    /// Moves the playhead, keeping it within the project.
    pub fn set_playhead_seconds(&mut self, seconds: f64) {
        let project_end = self.project.settings.duration_seconds();
        self.playhead_seconds = if seconds.is_finite() {
            seconds.clamp(0.0, project_end)
        } else {
            0.0
        };
    }

    pub fn selection(&self) -> Selection {
        self.selection
    }

    /// Changes the selection. Selecting is not an edit, so it does not mark
    /// the project as changed.
    pub fn select(&mut self, selection: Selection) {
        self.selection = selection;
    }

    /// The selected track, if a track is selected and still exists.
    pub fn selected_track(&self) -> Option<&Track> {
        match self.selection {
            Selection::Track(id) => self.project.track(id),
            Selection::Nothing => None,
        }
    }

    /// Adds a track to the timeline and selects it.
    pub fn add_track(&mut self, kind: TrackKind) -> TrackId {
        let id = self.project.add_default_named_track(kind);
        self.has_unsaved_changes = true;
        self.selection = Selection::Track(id);
        id
    }

    /// Imports an audio file: reads its properties, adds it to the project's
    /// media and places it on an audio track, which becomes selected.
    ///
    /// The project is left untouched if the file cannot be read.
    pub fn import_audio(&mut self, path: &Path) -> Result<TrackId, AudioFileError> {
        let info = vizx_audio::probe_audio_file(path)?;
        let properties = AudioProperties {
            duration_milliseconds: info.duration_milliseconds,
            sample_rate: info.sample_rate,
            channel_count: info.channel_count,
        };

        let (_media_id, track_id) = self
            .project
            .import_audio(path.to_string_lossy(), properties);
        self.has_unsaved_changes = true;
        self.selection = Selection::Track(track_id);
        Ok(track_id)
    }

    /// Adds a MilkDrop preset (`.milk` file) to the project and places it
    /// on a visual layer, which becomes selected.
    ///
    /// Only the file's existence is checked here. Whether projectM can
    /// actually run the preset is found out when it is first drawn.
    pub fn add_milkdrop_preset(&mut self, path: &Path) -> Result<TrackId, String> {
        let is_milk_file = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("milk"));
        if !is_milk_file {
            return Err("not a MilkDrop preset (.milk) file".to_owned());
        }
        if !path.is_file() {
            return Err("the file does not exist".to_owned());
        }

        let (_media_id, track_id) = self.project.add_milkdrop_preset(path.to_string_lossy());
        self.has_unsaved_changes = true;
        self.selection = Selection::Track(track_id);
        Ok(track_id)
    }

    /// Adds a preset that ships with VizX and places it on a visual layer,
    /// which becomes selected.
    ///
    /// The project stores where the preset is inside the bundled collection
    /// instead of a path on this computer, so the project also opens on
    /// other machines (see [`BUNDLED_PRESET_PREFIX`](crate::BUNDLED_PRESET_PREFIX)).
    pub fn add_bundled_milkdrop_preset(&mut self, entry: &PresetEntry) -> TrackId {
        let reference = PresetLibrary::project_reference(entry);
        let (_media_id, track_id) = self.project.add_milkdrop_preset(reference);
        self.has_unsaved_changes = true;
        self.selection = Selection::Track(track_id);
        track_id
    }

    /// Imports an image: reads its size, adds it to the project's media and
    /// places it on a visual layer, which becomes selected.
    ///
    /// The project is left untouched if the file cannot be read.
    pub fn import_image(&mut self, path: &Path) -> Result<TrackId, String> {
        if !crate::has_supported_image_extension(path) {
            return Err("not a PNG or JPEG image".to_owned());
        }
        let (width, height) = crate::image_store::image_file_size(path)?;

        let (_media_id, track_id) = self
            .project
            .add_image(path.to_string_lossy(), ImageProperties { width, height });
        self.has_unsaved_changes = true;
        self.selection = Selection::Track(track_id);
        Ok(track_id)
    }

    /// Imports an image as the background: on a new layer behind all
    /// others, covering the whole frame. The new layer becomes selected.
    pub fn import_background_image(&mut self, path: &Path) -> Result<TrackId, String> {
        if !crate::has_supported_image_extension(path) {
            return Err("not a PNG or JPEG image".to_owned());
        }
        let (width, height) = crate::image_store::image_file_size(path)?;

        let (_media_id, track_id) = self
            .project
            .add_background_image(path.to_string_lossy(), ImageProperties { width, height });
        self.has_unsaved_changes = true;
        self.selection = Selection::Track(track_id);
        Ok(track_id)
    }

    /// Adds a visualizer plugin and places it on a visual layer, which
    /// becomes selected. `stored_path` is the plugin's folder, or the
    /// reference of a bundled plugin (see `VisualizerCatalog`).
    pub fn add_visualizer_plugin(&mut self, stored_path: &str) -> TrackId {
        let (_media_id, track_id) = self.project.add_visualizer_plugin(stored_path);
        self.has_unsaved_changes = true;
        self.selection = Selection::Track(track_id);
        track_id
    }

    /// Removes a track, clearing the selection if it was selected.
    pub fn remove_track(&mut self, id: TrackId) {
        if self.project.remove_track(id) {
            self.has_unsaved_changes = true;
            if self.selection == Selection::Track(id) {
                self.selection = Selection::Nothing;
            }
        }
    }

    /// Read-only access to the project.
    pub fn project(&self) -> &Project {
        &self.project
    }

    /// Applies an edit to the project and records that it changed.
    ///
    /// All modifications go through this method so the "unsaved changes"
    /// flag cannot be forgotten. It is also the natural place to hook in
    /// undo/redo later. The closure returns `true` if it changed anything.
    pub fn edit_project(&mut self, edit: impl FnOnce(&mut Project) -> bool) {
        if edit(&mut self.project) {
            self.has_unsaved_changes = true;
        }
    }

    pub fn file_path(&self) -> Option<&Path> {
        self.file_path.as_deref()
    }

    /// The project's name: the name of its file without the extension, or
    /// `Untitled` while it has never been saved.
    ///
    /// This is the only name a project has, so the window title, dialogs
    /// and the file on disk can never disagree.
    pub fn project_name(&self) -> String {
        self.file_path
            .as_deref()
            .and_then(Path::file_stem)
            .map_or_else(
                || UNTITLED_PROJECT_NAME.to_owned(),
                |stem| stem.to_string_lossy().into_owned(),
            )
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.has_unsaved_changes
    }

    /// Saves the project to `path` and makes that its file from now on.
    ///
    /// If writing fails, the session is left exactly as it was: it keeps its
    /// previous path and still counts as having unsaved changes.
    pub fn save_as(&mut self, path: &Path) -> Result<(), ProjectFileError> {
        self.project.save_to_file(path)?;
        self.file_path = Some(path.to_owned());
        self.has_unsaved_changes = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique path in the system temp directory for one test.
    fn temporary_project_path(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "vizx-session-{test_name}-{}.vzx",
            std::process::id()
        ))
    }

    fn rename_project(session: &mut EditorSession, name: &str) {
        session.edit_project(|project| {
            project.metadata.author = name.to_owned();
            true
        });
    }

    #[test]
    fn bundled_preset_is_stored_by_its_place_in_the_collection() {
        let root =
            std::env::temp_dir().join(format!("vizx-session-bundled-{}", std::process::id()));
        let preset_file = root.join("pack/Fractal/Swirl.milk");
        std::fs::create_dir_all(preset_file.parent().unwrap()).unwrap();
        std::fs::write(&preset_file, "[preset00]").unwrap();
        let library = PresetLibrary::scan(&root).unwrap();
        let _ = std::fs::remove_dir_all(&root);

        let mut session = EditorSession::new_untitled();
        session.add_bundled_milkdrop_preset(&library.entries()[0]);

        assert!(session.has_unsaved_changes());
        let (_clip, preset) = session.project().milkdrop_clips().next().unwrap();
        // No folder of this computer ends up in the project file.
        assert_eq!(preset.path, "bundled-presets:/pack/Fractal/Swirl.milk");
        assert_eq!(
            crate::resolve_preset_path(&preset.path, Some(&root)),
            Some(root.join("pack/Fractal/Swirl.milk"))
        );
    }

    #[test]
    fn new_session_is_unsaved_on_disk_but_has_no_pending_changes() {
        let session = EditorSession::new_untitled();

        assert!(session.file_path().is_none());
        assert!(!session.has_unsaved_changes());
    }

    #[test]
    fn only_real_edits_mark_the_session_as_changed() {
        let mut session = EditorSession::new_untitled();

        session.edit_project(|_project| false);
        assert!(!session.has_unsaved_changes());

        rename_project(&mut session, "Renamed");
        assert!(session.has_unsaved_changes());
        assert_eq!(session.project().metadata.author, "Renamed");
    }

    #[test]
    fn adding_a_track_selects_it_and_marks_the_project_changed() {
        let mut session = EditorSession::new_untitled();
        let track_count_before = session.project().tracks().len();

        let id = session.add_track(TrackKind::Visual);

        assert_eq!(session.project().tracks().len(), track_count_before + 1);
        assert_eq!(session.selection(), Selection::Track(id));
        assert_eq!(session.selected_track().unwrap().kind, TrackKind::Visual);
        assert!(session.has_unsaved_changes());
    }

    #[test]
    fn removing_the_selected_track_clears_the_selection() {
        let mut session = EditorSession::new_untitled();
        let id = session.add_track(TrackKind::Audio);

        session.remove_track(id);

        assert_eq!(session.selection(), Selection::Nothing);
        assert!(session.selected_track().is_none());
    }

    #[test]
    fn importing_an_unreadable_file_changes_nothing() {
        let mut session = EditorSession::new_untitled();
        let missing = std::env::temp_dir().join("vizx-session-missing-audio.wav");

        assert!(session.import_audio(&missing).is_err());
        assert!(session.project().media().is_empty());
        assert!(!session.has_unsaved_changes());
        assert_eq!(session.selection(), Selection::Nothing);
    }

    #[test]
    fn playhead_stays_within_the_project_and_is_not_an_edit() {
        let mut session = EditorSession::new_untitled();
        let project_end = session.project().settings.duration_seconds();

        session.set_playhead_seconds(12.5);
        assert_eq!(session.playhead_seconds(), 12.5);

        session.set_playhead_seconds(-3.0);
        assert_eq!(session.playhead_seconds(), 0.0);

        session.set_playhead_seconds(project_end + 100.0);
        assert_eq!(session.playhead_seconds(), project_end);

        session.set_playhead_seconds(f64::NAN);
        assert_eq!(session.playhead_seconds(), 0.0);

        assert!(!session.has_unsaved_changes());
    }

    #[test]
    fn only_existing_milk_files_can_be_added_as_presets() {
        let mut session = EditorSession::new_untitled();
        let folder = std::env::temp_dir();
        let preset = folder.join(format!("vizx-session-preset-{}.milk", std::process::id()));
        std::fs::write(&preset, "[preset00]\n").unwrap();

        let wrong_extension = session.add_milkdrop_preset(&folder.join("song.mp3"));
        let missing = session.add_milkdrop_preset(&folder.join("vizx-no-such-preset.milk"));
        assert!(!session.has_unsaved_changes());

        let added = session.add_milkdrop_preset(&preset);
        let _ = std::fs::remove_file(&preset);

        assert!(wrong_extension.is_err());
        assert!(missing.is_err());
        let track_id = added.unwrap();
        assert_eq!(session.selection(), Selection::Track(track_id));
        assert!(session.has_unsaved_changes());
        assert!(session.project().milkdrop_preset_at(0).is_some());
    }

    #[test]
    fn selecting_does_not_count_as_a_change() {
        let mut session = EditorSession::new_untitled();
        let id = session.project().tracks()[0].id;

        session.select(Selection::Track(id));

        assert!(!session.has_unsaved_changes());
    }

    #[test]
    fn saving_remembers_the_path_and_clears_unsaved_changes() {
        let path = temporary_project_path("save");
        let mut session = EditorSession::new_untitled();
        rename_project(&mut session, "Saved");

        let result = session.save_as(&path);
        let _ = std::fs::remove_file(&path);

        result.unwrap();
        assert_eq!(session.file_path(), Some(path.as_path()));
        assert!(!session.has_unsaved_changes());
    }

    #[test]
    fn saved_project_can_be_opened_again() {
        let path = temporary_project_path("reopen");
        let mut session = EditorSession::new_untitled();
        rename_project(&mut session, "Reopened");
        session.save_as(&path).unwrap();

        let reopened = EditorSession::open(&path);
        let _ = std::fs::remove_file(&path);

        let reopened = reopened.unwrap();
        assert_eq!(reopened.project(), session.project());
        assert_eq!(reopened.file_path(), Some(path.as_path()));
        assert!(!reopened.has_unsaved_changes());
    }

    #[test]
    fn project_is_named_after_its_file() {
        let path = temporary_project_path("named");
        let mut session = EditorSession::new_untitled();
        assert_eq!(session.project_name(), "Untitled");

        let result = session.save_as(&path);
        let name_after_save = session.project_name();
        let reopened_name = EditorSession::open(&path).map(|session| session.project_name());
        let _ = std::fs::remove_file(&path);

        result.unwrap();
        let expected = path.file_stem().unwrap().to_string_lossy().into_owned();
        assert_eq!(name_after_save, expected);
        assert_eq!(reopened_name.unwrap(), expected);
    }

    #[test]
    fn saving_under_another_name_renames_the_project() {
        let first = temporary_project_path("first-name");
        let second = temporary_project_path("second-name");
        let mut session = EditorSession::new_untitled();

        let first_result = session.save_as(&first);
        let second_result = session.save_as(&second);
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);

        first_result.unwrap();
        second_result.unwrap();
        assert_eq!(
            session.project_name(),
            second.file_stem().unwrap().to_string_lossy()
        );
    }

    #[test]
    fn failed_save_leaves_the_session_unchanged() {
        // A directory that does not exist, so the write must fail.
        let path = std::env::temp_dir()
            .join("vizx-missing-directory")
            .join("project.vzx");
        let mut session = EditorSession::new_untitled();
        rename_project(&mut session, "Not saved");

        assert!(session.save_as(&path).is_err());
        assert!(session.file_path().is_none());
        assert!(session.has_unsaved_changes());
    }
}
