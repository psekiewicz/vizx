//! Recognizing the lyrics of a song in the background: first the vocals
//! are separated from the music (or the stems made before are used), then
//! the Whisper model listens to the vocals alone and writes down the words
//! with their times.
//!
//! The result is timed lyrics in LRC form, which the user can correct
//! before they become subtitles.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

use vizx_audio::DecodedAudio;

use crate::LyricLine;
use crate::stems::{STEM_NAMES, StemEngine, existing_stems, song_name, stem_folder};

/// Languages offered for recognition: (code, name). `None` lets the model
/// tell from the song.
pub const RECOGNITION_LANGUAGES: [(Option<&str>, &str); 9] = [
    (None, "Detect"),
    (Some("en"), "English"),
    (Some("pl"), "Polish"),
    (Some("de"), "German"),
    (Some("es"), "Spanish"),
    (Some("fr"), "French"),
    (Some("it"), "Italian"),
    (Some("ja"), "Japanese"),
    (Some("ko"), "Korean"),
];

/// A recognized line, in project time, with a time for each word.
#[derive(Debug, Clone, PartialEq)]
pub struct RecognizedLyric {
    pub line: LyricLine,
    /// Each word with its start and end, in seconds of project time.
    pub words: Vec<(String, f64, f64)>,
}

/// Where the song lies on the timeline: the audio's moment `t` is heard
/// at `clip_start_seconds + t - source_offset_seconds`, while the clip
/// lasts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SongPlacement {
    pub clip_start_seconds: f64,
    pub source_offset_seconds: f64,
    pub clip_duration_seconds: f64,
}

/// What a recognition is doing now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecognitionStep {
    SeparatingVocals,
    Listening,
}

/// A recognition running in the background.
pub struct LyricsRecognition {
    song_name: String,
    /// Progress in thousandths over both steps.
    progress: Arc<AtomicU32>,
    is_listening: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<Result<Vec<RecognizedLyric>, String>>,
}

impl LyricsRecognition {
    /// Starts recognizing the words of `audio`, the decoded file at
    /// `song_path`, placed on the timeline as `placement`. `language` is a
    /// code such as "en", or `None` to detect it. Fails at once if the
    /// Whisper model is not installed.
    pub fn start(
        audio: Arc<DecodedAudio>,
        song_path: &Path,
        placement: SongPlacement,
        language: Option<String>,
        quality: vizx_ml::RecognitionQuality,
        on_progress: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let model = vizx_ml::find_whisper_model()?;
        let engine = StemEngine::best_available();
        let folder = stem_folder(song_path, &engine);
        let name = song_name(song_path);
        let progress = Arc::new(AtomicU32::new(0));
        let is_listening = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, result) = mpsc::channel();

        let shared = (
            Arc::clone(&progress),
            Arc::clone(&is_listening),
            Arc::clone(&cancel),
            name.clone(),
        );
        thread::spawn(move || {
            let (progress, is_listening, cancel, name) = shared;
            let on_progress = Arc::new(on_progress);
            let report = |base: f32, share: f32| {
                let progress = Arc::clone(&progress);
                let on_progress = Arc::clone(&on_progress);
                move |part: f32| {
                    let total = base + share * part.clamp(0.0, 1.0);
                    progress.store((total * 1000.0) as u32, Ordering::Relaxed);
                    on_progress();
                }
            };
            let outcome = (|| {
                let vocals = vocals(&audio, &folder, &name, &engine, &cancel, &report(0.0, 0.5))?;
                is_listening.store(true, Ordering::Relaxed);
                let lines = vizx_ml::recognize_lyrics(
                    &model,
                    &vocals,
                    language.as_deref(),
                    quality,
                    &cancel,
                    &report(0.5, 0.5),
                )?;
                Ok(place_on_timeline(&lines, placement))
            })();
            let _ = sender.send(outcome);
        });

        Ok(Self {
            song_name: name,
            progress,
            is_listening,
            cancel,
            result,
        })
    }

    /// The song's name, for messages.
    pub fn song_name(&self) -> &str {
        &self.song_name
    }

    /// How far it is over both steps, from 0 to 1.
    pub fn progress(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.0
    }

    pub fn step(&self) -> RecognitionStep {
        if self.is_listening.load(Ordering::Relaxed) {
            RecognitionStep::Listening
        } else {
            RecognitionStep::SeparatingVocals
        }
    }

    /// The recognized lines once it has ended, or why it failed. `None`
    /// while it is still running.
    pub fn finished(&self) -> Option<Result<Vec<RecognizedLyric>, String>> {
        match self.result.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("the recognition stopped unexpectedly".to_owned()))
            }
        }
    }
}

impl Drop for LyricsRecognition {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// The song's vocals: from the stems made before, or separated now (and
/// the stems written, so the next time is quick).
pub(crate) fn vocals(
    audio: &DecodedAudio,
    folder: &Path,
    name: &str,
    engine: &StemEngine,
    cancel: &AtomicBool,
    progress: &dyn Fn(f32),
) -> Result<DecodedAudio, String> {
    if let Some(stems) = existing_stems(folder, name) {
        let vocals: &PathBuf = stems
            .iter()
            .find(|(stem, _)| stem == STEM_NAMES[0])
            .map(|(_, path)| path)
            .ok_or("the vocals stem is missing")?;
        progress(1.0);
        return vizx_audio::decode_audio_file(vocals)
            .map_err(|error| format!("could not read {}: {error}", vocals.display()));
    }
    let stems = engine.separate(audio, cancel, progress)?;
    stems
        .write_wav_files(folder, name)
        .map_err(|error| format!("could not write the stems: {error}"))?;
    Ok(stems.vocals)
}

/// Moves recognized lines from the audio's time to the timeline's, keeping
/// only what is heard while the clip plays.
fn place_on_timeline(
    lines: &[vizx_ml::RecognizedLine],
    placement: SongPlacement,
) -> Vec<RecognizedLyric> {
    let shift = placement.clip_start_seconds - placement.source_offset_seconds;
    let (first, last) = (
        placement.clip_start_seconds,
        placement.clip_start_seconds + placement.clip_duration_seconds,
    );
    lines
        .iter()
        .filter_map(|line| {
            let start = (line.start_seconds + shift).max(first);
            let end = (line.end_seconds + shift).min(last);
            if end <= start {
                return None;
            }
            Some(RecognizedLyric {
                line: LyricLine {
                    text: line.text(),
                    start_seconds: start,
                    end_seconds: end,
                },
                words: line
                    .words
                    .iter()
                    .map(|word| {
                        (
                            word.text.clone(),
                            word.start_seconds + shift,
                            word.end_seconds + shift,
                        )
                    })
                    .collect(),
            })
        })
        .collect()
}

/// The lines as LRC text (`[01:23.45]words`), easy to correct by hand and
/// read back with `parse_timed_lyrics`. A line that ends well before the
/// next one starts is followed by a time without words, which ends it.
pub fn recognized_lyrics_to_lrc(lyrics: &[RecognizedLyric]) -> String {
    let stamp = |seconds: f64| {
        let centiseconds = (seconds.max(0.0) * 100.0).round() as u64;
        format!(
            "[{:02}:{:02}.{:02}]",
            centiseconds / 6000,
            centiseconds / 100 % 60,
            centiseconds % 100
        )
    };
    let mut text = String::new();
    for (index, lyric) in lyrics.iter().enumerate() {
        text.push_str(&stamp(lyric.line.start_seconds));
        text.push_str(&lyric.line.text);
        text.push('\n');
        let next_start = lyrics
            .get(index + 1)
            .map_or(f64::INFINITY, |next| next.line.start_seconds);
        if next_start - lyric.line.end_seconds > 0.5 {
            text.push_str(&stamp(lyric.line.end_seconds));
            text.push('\n');
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use vizx_ml::{RecognizedLine, RecognizedWord};

    fn line(start: f64, end: f64, words: &[&str]) -> RecognizedLine {
        let step = (end - start) / words.len() as f64;
        RecognizedLine {
            start_seconds: start,
            end_seconds: end,
            words: words
                .iter()
                .enumerate()
                .map(|(index, text)| RecognizedWord {
                    text: (*text).to_owned(),
                    start_seconds: start + step * index as f64,
                    end_seconds: start + step * (index + 1) as f64,
                })
                .collect(),
        }
    }

    #[test]
    fn lines_are_moved_to_the_timeline_and_written_as_lrc() {
        // The song starts at 10 s on the timeline, its first 2 s trimmed
        // off, and plays for 20 s.
        let placement = SongPlacement {
            clip_start_seconds: 10.0,
            source_offset_seconds: 2.0,
            clip_duration_seconds: 20.0,
        };
        let lines = [
            line(0.5, 1.5, &["trimmed", "off"]),
            line(3.0, 5.0, &["Hello", "there"]),
            line(5.2, 7.0, &["second", "line"]),
            line(12.0, 14.0, &["after", "a", "pause"]),
        ];
        let placed = place_on_timeline(&lines, placement);
        let texts: Vec<&str> = placed
            .iter()
            .map(|lyric| lyric.line.text.as_str())
            .collect();
        assert_eq!(texts, ["Hello there", "second line", "after a pause"]);
        assert_eq!(placed[0].line.start_seconds, 11.0);
        assert_eq!(placed[0].words[1].1, 12.0);

        let lrc = recognized_lyrics_to_lrc(&placed);
        assert_eq!(
            lrc,
            "[00:11.00]Hello there\n\
             [00:13.20]second line\n\
             [00:15.00]\n\
             [00:20.00]after a pause\n\
             [00:22.00]\n"
        );
        // It reads back as the same lines.
        let read = crate::parse_timed_lyrics(&lrc).unwrap();
        assert_eq!(read.len(), 3);
        assert_eq!(read[1].text, "second line");
        assert!((read[1].end_seconds - 15.0).abs() < 1e-9);
    }
}
