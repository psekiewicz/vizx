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

    /// A file name for saving this project for the first time, derived from
    /// its name, e.g. `My Song.vzx`.
    pub fn suggested_file_name(&self) -> String {
        let cleaned: String = self
            .metadata
            .name
            .chars()
            .map(|character| {
                if INVALID_FILE_NAME_CHARACTERS.contains(&character) || character.is_control() {
                    '_'
                } else {
                    character
                }
            })
            .collect();
        let cleaned = cleaned.trim().trim_end_matches('.');
        let base_name = if cleaned.is_empty() {
            "Untitled"
        } else {
            cleaned
        };

        format!("{base_name}.{PROJECT_FILE_EXTENSION}")
    }

    pub fn save_to_file(&self, path: &Path) -> Result<(), ProjectFileError> {
        fs::write(path, self.to_vzx_string()?)?;
        Ok(())
    }

    pub fn load_from_file(path: &Path) -> Result<Self, ProjectFileError> {
        Self::from_vzx_str(&fs::read_to_string(path)?)
    }
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

/// Characters that are not allowed in file names on Windows. Avoiding them
/// everywhere keeps suggested names valid on every supported system.
const INVALID_FILE_NAME_CHARACTERS: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

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
    fn project_round_trips_through_vzx_text() {
        let mut project = Project::new("Round trip");
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
        let project = Project::new("On disk");

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
        assert_eq!(project.metadata.name, "Old Project");
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
    fn suggested_file_name_replaces_invalid_characters() {
        assert_eq!(
            Project::new("AC/DC: Live?").suggested_file_name(),
            "AC_DC_ Live_.vzx"
        );
        assert_eq!(Project::new("  ").suggested_file_name(), "Untitled.vzx");
    }

    #[test]
    fn missing_file_is_an_io_error() {
        let path = std::env::temp_dir().join("vizx-test-file-that-does-not-exist.vzx");

        let error = Project::load_from_file(&path).unwrap_err();

        assert!(matches!(error, ProjectFileError::Io(_)));
    }
}
