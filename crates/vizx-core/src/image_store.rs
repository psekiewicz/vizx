use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

/// File extensions of the image formats VizX can read, in lower case.
pub const SUPPORTED_IMAGE_EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

/// Longest edge, in pixels, of an image as it is kept for the preview. A
/// larger image is scaled down: the preview never shows more detail than
/// this, and a full-size 4K photo would take 33 MB of memory. Export will
/// read the original file.
const PREVIEW_EDGE_LIMIT: u32 = 1920;

/// An image decoded for drawing.
#[derive(Debug, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// RGBA bytes, top row first, with the color bytes already multiplied
    /// by the opacity ("premultiplied alpha"), like pre-rendered frames.
    pub pixels: Vec<u8>,
}

/// Progress of loading one image file.
#[derive(Debug, Clone)]
pub enum ImageState {
    /// The file is being decoded in the background.
    Loading,
    Ready(Arc<DecodedImage>),
    /// The file could not be decoded; the text says why.
    Failed(String),
}

/// Decodes and remembers the image files a project uses.
///
/// It works like `AudioStore`: each file is decoded once on a background
/// thread and the result comes back through a channel, so drawing a frame
/// never waits for a file.
pub struct ImageStore {
    states: HashMap<String, ImageState>,
    result_sender: Sender<(String, ImageState)>,
    result_receiver: Receiver<(String, ImageState)>,
}

impl Default for ImageStore {
    fn default() -> Self {
        let (result_sender, result_receiver) = mpsc::channel();
        Self {
            states: HashMap::new(),
            result_sender,
            result_receiver,
        }
    }
}

impl ImageStore {
    /// The state of a file, or `None` if it was never requested.
    pub fn state(&self, path: &str) -> Option<&ImageState> {
        self.states.get(path)
    }

    /// Starts loading a file unless that already happened. `on_finished`
    /// runs on the background thread once the result can be collected.
    pub fn request(&mut self, path: &str, on_finished: impl FnOnce() + Send + 'static) {
        if self.states.contains_key(path) {
            return;
        }
        self.states.insert(path.to_owned(), ImageState::Loading);

        let path = path.to_owned();
        let result_sender = self.result_sender.clone();
        thread::spawn(move || {
            let state = match decode_image_file(Path::new(&path), PREVIEW_EDGE_LIMIT) {
                Ok(image) => ImageState::Ready(Arc::new(image)),
                Err(problem) => ImageState::Failed(problem),
            };
            if result_sender.send((path, state)).is_ok() {
                on_finished();
            }
        });
    }

    /// Takes over the results that background threads have finished since
    /// the last call. Never waits. Call it once per frame.
    pub fn collect_finished(&mut self) {
        while let Ok((path, state)) = self.result_receiver.try_recv() {
            self.states.insert(path, state);
        }
    }
}

/// Whether a path has one of the [`SUPPORTED_IMAGE_EXTENSIONS`].
pub fn has_supported_image_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            SUPPORTED_IMAGE_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

/// Reads the size of an image in pixels without decoding its content.
pub fn image_file_size(path: &Path) -> Result<(u32, u32), String> {
    image::image_dimensions(path).map_err(|error| error.to_string())
}

/// Decodes an image file. An image whose longer edge exceeds `edge_limit`
/// pixels is scaled down to it.
pub fn decode_image_file(path: &Path, edge_limit: u32) -> Result<DecodedImage, String> {
    let image = image::open(path).map_err(|error| error.to_string())?;
    let image = if image.width().max(image.height()) > edge_limit {
        // Keeps the shape; the longer edge becomes the limit.
        image.thumbnail(edge_limit, edge_limit)
    } else {
        image
    };

    let rgba = image.into_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    let mut pixels = rgba.into_raw();
    premultiply(&mut pixels);
    Ok(DecodedImage {
        width,
        height,
        pixels,
    })
}

/// Multiplies each pixel's color by its opacity.
pub(crate) fn premultiply(rgba: &mut [u8]) {
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let opacity = u16::from(pixel[3]);
        if opacity < 255 {
            for part in &mut pixel[..3] {
                // Rounded division by 255.
                *part = ((u16::from(*part) * opacity + 127) / 255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn temporary_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("vizx-image-store-{}-{name}", std::process::id()))
    }

    /// Writes a PNG of the given size: left half opaque red, right half
    /// half-transparent white.
    fn write_test_png(path: &Path, width: u32, height: u32) {
        let image = image::RgbaImage::from_fn(width, height, |x, _y| {
            if x < width / 2 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([255, 255, 255, 128])
            }
        });
        image.save(path).unwrap();
    }

    #[test]
    fn decoding_keeps_transparency_and_premultiplies_the_color() {
        let path = temporary_path("alpha.png");
        write_test_png(&path, 8, 4);

        let size = image_file_size(&path);
        let image = decode_image_file(&path, PREVIEW_EDGE_LIMIT);
        let _ = std::fs::remove_file(&path);

        assert_eq!(size, Ok((8, 4)));
        let image = image.unwrap();
        assert_eq!((image.width, image.height), (8, 4));
        assert_eq!(image.pixels[..4], [255, 0, 0, 255]);
        // White at about half opacity: every part is about 128.
        assert_eq!(image.pixels[7 * 4..8 * 4], [128, 128, 128, 128]);
    }

    #[test]
    fn a_large_image_is_scaled_down_keeping_its_shape() {
        let path = temporary_path("large.png");
        write_test_png(&path, 3840, 960);

        let image = decode_image_file(&path, PREVIEW_EDGE_LIMIT);
        let _ = std::fs::remove_file(&path);

        let image = image.unwrap();
        assert_eq!((image.width, image.height), (1920, 480));
    }

    #[test]
    fn requested_file_is_loading_and_then_ready() {
        let path = temporary_path("store.png");
        write_test_png(&path, 4, 4);
        let path_text = path.to_string_lossy().into_owned();
        let (done_sender, done_receiver) = mpsc::channel();
        let mut store = ImageStore::default();

        assert!(store.state(&path_text).is_none());
        store.request(&path_text, move || {
            let _ = done_sender.send(());
        });
        assert!(matches!(
            store.state(&path_text),
            Some(ImageState::Loading | ImageState::Ready(_))
        ));

        done_receiver.recv_timeout(Duration::from_secs(30)).unwrap();
        store.collect_finished();
        let _ = std::fs::remove_file(&path);
        assert!(matches!(
            store.state(&path_text),
            Some(ImageState::Ready(_))
        ));
    }

    #[test]
    fn unreadable_file_ends_as_failed_and_extensions_are_checked() {
        let (done_sender, done_receiver) = mpsc::channel();
        let mut store = ImageStore::default();
        store.request("vizx-image-that-does-not-exist.png", move || {
            let _ = done_sender.send(());
        });
        done_receiver.recv_timeout(Duration::from_secs(30)).unwrap();
        store.collect_finished();

        assert!(matches!(
            store.state("vizx-image-that-does-not-exist.png"),
            Some(ImageState::Failed(_))
        ));
        assert!(has_supported_image_extension(Path::new("Cover.PNG")));
        assert!(has_supported_image_extension(Path::new("photo.jpeg")));
        assert!(!has_supported_image_extension(Path::new("song.mp3")));
    }
}
