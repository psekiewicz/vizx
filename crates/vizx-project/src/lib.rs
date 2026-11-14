//! VizX project data model.
//!
//! This crate describes *what a project is* as plain data. It knows nothing
//! about the GUI, rendering or audio devices, which is what keeps `.vzx`
//! files portable and lets every subsystem share one description of the
//! project.
//!
//! The model currently contains metadata and settings. Media sources, tracks,
//! clips, keyframes and effects will be added to [`Project`] in later
//! milestones.

mod file;
mod media;
mod metadata;
mod project;
mod settings;
mod track;

pub use file::{ProjectFileError, with_project_extension};
pub use media::{AudioProperties, MediaContent, MediaId, MediaSource};
pub use metadata::ProjectMetadata;
pub use project::Project;
pub use settings::{FrameRate, ProjectSettings};
pub use track::{Clip, ClipId, Track, TrackId, TrackKind};

/// File extension of native VizX project files, without the leading dot.
pub const PROJECT_FILE_EXTENSION: &str = "vzx";

/// Version of the project data layout written by this build.
///
/// It is stored in every project so that future versions of VizX can detect
/// and migrate older files. Increase it whenever the layout changes in a way
/// older builds cannot read.
///
/// History:
/// - 1: metadata and settings.
/// - 2: timeline tracks and project duration. Version 1 files still load;
///   the missing fields get default values.
/// - 3: track kinds reduced to `Audio` and `Visual`. The former
///   `Visualizer`, `Animation` and `Text` kinds load as `Visual`.
/// - 4: media sources and clips on tracks.
/// - 5: the project name is no longer stored in the file; the file name is
///   the project's name. A `name` in older files is ignored.
/// - 6: MilkDrop presets as a kind of media.
pub const CURRENT_FORMAT_VERSION: u32 = 6;
