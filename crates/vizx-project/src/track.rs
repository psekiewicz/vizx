use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Keyframe, MediaId};

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
    /// For an audio track: it is not heard and not part of the mix, but
    /// visuals can still follow it (stems separated from a song are added
    /// like this, so the song is not played twice).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub muted: bool,
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
    /// Where in its media the clip begins: 0 uses the media from its start,
    /// more skips the beginning. It grows when the clip is trimmed at its
    /// left edge. Only audio and video have a "where"; for everything else
    /// it stays 0.
    #[serde(default)]
    pub source_offset_milliseconds: u64,
    /// Which audio a visual clip reacts to. Not used by audio clips.
    ///
    /// `#[serde(default)]` lets files written before this field existed
    /// (format version 6 and older) load: there, every visualizer reacted
    /// to the whole mix.
    #[serde(default)]
    pub audio_input: AudioInput,
    /// How strongly a visual clip hears its audio: the sound is multiplied
    /// by this before the clip's visualizer or its animation sees it. 1
    /// leaves it as it is, less calms the visuals down, more excites them.
    /// It does not change what is played.
    #[serde(default = "unchanged_gain")]
    pub audio_gain: f64,
    /// Where in the picture a visual clip is drawn. Not used by audio
    /// clips. Files from format version 7 and older have no such field;
    /// their clips fill the whole frame.
    #[serde(default)]
    pub placement: Placement,
    /// Values the user chose for the parameters of a visualizer plugin, by
    /// parameter id: one number for a slider or a switch, three (red,
    /// green, blue) for a color. A parameter without an entry uses the
    /// plugin's default. Which parameters exist is up to the plugin, so the
    /// project stores them without interpreting them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parameters: BTreeMap<String, Vec<f64>>,
    /// How opaque a visual clip is as a whole: 1 hides what is below it
    /// (where the clip itself is not transparent), 0 makes it invisible.
    #[serde(default = "full_opacity")]
    pub opacity: f64,
    /// How a visual clip moves with the music.
    #[serde(default)]
    pub reaction: AudioReaction,
    /// How far a visual clip is turned around its middle, in degrees
    /// clockwise (its resting state; keyframes override it).
    #[serde(default)]
    pub rotation_degrees: f64,
    /// How fast a visual clip keeps turning, in degrees per second
    /// (negative: anticlockwise). 0 keeps it still.
    #[serde(default)]
    pub spin_degrees_per_second: f64,
    /// The states of a visual clip at chosen moments, in order of time (see
    /// `motion.rs`). Without keyframes the resting state (placement,
    /// rotation, opacity) holds for the whole clip.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keyframes: Vec<Keyframe>,
    /// Whether a picture drawn larger than it is shows its pixels as sharp
    /// squares (pixel art) instead of smoothing between them. None decides
    /// by the file: sharp for PNG images and animations, smooth otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sharp_pixels: Option<bool>,
    /// How a visual clip's colors combine with the layers below it.
    #[serde(default, skip_serializing_if = "BlendMode::is_normal")]
    pub blend_mode: BlendMode,
}

/// How a visual clip's colors combine with the layers below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BlendMode {
    /// Covers what is below, as far as it is opaque.
    #[default]
    Normal,
    /// Adds its colors: brightens, black disappears.
    Add,
    /// Like Add, but softer and never brighter than white.
    Screen,
    /// Multiplies colors: darkens, white disappears.
    Multiply,
    /// Keeps the brighter of the two colors.
    Lighten,
}

impl BlendMode {
    pub const ALL: [Self; 5] = [
        Self::Normal,
        Self::Add,
        Self::Screen,
        Self::Multiply,
        Self::Lighten,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Add => "Add",
            Self::Screen => "Screen",
            Self::Multiply => "Multiply",
            Self::Lighten => "Lighten",
        }
    }

    fn is_normal(&self) -> bool {
        *self == Self::Normal
    }
}

fn full_opacity() -> f64 {
    1.0
}

fn unchanged_gain() -> f64 {
    1.0
}

/// How a visual element is animated by the sound: the louder its chosen
/// part of the sound is, the more it grows, moves, turns and/or appears.
/// This is what lets a still image "dance" with the music without any
/// frame-by-frame animation. The sound itself is chosen with the clip's
/// [`AudioInput`].
///
/// Every amount is the change at full loudness; 0 turns that effect off.
/// `#[serde(default)]` lets files written before a field existed load.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioReaction {
    /// Which part of the sound is followed.
    pub band: ReactionBand,
    /// How much larger the element gets, as a fraction of its size (0.2
    /// means 20 % larger).
    pub scale: f64,
    /// How much of its opacity the element loses in silence (1 means it is
    /// invisible when silent and fully visible at full loudness).
    pub fade: f64,
    /// How far the element moves, as fractions of the frame's width and
    /// height. Negative values move left / up.
    pub move_x: f64,
    pub move_y: f64,
    /// How far the element turns, in degrees clockwise.
    pub rotate_degrees: f64,
    /// Multiplies the measured loudness before it is used, so quiet
    /// material can still drive a strong animation.
    pub sensitivity: f64,
    /// How long the animation takes to fall back after a loud moment, in
    /// seconds. It always rises at once. 0 follows the sound exactly, which
    /// can look nervous; a little release makes it look like a bounce.
    pub release_seconds: f64,
}

impl Default for AudioReaction {
    fn default() -> Self {
        Self {
            band: ReactionBand::default(),
            scale: 0.0,
            fade: 0.0,
            move_x: 0.0,
            move_y: 0.0,
            rotate_degrees: 0.0,
            sensitivity: 1.0,
            release_seconds: 0.15,
        }
    }
}

/// What a reaction does to its element at one moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReactionEffect {
    /// Size factor; 1 is the normal size.
    pub scale: f64,
    /// Opacity factor, 0 to 1.
    pub opacity: f64,
    /// Shift as fractions of the frame's width and height.
    pub offset_x: f64,
    pub offset_y: f64,
    /// Turn in degrees, clockwise.
    pub rotation_degrees: f64,
}

impl ReactionEffect {
    /// Leaves the element exactly as it is.
    pub const NONE: Self = Self {
        scale: 1.0,
        opacity: 1.0,
        offset_x: 0.0,
        offset_y: 0.0,
        rotation_degrees: 0.0,
    };
}

impl AudioReaction {
    /// Largest allowed `scale`.
    pub const MAX_SCALE: f64 = 2.0;
    /// Largest allowed move in either direction.
    pub const MAX_MOVE: f64 = 1.0;
    /// Largest allowed turn in either direction, in degrees.
    pub const MAX_ROTATION: f64 = 360.0;
    pub const SENSITIVITY_RANGE: (f64, f64) = (0.25, 8.0);
    pub const MAX_RELEASE_SECONDS: f64 = 2.0;

    /// Whether the element reacts at all.
    pub fn is_active(self) -> bool {
        [
            self.scale,
            self.fade,
            self.move_x,
            self.move_y,
            self.rotate_degrees,
        ]
        .iter()
        .any(|amount| *amount != 0.0)
    }

    /// The nearest valid reaction; values that are not numbers become the
    /// defaults.
    pub fn sanitized(self) -> Self {
        let defaults = Self::default();
        let within = |value: f64, min: f64, max: f64, fallback: f64| {
            if value.is_finite() {
                value.clamp(min, max)
            } else {
                fallback
            }
        };
        let (least_sensitive, most_sensitive) = Self::SENSITIVITY_RANGE;
        Self {
            band: self.band,
            scale: within(self.scale, 0.0, Self::MAX_SCALE, 0.0),
            fade: within(self.fade, 0.0, 1.0, 0.0),
            move_x: within(self.move_x, -Self::MAX_MOVE, Self::MAX_MOVE, 0.0),
            move_y: within(self.move_y, -Self::MAX_MOVE, Self::MAX_MOVE, 0.0),
            rotate_degrees: within(
                self.rotate_degrees,
                -Self::MAX_ROTATION,
                Self::MAX_ROTATION,
                0.0,
            ),
            sensitivity: within(
                self.sensitivity,
                least_sensitive,
                most_sensitive,
                defaults.sensitivity,
            ),
            release_seconds: within(
                self.release_seconds,
                0.0,
                Self::MAX_RELEASE_SECONDS,
                defaults.release_seconds,
            ),
        }
    }

    /// What the reaction does for a loudness `level` from 0 to 1 (as
    /// measured; the sensitivity is applied here).
    pub fn at_level(self, level: f64) -> ReactionEffect {
        let level = if level.is_finite() {
            (level * self.sensitivity).clamp(0.0, 1.0)
        } else {
            0.0
        };
        ReactionEffect {
            scale: 1.0 + self.scale * level,
            opacity: 1.0 - self.fade * (1.0 - level),
            offset_x: self.move_x * level,
            offset_y: self.move_y * level,
            rotation_degrees: self.rotate_degrees * level,
        }
    }
}
/// The part of the sound an [`AudioReaction`] follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ReactionBand {
    /// Overall loudness.
    #[default]
    Volume,
    /// Low frequencies: kick drum, bass.
    Bass,
    /// Middle frequencies: voice, most instruments.
    Mid,
    /// High frequencies: cymbals, hi-hats.
    Treble,
    /// The beats: a short push on every beat, fading out over the release
    /// time.
    Beat,
}

impl ReactionBand {
    pub const ALL: [Self; 5] = [
        Self::Volume,
        Self::Bass,
        Self::Mid,
        Self::Treble,
        Self::Beat,
    ];

    /// Name shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Self::Volume => "Volume",
            Self::Bass => "Bass",
            Self::Mid => "Mid",
            Self::Treble => "Treble",
            Self::Beat => "Beat",
        }
    }
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
    /// Smallest and largest allowed `audio_gain`.
    pub const AUDIO_GAIN_RANGE: (f64, f64) = (0.01, 4.0);
    /// The shortest a clip can be made, in milliseconds.
    pub const MIN_DURATION_MILLISECONDS: u64 = 100;

    pub fn end_milliseconds(&self) -> u64 {
        self.start_milliseconds + self.duration_milliseconds
    }
}
