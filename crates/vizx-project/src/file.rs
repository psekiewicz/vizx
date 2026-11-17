//! Reading and writing `.vzx` project files.
//!
//! A `.vzx` file is the [`Project`] encoded as UTF-8 JSON. JSON was chosen
//! because it is human-readable, portable between operating systems, and
//! produces meaningful diffs in version control.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::{CURRENT_FORMAT_VERSION, PROJECT_FILE_EXTENSION, Project};

/// Everything that can go wrong while saving or loading a project file.
#[derive(Debug)]
pub enum ProjectFileError {
    /// The file could not be read or written (missing, no permission, ...).
    Io(io::Error),
    /// The file is not a valid VizX project.
    InvalidFormat(serde_json::Error),
    /// The file was written by a newer VizX with a data layout this build
    /// does not understand.
    UnsupportedVersion { found: u32, supported: u32 },
}

impl fmt::Display for ProjectFileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "file access failed: {error}"),
            Self::InvalidFormat(error) => {
                write!(formatter, "not a valid VizX project: {error}")
            }
            Self::UnsupportedVersion { found, supported } => write!(
                formatter,
                "project format version {found} is newer than this version of VizX supports \
                 (up to {supported})"
            ),
        }
    }
}

impl std::error::Error for ProjectFileError {}

// These `From` implementations let the `?` operator convert the underlying
// errors into `ProjectFileError` automatically.
impl From<io::Error> for ProjectFileError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ProjectFileError {
    fn from(error: serde_json::Error) -> Self {
        Self::InvalidFormat(error)
    }
}

impl Project {
    /// Encodes the project as the text content of a `.vzx` file.
    pub fn to_vzx_string(&self) -> Result<String, ProjectFileError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Decodes a project from the text content of a `.vzx` file.
    pub fn from_vzx_str(content: &str) -> Result<Self, ProjectFileError> {
        // Read only the version first. A file from a newer VizX may contain
        // fields this build cannot parse, and "unsupported version" is a far
        // more useful message than a parse error.
        let header: FormatHeader = serde_json::from_str(content)?;
        if header.format_version > CURRENT_FORMAT_VERSION {
            return Err(ProjectFileError::UnsupportedVersion {
                found: header.format_version,
                supported: CURRENT_FORMAT_VERSION,
            });
        }

        let mut project: Self = serde_json::from_str(content)?;
        // An older file has now been upgraded in memory (fields it lacked
        // received defaults), so it will be saved in the current layout.
        project.format_version = CURRENT_FORMAT_VERSION;
        Ok(project)
    }

    /// Writes the project to `path`. Every file it uses is also noted
    /// relative to the project's folder, so the project still finds its
    /// files after being moved together with them.
    pub fn save_to_file(&self, path: &Path) -> Result<(), ProjectFileError> {
        let mut stored = self.clone();
        if let Some(folder) = path.parent() {
            for media in &mut stored.media {
                media.relative_path = relative_location(folder, Path::new(&media.path));
            }
        }
        fs::write(path, stored.to_vzx_string()?)?;
        Ok(())
    }

    /// Reads the project in `path`. A file that is no longer at its full
    /// path, but is at its place relative to the project, is used from
    /// there.
    pub fn load_from_file(path: &Path) -> Result<Self, ProjectFileError> {
        let mut project = Self::from_vzx_str(&fs::read_to_string(path)?)?;
        if let Some(folder) = path.parent() {
            for media in &mut project.media {
                if let Some(found) =
                    moved_location(folder, &media.path, media.relative_path.as_deref())
                {
                    media.path = found;
                }
            }
        }
        Ok(project)
    }

    /// Points every media file whose file is missing and that lies in
    /// `old_folder` (the folder of a file the user just located again) to
    /// the same name in `new_folder`, if a file is there. Returns how many
    /// were found this way.
    pub fn relink_missing_media_in_folder(
        &mut self,
        old_folder: &Path,
        new_folder: &Path,
    ) -> usize {
        let mut found = 0;
        for media in &mut self.media {
            let old_path = Path::new(&media.path);
            if !is_file_reference(&media.path)
                || old_path.exists()
                || old_path.parent() != Some(old_folder)
            {
                continue;
            }
            if let Some(name) = old_path.file_name() {
                let candidate = new_folder.join(name);
                if candidate.exists() {
                    media.path = candidate.to_string_lossy().into_owned();
                    found += 1;
                }
            }
        }
        found
    }

    /// The media sources whose file (or plugin folder) is not where the
    /// project says.
    pub fn missing_media(&self) -> Vec<crate::MediaId> {
        self.media
            .iter()
            .filter(|media| is_file_reference(&media.path) && !Path::new(&media.path).exists())
            .map(|media| media.id)
            .collect()
    }

    /// Points every file the project uses (media files, plugin folders,
    /// fonts) for which `new_path` knows a new place there. Returns how many
    /// changed.
    pub fn repoint_media(&mut self, new_path: impl Fn(&str) -> Option<String>) -> usize {
        let mut changed = 0;
        let mut repoint = |path: &mut String| {
            if let Some(new) = new_path(path).filter(|new| new != path) {
                *path = new;
                changed += 1;
            }
        };
        for media in &mut self.media {
            repoint(&mut media.path);
            if let crate::MediaContent::Text(text) = &mut media.content {
                repoint(&mut text.font_path);
            }
        }
        for track in self.tracks_mut() {
            if let Some(style) = &mut track.subtitle_style {
                repoint(&mut style.font_path);
            }
        }
        changed
    }

    /// Changes the file of one media source (after the user located a
    /// moved file). Returns `true` if it changed.
    pub fn relink_media(&mut self, id: crate::MediaId, path: &Path) -> bool {
        match self.media.iter_mut().find(|media| media.id == id) {
            Some(media) => {
                let path = path.to_string_lossy().into_owned();
                let has_changed = media.path != path;
                media.path = path;
                has_changed
            }
            None => false,
        }
    }
}

/// Whether a stored path names a file on disk (rather than something that
/// ships with VizX, like `bundled-presets:/...`, or nothing, like a text).
pub(crate) fn is_file_reference(path: &str) -> bool {
    !path.is_empty() && !path.starts_with("bundled-")
}

/// `file` relative to `folder`, with `/` between parts; `None` if there is
/// no such path (another drive) or `file` is not a file reference.
fn relative_location(folder: &Path, file: &Path) -> Option<String> {
    use std::path::Component;
    if !is_file_reference(&file.to_string_lossy()) || !file.is_absolute() {
        return None;
    }
    let folder: Vec<Component> = folder.components().collect();
    let file: Vec<Component> = file.components().collect();
    // Different drives (C: and D:) share no path.
    if folder.first() != file.first() {
        return None;
    }
    let common = folder.iter().zip(&file).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".to_owned(); folder.len() - common];
    parts.extend(
        file[common..]
            .iter()
            .map(|part| part.as_os_str().to_string_lossy().into_owned()),
    );
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Where a moved file is now: `None` if it is still at `path` (or cannot
/// be found anywhere). Tried in order: its place relative to the project,
/// then its name in the project's folder.
fn moved_location(folder: &Path, path: &str, relative: Option<&str>) -> Option<String> {
    if !is_file_reference(path) || Path::new(path).exists() {
        return None;
    }
    let by_relative = relative.map(|relative| folder.join(relative));
    let by_name = Path::new(path).file_name().map(|name| folder.join(name));
    by_relative
        .into_iter()
        .chain(by_name)
        .find(|candidate| candidate.exists())
        .map(|found| normalized(&found).to_string_lossy().into_owned())
}

/// `path` with `.` and `..` parts worked out, without touching the disk.
fn normalized(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

/// Returns `path` with the `.vzx` extension, appending it if it is missing.
///
/// The extension is appended rather than replaced, so a name the user typed
/// such as `song.v2` becomes `song.v2.vzx` instead of losing its `.v2`.
pub fn with_project_extension(path: PathBuf) -> PathBuf {
    let has_extension = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(PROJECT_FILE_EXTENSION));
    if has_extension {
        return path;
    }

    let mut file_name = path.into_os_string();
    file_name.push(".");
    file_name.push(PROJECT_FILE_EXTENSION);
    PathBuf::from(file_name)
}

/// The one field every version of the format is guaranteed to have.
#[derive(serde::Deserialize)]
struct FormatHeader {
    format_version: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TrackKind;

    #[test]
    fn a_project_moved_with_its_files_finds_them_again() {
        let root = std::env::temp_dir().join(format!("vizx-moved-project-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let before = root.join("before");
        fs::create_dir_all(before.join("music")).unwrap();
        fs::write(before.join("music").join("song.wav"), b"x").unwrap();
        fs::write(before.join("cover.png"), b"x").unwrap();

        let mut project = Project::new();
        let audio = crate::AudioProperties {
            duration_milliseconds: 1000,
            sample_rate: 44_100,
            channel_count: 2,
        };
        let song = before.join("music").join("song.wav");
        let (song_id, _) = project.import_audio(song.to_string_lossy(), audio);
        project.save_to_file(&before.join("song.vzx")).unwrap();

        // The whole folder moves elsewhere.
        let after = root.join("after");
        fs::rename(&before, &after).unwrap();
        let mut moved = Project::load_from_file(&after.join("song.vzx")).unwrap();
        let found = PathBuf::from(&moved.media_source(song_id).unwrap().path);
        assert_eq!(found, after.join("music").join("song.wav"));

        // A file located again by hand brings its neighbours along.
        moved.media[0].path = before.join("cover.png").to_string_lossy().into_owned();
        moved.media[0].relative_path = None;
        assert_eq!(moved.relink_missing_media_in_folder(&before, &after), 1);
        assert_eq!(PathBuf::from(&moved.media[0].path), after.join("cover.png"));

        assert_eq!(
            relative_location(Path::new("C:/a/b"), Path::new("C:/a/c/d.wav")).as_deref(),
            Some("../c/d.wav")
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn project_round_trips_through_vzx_text() {
        let mut project = Project::new();
        project.metadata.author = "Tester".to_owned();

        let content = project.to_vzx_string().unwrap();
        let decoded = Project::from_vzx_str(&content).unwrap();

        assert_eq!(decoded, project);
    }

    #[test]
    fn project_round_trips_through_a_file() {
        let path = std::env::temp_dir().join(format!(
            "vizx-test-{}.{}",
            std::process::id(),
            crate::PROJECT_FILE_EXTENSION
        ));
        let project = Project::new();

        project.save_to_file(&path).unwrap();
        let loaded = Project::load_from_file(&path);
        fs::remove_file(&path).unwrap();

        assert_eq!(loaded.unwrap(), project);
    }

    #[test]
    fn newer_format_version_is_reported_as_unsupported() {
        let content = format!(
            r#"{{ "format_version": {}, "something_new": true }}"#,
            CURRENT_FORMAT_VERSION + 1
        );

        let error = Project::from_vzx_str(&content).unwrap_err();

        assert!(matches!(
            error,
            ProjectFileError::UnsupportedVersion { found, .. } if found == CURRENT_FORMAT_VERSION + 1
        ));
    }

    #[test]
    fn version_1_file_loads_and_is_upgraded() {
        // Exactly what VizX wrote before tracks and duration existed.
        let version_1_content = r#"{
            "format_version": 1,
            "metadata": {
                "name": "Old Project",
                "author": "",
                "description": "",
                "created_with_version": "0.1.0"
            },
            "settings": {
                "width": 1280,
                "height": 720,
                "frame_rate": { "numerator": 30, "denominator": 1 }
            }
        }"#;

        let project = Project::from_vzx_str(version_1_content).unwrap();

        assert_eq!(project.format_version, CURRENT_FORMAT_VERSION);
        // Version 1 stored a name inside the file. It is ignored now: the
        // project is named by its file.
        assert_eq!(project.metadata.author, "");
        assert_eq!(project.settings.width, 1280);
        assert_eq!(project.settings.duration_milliseconds, 180_000);
        assert!(project.tracks().is_empty());
    }

    #[test]
    fn version_2_track_kinds_load_as_audio_and_visual() {
        let version_2_content = r#"{
            "format_version": 2,
            "metadata": {
                "name": "Six Tracks",
                "author": "",
                "description": "",
                "created_with_version": "0.1.0"
            },
            "settings": {
                "width": 1920,
                "height": 1080,
                "frame_rate": { "numerator": 30, "denominator": 1 },
                "duration_milliseconds": 60000
            },
            "tracks": [
                { "id": 0, "name": "Vocals", "kind": "Audio" },
                { "id": 1, "name": "Visualizer", "kind": "Visualizer" },
                { "id": 2, "name": "Animation", "kind": "Animation" },
                { "id": 3, "name": "Lyrics", "kind": "Text" }
            ],
            "next_track_id": 4
        }"#;

        let project = Project::from_vzx_str(version_2_content).unwrap();

        let kinds: Vec<TrackKind> = project.tracks().iter().map(|track| track.kind).collect();
        assert_eq!(
            kinds,
            [
                TrackKind::Audio,
                TrackKind::Visual,
                TrackKind::Visual,
                TrackKind::Visual
            ]
        );
        // The user's track names and order are kept as they were.
        assert_eq!(project.tracks()[3].name, "Lyrics");
        assert_eq!(project.format_version, CURRENT_FORMAT_VERSION);
    }

    #[test]
    fn version_6_clip_without_audio_input_reacts_to_the_mix() {
        let version_6_content = r#"{
            "format_version": 6,
            "metadata": { "author": "", "description": "", "created_with_version": "0.1.0" },
            "settings": {
                "width": 1920,
                "height": 1080,
                "frame_rate": { "numerator": 30, "denominator": 1 },
                "duration_milliseconds": 60000
            },
            "tracks": [
                { "id": 1, "name": "Layer 1", "kind": "Visual", "clips": [
                    { "id": 0, "media": 0, "start_milliseconds": 0, "duration_milliseconds": 60000 }
                ] }
            ],
            "next_track_id": 2,
            "media": [ { "id": 0, "path": "a.milk", "content": "MilkdropPreset" } ],
            "next_media_id": 1,
            "next_clip_id": 1
        }"#;

        let project = Project::from_vzx_str(version_6_content).unwrap();

        let clip = &project.tracks()[0].clips[0];
        assert_eq!(clip.audio_input, crate::AudioInput::Mix);
        // Fields added after version 6 get their neutral values.
        assert_eq!(clip.placement, crate::Placement::FULL);
        assert_eq!(clip.opacity, 1.0);
        assert!(!clip.reaction.is_active());
        assert_eq!(project.format_version, CURRENT_FORMAT_VERSION);
    }

    #[test]
    fn text_that_is_not_a_project_is_rejected() {
        let error = Project::from_vzx_str("this is not a project").unwrap_err();

        assert!(matches!(error, ProjectFileError::InvalidFormat(_)));
    }

    #[test]
    fn project_extension_is_appended_only_when_missing() {
        assert_eq!(
            with_project_extension(PathBuf::from("song")),
            PathBuf::from("song.vzx")
        );
        assert_eq!(
            with_project_extension(PathBuf::from("song.v2")),
            PathBuf::from("song.v2.vzx")
        );
        assert_eq!(
            with_project_extension(PathBuf::from("song.VZX")),
            PathBuf::from("song.VZX")
        );
    }

    #[test]
    fn missing_file_is_an_io_error() {
        let path = std::env::temp_dir().join("vizx-test-file-that-does-not-exist.vzx");

        let error = Project::load_from_file(&path).unwrap_err();

        assert!(matches!(error, ProjectFileError::Io(_)));
    }
}
