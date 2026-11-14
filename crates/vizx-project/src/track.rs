use serde::{Deserialize, Serialize};

use crate::MediaId;

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

/// One horizontal lane of the timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub name: String,
    pub kind: TrackKind,
    /// What is placed on this track, in no particular order.
    #[serde(default)]
    pub clips: Vec<Clip>,
}

impl Track {
    /// The time at which the last clip on this track ends.
    pub fn end_milliseconds(&self) -> u64 {
        self.clips
            .iter()
            .map(Clip::end_milliseconds)
            .max()
            .unwrap_or(0)
    }
}

/// Identifies a clip within one project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClipId(pub(crate) u64);

/// A piece of media placed on a track at a point in project time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    /// The media source this clip plays.
    pub media: MediaId,
    /// Where on the project timeline the clip begins.
    pub start_milliseconds: u64,
    pub duration_milliseconds: u64,
    /// Which audio a visual clip reacts to. Not used by audio clips.
    ///
    /// `#[serde(default)]` lets files written before this field existed
    /// (format version 6 and older) load: there, every visualizer reacted
    /// to the whole mix.
    #[serde(default)]
    pub audio_input: AudioInput,
}

/// The audio a visual element reacts to.
///
/// This is what separate stems are for: with the vocals, the melody and the
/// bass on three audio tracks, three visualizers can each follow one of
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AudioInput {
    /// Everything that is audible: all audio tracks together.
    #[default]
    Mix,
    /// Only the audio on one audio track.
    Track(TrackId),
}

impl Clip {
    pub fn end_milliseconds(&self) -> u64 {
        self.start_milliseconds + self.duration_milliseconds
    }
}
