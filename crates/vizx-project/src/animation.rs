//! Frame animations: pictures that change by themselves, made of separate
//! frames. Either a numbered image sequence (`walk_0001.png`,
//! `walk_0002.png`, ...) or a sprite sheet (one image with the frames in a
//! grid).

use serde::{Deserialize, Serialize};

/// A frame animation: how its frames are stored and how they are played.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameAnimation {
    pub layout: FrameLayout,
    /// Size of one frame in pixels.
    pub frame_width: u32,
    pub frame_height: u32,
    pub frame_count: u32,
    /// Speed when playing on its own.
    pub frames_per_second: f64,
    /// Instead of at a steady speed, step one frame on every beat of the
    /// clip's audio.
    #[serde(default)]
    pub on_beat: bool,
}

/// How the frames of an animation are stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrameLayout {
    /// Separate files whose names differ only in a number: the media's path
    /// is the first one, the others count up from `first_number`, written
    /// with at least `digits` digits (leading zeros).
    Sequence { first_number: u64, digits: u32 },
    /// One image with the frames in a grid of `columns` x `rows`, read left
    /// to right, then top to bottom.
    SpriteSheet {
        columns: u32,
        rows: u32,
        sheet_width: u32,
        sheet_height: u32,
        /// How many frames each row holds, from the top, when rows are not
        /// all full (for example 5, 4 and 6 in a grid 6 columns wide; the
        /// frames of a row start at its left edge). Empty: every row is
        /// full, and `frame_count` may still leave the last ones out.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        row_lengths: Vec<u32>,
    },
}

impl FrameAnimation {
    /// Slowest and fastest allowed speed, in frames per second.
    pub const SPEED_RANGE: (f64, f64) = (0.1, 120.0);

    /// A sprite sheet of `columns` x `rows` frames, all of them used.
    pub fn sprite_sheet(
        (sheet_width, sheet_height): (u32, u32),
        columns: u32,
        rows: u32,
        frames_per_second: f64,
    ) -> Self {
        Self {
            layout: FrameLayout::SpriteSheet {
                columns,
                rows,
                sheet_width,
                sheet_height,
                row_lengths: Vec::new(),
            },
            frame_width: 0,
            frame_height: 0,
            frame_count: columns.saturating_mul(rows),
            frames_per_second,
            on_beat: false,
        }
        .sanitized()
    }

    /// The nearest valid animation: at least one frame, a sensible speed,
    /// and for a sprite sheet a frame size and count that fit its grid.
    pub fn sanitized(mut self) -> Self {
        let (slowest, fastest) = Self::SPEED_RANGE;
        self.frames_per_second = if self.frames_per_second.is_finite() {
            self.frames_per_second.clamp(slowest, fastest)
        } else {
            12.0
        };
        if let FrameLayout::SpriteSheet {
            columns,
            rows,
            sheet_width,
            sheet_height,
            row_lengths,
        } = &mut self.layout
        {
            *columns = (*columns).clamp(1, (*sheet_width).max(1));
            *rows = (*rows).clamp(1, (*sheet_height).max(1));
            self.frame_width = (*sheet_width / *columns).max(1);
            self.frame_height = (*sheet_height / *rows).max(1);
            if row_lengths.is_empty() {
                self.frame_count = self.frame_count.clamp(1, *columns * *rows);
            } else {
                // One count per row, none longer than the grid is wide.
                row_lengths.resize(*rows as usize, *columns);
                for length in row_lengths.iter_mut() {
                    *length = (*length).min(*columns);
                }
                self.frame_count = row_lengths.iter().sum();
                if self.frame_count == 0 {
                    row_lengths[0] = 1;
                    self.frame_count = 1;
                }
            }
        }
        self.frame_count = self.frame_count.max(1);
        self
    }

    /// Where frame `index` (counted from 0) of a sprite sheet sits: its
    /// column and row in the grid. `None` for an image sequence.
    pub fn sheet_cell(&self, index: u32) -> Option<(u32, u32)> {
        let FrameLayout::SpriteSheet {
            columns,
            row_lengths,
            ..
        } = &self.layout
        else {
            return None;
        };
        if row_lengths.is_empty() {
            let columns = (*columns).max(1);
            return Some((index % columns, index / columns));
        }
        let mut left = index;
        for (row, length) in row_lengths.iter().enumerate() {
            if left < *length {
                return Some((left, row as u32));
            }
            left -= length;
        }
        // Past the end: the last frame.
        let (row, length) = row_lengths
            .iter()
            .enumerate()
            .rev()
            .find(|(_, length)| **length > 0)?;
        Some((length - 1, row as u32))
    }

    /// Which frame is shown `seconds_into_clip` into the clip, when
    /// `beats_so_far` beats of the clip's audio have happened.
    pub fn frame_at(&self, seconds_into_clip: f64, beats_so_far: usize) -> u32 {
        let count = u64::from(self.frame_count.max(1));
        let step = if self.on_beat {
            beats_so_far as u64
        } else {
            (seconds_into_clip.max(0.0) * self.frames_per_second) as u64
        };
        (step % count) as u32
    }
}

/// The parts of a numbered file name: everything before the number, the
/// number, how many digits it has, and everything after it.
/// `C:/walk/walk_0007.png` gives (`C:/walk/walk_`, 7, 4, `.png`). `None` if
/// the file name (without its extension) does not end in a number.
pub fn split_numbered_path(path: &str) -> Option<(&str, u64, u32, &str)> {
    let name_start = path.rfind(['/', '\\']).map_or(0, |index| index + 1);
    let extension_start = path[name_start..]
        .rfind('.')
        .map_or(path.len(), |index| name_start + index);
    let stem = &path[name_start..extension_start];
    let digit_count = stem.chars().rev().take_while(char::is_ascii_digit).count();
    if digit_count == 0 || digit_count > 18 {
        return None;
    }
    let number_start = extension_start - digit_count;
    let number = path[number_start..extension_start].parse().ok()?;
    Some((
        &path[..number_start],
        number,
        digit_count as u32,
        &path[extension_start..],
    ))
}

/// The file of frame `index` of a sequence whose first file is `path`.
pub fn sequence_frame_path(path: &str, first_number: u64, digits: u32, index: u32) -> String {
    match split_numbered_path(path) {
        Some((before, _, _, after)) => format!(
            "{before}{:0width$}{after}",
            first_number + u64::from(index),
            width = digits as usize
        ),
        None => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbered_file_names_are_taken_apart_and_counted_on() {
        assert_eq!(
            split_numbered_path("C:/walk/walk_0007.png"),
            Some(("C:/walk/walk_", 7, 4, ".png"))
        );
        assert_eq!(
            split_numbered_path(r"D:\frames.v2\12.jpg"),
            Some((r"D:\frames.v2\", 12, 2, ".jpg"))
        );
        assert_eq!(split_numbered_path("cover.png"), None);
        // A number in a folder name does not count.
        assert_eq!(split_numbered_path("take2/cover.png"), None);

        assert_eq!(
            sequence_frame_path("C:/walk/walk_0007.png", 7, 4, 5),
            "C:/walk/walk_0012.png"
        );
        // Without leading zeros the number simply grows.
        assert_eq!(sequence_frame_path("f9.png", 9, 1, 3), "f12.png");
    }

    #[test]
    fn a_sprite_sheet_gets_its_frame_size_from_the_grid() {
        let sheet = FrameAnimation::sprite_sheet((400, 200), 4, 2, 10.0);
        assert_eq!((sheet.frame_width, sheet.frame_height), (100, 100));
        assert_eq!(sheet.frame_count, 8);

        // Fewer frames than cells is fine (the last row may be half
        // empty); more is not, and the grid stays within the sheet.
        let mut fewer = sheet.clone();
        fewer.frame_count = 6;
        assert_eq!(fewer.clone().sanitized().frame_count, 6);
        fewer.frame_count = 99;
        assert_eq!(fewer.sanitized().frame_count, 8);
        let silly = FrameAnimation::sprite_sheet((10, 10), 0, 50, f64::NAN);
        assert_eq!((silly.frame_width, silly.frame_height), (10, 1));
        assert_eq!(silly.frames_per_second, 12.0);
    }

    #[test]
    fn frames_follow_time_or_beats_and_start_over() {
        let mut walk = FrameAnimation::sprite_sheet((400, 100), 4, 1, 8.0);

        assert_eq!(walk.frame_at(0.0, 0), 0);
        assert_eq!(walk.frame_at(0.26, 0), 2);
        assert_eq!(walk.frame_at(0.5, 0), 0); // 4 frames later: again
        walk.on_beat = true;
        assert_eq!(walk.frame_at(0.26, 0), 0);
        assert_eq!(walk.frame_at(9.0, 5), 1);
    }

    #[test]
    fn rows_of_different_lengths_are_read_one_after_another() {
        // Rows of 5, 4 and 6 frames in a grid 6 wide.
        let mut sheet = FrameAnimation::sprite_sheet((600, 300), 6, 3, 10.0);
        if let FrameLayout::SpriteSheet { row_lengths, .. } = &mut sheet.layout {
            *row_lengths = vec![5, 4, 6];
        }
        let sheet = sheet.sanitized();

        assert_eq!(sheet.frame_count, 15);
        assert_eq!(sheet.sheet_cell(0), Some((0, 0)));
        assert_eq!(sheet.sheet_cell(4), Some((4, 0)));
        assert_eq!(sheet.sheet_cell(5), Some((0, 1)));
        assert_eq!(sheet.sheet_cell(8), Some((3, 1)));
        assert_eq!(sheet.sheet_cell(9), Some((0, 2)));
        assert_eq!(sheet.sheet_cell(14), Some((5, 2)));
        assert_eq!(sheet.sheet_cell(99), Some((5, 2)));

        // Too long a row is cut to the grid; a missing row counts as full.
        let mut odd = FrameAnimation::sprite_sheet((400, 200), 4, 2, 10.0);
        if let FrameLayout::SpriteSheet { row_lengths, .. } = &mut odd.layout {
            *row_lengths = vec![9];
        }
        assert_eq!(odd.sanitized().frame_count, 8);
    }
}
