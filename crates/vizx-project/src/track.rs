use serde::{Deserialize, Serialize};

/// Identifies a track within one project.
///
/// Other parts of the project and the editor refer to a track by its id
/// instead of its position in the list, so a reference stays valid when
/// tracks are reordered or other tracks are removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TrackId(pub(crate) u64);

/// What kind of content a track holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrackKind {
    Audio,
    Visualizer,
    Animation,
    Text,
}

impl TrackKind {
    pub const ALL: [Self; 4] = [Self::Audio, Self::Visualizer, Self::Animation, Self::Text];

    /// Name of the kind as shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Self::Audio => "Audio",
            Self::Visualizer => "Visualizer",
            Self::Animation => "Animation",
            Self::Text => "Text",
        }
    }
}

/// One horizontal lane of the timeline. Clips will be added to tracks in a
/// later milestone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub name: String,
    pub kind: TrackKind,
}
