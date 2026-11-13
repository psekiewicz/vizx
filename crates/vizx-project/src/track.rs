use serde::{Deserialize, Serialize};

/// Identifies a track within one project.
///
/// Other parts of the project and the editor refer to a track by its id
/// instead of its position in the list, so a reference stays valid when
/// tracks are reordered or other tracks are removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TrackId(pub(crate) u64);

/// The two kinds of timeline tracks.
///
/// VizX is not an audio workstation: audio is the input that drives the
/// visuals, not material to be mixed. So there are only two kinds of track,
/// and what a visual element *is* (visualizer, image, animation, text) is a
/// property of the element placed on a layer, not of the layer itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrackKind {
    /// Holds audio. Most projects have exactly one: the song. More exist
    /// only when the user brings separate stems (vocals, drums, ...), which
    /// visual elements can then react to individually.
    Audio,
    /// A layer of the picture. Layers stack like in a video editor: a layer
    /// higher in the list is drawn on top of the ones below it.
    ///
    /// The aliases let project files from format version 2 load, where
    /// visual tracks were split into three kinds.
    #[serde(alias = "Visualizer", alias = "Animation", alias = "Text")]
    Visual,
}

impl TrackKind {
    pub const ALL: [Self; 2] = [Self::Visual, Self::Audio];

    /// Name of the kind as shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Self::Audio => "Audio Track",
            Self::Visual => "Visual Layer",
        }
    }

    /// First word of automatic track names such as `Layer 2`.
    pub(crate) fn default_name_prefix(self) -> &'static str {
        match self {
            Self::Audio => "Audio",
            Self::Visual => "Layer",
        }
    }
}

/// One horizontal lane of the timeline. Elements will be placed on tracks
/// in a later milestone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub name: String,
    pub kind: TrackKind,
}
