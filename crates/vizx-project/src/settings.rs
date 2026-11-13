use serde::{Deserialize, Serialize};

/// Frames per second stored as an exact fraction.
///
/// Rates such as 29.97 fps are really 30000/1001. Storing the fraction instead
/// of a floating-point number keeps frame timing exact, which the future
/// deterministic export clock depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameRate {
    pub numerator: u32,
    pub denominator: u32,
}

impl FrameRate {
    pub const FPS_30: Self = Self::whole(30);
    pub const FPS_60: Self = Self::whole(60);

    /// A whole-number frame rate such as 30 or 60 fps.
    pub const fn whole(frames_per_second: u32) -> Self {
        Self {
            numerator: frames_per_second,
            denominator: 1,
        }
    }

    /// Approximate value for display. Do not use it for timing calculations.
    pub fn as_f64(self) -> f64 {
        f64::from(self.numerator) / f64::from(self.denominator)
    }
}

/// Settings that define the output composition of a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSettings {
    /// Composition width in pixels.
    pub width: u32,
    /// Composition height in pixels.
    pub height: u32,
    pub frame_rate: FrameRate,
}

impl Default for ProjectSettings {
    /// 1920x1080 at 30 fps.
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::FPS_30,
        }
    }
}
