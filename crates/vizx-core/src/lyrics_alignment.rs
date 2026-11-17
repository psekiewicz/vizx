//! Putting the correct words on recognized timing: the official lyrics of a
//! song (pasted by the user) replace the words that were recognized, while
//! the times stay those that were heard.
//!
//! Both texts are cut into words and lined up with each other the way a
//! spelling checker compares two texts (an edit-distance alignment): words
//! that are the same, or were misheard as another word, are paired; words
//! only one side has are skipped. Each recognized word has a time (its
//! line's time shared out by the length of the words), so every paired
//! official word gets one, and an official line lasts from its first timed
//! word to its last. Lines with no paired word at all are placed in the gap
//! between their neighbors.
//!
//! The official text decides where lines break. Each new line also says
//! which recognized line most of its words came from, so it can go on that
//! line's layer (the singer's, after Separate Singers).

use crate::LyricLine;

/// A recognized line: when it is shown and what it says.
#[derive(Debug, Clone, PartialEq)]
pub struct HeardLine {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
}

/// An official line on the recognized timing, and the recognized line most
/// of its words were paired with (an index into the heard lines).
#[derive(Debug, Clone, PartialEq)]
pub struct AlignedLine {
    pub line: LyricLine,
    pub heard_line: usize,
}

/// The official lines of `official` (one per line of text; empty lines and
/// section names like `[Chorus]` are left out; `|` breaks a line on screen)
/// timed by `heard`.
pub fn align_official_lyrics(official: &str, heard: &[HeardLine]) -> Vec<AlignedLine> {
    let official_lines: Vec<String> = crate::split_lyrics(official)
        .into_iter()
        .filter(|line| !(line.starts_with('[') && line.ends_with(']')))
        .collect();
    if official_lines.is_empty() || heard.is_empty() {
        return Vec::new();
    }

    // Every heard word with its time and line.
    let mut heard_words: Vec<(String, f64, f64, usize)> = Vec::new();
    for (index, line) in heard.iter().enumerate() {
        let words: Vec<&str> = line.text.split_whitespace().collect();
        let weights: Vec<f64> = words
            .iter()
            .map(|word| word.chars().count() as f64 + 1.0)
            .collect();
        let total: f64 = weights.iter().sum::<f64>().max(1.0);
        let length = (line.end_seconds - line.start_seconds).max(0.0);
        let mut time = line.start_seconds;
        for (word, weight) in words.iter().zip(&weights) {
            let start = time;
            time += length * weight / total;
            heard_words.push((normalized(word), start, time, index));
        }
    }
    // Every official word with its line.
    let official_words: Vec<(String, usize)> = official_lines
        .iter()
        .enumerate()
        .flat_map(|(index, line)| {
            line.split_whitespace()
                .map(move |word| (normalized(word), index))
        })
        .filter(|(word, _)| !word.is_empty())
        .collect();

    let pairs = align(
        &official_words
            .iter()
            .map(|(word, _)| word.as_str())
            .collect::<Vec<_>>(),
        &heard_words
            .iter()
            .map(|(word, ..)| word.as_str())
            .collect::<Vec<_>>(),
    );

    // Per official line: its earliest and latest paired time, and how many
    // of its words came from each heard line.
    let mut times: Vec<Option<(f64, f64)>> = vec![None; official_lines.len()];
    let mut sources: Vec<Vec<usize>> = vec![Vec::new(); official_lines.len()];
    for (official_index, heard_index) in pairs {
        let line = official_words[official_index].1;
        let (_, start, end, source) = heard_words[heard_index];
        times[line] = Some(match times[line] {
            Some((first, last)) => (first.min(start), last.max(end)),
            None => (start, end),
        });
        sources[line].push(source);
    }

    // Lines without a paired word share the gap between their neighbors.
    let song_end = heard
        .iter()
        .map(|line| line.end_seconds)
        .fold(0.0, f64::max);
    let mut index = 0;
    while index < times.len() {
        if times[index].is_some() {
            index += 1;
            continue;
        }
        let first = index;
        while index < times.len() && times[index].is_none() {
            index += 1;
        }
        let from = first
            .checked_sub(1)
            .and_then(|before| times[before])
            .map_or(heard[0].start_seconds, |(_, end)| end);
        let to = times
            .get(index)
            .copied()
            .flatten()
            .map_or(song_end.max(from + 2.0), |(start, _)| start);
        let count = (index - first) as f64;
        let step = ((to - from) / count).max(0.5);
        for (offset, slot) in times[first..index].iter_mut().enumerate() {
            let start = from + step * offset as f64;
            *slot = Some((start, start + step));
        }
    }

    let mut aligned: Vec<AlignedLine> = official_lines
        .into_iter()
        .zip(times)
        .zip(sources)
        .enumerate()
        .map(|(index, ((text, time), sources))| {
            let (start, end) = time.unwrap_or((0.0, 0.0));
            AlignedLine {
                line: LyricLine {
                    text,
                    start_seconds: start,
                    end_seconds: end.max(start + 0.3),
                },
                heard_line: most_common(&sources).unwrap_or(index.min(heard.len() - 1)),
            }
        })
        .collect();
    // Lines keep their order: a line never starts before the one above it.
    for index in 1..aligned.len() {
        let previous = aligned[index - 1].line.start_seconds;
        let line = &mut aligned[index].line;
        if line.start_seconds < previous {
            let length = line.end_seconds - line.start_seconds;
            line.start_seconds = previous;
            line.end_seconds = previous + length;
        }
    }
    // A heard line without a paired word takes its neighbor's source.
    for index in 0..aligned.len() {
        if index > 0 && !heard.is_empty() && aligned[index].heard_line >= heard.len() {
            aligned[index].heard_line = aligned[index - 1].heard_line;
        }
    }
    aligned
}

/// Replaces the lines of the subtitle layers `layers` with the official
/// lyrics `official`, timed like the lines they replace; each new line goes
/// on the layer of the line most of its words were heard in, so singers
/// keep their lines. Returns how many lines there are now (0 changes
/// nothing).
pub fn apply_official_lyrics(
    project: &mut vizx_project::Project,
    layers: &[vizx_project::TrackId],
    official: &str,
) -> usize {
    let mut heard: Vec<(HeardLine, vizx_project::TrackId, vizx_project::ClipId)> = layers
        .iter()
        .filter_map(|id| project.track(*id))
        .filter(|track| track.kind == vizx_project::TrackKind::Subtitles)
        .flat_map(|track| {
            track.clips.iter().map(|clip| {
                (
                    HeardLine {
                        start_seconds: clip.start_milliseconds as f64 / 1000.0,
                        end_seconds: clip.end_milliseconds() as f64 / 1000.0,
                        text: project.clip_text(clip.id).unwrap_or_default().to_owned(),
                    },
                    track.id,
                    clip.id,
                )
            })
        })
        .collect();
    heard.sort_by(|a, b| a.0.start_seconds.total_cmp(&b.0.start_seconds));
    let lines: Vec<HeardLine> = heard.iter().map(|(line, ..)| line.clone()).collect();
    let aligned = align_official_lyrics(official, &lines);
    if aligned.is_empty() {
        return 0;
    }
    for (_, _, clip) in &heard {
        project.remove_clip(*clip);
    }
    let milliseconds = |seconds: f64| (seconds.max(0.0) * 1000.0).round() as u64;
    let mut added = 0;
    for line in &aligned {
        let layer = heard[line.heard_line.min(heard.len() - 1)].1;
        let start = milliseconds(line.line.start_seconds);
        let duration = milliseconds(line.line.end_seconds).saturating_sub(start);
        if project
            .add_subtitle_line(layer, &line.line.text, start, duration)
            .is_some()
        {
            added += 1;
        }
    }
    added
}
/// A word for comparing: lower case, letters and digits only.
fn normalized(word: &str) -> String {
    word.chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn most_common(values: &[usize]) -> Option<usize> {
    let mut counted: Vec<(usize, usize)> = Vec::new();
    for value in values {
        match counted.iter_mut().find(|(known, _)| known == value) {
            Some((_, count)) => *count += 1,
            None => counted.push((*value, 1)),
        }
    }
    counted
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(value, _)| value)
}

/// Pairs of word positions (in `a`, in `b`) of the cheapest way to turn `a`
/// into `b`: equal words cost nothing, a changed word costs 1 (less when
/// the words are alike), a word only one side has costs 1.
fn align(a: &[&str], b: &[&str]) -> Vec<(usize, usize)> {
    let (rows, columns) = (a.len() + 1, b.len() + 1);
    let mut cost = vec![0.0f32; rows * columns];
    for row in 0..rows {
        cost[row * columns] = row as f32;
    }
    for (column, first_row) in cost.iter_mut().take(columns).enumerate() {
        *first_row = column as f32;
    }
    for row in 1..rows {
        for column in 1..columns {
            let change =
                cost[(row - 1) * columns + column - 1] + difference(a[row - 1], b[column - 1]);
            let skip_a = cost[(row - 1) * columns + column] + 1.0;
            let skip_b = cost[row * columns + column - 1] + 1.0;
            cost[row * columns + column] = change.min(skip_a).min(skip_b);
        }
    }
    // Back from the end, keeping the pairs that were matched or changed.
    let mut pairs = Vec::new();
    let (mut row, mut column) = (a.len(), b.len());
    while row > 0 && column > 0 {
        let here = cost[row * columns + column];
        let change = cost[(row - 1) * columns + column - 1] + difference(a[row - 1], b[column - 1]);
        if (here - change).abs() < 1e-4 {
            pairs.push((row - 1, column - 1));
            row -= 1;
            column -= 1;
        } else if (here - (cost[(row - 1) * columns + column] + 1.0)).abs() < 1e-4 {
            row -= 1;
        } else {
            column -= 1;
        }
    }
    pairs.reverse();
    pairs
}

/// 0 for the same word, up to 1.2 for unrelated words (a little more than
/// skipping, so unrelated words are not forced into pairs).
fn difference(a: &str, b: &str) -> f32 {
    if a == b {
        return 0.0;
    }
    let shared = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
    let longest = a.chars().count().max(b.chars().count()).max(1);
    // Misheard words often share their beginning ("perfection"/"affection"
    // share their end; "its"/"it's" become equal once normalized).
    let shared_end = a
        .chars()
        .rev()
        .zip(b.chars().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let alike = shared.max(shared_end) as f32 / longest as f32;
    1.2 - 0.6 * alike
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heard(start: f64, end: f64, text: &str) -> HeardLine {
        HeardLine {
            start_seconds: start,
            end_seconds: end,
            text: text.to_owned(),
        }
    }

    #[test]
    fn correct_words_take_the_heard_times_and_the_singer() {
        // Misheard words, a line split differently, and a heard line the
        // official text does not have.
        let heard_lines = [
            heard(10.0, 12.0, "the quick brown socks"),
            heard(12.0, 14.0, "jumps over the lazy"),
            heard(14.0, 15.0, "dog"),
            heard(20.0, 22.0, "yeah yeah"),
            heard(30.0, 33.0, "and then it ended well"),
        ];
        let official = "[Verse]\nThe quick brown fox\njumps over the lazy dog\n\nAnd then it ended";
        let aligned = align_official_lyrics(official, &heard_lines);

        let texts: Vec<&str> = aligned.iter().map(|line| line.line.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "The quick brown fox",
                "jumps over the lazy dog",
                "And then it ended"
            ]
        );
        // The first line keeps the time of the first heard line, "fox"
        // standing in for the misheard "socks".
        assert!((aligned[0].line.start_seconds - 10.0).abs() < 1e-6);
        assert!((aligned[0].line.end_seconds - 12.0).abs() < 1e-6);
        // The second spans two heard lines.
        assert!((aligned[1].line.start_seconds - 12.0).abs() < 1e-6);
        assert!((aligned[1].line.end_seconds - 15.0).abs() < 1e-6);
        assert_eq!(aligned[1].heard_line, 1);
        // The third ends before the heard "well".
        assert!((aligned[2].line.start_seconds - 30.0).abs() < 1e-6);
        assert!(aligned[2].line.end_seconds < 33.0);
        assert_eq!(aligned[2].heard_line, 4);
    }

    #[test]
    fn a_line_nobody_heard_goes_into_the_gap() {
        let heard_lines = [
            heard(0.0, 2.0, "first line"),
            heard(10.0, 12.0, "last line"),
        ];
        let aligned =
            align_official_lyrics("first line\nsomething unheard\nlast line", &heard_lines);
        assert_eq!(aligned.len(), 3);
        assert!((aligned[1].line.start_seconds - 2.0).abs() < 1e-6);
        assert!(aligned[1].line.end_seconds <= 10.0 + 1e-6);
        assert!(align_official_lyrics("", &heard_lines).is_empty());
    }

    #[test]
    fn official_lyrics_replace_the_lines_and_singers_keep_theirs() {
        let mut project = vizx_project::Project::new();
        let first = project.add_subtitle_track("Singer 1");
        let second = project.add_subtitle_track("Singer 2");
        project.add_subtitle_line(first, "hello their world", 0, 2000);
        project.add_subtitle_line(second, "an answer from me", 3000, 2000);
        let count = apply_official_lyrics(
            &mut project,
            &[first, second],
            "Hello there, world\nAn answer from me",
        );
        assert_eq!(count, 2);
        let texts = |project: &vizx_project::Project, id| -> Vec<String> {
            let track = project.track(id).unwrap();
            track
                .clips
                .iter()
                .map(|clip| project.clip_text(clip.id).unwrap().to_owned())
                .collect()
        };
        assert_eq!(texts(&project, first), ["Hello there, world"]);
        assert_eq!(texts(&project, second), ["An answer from me"]);
        // Nothing to apply: nothing changes.
        assert_eq!(apply_official_lyrics(&mut project, &[first], "  "), 0);
        assert_eq!(texts(&project, first).len(), 1);
    }
}
