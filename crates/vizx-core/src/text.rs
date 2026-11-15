//! Turning text into a picture.
//!
//! A text element is drawn once into an image (white-on-transparent in the
//! chosen color) and from then on handled like any other picture: placed,
//! faded, animated and composed by the same code. The preview and the
//! export both call [`render_text`], so a title looks the same in both.

use std::path::Path;

use ab_glyph::{Font, FontVec, Glyph, PxScale, ScaleFont, point};
use vizx_project::{TextAlignment, TextProperties};

use crate::DecodedImage;

/// Smallest and largest letter height the text is drawn at, in pixels.
const FONT_PIXEL_RANGE: (f32, f32) = (4.0, 600.0);
/// Largest width or height of the picture of a text, in pixels. Longer
/// lines are cut off rather than using unbounded memory.
const IMAGE_EDGE_LIMIT: usize = 8192;

/// Draws `properties` as a picture, for a frame that is `frame_height`
/// pixels high (the text size is a fraction of that height).
///
/// The picture is just large enough for the text plus a small margin, with
/// a transparent background.
pub fn render_text(properties: &TextProperties, frame_height: u32) -> Result<DecodedImage, String> {
    let font = load_font(&properties.font_path)?;
    let (smallest, largest) = FONT_PIXEL_RANGE;
    let font_pixels = (properties.size * f64::from(frame_height)) as f32;
    let scaled = font.as_scaled(PxScale::from(font_pixels.clamp(smallest, largest)));

    // Where every letter goes, line by line. `x` positions are relative to
    // the start of the line for now.
    let line_height = scaled.ascent() - scaled.descent() + scaled.line_gap();
    let lines: Vec<(Vec<Glyph>, f32)> = properties
        .text
        .lines()
        .enumerate()
        .map(|(line_number, line)| {
            let baseline = scaled.ascent() + line_number as f32 * line_height;
            let mut glyphs = Vec::new();
            let mut x = 0.0;
            let mut previous = None;
            for character in line.chars() {
                let mut glyph = scaled.scaled_glyph(character);
                if let Some(previous) = previous {
                    // Kerning: some pairs of letters sit closer together.
                    x += scaled.kern(previous, glyph.id);
                }
                glyph.position = point(x, baseline);
                x += scaled.h_advance(glyph.id);
                previous = Some(glyph.id);
                glyphs.push(glyph);
            }
            (glyphs, x)
        })
        .collect();

    let margin = (scaled.height() * 0.15).ceil();
    let text_width = lines.iter().map(|(_, width)| *width).fold(0.0, f32::max);
    let text_height = lines.len().max(1) as f32 * line_height;
    let width = ((text_width + 2.0 * margin).ceil() as usize).clamp(2, IMAGE_EDGE_LIMIT);
    let height = ((text_height + 2.0 * margin).ceil() as usize).clamp(2, IMAGE_EDGE_LIMIT);

    // How much of each pixel is covered by a letter, 0 to 1.
    let mut coverage = vec![0.0f32; width * height];
    for (glyphs, line_width) in lines {
        let line_start = margin
            + match properties.alignment {
                TextAlignment::Left => 0.0,
                TextAlignment::Center => (text_width - line_width) / 2.0,
                TextAlignment::Right => text_width - line_width,
            };
        for mut glyph in glyphs {
            glyph.position.x += line_start;
            glyph.position.y += margin;
            // Characters without a shape (spaces) have no outline.
            let Some(outline) = font.outline_glyph(glyph) else {
                continue;
            };
            let bounds = outline.px_bounds();
            outline.draw(|x, y, amount| {
                let pixel_x = bounds.min.x as i64 + i64::from(x);
                let pixel_y = bounds.min.y as i64 + i64::from(y);
                if (0..width as i64).contains(&pixel_x) && (0..height as i64).contains(&pixel_y) {
                    let pixel = &mut coverage[pixel_y as usize * width + pixel_x as usize];
                    // Overlapping letters do not add up beyond full cover.
                    *pixel = (*pixel + amount).min(1.0);
                }
            });
        }
    }

    // The color, premultiplied by the coverage as its opacity.
    let color = properties.color.map(|part| part.clamp(0.0, 1.0) as f32);
    let pixels = coverage
        .iter()
        .flat_map(|amount| {
            let byte = |value: f32| (value * amount * 255.0).round() as u8;
            [byte(color[0]), byte(color[1]), byte(color[2]), byte(1.0)]
        })
        .collect();
    Ok(DecodedImage {
        width: width as u32,
        height: height as u32,
        pixels,
    })
}

/// Loads the font file at `path`, or the built-in font if `path` is empty.
fn load_font(path: &str) -> Result<FontVec, String> {
    let bytes = if path.is_empty() {
        // The same font the VizX interface uses; it is part of the program.
        epaint_default_fonts::UBUNTU_LIGHT.to_vec()
    } else {
        std::fs::read(Path::new(path))
            .map_err(|error| format!("could not read the font {path}: {error}"))?
    };
    FontVec::try_from_vec(bytes)
        .map_err(|_| format!("{path} is not a font file VizX can use (TrueType or OpenType)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(content: &str) -> TextProperties {
        TextProperties {
            text: content.to_owned(),
            ..TextProperties::default()
        }
    }

    /// Opacity of every pixel, row by row.
    fn opacity(image: &DecodedImage) -> Vec<u8> {
        image.pixels.chunks(4).map(|pixel| pixel[3]).collect()
    }

    /// The first and last column that contain any ink in a range of rows.
    fn ink_columns(image: &DecodedImage, rows: std::ops::Range<u32>) -> (u32, u32) {
        let has_ink = |column: u32| {
            rows.clone()
                .any(|row| image.pixels[((row * image.width + column) * 4 + 3) as usize] > 0)
        };
        let columns: Vec<u32> = (0..image.width).filter(|column| has_ink(*column)).collect();
        (*columns.first().unwrap(), *columns.last().unwrap())
    }

    #[test]
    fn text_becomes_a_picture_with_a_transparent_background() {
        let image = render_text(&text("VizX"), 1080).unwrap();

        // 12 % of 1080 pixels is about 130 pixels of letter height.
        assert!((120..260).contains(&image.height), "{}", image.height);
        assert!(
            image.width > image.height,
            "{} x {}",
            image.width,
            image.height
        );
        let opacity = opacity(&image);
        assert!(opacity.contains(&255), "no solid ink");
        assert_eq!(opacity[0], 0, "the corner must be transparent");
        // White text: a solid pixel is white.
        let solid = image
            .pixels
            .chunks(4)
            .find(|pixel| pixel[3] == 255)
            .unwrap();
        assert_eq!(solid, [255, 255, 255, 255]);
    }

    #[test]
    fn size_color_and_lines_change_the_picture() {
        let normal = render_text(&text("Hello"), 1080).unwrap();

        let small = render_text(
            &TextProperties {
                size: 0.06,
                ..text("Hello")
            },
            1080,
        )
        .unwrap();
        assert!(small.height < normal.height * 6 / 10);

        // The same text for a frame half as high is half as large.
        let half_frame = render_text(&text("Hello"), 540).unwrap();
        assert!(half_frame.width.abs_diff(normal.width / 2) <= 4);

        let two_lines = render_text(&text("Hello\nHello"), 1080).unwrap();
        assert!(two_lines.height > normal.height * 17 / 10);
        assert_eq!(two_lines.width, normal.width);

        let red = render_text(
            &TextProperties {
                color: [1.0, 0.0, 0.0],
                ..text("Hello")
            },
            1080,
        )
        .unwrap();
        let solid = red.pixels.chunks(4).find(|pixel| pixel[3] == 255).unwrap();
        assert_eq!(solid, [255, 0, 0, 255]);
    }

    #[test]
    fn a_short_line_is_aligned_within_the_longest_one() {
        let aligned = |alignment| {
            let image = render_text(
                &TextProperties {
                    alignment,
                    ..text("A long first line\nxx")
                },
                1080,
            )
            .unwrap();
            // The second line occupies the lower half of the picture.
            let (first, last) = ink_columns(&image, image.height / 2..image.height);
            (first, last, image.width)
        };

        let (left_start, left_end, width) = aligned(TextAlignment::Left);
        let (center_start, center_end, _) = aligned(TextAlignment::Center);
        let (right_start, right_end, _) = aligned(TextAlignment::Right);

        assert!(
            left_end < width / 2,
            "left: {left_start}..{left_end} of {width}"
        );
        assert!(right_start > width / 2, "right: {right_start}..{right_end}");
        let center = (center_start + center_end) / 2;
        assert!(
            center.abs_diff(width / 2) < width / 20,
            "center at {center} of {width}"
        );
    }

    #[test]
    fn empty_text_and_a_missing_font_are_handled() {
        let empty = render_text(&text(""), 1080).unwrap();
        assert!(empty.width >= 2 && empty.height >= 2);
        assert!(opacity(&empty).iter().all(|value| *value == 0));

        let error = render_text(
            &TextProperties {
                font_path: "vizx-font-that-does-not-exist.ttf".to_owned(),
                ..text("Hello")
            },
            1080,
        )
        .unwrap_err();
        assert!(
            error.contains("vizx-font-that-does-not-exist.ttf"),
            "{error}"
        );
    }
}
