//! Lyrics as subtitles ("text video"): turning the words of a song into
//! timed lines, each of which becomes a text clip.
//!
//! Lines can be timed in several ways: read from an `.lrc` or `.srt` file,
//! spread over a stretch of time, one every few beats, or tapped along with
//! the music (see `EditorSession::start_lyric_line`).

use vizx_project::{Easing, Keyframe, Placement, TextProperties, Transform};

/// One line of lyrics and when it is shown, in seconds of project time.
#[derive(Debug, Clone, PartialEq)]
pub struct LyricLine {
    pub text: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

/// How lines of lyrics look and appear. Applied to every line at once.
#[derive(Debug, Clone, PartialEq)]
pub struct LyricsStyle {
    /// Font, size, color and alignment (its own text is not used).
    pub text: TextProperties,
    /// Where the middle of a line is, as a fraction of the picture's height
    /// from its top: 0.85 is near the bottom, like subtitles.
    pub height: f64,
    /// How long a line takes to fade in and out; 0 shows it at once.
    pub fade_seconds: f64,
    /// Lines also grow into their size while they fade in.
    pub pop: bool,
}

impl Default for LyricsStyle {
    fn default() -> Self {
        Self {
            text: TextProperties {
                size: 0.07,
                ..TextProperties::default()
            },
            height: 0.85,
            fade_seconds: 0.15,
            pop: false,
        }
    }
}

/// The keyframes that fade a line in and out (and let it pop), for a clip
/// at `placement` lasting `duration_milliseconds`. Empty without a fade.
pub fn lyric_keyframes(
    placement: Placement,
    duration_milliseconds: u64,
    style: &LyricsStyle,
) -> Vec<Keyframe> {
    if style.fade_seconds <= 0.0 || !style.fade_seconds.is_finite() {
        return Vec::new();
    }
    let duration = duration_milliseconds as i64;
    // A short line fades for at most a third of its time each way.
    let fade = ((style.fade_seconds * 1000.0) as i64)
        .min(duration / 3)
        .max(1);
    let shown = Transform {
        placement,
        rotation_degrees: 0.0,
        opacity: 1.0,
    };
    let mut hidden = Transform {
        opacity: 0.0,
        ..shown
    };
    if style.pop {
        let scale = 0.8;
        let (width, height) = (placement.width * scale, placement.height * scale);
        hidden.placement = Placement {
            x: placement.x + (placement.width - width) / 2.0,
            y: placement.y + (placement.height - height) / 2.0,
            width,
            height,
        };
    }
    let key = |time_milliseconds: i64, transform: Transform| Keyframe {
        time_milliseconds,
        transform,
        easing: Easing::Smooth,
    };
    vec![
        key(0, hidden),
        key(fade, shown),
        key(duration - fade, shown),
        key(
            duration,
            Transform {
                opacity: 0.0,
                ..shown
            },
        ),
    ]
}

/// How long the last line of a timed file stays when the file does not say.
const LAST_LINE_SECONDS: f64 = 4.0;

/// The lines of pasted lyrics: every line that is not empty is one
/// subtitle. A `|` inside a line breaks it in two rows on screen.
pub fn split_lyrics(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            line.split('|')
                .map(str::trim)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect()
}

/// Spreads `lines` over the time from `start_seconds` to `end_seconds`, one
/// after another without gaps. Longer lines get more time, as they take
/// longer to sing and to read.
pub fn spread_lines(lines: &[String], start_seconds: f64, end_seconds: f64) -> Vec<LyricLine> {
    // A short line still needs a moment: every line counts a few letters
    // more than it has.
    let weight = |line: &String| line.chars().count() as f64 + 8.0;
    let total: f64 = lines.iter().map(weight).sum();
    let length = (end_seconds - start_seconds).max(0.0);
    let mut time = start_seconds;
    lines
        .iter()
        .map(|line| {
            let start = time;
            time += length * weight(line) / total.max(1.0);
            LyricLine {
                text: line.clone(),
                start_seconds: start,
                end_seconds: time,
            }
        })
        .collect()
}

/// One line every `beats_per_line` beats, starting on the first beat at or
/// after `from_seconds`. Lines that run past the last beat keep the same
/// rhythm (the average distance of the beats). Empty without beats.
pub fn lines_on_beats(
    lines: &[String],
    beats: &[f64],
    from_seconds: f64,
    beats_per_line: usize,
) -> Vec<LyricLine> {
    let beats: Vec<f64> = beats
        .iter()
        .copied()
        .filter(|beat| *beat >= from_seconds - 1e-6)
        .collect();
    if beats.is_empty() {
        return Vec::new();
    }
    let beats_per_line = beats_per_line.max(1);
    let spacing = if beats.len() > 1 {
        (beats[beats.len() - 1] - beats[0]) / (beats.len() - 1) as f64
    } else {
        0.5
    };
    let beat_time = |index: usize| match beats.get(index) {
        Some(time) => *time,
        None => beats[beats.len() - 1] + (index + 1 - beats.len()) as f64 * spacing,
    };
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| LyricLine {
            text: line.clone(),
            start_seconds: beat_time(index * beats_per_line),
            end_seconds: beat_time((index + 1) * beats_per_line),
        })
        .collect()
}

/// Reads timed lyrics from the text of an `.srt` subtitle file or an
/// `.lrc` lyrics file (which one is recognized by the content).
pub fn parse_timed_lyrics(text: &str) -> Result<Vec<LyricLine>, String> {
    let lines = if text.contains("-->") {
        parse_srt(text)
    } else {
        parse_lrc(text)
    };
    if lines.is_empty() {
        Err("no timed lines found; is this an .lrc or .srt file?".to_owned())
    } else {
        Ok(lines)
    }
}

/// `[01:23.45]words` lines. A line can carry several times (a chorus that
/// repeats), and a time with no words ends the line before it. Tags such as
/// `[ar:Artist]` and word times `<01:23.45>` are skipped.
fn parse_lrc(text: &str) -> Vec<LyricLine> {
    // (start, words); words are empty for a pause.
    let mut timed: Vec<(f64, String)> = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim();
        let mut times = Vec::new();
        while let Some(after) = rest.strip_prefix('[') {
            let Some(end) = after.find(']') else {
                break;
            };
            if let Some(time) = parse_clock(&after[..end]) {
                times.push(time);
            }
            rest = after[end + 1..].trim_start();
        }
        let words = strip_word_times(rest).replace('|', "\n");
        for time in times {
            timed.push((time, words.trim().to_owned()));
        }
    }
    timed.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut lines = Vec::new();
    for (index, (start, words)) in timed.iter().enumerate() {
        if words.is_empty() {
            continue;
        }
        let end = timed
            .get(index + 1)
            .map_or(start + LAST_LINE_SECONDS, |(next, _)| *next);
        lines.push(LyricLine {
            text: words.clone(),
            start_seconds: *start,
            end_seconds: end.max(start + 0.1),
        });
    }
    lines
}

/// Removes `<01:23.45>` word times of "enhanced" LRC files.
fn strip_word_times(text: &str) -> String {
    let mut result = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        let Some(close) = rest[open..].find('>') else {
            break;
        };
        result.push_str(&rest[..open]);
        let inside = &rest[open + 1..open + close];
        if parse_clock(inside).is_none() {
            // Not a time: keep it as it was.
            result.push_str(&rest[open..=open + close]);
        }
        rest = &rest[open + close + 1..];
    }
    result.push_str(rest);
    result
}

/// Numbered blocks of `00:01:02,500 --> 00:01:05,000` and the text below.
fn parse_srt(text: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    let text = text.replace("\r\n", "\n");
    for block in text.split("\n\n") {
        let mut rows = block.lines().map(str::trim).filter(|row| !row.is_empty());
        let Some(times) = rows.find(|row| row.contains("-->")) else {
            continue;
        };
        let Some((start, end)) = times.split_once("-->") else {
            continue;
        };
        // After the end time, a position may follow ("X1:..."): only the
        // first word counts.
        let end = end.split_whitespace().next().unwrap_or("");
        let (Some(start), Some(end)) = (parse_clock(start.trim()), parse_clock(end)) else {
            continue;
        };
        let words: Vec<String> = rows.map(strip_tags).collect();
        if words.is_empty() {
            continue;
        }
        lines.push(LyricLine {
            text: words.join("\n"),
            start_seconds: start,
            end_seconds: end.max(start + 0.1),
        });
    }
    lines.sort_by(|a, b| a.start_seconds.total_cmp(&b.start_seconds));
    lines
}

/// Removes `<i>`-like formatting tags of subtitle files.
fn strip_tags(row: &str) -> String {
    let mut result = String::new();
    let mut inside_tag = false;
    for character in row.chars() {
        match character {
            '<' => inside_tag = true,
            '>' if inside_tag => inside_tag = false,
            _ if !inside_tag => result.push(character),
            _ => {}
        }
    }
    result.trim().to_owned()
}

/// A clock time such as `1:23.45`, `01:23:45,678` or `83.5`, in seconds.
/// `None` if it is not one (like the `ar:Artist` tag of a lyrics file).
fn parse_clock(text: &str) -> Option<f64> {
    let text = text.trim().replace(',', ".");
    if text.is_empty() {
        return None;
    }
    let mut seconds = 0.0;
    for part in text.split(':') {
        let is_number = !part.is_empty() && part.chars().all(|c| c.is_ascii_digit() || c == '.');
        if !is_number {
            return None;
        }
        seconds = seconds * 60.0 + part.parse::<f64>().ok()?;
    }
    Some(seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(lines: &[LyricLine]) -> Vec<&str> {
        lines.iter().map(|line| line.text.as_str()).collect()
    }

    #[test]
    fn pasted_lyrics_become_one_subtitle_per_line() {
        let lines = split_lyrics("  First line \n\n second | in two rows\r\n\nlast");
        assert_eq!(lines, ["First line", "second\nin two rows", "last"]);
    }

    #[test]
    fn lines_are_spread_by_their_length_without_gaps() {
        let lines = vec!["a".to_owned(), "a much longer line here".to_owned()];
        let spread = spread_lines(&lines, 10.0, 40.0);

        assert_eq!(spread[0].start_seconds, 10.0);
        assert_eq!(spread[0].end_seconds, spread[1].start_seconds);
        assert!((spread[1].end_seconds - 40.0).abs() < 1e-9);
        let first = spread[0].end_seconds - spread[0].start_seconds;
        let second = spread[1].end_seconds - spread[1].start_seconds;
        assert!(second > 2.0 * first, "{first} {second}");
    }

    #[test]
    fn lines_follow_the_beats_and_keep_the_rhythm_after_them() {
        let lines: Vec<String> = ["one", "two", "three"].map(str::to_owned).to_vec();
        let beats = [0.5, 1.0, 1.5, 2.0, 2.5];

        let timed = lines_on_beats(&lines, &beats, 0.9, 2);

        let times: Vec<(f64, f64)> = timed
            .iter()
            .map(|line| (line.start_seconds, line.end_seconds))
            .collect();
        assert_eq!(times, [(1.0, 2.0), (2.0, 3.0), (3.0, 4.0)]);
        assert!(lines_on_beats(&lines, &[], 0.0, 2).is_empty());
    }

    #[test]
    fn lrc_files_are_read_with_pauses_repeats_and_tags() {
        let lrc = "[ar:Someone]\n[ti:Song]\n[00:01.00]Hello <00:01.50>world\n\
                   [00:03.00]\n[00:05.00][00:20.00]Chorus\n[00:07.50]Bye";
        let lines = parse_timed_lyrics(lrc).unwrap();

        assert_eq!(texts(&lines), ["Hello world", "Chorus", "Bye", "Chorus"]);
        // The empty time ends the first line.
        assert_eq!((lines[0].start_seconds, lines[0].end_seconds), (1.0, 3.0));
        assert_eq!((lines[1].start_seconds, lines[1].end_seconds), (5.0, 7.5));
        assert_eq!(lines[2].end_seconds, 20.0);
        assert_eq!(lines[3].end_seconds, 24.0);
    }

    #[test]
    fn srt_files_are_read_with_several_rows_and_without_tags() {
        let srt = "1\r\n00:00:01,000 --> 00:00:02,500\r\n<i>Hello</i>\r\nthere\r\n\r\n\
                   2\r\n00:01:00,250 --> 00:01:03,000 X1:10\r\nBye\r\n";
        let lines = parse_timed_lyrics(srt).unwrap();

        assert_eq!(texts(&lines), ["Hello\nthere", "Bye"]);
        assert_eq!((lines[0].start_seconds, lines[0].end_seconds), (1.0, 2.5));
        assert_eq!(
            (lines[1].start_seconds, lines[1].end_seconds),
            (60.25, 63.0)
        );
        assert!(parse_timed_lyrics("just words").is_err());
    }
}
