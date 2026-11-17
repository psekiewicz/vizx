use serde::{Deserialize, Serialize};

use crate::FrameAnimation;

/// Identifies a media source within one project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MediaId(pub(crate) u64);

/// A file the project uses, such as the song.
///
/// The project stores a *reference* to the file and the facts needed to lay
/// it out on the timeline. The file's content is never copied into the
/// project, so `.vzx` files stay small.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaSource {
    pub id: MediaId,
    /// Location of the file, as text so it is stored identically on every
    /// operating system: the full path it was imported from.
    pub path: String,
    /// The same location relative to the folder of the project file, with
    /// `/` between folders. Written when the project is saved; when the full
    /// path no longer exists on opening (the project was moved together with
    /// its files, or opened on another computer), this one is tried.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative_path: Option<String>,
    pub content: MediaContent,
    /// For an audio file: beats placed by hand, in seconds from the start
    /// of the file and in order. They replace the beats VizX finds by
    /// itself; `None` uses those.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_beats: Option<Vec<f64>>,
    /// For an audio file: the tempo the user says it has, in beats per
    /// minute. The beats are then found at that tempo; `None` uses the
    /// tempo found automatically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tempo_bpm: Option<f64>,
}

impl MediaSource {
    /// The file name without its folders, for display.
    pub fn file_name(&self) -> &str {
        self.path
            .rsplit(['/', '\\'])
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.path)
    }

    /// The audio properties, if this is an audio file.
    pub fn audio(&self) -> Option<&AudioProperties> {
        match &self.content {
            MediaContent::Audio(audio) => Some(audio),
            MediaContent::MilkdropPreset
            | MediaContent::VisualizerPlugin
            | MediaContent::Image(_)
            | MediaContent::Video(_)
            | MediaContent::Animation(_)
            | MediaContent::Text(_)
            | MediaContent::EffectsLayer => None,
        }
    }

    /// Whether this is an effects layer, which changes the layers below it.
    pub fn is_effects_layer(&self) -> bool {
        matches!(self.content, MediaContent::EffectsLayer)
    }

    /// The text and its look, if this is a text element.
    pub fn text(&self) -> Option<&TextProperties> {
        match &self.content {
            MediaContent::Text(text) => Some(text),
            _ => None,
        }
    }

    /// Whether this source is one unchanging picture (an image or a text),
    /// as opposed to something drawn frame by frame.
    pub fn is_still(&self) -> bool {
        self.image().is_some() || self.text().is_some()
    }

    /// Whether pictures from this source keep sharp pixels when enlarged,
    /// unless the clip says otherwise: PNG images and animations (often
    /// pixel art) do, everything else is smoothed.
    pub fn sharp_pixels_by_default(&self) -> bool {
        let is_png = self.path.to_ascii_lowercase().ends_with(".png");
        is_png && (self.image().is_some() || self.animation().is_some())
    }

    /// The name to show for this source: its file name, or for a text its
    /// first line.
    pub fn display_name(&self) -> &str {
        if self.is_effects_layer() {
            return "Effects Layer";
        }
        match self.text() {
            Some(text) => text
                .text
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or("Text"),
            None => self.file_name(),
        }
    }

    /// The frame animation, if this is an image sequence or a sprite sheet.
    pub fn animation(&self) -> Option<&FrameAnimation> {
        match &self.content {
            MediaContent::Animation(animation) => Some(animation),
            _ => None,
        }
    }

    /// The picture properties, if this is a video file.
    pub fn video(&self) -> Option<&ImageProperties> {
        match &self.content {
            MediaContent::Video(video) => Some(video),
            _ => None,
        }
    }

    /// The image properties, if this is an image file.
    pub fn image(&self) -> Option<&ImageProperties> {
        match &self.content {
            MediaContent::Image(image) => Some(image),
            _ => None,
        }
    }

    /// Whether this source is a visualizer: a MilkDrop preset or a plugin.
    pub fn is_visualizer(&self) -> bool {
        self.is_milkdrop_preset() || self.is_visualizer_plugin()
    }

    pub fn is_milkdrop_preset(&self) -> bool {
        matches!(self.content, MediaContent::MilkdropPreset)
    }

    pub fn is_visualizer_plugin(&self) -> bool {
        matches!(self.content, MediaContent::VisualizerPlugin)
    }

    /// Whether this source produces a picture (as opposed to sound).
    pub fn is_visual(&self) -> bool {
        self.is_visualizer()
            || self.is_still()
            || self.video().is_some()
            || self.animation().is_some()
            || self.is_effects_layer()
    }
}

/// What kind of media a source is, with the properties of that kind.
/// Images and animations will become further variants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MediaContent {
    Audio(AudioProperties),
    /// A MilkDrop visualizer preset (a `.milk` file). It has no length of
    /// its own: it produces a picture for as long as its clip lasts.
    MilkdropPreset,
    /// A visualizer plugin: a folder with a `visualizer.json` and a
    /// `shader.wgsl`. `path` is that folder. Like a preset it has no length
    /// of its own.
    VisualizerPlugin,
    /// A still image (PNG or JPEG). It is shown for as long as its clip
    /// lasts.
    Image(ImageProperties),
    /// A video file, used for its picture only. It is played from its
    /// start when its clip starts and begins again when it ends. The
    /// properties are the size of its picture.
    Video(ImageProperties),
    /// A frame animation: an image sequence (`path` is its first file) or
    /// a sprite sheet (`path` is the sheet). It plays in a loop.
    Animation(FrameAnimation),
    /// A piece of text (a title, a caption). It is not a file: `path` is
    /// empty and everything about it is stored here.
    Text(TextProperties),
    /// An effects layer: it has no picture of its own, but moves, turns
    /// and changes everything on the layers below it, like a camera (shake
    /// and zoom on the beat, a glitch for a moment). `path` is empty.
    EffectsLayer,
}

/// A text element: what it says and how it looks.
///
/// `#[serde(default)]` lets files written before a field existed load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextProperties {
    /// The text. It may have several lines.
    pub text: String,
    /// The font file (TrueType or OpenType) to draw it with. Empty means
    /// the font built into VizX.
    pub font_path: String,
    /// Height of the letters as a fraction of the frame's height (0.1 means
    /// a tenth of the picture).
    pub size: f64,
    /// Red, green and blue, each from 0 to 1.
    pub color: [f64; 3],
    /// How lines of different length are lined up.
    pub alignment: TextAlignment,
    /// Thickness of an outline around the letters, as a fraction of their
    /// size; 0 draws none.
    pub outline: f64,
    pub outline_color: [f64; 3],
}

impl TextProperties {
    /// Smallest and largest allowed `size`.
    pub const SIZE_RANGE: (f64, f64) = (0.01, 1.0);

    /// The nearest valid properties.
    pub fn sanitized(mut self) -> Self {
        let (smallest, largest) = Self::SIZE_RANGE;
        self.size = if self.size.is_finite() {
            self.size.clamp(smallest, largest)
        } else {
            Self::default().size
        };
        let unit = |part: f64| {
            if part.is_finite() {
                part.clamp(0.0, 1.0)
            } else {
                1.0
            }
        };
        self.color = self.color.map(unit);
        self.outline_color = self.outline_color.map(unit);
        self.outline = if self.outline.is_finite() {
            self.outline.clamp(0.0, crate::SubtitleStyle::OUTLINE_MAX)
        } else {
            0.0
        };
        self
    }
}

impl Default for TextProperties {
    fn default() -> Self {
        Self {
            text: "Text".to_owned(),
            font_path: String::new(),
            size: 0.12,
            color: [1.0, 1.0, 1.0],
            alignment: TextAlignment::Center,
            outline: 0.0,
            outline_color: [0.0, 0.0, 0.0],
        }
    }
}

/// How the lines of a text are lined up with each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TextAlignment {
    Left,
    #[default]
    Center,
    Right,
}

impl TextAlignment {
    pub const ALL: [Self; 3] = [Self::Left, Self::Center, Self::Right];

    /// Name shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Center => "Center",
            Self::Right => "Right",
        }
    }
}

/// Size of the picture of an image or video file, read once when it is
/// imported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageProperties {
    /// Size in pixels.
    pub width: u32,
    pub height: u32,
}

/// Properties of an audio file, read once when it is imported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioProperties {
    pub duration_milliseconds: u64,
    /// Samples per second, e.g. 44100.
    pub sample_rate: u32,
    /// 1 for mono, 2 for stereo.
    pub channel_count: u16,
}
