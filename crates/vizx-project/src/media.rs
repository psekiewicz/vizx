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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaSource {
    pub id: MediaId,
    /// Location of the file, as text so it is stored identically on every
    /// operating system.
    ///
    /// For now this is the path the file was imported from. Storing it
    /// relative to the project file, and re-linking moved files, is planned.
    pub path: String,
    pub content: MediaContent,
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
            | MediaContent::Video(_) => None,
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
        self.is_visualizer() || self.image().is_some() || self.video().is_some()
    }
}

/// What kind of media a source is, with the properties of that kind.
/// Images and animations will become further variants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
