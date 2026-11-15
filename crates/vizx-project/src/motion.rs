//! How a visual clip moves over time: where it is, how large, how turned
//! and how opaque, at every moment of the clip.
//!
//! A clip has a resting state (its own placement, rotation and opacity).
//! Once it has keyframes, the keyframes decide instead: between two of them
//! every value glides from one to the other. A clip can also keep turning
//! at a steady speed ("spin"), on top of everything else.

use serde::{Deserialize, Serialize};

use crate::{Clip, Placement};

/// Two moments closer than this count as the same keyframe, in
/// milliseconds (about one frame at 30 frames per second).
pub const KEYFRAME_TOLERANCE_MILLISECONDS: u64 = 20;

/// Where and how a visual clip is drawn at one moment.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub placement: Placement,
    /// Turn around its middle, in degrees clockwise.
    pub rotation_degrees: f64,
    /// 0 (invisible) to 1.
    pub opacity: f64,
}

impl Transform {
    /// The value `amount` of the way from `self` to `other` (0 gives
    /// `self`, 1 gives `other`).
    pub fn blend(&self, other: &Self, amount: f64) -> Self {
        let mix = |from: f64, to: f64| from + (to - from) * amount;
        Self {
            placement: Placement {
                x: mix(self.placement.x, other.placement.x),
                y: mix(self.placement.y, other.placement.y),
                width: mix(self.placement.width, other.placement.width),
                height: mix(self.placement.height, other.placement.height),
            },
            rotation_degrees: mix(self.rotation_degrees, other.rotation_degrees),
            opacity: mix(self.opacity, other.opacity),
        }
    }

    /// The nearest valid transform.
    pub fn sanitized(self) -> Self {
        Self {
            placement: self.placement.sanitized(),
            rotation_degrees: if self.rotation_degrees.is_finite() {
                self.rotation_degrees.clamp(-3600.0, 3600.0)
            } else {
                0.0
            },
            opacity: if self.opacity.is_finite() {
                self.opacity.clamp(0.0, 1.0)
            } else {
                1.0
            },
        }
    }
}

/// How the values travel towards a keyframe from the one before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Easing {
    /// Starts gently and arrives gently.
    #[default]
    Smooth,
    /// At an even speed.
    Linear,
    /// Stays at the previous keyframe and jumps on this one.
    Hold,
}

impl Easing {
    pub const ALL: [Self; 3] = [Self::Smooth, Self::Linear, Self::Hold];

    /// Name shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Self::Smooth => "Smooth",
            Self::Linear => "Linear",
            Self::Hold => "Hold",
        }
    }

    /// How far along the way the values are when `progress` of the time
    /// between the two keyframes has passed (both from 0 to 1).
    fn shape(self, progress: f64) -> f64 {
        match self {
            Self::Smooth => progress * progress * (3.0 - 2.0 * progress),
            Self::Linear => progress,
            Self::Hold => 0.0,
        }
    }
}

/// The state of a clip at one moment of it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    /// When, counted from the start of the clip. After the clip has been
    /// trimmed, a keyframe can lie before its start (negative) or after its
    /// end: it is not shown, but it still shapes the motion, so trimming
    /// never changes how the rest of the clip moves.
    pub time_milliseconds: i64,
    pub transform: Transform,
    /// How the values travel to this keyframe from the one before.
    #[serde(default)]
    pub easing: Easing,
}

/// Everything about how a clip moves, on its own (for code that does not
/// need the rest of the clip, such as the export).
#[derive(Debug, Clone, PartialEq)]
pub struct Motion {
    pub resting: Transform,
    pub keyframes: Vec<Keyframe>,
    pub spin_degrees_per_second: f64,
}

impl Motion {
    /// A motion that stays at `placement`, fully opaque and not turned.
    pub fn still(placement: Placement) -> Self {
        Self {
            resting: Transform {
                placement,
                rotation_degrees: 0.0,
                opacity: 1.0,
            },
            keyframes: Vec::new(),
            spin_degrees_per_second: 0.0,
        }
    }

    /// The keyed state at `milliseconds_into_clip` (without spin).
    pub fn transform_at(&self, milliseconds_into_clip: f64) -> Transform {
        transform_at(&self.resting, &self.keyframes, milliseconds_into_clip)
    }

    /// Everything at `seconds_into_clip`, spin included: what is drawn.
    pub fn animated_at(&self, seconds_into_clip: f64) -> Transform {
        let mut transform = self.transform_at(seconds_into_clip * 1000.0);
        transform.rotation_degrees += self.spin_degrees_per_second * seconds_into_clip;
        transform
    }

    /// The largest width and height the clip ever has, as fractions of the
    /// frame. A visualizer is rendered at that size, so it stays sharp at
    /// its largest.
    pub fn largest_size(&self) -> (f64, f64) {
        let placements = std::iter::once(&self.resting)
            .chain(self.keyframes.iter().map(|keyframe| &keyframe.transform))
            .map(|transform| transform.placement);
        placements.fold((0.0f64, 0.0f64), |(width, height), placement| {
            (width.max(placement.width), height.max(placement.height))
        })
    }
}

/// The keyed state at a moment: the resting state without keyframes;
/// before the first and after the last keyframe, that keyframe; in between,
/// a blend of the two around it.
fn transform_at(resting: &Transform, keyframes: &[Keyframe], milliseconds: f64) -> Transform {
    let (Some(first), Some(last)) = (keyframes.first(), keyframes.last()) else {
        return *resting;
    };
    if milliseconds <= first.time_milliseconds as f64 {
        return first.transform;
    }
    if milliseconds >= last.time_milliseconds as f64 {
        return last.transform;
    }
    let next =
        keyframes.partition_point(|keyframe| keyframe.time_milliseconds as f64 <= milliseconds);
    let (from, to) = (&keyframes[next - 1], &keyframes[next]);
    let span = (to.time_milliseconds - from.time_milliseconds).max(1) as f64;
    let progress = (milliseconds - from.time_milliseconds as f64) / span;
    from.transform
        .blend(&to.transform, to.easing.shape(progress))
}

impl Clip {
    /// The resting state: what the clip looks like without keyframes.
    pub fn resting_transform(&self) -> Transform {
        Transform {
            placement: self.placement,
            rotation_degrees: self.rotation_degrees,
            opacity: self.opacity,
        }
    }

    /// Everything about how the clip moves.
    pub fn motion(&self) -> Motion {
        Motion {
            resting: self.resting_transform(),
            keyframes: self.keyframes.clone(),
            spin_degrees_per_second: self.spin_degrees_per_second,
        }
    }

    /// The keyed state at `milliseconds_into_clip` (without spin).
    pub fn transform_at(&self, milliseconds_into_clip: f64) -> Transform {
        transform_at(
            &self.resting_transform(),
            &self.keyframes,
            milliseconds_into_clip,
        )
    }

    /// The keyframe at `milliseconds_into_clip`, if there is one there.
    pub fn keyframe_at(&self, milliseconds_into_clip: u64) -> Option<&Keyframe> {
        self.keyframe_index_at(milliseconds_into_clip)
            .map(|index| &self.keyframes[index])
    }

    fn keyframe_index_at(&self, milliseconds_into_clip: u64) -> Option<usize> {
        self.keyframes.iter().position(|keyframe| {
            keyframe
                .time_milliseconds
                .abs_diff(milliseconds_into_clip as i64)
                <= KEYFRAME_TOLERANCE_MILLISECONDS
        })
    }

    /// Sets the state at `milliseconds_into_clip`. Without keyframes that
    /// is the resting state; with keyframes, the keyframe there is changed,
    /// or a new one is made. Returns `true` only if something changed.
    pub(crate) fn set_transform_at(
        &mut self,
        milliseconds_into_clip: u64,
        transform: Transform,
    ) -> bool {
        let transform = transform.sanitized();
        if self.keyframes.is_empty() {
            let has_changed = self.resting_transform() != transform;
            self.placement = transform.placement;
            self.rotation_degrees = transform.rotation_degrees;
            self.opacity = transform.opacity;
            return has_changed;
        }
        match self.keyframe_index_at(milliseconds_into_clip) {
            Some(index) => {
                let has_changed = self.keyframes[index].transform != transform;
                self.keyframes[index].transform = transform;
                has_changed
            }
            None => {
                self.insert_keyframe(Keyframe {
                    time_milliseconds: milliseconds_into_clip as i64,
                    transform,
                    easing: Easing::default(),
                });
                true
            }
        }
    }

    /// Records the current state at `milliseconds_into_clip` as a
    /// keyframe. Returns `false` if there already is one there.
    pub(crate) fn add_keyframe_at(&mut self, milliseconds_into_clip: u64) -> bool {
        if self.keyframe_index_at(milliseconds_into_clip).is_some() {
            return false;
        }
        let transform = self.transform_at(milliseconds_into_clip as f64);
        self.insert_keyframe(Keyframe {
            time_milliseconds: milliseconds_into_clip as i64,
            transform,
            easing: Easing::default(),
        });
        true
    }

    /// Removes the keyframe at `milliseconds_into_clip`. When the last one
    /// goes, its state becomes the resting state, so the clip stays as it
    /// was. Returns `false` if there is no keyframe there.
    pub(crate) fn remove_keyframe_at(&mut self, milliseconds_into_clip: u64) -> bool {
        let Some(index) = self.keyframe_index_at(milliseconds_into_clip) else {
            return false;
        };
        let removed = self.keyframes.remove(index);
        if self.keyframes.is_empty() {
            self.placement = removed.transform.placement;
            self.rotation_degrees = removed.transform.rotation_degrees;
            self.opacity = removed.transform.opacity;
        }
        true
    }

    /// Sets how the values travel to the keyframe at
    /// `milliseconds_into_clip`. Returns `true` only if something changed.
    pub(crate) fn set_keyframe_easing_at(
        &mut self,
        milliseconds_into_clip: u64,
        easing: Easing,
    ) -> bool {
        match self.keyframe_index_at(milliseconds_into_clip) {
            Some(index) if self.keyframes[index].easing != easing => {
                self.keyframes[index].easing = easing;
                true
            }
            _ => false,
        }
    }

    fn insert_keyframe(&mut self, keyframe: Keyframe) {
        let index = self
            .keyframes
            .partition_point(|other| other.time_milliseconds < keyframe.time_milliseconds);
        self.keyframes.insert(index, keyframe);
    }

    /// The keyframes after the start of the clip has moved by
    /// `shift_milliseconds` against them (positive: the clip now starts
    /// earlier). They keep their moments in project time.
    pub(crate) fn shifted_keyframes(&self, shift_milliseconds: i64) -> Vec<Keyframe> {
        self.keyframes
            .iter()
            .map(|keyframe| Keyframe {
                time_milliseconds: keyframe.time_milliseconds + shift_milliseconds,
                ..*keyframe
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f64, opacity: f64) -> Transform {
        Transform {
            placement: Placement {
                x,
                y: 0.0,
                width: 0.5,
                height: 0.5,
            },
            rotation_degrees: x * 100.0,
            opacity,
        }
    }

    fn keyframe(time_milliseconds: i64, x: f64, easing: Easing) -> Keyframe {
        Keyframe {
            time_milliseconds,
            transform: at(x, 1.0),
            easing,
        }
    }

    fn motion(keyframes: Vec<Keyframe>) -> Motion {
        Motion {
            resting: at(0.9, 0.5),
            keyframes,
            spin_degrees_per_second: 0.0,
        }
    }

    #[test]
    fn without_keyframes_the_resting_state_holds_and_spin_turns_it() {
        let mut still = motion(Vec::new());
        assert_eq!(still.transform_at(5_000.0), at(0.9, 0.5));

        still.spin_degrees_per_second = 90.0;
        let turned = still.animated_at(2.0);
        assert_eq!(turned.rotation_degrees, 90.0 + 180.0);
        assert_eq!(turned.placement, at(0.9, 0.5).placement);
    }

    #[test]
    fn values_glide_between_keyframes_according_to_the_easing() {
        let linear = motion(vec![
            keyframe(1_000, 0.0, Easing::Smooth),
            keyframe(2_000, 0.4, Easing::Linear),
        ]);
        // Before the first and after the last keyframe they hold.
        assert_eq!(linear.transform_at(0.0).placement.x, 0.0);
        assert_eq!(linear.transform_at(9_000.0).placement.x, 0.4);
        let quarter = linear.transform_at(1_250.0);
        assert!((quarter.placement.x - 0.1).abs() < 1e-12);
        assert!((quarter.rotation_degrees - 10.0).abs() < 1e-9);

        // Smooth starts slower than linear and meets it halfway.
        let smooth = motion(vec![
            keyframe(1_000, 0.0, Easing::Smooth),
            keyframe(2_000, 0.4, Easing::Smooth),
        ]);
        assert!(smooth.transform_at(1_250.0).placement.x < 0.1);
        assert!((smooth.transform_at(1_500.0).placement.x - 0.2).abs() < 1e-12);

        // Hold stays and then jumps.
        let hold = motion(vec![
            keyframe(1_000, 0.0, Easing::Smooth),
            keyframe(2_000, 0.4, Easing::Hold),
        ]);
        assert_eq!(hold.transform_at(1_999.0).placement.x, 0.0);
        assert_eq!(hold.transform_at(2_000.0).placement.x, 0.4);
    }

    #[test]
    fn the_largest_size_covers_every_keyframe() {
        let mut big = keyframe(1_000, 0.0, Easing::Smooth);
        big.transform.placement.width = 1.5;
        let still = motion(vec![keyframe(0, 0.0, Easing::Smooth), big]);

        assert_eq!(still.largest_size(), (1.5, 0.5));
    }
}
