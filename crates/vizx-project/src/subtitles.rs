//! Subtitles: a kind of layer for lines of text over time (lyrics, what
//! people say). Every line is a clip on the layer; how the lines look and
//! where they appear belongs to the layer, so one change restyles them all.
//! Several subtitle layers can show lines at the same time, for example one
//! per singer.
//!
//! Where exactly a line is drawn is worked out when the picture is made
//! (`vizx_core::subtitle_layout`): lines are wrapped or shrunk to stay
//! inside the frame, and lines shown at the same time are moved apart.

use serde::{Deserialize, Serialize};

use crate::{Placement, TextAlignment, TextProperties};

/// How the lines of one subtitle layer look and where they appear.
///
/// `#[serde(default)]` lets files written before a field existed load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SubtitleStyle {
    /// Font file (TrueType or OpenType); empty uses the built-in font.
    pub font_path: String,
    /// Height of the letters as a fraction of the frame's height.
    pub size: f64,
    /// Red, green and blue, each from 0 to 1.
    pub color: [f64; 3],
    /// Thickness of an outline around the letters, as a fraction of their
    /// size; 0 draws none. An outline keeps text readable on any picture.
    pub outline: f64,
    pub outline_color: [f64; 3],
    pub alignment: TextAlignment,
    pub position: SubtitlePosition,
    /// How much of the frame's edge stays free, as a fraction of its
    /// height: lines never come closer to the edge than this.
    pub margin: f64,
    /// What happens to a line that is wider than the frame allows.
    pub fit: SubtitleFit,
    /// How long a line takes to fade in and out, in seconds; 0 shows it at
    /// once.
    pub fade_seconds: f64,
    /// Lines also grow into their size while they fade in.
    pub pop: bool,
    /// Picks the random places (for [`SubtitlePosition::Random`]); a new
    /// number gives new places.
    pub seed: u64,
}

impl Default for SubtitleStyle {
    fn default() -> Self {
        Self {
            font_path: String::new(),
            size: 0.06,
            color: [1.0, 1.0, 1.0],
            outline: 0.08,
            outline_color: [0.0, 0.0, 0.0],
            alignment: TextAlignment::Center,
            position: SubtitlePosition::default(),
            margin: 0.05,
            fit: SubtitleFit::Wrap,
            fade_seconds: 0.15,
            pop: false,
            seed: 1,
        }
    }
}

impl SubtitleStyle {
    /// Smallest and largest letter size.
    pub const SIZE_RANGE: (f64, f64) = (0.015, 0.3);
    /// Largest outline, as a fraction of the letter size.
    pub const OUTLINE_MAX: f64 = 0.3;
    /// Largest margin, as a fraction of the frame's height.
    pub const MARGIN_MAX: f64 = 0.3;

    /// The nearest valid style.
    pub fn sanitized(mut self) -> Self {
        let finite_or =
            |value: f64, fallback: f64| if value.is_finite() { value } else { fallback };
        let defaults = Self::default();
        let (smallest, largest) = Self::SIZE_RANGE;
        self.size = finite_or(self.size, defaults.size).clamp(smallest, largest);
        self.outline = finite_or(self.outline, 0.0).clamp(0.0, Self::OUTLINE_MAX);
        self.margin = finite_or(self.margin, defaults.margin).clamp(0.0, Self::MARGIN_MAX);
        self.fade_seconds = finite_or(self.fade_seconds, 0.0).clamp(0.0, 5.0);
        let color = |part: f64| finite_or(part, 1.0).clamp(0.0, 1.0);
        self.color = self.color.map(color);
        self.outline_color = self.outline_color.map(color);
        self.position = self.position.sanitized();
        self
    }

    /// The look of one line of text in this style.
    pub fn text(&self, text: &str) -> TextProperties {
        TextProperties {
            text: text.to_owned(),
            font_path: self.font_path.clone(),
            size: self.size,
            color: self.color,
            alignment: self.alignment,
            outline: self.outline,
            outline_color: self.outline_color,
        }
    }

    /// A style that looks like `text` (when a layer of texts becomes a
    /// subtitle layer).
    pub fn like(text: &TextProperties) -> Self {
        Self {
            font_path: text.font_path.clone(),
            size: text.size,
            color: text.color,
            outline: text.outline,
            outline_color: text.outline_color,
            alignment: text.alignment,
            ..Self::default()
        }
        .sanitized()
    }
}

/// Where the lines of a subtitle layer appear.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SubtitlePosition {
    /// Always around the same point: the middle of a line is at `x`, `y`
    /// (fractions of the frame's width and height).
    Fixed { x: f64, y: f64 },
    /// Each line somewhere else inside `area` (fractions of the frame),
    /// picked at random but the same every time the project is played.
    Random { area: Placement },
}

impl Default for SubtitlePosition {
    /// At the bottom, like subtitles in a film.
    fn default() -> Self {
        Self::Fixed { x: 0.5, y: 0.86 }
    }
}

impl SubtitlePosition {
    /// The whole frame, for random places.
    pub const ANYWHERE: Self = Self::Random {
        area: Placement {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
    };

    /// Name shown to the user.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Fixed { .. } => "Fixed",
            Self::Random { .. } => "Random",
        }
    }

    fn sanitized(self) -> Self {
        let unit = |value: f64, fallback: f64| {
            if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                fallback
            }
        };
        match self {
            Self::Fixed { x, y } => Self::Fixed {
                x: unit(x, 0.5),
                y: unit(y, 0.86),
            },
            Self::Random { area } => {
                let x = unit(area.x, 0.0);
                let y = unit(area.y, 0.0);
                Self::Random {
                    area: Placement {
                        x,
                        y,
                        width: unit(area.width, 1.0).min(1.0 - x).max(0.01),
                        height: unit(area.height, 1.0).min(1.0 - y).max(0.01),
                    },
                }
            }
        }
    }
}

/// What happens to a line too wide for the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SubtitleFit {
    /// Broken into more rows between words, and made smaller only if that
    /// is not enough.
    #[default]
    Wrap,
    /// Kept in one row and made smaller.
    Shrink,
    /// Left as it is, even if it goes past the edge.
    Overflow,
}

impl SubtitleFit {
    pub const ALL: [Self; 3] = [Self::Wrap, Self::Shrink, Self::Overflow];

    /// Name shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Self::Wrap => "Wrap into rows",
            Self::Shrink => "Shrink",
            Self::Overflow => "Let it overflow",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broken_style_is_made_valid() {
        let style = SubtitleStyle {
            size: f64::NAN,
            outline: 9.0,
            margin: -1.0,
            color: [2.0, -1.0, f64::INFINITY],
            position: SubtitlePosition::Random {
                area: Placement {
                    x: 0.8,
                    y: 0.5,
                    width: 0.9,
                    height: f64::NAN,
                },
            },
            ..SubtitleStyle::default()
        }
        .sanitized();

        assert_eq!(style.size, SubtitleStyle::default().size);
        assert_eq!(style.outline, SubtitleStyle::OUTLINE_MAX);
        assert_eq!(style.margin, 0.0);
        assert_eq!(style.color, [1.0, 0.0, 1.0]);
        let SubtitlePosition::Random { area } = style.position else {
            panic!("still random");
        };
        // The area stays inside the frame.
        assert!((area.x + area.width - 1.0).abs() < 1e-9);
        assert!((area.y + area.height - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_line_takes_the_look_of_its_layer() {
        let style = SubtitleStyle {
            size: 0.1,
            color: [1.0, 0.0, 0.0],
            ..SubtitleStyle::default()
        };
        let text = style.text("Hello");
        assert_eq!(text.text, "Hello");
        assert_eq!(text.size, 0.1);
        assert_eq!(text.color, [1.0, 0.0, 0.0]);
        assert_eq!(SubtitleStyle::like(&text).size, 0.1);
    }
}
