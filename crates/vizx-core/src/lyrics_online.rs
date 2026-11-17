//! Finding the words of a song online, in LRCLIB (lrclib.net): a free,
//! open database of song lyrics, many of them timed line by line.
//!
//! Recognizing sung words is unreliable on stylized singing; the published
//! lyrics are not. LRCLIB is asked only when the user presses the button,
//! and is sent the song's title, its artist (if known) and nothing else.
//!
//! Many songs share a title, so of the songs LRCLIB knows by that name the
//! one whose length matches the file's is taken. Its times are trusted only
//! then: the lyrics of a version of another length (a radio edit, a live
//! take) keep their words, which can then be timed by recognition.

use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde::Deserialize;

use crate::LyricLine;
use crate::lyrics_recognition::SongPlacement;

/// The search address of LRCLIB's public interface.
const SEARCH_URL: &str = "https://lrclib.net/api/search";

/// How far, in seconds, a song's length may be from the file's for its
/// times to be trusted.
const LENGTH_TOLERANCE_SECONDS: f64 = 4.0;

/// The longest a line of timed lyrics stays on screen when the next line
/// is far away (an instrumental break follows): LRC gives only start times.
const LONGEST_LINE_SECONDS: f64 = 6.0;

/// A song found online.
#[derive(Debug, Clone, PartialEq)]
pub struct OnlineLyrics {
    pub title: String,
    pub artist: String,
    /// The words with a time for each line (LRC), when the song found has
    /// the file's length and was timed.
    pub synced: Option<String>,
    /// The words alone, one line per line.
    pub plain: String,
    /// LRCLIB lists the song as having no words.
    pub instrumental: bool,
    /// It was looked up by its title alone: it may be another song of the
    /// same name (a title alone mostly finds the most famous one).
    pub by_title_only: bool,
}

/// One song as LRCLIB describes it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Candidate {
    #[serde(default)]
    track_name: String,
    #[serde(default)]
    artist_name: String,
    /// Seconds.
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    instrumental: bool,
    #[serde(default)]
    plain_lyrics: Option<String>,
    #[serde(default)]
    synced_lyrics: Option<String>,
}

/// A search running in the background.
pub struct LyricsSearch {
    result: mpsc::Receiver<Result<OnlineLyrics, String>>,
}

impl LyricsSearch {
    /// Starts looking for the song `title` by `artist` (may be empty),
    /// `length_seconds` long. `on_done` is called from the background
    /// thread when it has ended (to repaint the window).
    pub fn start(
        title: &str,
        artist: &str,
        length_seconds: f64,
        on_done: impl FnOnce() + Send + 'static,
    ) -> Self {
        let (sender, result) = mpsc::channel();
        let (title, artist) = (title.trim().to_owned(), artist.trim().to_owned());
        thread::spawn(move || {
            let _ = sender.send(search(&title, &artist, length_seconds));
            on_done();
        });
        Self { result }
    }

    /// The song found once the search has ended, or why it failed. `None`
    /// while it is still running.
    pub fn finished(&self) -> Option<Result<OnlineLyrics, String>> {
        match self.result.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("the search stopped unexpectedly".to_owned()))
            }
        }
    }
}

/// Asks LRCLIB for the song and picks the best of what it knows.
fn search(title: &str, artist: &str, length_seconds: f64) -> Result<OnlineLyrics, String> {
    if title.is_empty() {
        return Err("the song needs a title".to_owned());
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .into();
    let ask = |artist: &str| -> Result<Vec<Candidate>, String> {
        let mut request = agent.get(SEARCH_URL).query("track_name", title);
        if !artist.is_empty() {
            request = request.query("artist_name", artist);
        }
        let mut response = request
            .call()
            .map_err(|error| format!("LRCLIB could not be reached: {error}"))?;
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|error| format!("LRCLIB's answer could not be read: {error}"))?;
        serde_json::from_str(&body)
            .map_err(|error| format!("LRCLIB's answer is not understood: {error}"))
    };
    let mut candidates = ask(artist)?;
    if !artist.is_empty() {
        // The artist may be written differently there ("feat." and the
        // like): then by the title alone, keeping only that artist's songs
        // (a title alone finds mostly the most famous song of that name).
        if candidates.is_empty() {
            candidates = ask("")?;
        }
        candidates.retain(|candidate| same_artist(&candidate.artist_name, artist));
    }
    let mut found = choose(candidates, length_seconds).ok_or_else(|| {
        if artist.is_empty() {
            format!("LRCLIB does not know the words of \"{title}\"")
        } else {
            format!("LRCLIB does not know the words of \"{title}\" by {artist}")
        }
    })?;
    found.by_title_only = artist.is_empty();
    Ok(found)
}

/// Whether LRCLIB's `listed` artist is the `typed` one: either name holds
/// the other, ignoring case and punctuation ("Ivycomb" and "ivycomb,
/// OodleZz" are the same).
fn same_artist(listed: &str, typed: &str) -> bool {
    let simple = |name: &str| -> String {
        name.chars()
            .filter(|character| character.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    let (listed, typed) = (simple(listed), simple(typed));
    !typed.is_empty() && (listed.contains(&typed) || typed.contains(&listed))
}

/// The best of the songs found for a file `length_seconds` long: one of the
/// file's length, timed if possible; otherwise the first with words, its
/// times not trusted.
fn choose(candidates: Vec<Candidate>, length_seconds: f64) -> Option<OnlineLyrics> {
    let has_words = |candidate: &Candidate| {
        candidate.instrumental
            || [&candidate.plain_lyrics, &candidate.synced_lyrics]
                .into_iter()
                .flatten()
                .any(|text| !text.trim().is_empty())
    };
    let is_timed = |candidate: &Candidate| {
        candidate
            .synced_lyrics
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty())
    };
    let length_off = |candidate: &Candidate| {
        candidate
            .duration
            .map_or(f64::INFINITY, |duration| (duration - length_seconds).abs())
    };
    let mut usable: Vec<Candidate> = candidates.into_iter().filter(has_words).collect();
    // Closest in length first; among equally close ones, timed first.
    usable.sort_by(|a, b| {
        length_off(a)
            .total_cmp(&length_off(b))
            .then(is_timed(b).cmp(&is_timed(a)))
    });
    let matching = usable
        .iter()
        .position(|candidate| length_off(candidate) <= LENGTH_TOLERANCE_SECONDS);
    let (chosen, same_length) = match matching {
        Some(index) => (usable.swap_remove(index), true),
        // Nothing of this length: the order LRCLIB gave (its best match
        // first) is better than the closest length of another song.
        None => (usable.into_iter().next()?, false),
    };
    let synced = chosen
        .synced_lyrics
        .clone()
        .filter(|text| same_length && !text.trim().is_empty());
    let plain = chosen
        .plain_lyrics
        .filter(|text| !text.trim().is_empty())
        .or_else(|| {
            // Only timed words: the words without their times.
            synced_words(chosen.synced_lyrics.as_deref()?)
        })
        .unwrap_or_default();
    Some(OnlineLyrics {
        title: chosen.track_name,
        artist: chosen.artist_name,
        synced,
        plain,
        instrumental: chosen.instrumental,
        by_title_only: false,
    })
}

/// The words of timed lyrics, one line per line, without their times.
fn synced_words(synced: &str) -> Option<String> {
    let lines = crate::parse_timed_lyrics(synced).ok()?;
    Some(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// The title and artist a song file's name suggests: "Artist - Title" is
/// split, any other name is the title alone.
pub fn guess_title_and_artist(song_path: &Path) -> (String, String) {
    let name = crate::stems::song_name(song_path);
    let name = name.replace('_', " ");
    match name.split_once(" - ") {
        Some((artist, title)) if !artist.trim().is_empty() && !title.trim().is_empty() => {
            (title.trim().to_owned(), artist.trim().to_owned())
        }
        _ => (name.trim().to_owned(), String::new()),
    }
}

/// Timed lyrics of the song file (LRC, as LRCLIB gives them) moved to the
/// timeline where the song is `placement`d, keeping only what is heard
/// while the clip plays. A line ends when the next one starts, but stays
/// at most [`LONGEST_LINE_SECONDS`] (LRC has no end times, and a line
/// before an instrumental break would otherwise stay through it).
pub fn timed_lyrics_on_timeline(synced: &str, placement: SongPlacement) -> Vec<LyricLine> {
    let Ok(lines) = crate::parse_timed_lyrics(synced) else {
        return Vec::new();
    };
    let shift = placement.clip_start_seconds - placement.source_offset_seconds;
    let (first, last) = (
        placement.clip_start_seconds,
        placement.clip_start_seconds + placement.clip_duration_seconds,
    );
    lines
        .into_iter()
        .filter_map(|line| {
            let end = line
                .end_seconds
                .min(line.start_seconds + LONGEST_LINE_SECONDS);
            let start = (line.start_seconds + shift).max(first);
            let end = (end + shift).min(last);
            (end > start).then_some(LyricLine {
                text: line.text,
                start_seconds: start,
                end_seconds: end,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str, artist: &str, duration: f64, synced: bool) -> Candidate {
        Candidate {
            track_name: name.to_owned(),
            artist_name: artist.to_owned(),
            duration: Some(duration),
            instrumental: false,
            plain_lyrics: Some(format!("words of {artist}")),
            synced_lyrics: synced.then(|| format!("[00:01.00]words of {artist}")),
        }
    }

    #[test]
    fn the_song_of_the_files_length_is_chosen_and_its_times_trusted() {
        let found = choose(
            vec![
                candidate("Party Favor", "Someone Else", 204.0, true),
                candidate("Party Favor", "Ivycomb", 195.0, true),
                candidate("Party Favor", "Ivycomb", 195.0, false),
            ],
            195.2,
        )
        .unwrap();
        assert_eq!(found.artist, "Ivycomb");
        assert_eq!(found.synced.as_deref(), Some("[00:01.00]words of Ivycomb"));
        assert_eq!(found.plain, "words of Ivycomb");
    }

    #[test]
    fn a_song_of_another_length_gives_only_its_words() {
        let found = choose(
            vec![
                candidate("Song", "First", 100.0, true),
                candidate("Song", "Second", 300.0, true),
            ],
            200.0,
        )
        .unwrap();
        // LRCLIB's own order, not the closest length of a different song.
        assert_eq!(found.artist, "First");
        assert_eq!(found.synced, None);
        assert_eq!(found.plain, "words of First");
    }

    #[test]
    fn songs_without_words_are_skipped() {
        let mut empty = candidate("Song", "Empty", 200.0, false);
        empty.plain_lyrics = Some("  ".to_owned());
        assert_eq!(choose(vec![empty.clone()], 200.0), None);
        // Only timed words: the plain words are taken from them.
        empty.synced_lyrics = Some("[00:01.00]one\n[00:02.00]two".to_owned());
        let found = choose(vec![empty], 200.0).unwrap();
        assert_eq!(found.plain, "one\ntwo");
        assert!(found.synced.is_some());
    }

    #[test]
    fn lrclib_answers_are_read() {
        let answer = r#"[{"id":1,"trackName":"Party Favor","artistName":"Ivycomb",
            "albumName":"Party Favor","duration":195.0,"instrumental":false,
            "plainLyrics":"a\nb","syncedLyrics":null}]"#;
        let candidates: Vec<Candidate> = serde_json::from_str(answer).unwrap();
        assert_eq!(candidates[0].track_name, "Party Favor");
        assert_eq!(candidates[0].synced_lyrics, None);
    }

    /// Asks the real LRCLIB (needs the internet, so only when asked:
    /// `cargo test -p vizx-core lyrics_online -- --ignored`). A title
    /// alone finds the most famous song of that name, even one of a
    /// similar length; with the artist, the right one is found.
    #[test]
    #[ignore = "needs the internet"]
    fn lrclib_finds_a_song_by_title_and_artist() {
        let famous = search("Party Favor", "", 195.2).unwrap();
        assert!(famous.by_title_only);
        let found = search("Party Favor", "Ivycomb", 195.2).unwrap();
        assert!(!found.by_title_only);
        assert!(found.artist.to_lowercase().contains("ivycomb"), "{found:?}");
        let synced = found.synced.expect("timed lyrics");
        assert!(crate::parse_timed_lyrics(&synced).unwrap().len() > 20);
        // An artist LRCLIB has no song of this title by.
        assert!(search("Party Favor", "Nobody Of That Name", 195.2).is_err());
    }

    #[test]
    fn artists_match_ignoring_case_punctuation_and_guests() {
        assert!(same_artist("ivycomb, OodleZz", "Ivycomb"));
        assert!(same_artist("Ivycomb", "ivycomb feat. OodleZz"));
        assert!(!same_artist("Billie Eilish", "Ivycomb"));
        assert!(!same_artist("Billie Eilish", " "));
    }

    #[test]
    fn titles_and_artists_come_from_file_names() {
        assert_eq!(
            guess_title_and_artist(Path::new("C:/Music/Party Favor.mp3")),
            ("Party Favor".to_owned(), String::new())
        );
        assert_eq!(
            guess_title_and_artist(Path::new("/music/Ivycomb - Party_Favor.flac")),
            ("Party Favor".to_owned(), "Ivycomb".to_owned())
        );
    }

    #[test]
    fn timed_lyrics_are_placed_on_the_timeline_and_end_before_breaks() {
        let synced = "[00:10.00]First\n[00:12.00]Second\n[00:50.00]After the break";
        // The song starts at 5 s on the timeline with its first 2 s trimmed.
        let placement = SongPlacement {
            clip_start_seconds: 5.0,
            source_offset_seconds: 2.0,
            clip_duration_seconds: 100.0,
        };
        let lines = timed_lyrics_on_timeline(synced, placement);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].start_seconds, 13.0);
        assert_eq!(lines[0].end_seconds, 15.0);
        // Not until the line after the break: six seconds at most.
        assert_eq!(lines[1].end_seconds, 21.0);
        assert_eq!(lines[2].start_seconds, 53.0);
    }
}
