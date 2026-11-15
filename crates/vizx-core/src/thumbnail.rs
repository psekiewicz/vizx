//! Small pictures of visual media, shown on their clips in the timeline.

use std::path::Path;

use vizx_project::MediaSource;

use crate::DecodedImage;

/// Longest edge of a thumbnail, in pixels.
pub const THUMBNAIL_EDGE: u32 = 128;

/// A small picture of what `source` shows: an image, the first frame of a
/// frame animation or a video (which needs `ffmpeg`), or a text. Fails for
/// media without a fixed picture (audio, visualizers, MilkDrop presets).
pub fn make_thumbnail(source: &MediaSource, ffmpeg: Option<&Path>) -> Result<DecodedImage, String> {
    if source.image().is_some() {
        return crate::image_store::decode_image_file(Path::new(&source.path), THUMBNAIL_EDGE);
    }
    if let Some(animation) = source.animation() {
        let mut reader = crate::AnimationReader::new(&source.path, animation, THUMBNAIL_EDGE, 0);
        let frame = reader.frame(0)?;
        return Ok(DecodedImage {
            width: frame.width,
            height: frame.height,
            pixels: frame.pixels.clone(),
        });
    }
    if let Some(text) = source.text() {
        // Drawn as for a picture 360 pixels high, then made small.
        return crate::render_text(text, 360).map(shrunk);
    }
    if source.video().is_some() {
        let ffmpeg = ffmpeg.ok_or("FFmpeg is needed for a video's picture")?;
        let file = Path::new(&source.path);
        let (width, height) = crate::probe_video(ffmpeg, file)?;
        let (width, height) = fit(width, height);
        let mut reader = crate::VideoReader::open(ffmpeg, file, (width, height), 1, 0.0)?;
        let mut pixels = Vec::new();
        reader.read_frame(&mut pixels)?;
        return Ok(DecodedImage {
            width,
            height,
            pixels,
        });
    }
    Err("this media has no fixed picture".to_owned())
}

/// A size of at most [`THUMBNAIL_EDGE`] with the same shape, in even
/// numbers (which video scaling needs).
fn fit(width: u32, height: u32) -> (u32, u32) {
    let scale = f64::from(THUMBNAIL_EDGE) / f64::from(width.max(height).max(1));
    let even = |value: u32| ((f64::from(value) * scale.min(1.0)).round() as u32 / 2 * 2).max(2);
    (even(width), even(height))
}

/// `image` made small enough for a thumbnail.
fn shrunk(image: DecodedImage) -> DecodedImage {
    if image.width.max(image.height) <= THUMBNAIL_EDGE {
        return image;
    }
    let Some(buffer) = image::RgbaImage::from_raw(image.width, image.height, image.pixels.clone())
    else {
        return image;
    };
    let small = image::DynamicImage::ImageRgba8(buffer)
        .thumbnail(THUMBNAIL_EDGE, THUMBNAIL_EDGE)
        .into_rgba8();
    DecodedImage {
        width: small.width(),
        height: small.height(),
        pixels: small.into_raw(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnails_keep_the_shape_and_stay_small() {
        assert_eq!(fit(1920, 1080), (128, 72));
        assert_eq!(fit(100, 50), (100, 50));
        assert_eq!(fit(1, 4000), (2, 128));

        let mut project = vizx_project::Project::new();
        let (media_id, _) = project.add_text(
            vizx_project::TextProperties::default(),
            vizx_project::Placement::FULL,
        );
        let text = project.media_source(media_id).unwrap();
        let picture = make_thumbnail(text, None).unwrap();
        assert!(picture.width <= THUMBNAIL_EDGE && picture.height <= THUMBNAIL_EDGE);
        assert!(picture.pixels.iter().any(|value| *value > 0));

        let (preset, _) = project.add_visualizer_plugin("bundled-visualizers:/core");
        assert!(make_thumbnail(project.media_source(preset).unwrap(), None).is_err());
    }
}
