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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Where in the picture a visual clip is drawn. Not used by audio
    /// clips. Files from format version 7 and older have no such field;
    /// their clips fill the whole frame.
    #[serde(default)]
    pub placement: Placement,
}

/// The rectangle of the composition a visual clip is drawn into.
///
/// All four values are fractions of the composition: `x` and `width` of its
/// width, `y` and `height` of its height, with (0, 0) at the top left. A
/// placement therefore keeps its meaning when the composition size changes,
/// e.g. from a 1080p draft to a 4K export. A clip may reach beyond the
/// frame; the part outside is simply not visible.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Placement {
    /// Covers the whole composition.
    pub const FULL: Self = Self {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };
    /// Smallest and largest allowed width and height.
    pub const MIN_SIZE: f64 = 0.02;
    pub const MAX_SIZE: f64 = 3.0;

    pub fn right(self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(self) -> f64 {
        self.y + self.height
    }

    /// The nearest valid placement: a sensible size, and at least a sliver
    /// of the clip still inside the frame, so it can always be found and
    /// dragged back. Values that are not numbers give [`Placement::FULL`].
    pub fn sanitized(self) -> Self {
        let all_finite = [self.x, self.y, self.width, self.height]
            .iter()
            .all(|value| value.is_finite());
        if !all_finite {
            return Self::FULL;
        }
        let width = self.width.clamp(Self::MIN_SIZE, Self::MAX_SIZE);
        let height = self.height.clamp(Self::MIN_SIZE, Self::MAX_SIZE);
        Self {
            x: self.x.clamp(Self::MIN_SIZE - width, 1.0 - Self::MIN_SIZE),
            y: self.y.clamp(Self::MIN_SIZE - height, 1.0 - Self::MIN_SIZE),
            width,
            height,
        }
    }
}

impl Default for Placement {
    fn default() -> Self {
        Self::FULL
    }
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
