//! Reading the frames of frame animations (image sequences and sprite
//! sheets), for the preview and the export alike.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use image::{GenericImageView, imageops};
use vizx_project::{FrameAnimation, FrameLayout, sequence_frame_path, split_numbered_path};

use crate::DecodedImage;
use crate::image_store::premultiply;

/// Reads the frames of one frame animation and keeps some of them.
///
/// Frames are decoded when they are first needed. They are kept while they
/// fit into the memory budget, so a short animation is decoded only once
/// however often it loops; a long one keeps its first frames and decodes
/// the rest each time.
pub struct AnimationReader {
    path: String,
    animation: FrameAnimation,
    /// Longest edge of a frame as kept; larger frames are scaled down.
    edge_limit: u32,
    /// The decoded sprite sheet, cut into frames on demand.
    sheet: Option<image::RgbaImage>,
    frames: HashMap<u32, Arc<DecodedImage>>,
    bytes_kept: usize,
    byte_budget: usize,
}

impl AnimationReader {
    /// Prepares to read `animation`, stored at `path`. Frames are scaled
    /// down to at most `edge_limit` pixels on their longer edge, and at
    /// most `byte_budget` bytes of frames are kept.
    pub fn new(
        path: &str,
        animation: &FrameAnimation,
        edge_limit: u32,
        byte_budget: usize,
    ) -> Self {
        Self {
            path: path.to_owned(),
            animation: animation.clone(),
            edge_limit,
            sheet: None,
            frames: HashMap::new(),
            bytes_kept: 0,
            byte_budget,
        }
    }

    /// Whether this reader reads `animation` at `path` (it has to be made
    /// anew when either changes).
    pub fn reads(&self, path: &str, animation: &FrameAnimation) -> bool {
        self.path == path && self.animation == *animation
    }

    /// Frame `index` (counted from 0), premultiplied like every picture.
    pub fn frame(&mut self, index: u32) -> Result<Arc<DecodedImage>, String> {
        let index = index.min(self.animation.frame_count.saturating_sub(1));
        if let Some(frame) = self.frames.get(&index) {
            return Ok(Arc::clone(frame));
        }

        let picture = match &self.animation.layout {
            FrameLayout::Sequence {
                first_number,
                digits,
            } => {
                let file = sequence_frame_path(&self.path, *first_number, *digits, index);
                image::open(&file)
                    .map_err(|error| format!("could not read {file}: {error}"))?
                    .into_rgba8()
            }
            FrameLayout::SpriteSheet { .. } => {
                if self.sheet.is_none() {
                    let sheet = image::open(&self.path)
                        .map_err(|error| format!("could not read {}: {error}", self.path))?;
                    self.sheet = Some(sheet.into_rgba8());
                }
                let sheet = self.sheet.as_ref().ok_or("the sprite sheet is missing")?;
                let (width, height) = (self.animation.frame_width, self.animation.frame_height);
                let (column, row) = self.animation.sheet_cell(index).unwrap_or((0, 0));
                imageops::crop_imm(sheet, column * width, row * height, width, height).to_image()
            }
        };

        let picture = if picture.width().max(picture.height()) > self.edge_limit {
            image::DynamicImage::ImageRgba8(picture)
                .thumbnail(self.edge_limit, self.edge_limit)
                .into_rgba8()
        } else {
            picture
        };
        let (width, height) = picture.dimensions();
        let mut pixels = picture.into_raw();
        premultiply(&mut pixels);
        let frame = Arc::new(DecodedImage {
            width,
            height,
            pixels,
        });

        if self.bytes_kept + frame.pixels.len() <= self.byte_budget {
            self.bytes_kept += frame.pixels.len();
            self.frames.insert(index, Arc::clone(&frame));
        }
        Ok(frame)
    }
}

/// The image sequence that `file` belongs to: the numbered files next to
/// it with the same name and extension, counting up without gaps from the
/// lowest number found. Returns the first file and the animation.
pub fn find_image_sequence(
    file: &Path,
    frames_per_second: f64,
) -> Result<(String, FrameAnimation), String> {
    let path = file.to_string_lossy().into_owned();
    let (before, _, digits, after) = split_numbered_path(&path)
        .ok_or("the file name does not end in a number, so it is not part of a sequence")?;
    let folder = file.parent().ok_or("the file has no folder")?;
    let name_start = before.rfind(['/', '\\']).map_or(0, |index| index + 1);
    let prefix = &before[name_start..];

    // Every number used by a file of the same name pattern.
    let mut numbers: Vec<u64> = fs::read_dir(folder)
        .map_err(|error| format!("could not read the folder: {error}"))?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let middle = name.strip_prefix(prefix)?.strip_suffix(after)?;
            let is_number = !middle.is_empty() && middle.chars().all(|c| c.is_ascii_digit());
            // With leading zeros, every frame has the same number of digits.
            let fits = middle.len() as u32 == digits || !middle.starts_with('0');
            (is_number && fits).then(|| middle.parse().ok()).flatten()
        })
        .collect();
    numbers.sort_unstable();
    numbers.dedup();
    let first_number = *numbers.first().ok_or("no frames found")?;
    let frame_count = numbers
        .iter()
        .enumerate()
        .take_while(|(index, number)| **number == first_number + *index as u64)
        .count() as u32;

    let first_file = sequence_frame_path(&path, first_number, digits, 0);
    let (frame_width, frame_height) = image::image_dimensions(&first_file)
        .map_err(|error| format!("could not read {first_file}: {error}"))?;
    let animation = FrameAnimation {
        layout: FrameLayout::Sequence {
            first_number,
            digits,
        },
        frame_width,
        frame_height,
        frame_count,
        frames_per_second,
        on_beat: false,
    }
    .sanitized();
    Ok((first_file, animation))
}

/// A sprite sheet in `file`, with a first guess at its grid: a row of
/// square frames if the sheet is a long strip of them, otherwise one
/// frame (the user sets the grid in the Properties panel).
pub fn guess_sprite_sheet(file: &Path, frames_per_second: f64) -> Result<FrameAnimation, String> {
    let (width, height) = image::image_dimensions(file)
        .map_err(|error| format!("could not read the image: {error}"))?;
    let (columns, rows) = if height > 0 && width > height && width % height == 0 {
        (width / height, 1)
    } else if width > 0 && height > width && height % width == 0 {
        (1, height / width)
    } else {
        (1, 1)
    };
    Ok(FrameAnimation::sprite_sheet(
        (width, height),
        columns,
        rows,
        frames_per_second,
    ))
}

/// How many frames each row of the sprite sheet at `path` holds, cut into`n/// the grid of `animation`: a row ends after its last cell that is not empty.
/// A cell is empty when it is fully transparent, or when the sheet has no
/// transparency and the cell is one flat color like the sheet's top left
/// corner (a plain background).
pub fn find_row_lengths(path: &str, animation: &FrameAnimation) -> Result<Vec<u32>, String> {
    let FrameLayout::SpriteSheet { columns, rows, .. } = animation.layout else {
        return Err("this is not a sprite sheet".to_owned());
    };
    let sheet = image::open(path)
        .map_err(|error| format!("could not read {path}: {error}"))?
        .into_rgba8();
    let background = *sheet.get_pixel(0, 0);
    let (width, height) = (animation.frame_width, animation.frame_height);
    let is_empty = |column: u32, row: u32| {
        let cell = imageops::crop_imm(&sheet, column * width, row * height, width, height);
        let mut pixels = cell.pixels().map(|(_, _, pixel)| pixel);
        pixels.all(|pixel| pixel[3] == 0 || pixel == background)
    };
    let lengths = (0..rows)
        .map(|row| {
            (0..columns)
                .rev()
                .find(|column| !is_empty(*column, row))
                .map_or(0, |column| column + 1)
        })
        .collect();
    Ok(lengths)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_folder(name: &str) -> std::path::PathBuf {
        let folder =
            std::env::temp_dir().join(format!("vizx-animation-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn solid(width: u32, height: u32, color: [u8; 4]) -> image::RgbaImage {
        image::RgbaImage::from_pixel(width, height, image::Rgba(color))
    }

    #[test]
    fn a_sequence_is_found_from_any_of_its_files_and_read_frame_by_frame() {
        let folder = temporary_folder("sequence");
        let colors = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 128]];
        for (index, color) in colors.iter().enumerate() {
            solid(8, 4, *color)
                .save(folder.join(format!("walk_{:03}.png", index + 1)))
                .unwrap();
        }
        // Not part of it: a gap, another name, another extension.
        solid(8, 4, [0; 4])
            .save(folder.join("walk_005.png"))
            .unwrap();
        solid(8, 4, [0; 4])
            .save(folder.join("run_002.png"))
            .unwrap();
        fs::write(folder.join("walk_004.txt"), "").unwrap();

        let (first, animation) = find_image_sequence(&folder.join("walk_002.png"), 12.0).unwrap();
        let mut reader = AnimationReader::new(&first, &animation, 1024, 1 << 20);
        let frames: Vec<_> = (0..4).map(|index| reader.frame(index).unwrap()).collect();
        let _ = fs::remove_dir_all(&folder);

        assert!(first.ends_with("walk_001.png"), "{first}");
        assert_eq!(animation.frame_count, 3);
        assert_eq!((animation.frame_width, animation.frame_height), (8, 4));
        assert_eq!(frames[0].pixels[..4], [255, 0, 0, 255]);
        assert_eq!(frames[1].pixels[..4], [0, 255, 0, 255]);
        // Premultiplied: half-transparent blue.
        assert_eq!(frames[2].pixels[..4], [0, 0, 128, 128]);
        // Past the end, the last frame.
        assert_eq!(frames[3].pixels[..4], [0, 0, 128, 128]);
        assert!(find_image_sequence(Path::new("cover.png"), 12.0).is_err());
    }

    #[test]
    fn a_sprite_sheet_is_cut_into_its_frames() {
        let folder = temporary_folder("sheet");
        // 2 x 2 frames of 4 x 3 pixels: red, green / blue, white.
        let mut sheet = solid(8, 6, [255, 0, 0, 255]);
        imageops::replace(&mut sheet, &solid(4, 3, [0, 255, 0, 255]), 4, 0);
        imageops::replace(&mut sheet, &solid(4, 3, [0, 0, 255, 255]), 0, 3);
        imageops::replace(&mut sheet, &solid(4, 3, [255, 255, 255, 255]), 4, 3);
        let path = folder.join("sheet.png");
        sheet.save(&path).unwrap();
        let animation = FrameAnimation::sprite_sheet((8, 6), 2, 2, 10.0);
        let path_text = path.to_string_lossy().into_owned();

        let mut reader = AnimationReader::new(&path_text, &animation, 1024, 0);
        let filled = find_row_lengths(&path_text, &animation);
        let firsts: Vec<[u8; 4]> = (0..4)
            .map(|index| {
                let frame = reader.frame(index).unwrap();
                assert_eq!((frame.width, frame.height), (4, 3));
                frame.pixels[..4].try_into().unwrap()
            })
            .collect();
        let strip = guess_sprite_sheet(&path, 12.0);
        let _ = fs::remove_dir_all(&folder);

        assert_eq!(
            firsts,
            [
                [255, 0, 0, 255],
                [0, 255, 0, 255],
                [0, 0, 255, 255],
                [255, 255, 255, 255]
            ]
        );
        // 8 x 6 is no strip of squares: one frame until the user says more.
        assert_eq!(strip.unwrap().frame_count, 1);
        assert!(reader.reads(&path_text, &animation));
        // Red is the corner color, so the first cell counts as background;
        // the green one after it still ends the row.
        assert_eq!(filled.unwrap(), [2, 2]);
    }

    #[test]
    fn empty_cells_at_the_ends_of_rows_are_found_and_skipped() {
        let folder = temporary_folder("uneven");
        // 3 x 2 cells of 2 x 2 pixels on a transparent sheet: three frames
        // in the first row, one in the second.
        let mut sheet = solid(6, 4, [0; 4]);
        for (column, row, color) in [
            (0, 0, [255, 0, 0, 255]),
            (1, 0, [0, 255, 0, 255]),
            (2, 0, [0, 0, 255, 255]),
            (0, 1, [255, 255, 0, 255]),
        ] {
            imageops::replace(&mut sheet, &solid(2, 2, color), column * 2, row * 2);
        }
        let path = folder.join("uneven.png");
        sheet.save(&path).unwrap();
        let path_text = path.to_string_lossy().into_owned();
        let mut animation = FrameAnimation::sprite_sheet((6, 4), 3, 2, 10.0);

        let lengths = find_row_lengths(&path_text, &animation).unwrap();
        if let FrameLayout::SpriteSheet { row_lengths, .. } = &mut animation.layout {
            *row_lengths = lengths.clone();
        }
        let animation = animation.sanitized();
        let mut reader = AnimationReader::new(&path_text, &animation, 1024, 0);
        let fourth = reader.frame(3).unwrap();
        let _ = fs::remove_dir_all(&folder);

        assert_eq!(lengths, [3, 1]);
        assert_eq!(animation.frame_count, 4);
        assert_eq!(fourth.pixels[..4], [255, 255, 0, 255]);
    }

    #[test]
    fn a_strip_of_square_frames_is_guessed() {
        let folder = temporary_folder("strip");
        let path = folder.join("run.png");
        solid(40, 10, [9; 4]).save(&path).unwrap();

        let guess = guess_sprite_sheet(&path, 12.0);
        let _ = fs::remove_dir_all(&folder);

        let guess = guess.unwrap();
        assert_eq!(guess.frame_count, 4);
        assert_eq!((guess.frame_width, guess.frame_height), (10, 10));
    }
}
