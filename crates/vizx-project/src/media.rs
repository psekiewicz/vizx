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
            MediaContent::MilkdropPreset => None,
        }
    }

    pub fn is_milkdrop_preset(&self) -> bool {
        matches!(self.content, MediaContent::MilkdropPreset)
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
