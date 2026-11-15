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

mod animation;
mod file;
mod media;
mod metadata;
mod motion;
mod project;
mod settings;
mod track;

pub use animation::{FrameAnimation, FrameLayout, sequence_frame_path, split_numbered_path};
pub use file::{ProjectFileError, with_project_extension};
pub use media::{
    AudioProperties, ImageProperties, MediaContent, MediaId, MediaSource, TextAlignment,
    TextProperties,
};
pub use metadata::ProjectMetadata;
pub use motion::{Easing, KEYFRAME_TOLERANCE_MILLISECONDS, Keyframe, Motion, Transform};
pub use project::{LayerMove, Project};
pub use settings::{FrameRate, ProjectSettings};
pub use track::{
    AudioInput, AudioReaction, Clip, ClipId, Placement, ReactionBand, ReactionEffect, Track,
    TrackId, TrackKind,
};

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
/// - 7: a clip records which audio it reacts to (`audio_input`). Older
///   files load with every clip reacting to the whole mix.
/// - 8: a clip records where in the picture it is drawn (`placement`).
///   Older files load with every clip filling the frame.
/// - 9: visualizer plugins as a kind of media, and a clip's values for the
///   plugin's parameters (`parameters`).
/// - 10: images as a kind of media, and a clip's `opacity` and audio
///   `reaction`. Older files load fully opaque and without a reaction.
/// - 11: a reaction can also move and turn its element and has a
///   sensitivity and a release time. Older reactions keep working.
/// - 12: video files as a kind of media.
/// - 13: text elements as a kind of media.
/// - 14: a clip records where in its media it begins
///   (`source_offset_milliseconds`), so clips can be trimmed and split.
/// - 15: a visual clip's `audio_gain` (how strongly it hears its audio).
/// - 16: a visual clip's `rotation_degrees`, `spin_degrees_per_second` and
///   `keyframes`. Older files load unturned and without keyframes.
/// - 17: an audio file's hand-placed `manual_beats` and chosen `tempo_bpm`.
/// - 18: frame animations (image sequences and sprite sheets) as a kind of
///   media.
pub const CURRENT_FORMAT_VERSION: u32 = 18;
