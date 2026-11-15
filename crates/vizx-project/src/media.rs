use serde::{Deserialize, Serialize};

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
    /// operating system.
    ///
    /// For now this is the path the file was imported from. Storing it
    /// relative to the project file, and re-linking moved files, is planned.
    pub path: String,
    pub content: MediaContent,
    /// For an audio file: beats placed by hand, in seconds from the start
    /// of the file and in order. They replace the beats VizX finds by
    /// itself; `None` uses those.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_beats: Option<Vec<f64>>,
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
            | MediaContent::Text(_) => None,
        }
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

    /// The name to show for this source: its file name, or for a text its
    /// first line.
    pub fn display_name(&self) -> &str {
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
        self.is_visualizer() || self.is_still() || self.video().is_some()
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
    /// A piece of text (a title, a caption). It is not a file: `path` is
    /// empty and everything about it is stored here.
    Text(TextProperties),
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
        self.color = self.color.map(|part| {
            if part.is_finite() {
                part.clamp(0.0, 1.0)
            } else {
                1.0
            }
        });
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
