//! VizX project data model.
//!
//! This crate describes *what a project is* as plain data. It knows nothing
//! about the GUI, rendering or audio devices, which is what keeps `.vzx`
//! files portable and lets every subsystem share one description of the
//! project.
//!
//! Milestone 0.1 contains only metadata and settings. Media sources, tracks,
//! clips, keyframes and effects will be added to [`Project`] in later
//! milestones.

mod metadata;
mod project;
mod settings;

pub use metadata::ProjectMetadata;
pub use project::Project;
pub use settings::{FrameRate, ProjectSettings};

/// File extension of native VizX project files, without the leading dot.
pub const PROJECT_FILE_EXTENSION: &str = "vzx";

/// Version of the project data layout written by this build.
///
/// It is stored in every project so that future versions of VizX can detect
/// and migrate older files. Increase it whenever the layout changes in a way
/// older builds cannot read.
pub const CURRENT_FORMAT_VERSION: u32 = 1;
