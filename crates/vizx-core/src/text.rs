//! Turning text into a picture.
//!
//! A text element is drawn once into an image (white-on-transparent in the
//! chosen color) and from then on handled like any other picture: placed,
//! faded, animated and composed by the same code. The preview and the
//! export both call [`render_text`], so a title looks the same in both.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use ab_glyph::{Font, FontVec, Glyph, PxScale, ScaleFont, point};
use vizx_project::{TextAlignment, TextProperties};

use crate::DecodedImage;

/// Smallest and largest letter height the text is drawn at, in pixels.
const FONT_PIXEL_RANGE: (f32, f32) = (4.0, 600.0);
/// Largest width or height of the picture of a text, in pixels. Longer
/// lines are cut off rather than using unbounded memory.
const IMAGE_EDGE_LIMIT: usize = 8192;

/// Where every letter of a text goes, and how large its picture is.
struct Layout {
    /// The letters of each line (`x` relative to the line's start) and the
    /// line's width.
    lines: Vec<(Vec<Glyph>, f32)>,
    text_width: f32,
    margin: f32,
    /// Thickness of the outline in pixels.
    outline: f32,
    width: usize,
    height: usize,
}

fn layout(font: &FontVec, properties: &TextProperties, frame_height: u32) -> Layout {
    let (smallest, largest) = FONT_PIXEL_RANGE;
    let font_pixels = ((properties.size * f64::from(frame_height)) as f32).clamp(smallest, largest);
    let scaled = font.as_scaled(PxScale::from(font_pixels));

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

    let outline = (properties.outline.max(0.0) as f32 * font_pixels).round();
    let margin = (scaled.height() * 0.15).ceil() + outline;
    let text_width = lines.iter().map(|(_, width)| *width).fold(0.0, f32::max);
    let text_height = lines.len().max(1) as f32 * line_height;
    Layout {
        width: ((text_width + 2.0 * margin).ceil() as usize).clamp(2, IMAGE_EDGE_LIMIT),
        height: ((text_height + 2.0 * margin).ceil() as usize).clamp(2, IMAGE_EDGE_LIMIT),
        lines,
        text_width,
        margin,
        outline,
    }
}

/// Draws `properties` as a picture, for a frame that is `frame_height`
/// pixels high (the text size is a fraction of that height).
///
/// The picture is just large enough for the text (and its outline) plus a
/// small margin, with a transparent background.
pub fn render_text(properties: &TextProperties, frame_height: u32) -> Result<DecodedImage, String> {
    let font = load_font(&properties.font_path)?;
    let Layout {
        lines,
        text_width,
        margin,
        outline,
        width,
        height,
    } = layout(&font, properties, frame_height);

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
    let ring = (outline >= 1.0).then(|| widened(&coverage, width, height, outline as usize));

    // The color, premultiplied by the coverage as its opacity, over the
    // outline's color.
    let color = properties.color.map(|part| part.clamp(0.0, 1.0) as f32);
    let ring_color = properties
        .outline_color
        .map(|part| part.clamp(0.0, 1.0) as f32);
    let pixels = coverage
        .iter()
        .enumerate()
        .flat_map(|(index, amount)| {
            let behind = ring.as_ref().map_or(0.0, |ring| ring[index]) * (1.0 - amount);
            let byte = |value: f32| (value * 255.0).round().clamp(0.0, 255.0) as u8;
            [
                byte(color[0] * amount + ring_color[0] * behind),
                byte(color[1] * amount + ring_color[1] * behind),
                byte(color[2] * amount + ring_color[2] * behind),
                byte(amount + behind),
            ]
        })
        .collect();
    Ok(DecodedImage {
        width: width as u32,
        height: height as u32,
        pixels,
    })
}

/// `coverage` grown by `radius` pixels in every direction (a round brush):
/// the shape of the outline drawn behind the letters.
fn widened(coverage: &[f32], width: usize, height: usize, radius: usize) -> Vec<f32> {
    let mut result = vec![0.0f32; coverage.len()];
    // `spread` holds, for every pixel, the largest coverage within
    // `reach` pixels to its left and right; it grows by one each round.
    let mut spread = coverage.to_vec();
    let mut next = vec![0.0f32; coverage.len()];
    let radius_squared = (radius * radius) as f64;
    for reach in 0..=radius {
        if reach > 0 {
            for row in 0..height {
                let line = &spread[row * width..(row + 1) * width];
                let out = &mut next[row * width..(row + 1) * width];
                for x in 0..width {
                    let left = line[x.saturating_sub(1)];
                    let right = line[(x + 1).min(width - 1)];
                    out[x] = line[x].max(left).max(right);
                }
            }
            std::mem::swap(&mut spread, &mut next);
        }
        // The rows of the round brush exactly `reach` wide.
        for offset in -(radius as i64)..=radius as i64 {
            let half_width = (radius_squared - (offset * offset) as f64).max(0.0).sqrt() as usize;
            if half_width != reach {
                continue;
            }
            for row in 0..height {
                let source = row as i64 + offset;
                if !(0..height as i64).contains(&source) {
                    continue;
                }
                let from = &spread[source as usize * width..(source as usize + 1) * width];
                let to = &mut result[row * width..(row + 1) * width];
                for (target, value) in to.iter_mut().zip(from) {
                    *target = target.max(*value);
                }
            }
        }
    }
    result
}

/// The size in pixels of the picture [`render_text`] would draw, without
/// drawing it.
pub fn measure_text(properties: &TextProperties, frame_height: u32) -> Result<(u32, u32), String> {
    let font = load_font(&properties.font_path)?;
    let layout = layout(&font, properties, frame_height);
    Ok((layout.width as u32, layout.height as u32))
}

/// The text broken into rows between words so that its picture is at most
/// `max_width` pixels wide where possible (a single word longer than that
/// stays whole). Breaks already in the text are kept.
pub fn wrap_text(
    properties: &TextProperties,
    frame_height: u32,
    max_width: u32,
) -> Result<String, String> {
    let font = load_font(&properties.font_path)?;
    let fits = |text: &str| {
        let candidate = TextProperties {
            text: text.to_owned(),
            ..properties.clone()
        };
        layout(&font, &candidate, frame_height).width as u32 <= max_width
    };
    let mut rows = Vec::new();
    for given in properties.text.lines() {
        let mut row = String::new();
        for word in given.split_whitespace() {
            let longer = if row.is_empty() {
                word.to_owned()
            } else {
                format!("{row} {word}")
            };
            if row.is_empty() || fits(&longer) {
                row = longer;
            } else {
                rows.push(std::mem::replace(&mut row, word.to_owned()));
            }
        }
        rows.push(row);
    }
    Ok(balanced(rows, &fits).join("\n"))
}

/// Rows of more even length, when the last row came out much shorter: a
/// word moves down as long as everything still fits ("I wanted / more"
/// reads worse than "I / wanted more" only when the second is not much
/// longer).
fn balanced(mut rows: Vec<String>, fits: &dyn Fn(&str) -> bool) -> Vec<String> {
    if rows.len() < 2 {
        return rows;
    }
    for index in (1..rows.len()).rev() {
        loop {
            let (upper, lower) = (&rows[index - 1], &rows[index]);
            let Some((rest, moved)) = upper.rsplit_once(' ') else {
                break;
            };
            let new_lower = format!("{moved} {lower}");
            if new_lower.len() > rest.len() || !fits(&new_lower) {
                break;
            }
            let rest = rest.to_owned();
            rows[index] = new_lower;
            rows[index - 1] = rest;
        }
    }
    rows
}

/// Loads the font file at `path`, or the built-in font if `path` is empty.
/// Fonts are kept once loaded: subtitles measure many lines in one font.
fn load_font(path: &str) -> Result<Arc<FontVec>, String> {
    static LOADED: OnceLock<Mutex<HashMap<String, Arc<FontVec>>>> = OnceLock::new();
    let loaded = LOADED.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(font) = loaded
        .lock()
        .ok()
        .and_then(|fonts| fonts.get(path).cloned())
    {
        return Ok(font);
    }
    let bytes = if path.is_empty() {
        // The same font the VizX interface uses; it is part of the program.
        epaint_default_fonts::UBUNTU_LIGHT.to_vec()
    } else {
        std::fs::read(Path::new(path))
            .map_err(|error| format!("could not read the font {path}: {error}"))?
    };
    let font =
        Arc::new(FontVec::try_from_vec(bytes).map_err(|_| {
            format!("{path} is not a font file VizX can use (TrueType or OpenType)")
        })?);
    if let Ok(mut fonts) = loaded.lock() {
        // A handful of fonts at most; a file changed on disk is read again
        // after a restart.
        if fonts.len() > 32 {
            fonts.clear();
        }
        fonts.insert(path.to_owned(), Arc::clone(&font));
    }
    Ok(font)
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

    #[test]
    fn an_outline_surrounds_the_letters_in_its_color() {
        let plain = render_text(&text("O"), 1080).unwrap();
        let outlined = render_text(
            &TextProperties {
                outline: 0.1,
                outline_color: [0.0, 0.0, 1.0],
                ..text("O")
            },
            1080,
        )
        .unwrap();
        // Room for the outline on every side.
        assert!(outlined.width > plain.width && outlined.height > plain.height);
        // Some pixels are fully the outline's color, and the letter itself
        // stays white.
        let pixels: Vec<&[u8]> = outlined.pixels.chunks(4).collect();
        assert!(pixels.contains(&&[0, 0, 255, 255][..]));
        assert!(pixels.contains(&&[255, 255, 255, 255][..]));
        // Measuring gives the same size without drawing.
        assert_eq!(
            measure_text(
                &TextProperties {
                    outline: 0.1,
                    ..text("O")
                },
                1080
            )
            .unwrap(),
            (outlined.width, outlined.height)
        );
    }

    #[test]
    fn long_text_is_wrapped_between_words_to_fit() {
        let long = text("Take a long look at all the people all around you");
        let (one_row, _) = measure_text(&long, 1080).unwrap();
        let wrapped = wrap_text(&long, 1080, one_row / 2).unwrap();
        assert!(wrapped.lines().count() >= 2, "{wrapped}");
        for row in wrapped.lines() {
            let (width, _) = measure_text(&text(row), 1080).unwrap();
            assert!(width <= one_row / 2, "{row}");
        }
        // Nothing is lost or changed but the breaks.
        assert_eq!(
            wrapped.split_whitespace().collect::<Vec<_>>(),
            long.text.split_whitespace().collect::<Vec<_>>()
        );
        // Text that fits stays as it is.
        assert_eq!(wrap_text(&long, 1080, one_row).unwrap(), long.text);
    }
}
