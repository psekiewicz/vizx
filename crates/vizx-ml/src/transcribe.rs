//! Recognizing the words of a song from its vocals, with the Whisper model
//! (whisper.cpp), on this computer.
//!
//! Whisper is trained on speech; sung words are harder, which is why it is
//! given the separated vocals rather than the whole song. It finds lines
//! ("segments") with a time for every word. Parts where the vocals are
//! silent are dropped: on silence Whisper tends to make up words.

use std::ffi::c_void;
use std::os::raw::c_int;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use vizx_audio::DecodedAudio;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Environment variable with the Whisper model file (a whisper.cpp
/// `ggml-*.bin`). Used during development; an installed VizX looks in
/// `models/` next to its executable.
pub const WHISPER_MODEL_VARIABLE: &str = "VIZX_WHISPER_MODEL";

/// Model file names looked for in `models/` next to the executable, best
/// first.
const WHISPER_MODEL_FILE_NAMES: [&str; 4] = [
    "ggml-large-v3-turbo-q5_0.bin",
    "ggml-large-v3-turbo.bin",
    "ggml-medium.bin",
    "ggml-small.bin",
];

/// The sample rate Whisper works at.
const WHISPER_SAMPLE_RATE: u32 = 16_000;

/// A line is dropped when its vocals are quieter than this share of the
/// song's typical vocal loudness: Whisper hears words in silence.
const SILENT_SHARE: f32 = 0.08;

/// One recognized word and when it is sung, in seconds from the start of
/// the audio.
#[derive(Debug, Clone, PartialEq)]
pub struct RecognizedWord {
    pub text: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

/// One recognized line: its words, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct RecognizedLine {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub words: Vec<RecognizedWord>,
}

impl RecognizedLine {
    /// The words joined with spaces.
    pub fn text(&self) -> String {
        self.words
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Finds the Whisper model: in `models/` next to the VizX executable, or
/// where [`WHISPER_MODEL_VARIABLE`] points. Fails with a message saying
/// where it was looked for.
pub fn find_whisper_model() -> Result<PathBuf, String> {
    let executable_folder = std::env::current_exe()
        .ok()
        .and_then(|path| Some(path.parent()?.to_path_buf()));
    executable_folder
        .iter()
        .flat_map(|folder| {
            WHISPER_MODEL_FILE_NAMES
                .iter()
                .map(move |name| folder.join("models").join(name))
        })
        .chain(std::env::var_os(WHISPER_MODEL_VARIABLE).map(PathBuf::from))
        .find(|path| path.is_file())
        .ok_or_else(|| {
            format!(
                "the Whisper model was not found in models/ next to VizX or at \
                 %{WHISPER_MODEL_VARIABLE}%"
            )
        })
}

/// How hard Whisper tries: faster, or more accurate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RecognitionQuality {
    /// The most likely word each time, no second tries. Sung words that
    /// are unclear are more often wrong, but a song takes minutes.
    Fast,
    /// Unclear pieces are tried again a little more freely.
    #[default]
    Balanced,
    /// Several candidate sentences are weighed against each other (a beam
    /// search). Best on unclear singing, several times slower.
    Accurate,
}

impl RecognitionQuality {
    pub const ALL: [Self; 3] = [Self::Fast, Self::Balanced, Self::Accurate];

    /// Name shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Self::Fast => "Fast",
            Self::Balanced => "Balanced",
            Self::Accurate => "Accurate",
        }
    }

    /// The decoding settings for this quality.
    fn params<'a, 'b>(self) -> FullParams<'a, 'b> {
        let mut params = match self {
            Self::Fast | Self::Balanced => FullParams::new(SamplingStrategy::Greedy { best_of: 2 }),
            Self::Accurate => FullParams::new(SamplingStrategy::BeamSearch {
                beam_size: 5,
                patience: -1.0,
            }),
        };
        // A retry at a higher "temperature" when a piece comes out unsure
        // or repetitive. On singing that happens often and each retry costs
        // as much as the first try, so Fast does not retry.
        params.set_temperature_inc(match self {
            Self::Fast => 0.0,
            Self::Balanced | Self::Accurate => 0.2,
        });
        // Every piece is decoded on its own, not continuing the words of
        // the piece before: on songs, the words before pull Whisper into
        // merging or dropping lines (on ANTIVILLAIN, with them, 28 lines
        // came out instead of 63).
        params.set_no_context(true);
        params
    }
}

/// Recognizes the words sung in `vocals`. `language` is a code such as
/// "en" or "pl", or `None` to let the model tell. `progress` is told how
/// far it is (0 to 1); setting `cancel` stops it.
pub fn recognize_lyrics(
    model: &Path,
    vocals: &DecodedAudio,
    language: Option<&str>,
    quality: RecognitionQuality,
    cancel: &AtomicBool,
    progress: &dyn Fn(f32),
) -> Result<Vec<RecognizedLine>, String> {
    let samples = mono_at_whisper_rate(vocals);
    if samples.is_empty() {
        return Err("the vocals have no samples".to_owned());
    }
    progress(0.0);
    // Only the sung parts go to Whisper: it works through the audio in
    // 30-second windows at a fixed cost each, so the silent parts of the
    // vocals (intro, instrumental breaks) would cost as much as singing,
    // and on silence it makes up words.
    let stretches = sung_stretches(&samples);
    if stretches.is_empty() {
        return Ok(Vec::new());
    }
    let (packed, map) = pack(&samples, &stretches);
    let context = WhisperContext::new_with_params(model, WhisperContextParameters::default())
        .map_err(|error| format!("could not load the Whisper model: {error}"))?;
    let mut state = context
        .create_state()
        .map_err(|error| format!("could not start Whisper: {error}"))?;

    // The callbacks get plain pointers to `cancel` and to the progress of
    // the current piece. Both outlive `full`, which is the only time the
    // callbacks are called.
    struct PieceProgress<'a> {
        progress: &'a dyn Fn(f32),
        done: f32,
        share: f32,
    }
    unsafe extern "C" fn should_stop(user_data: *mut c_void) -> bool {
        // SAFETY: `user_data` is the `&AtomicBool` given below, alive for
        // the whole of `full`.
        unsafe { (*(user_data as *const AtomicBool)).load(Ordering::Relaxed) }
    }
    unsafe extern "C" fn report(
        _: *mut whisper_rs::WhisperSysContext,
        _: *mut whisper_rs::WhisperSysState,
        percent: c_int,
        user_data: *mut c_void,
    ) {
        // SAFETY: `user_data` is the `&PieceProgress` given below, alive
        // for the whole of `full`.
        let piece = unsafe { &*(user_data as *const PieceProgress<'_>) };
        (piece.progress)(piece.done + piece.share * percent as f32 / 100.0);
    }

    // Whisper hears the audio in 30-second windows, and each window costs
    // the same, however much of it is used. Left to itself it starts a new
    // window after every line it finds, so a song costs many more windows
    // than it has 30-second stretches. Cut into pieces that fill a window,
    // each piece costs one.
    let pieces = window_pieces(&packed);
    let end_of_text = context.token_eot();
    let rate = f64::from(WHISPER_SAMPLE_RATE);
    let mut lines = Vec::new();
    for (index, piece) in pieces.iter().enumerate() {
        let mut params = quality.params();
        params.set_language(Some(language.unwrap_or("auto")));
        params.set_token_timestamps(true);
        params.set_n_threads(thread_count());
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_print_special(false);
        // Text in brackets ("[Music]") and other non-speech marks are left
        // out.
        params.set_suppress_nst(true);
        let piece_progress = PieceProgress {
            progress,
            done: index as f32 / pieces.len() as f32,
            share: 1.0 / pieces.len() as f32,
        };
        // SAFETY: the pointers stay valid while `state.full` runs (see
        // above), and the functions match whisper.cpp's callback types.
        unsafe {
            params.set_abort_callback(Some(should_stop));
            params.set_abort_callback_user_data(cancel as *const AtomicBool as *mut c_void);
            params.set_progress_callback(Some(report));
            params.set_progress_callback_user_data(
                &piece_progress as *const PieceProgress<'_> as *mut c_void,
            );
        }
        let outcome = state.full(params, &packed[piece.clone()]);
        if cancel.load(Ordering::Relaxed) {
            return Err("the recognition was cancelled".to_owned());
        }
        outcome.map_err(|error| format!("Whisper failed: {error}"))?;
        let offset = piece.start as f64 / rate;
        lines.extend(collect_lines(&state, end_of_text, |seconds| {
            original_time(offset + seconds, &map)
        }));
    }
    progress(1.0);
    let lines = lines.into_iter().flat_map(split_long_line).collect();
    Ok(drop_silent_lines(lines, &samples))
}

/// Most words a subtitle line keeps; longer lines are split.
const LONGEST_LINE_WORDS: usize = 11;

/// `line` as subtitle-sized lines: a line of more than
/// [`LONGEST_LINE_WORDS`] words is split where a word ends a sentence or a
/// clause, or else at the longest pause between words, nearest the middle.
fn split_long_line(line: RecognizedLine) -> Vec<RecognizedLine> {
    if line.words.len() <= LONGEST_LINE_WORDS {
        return vec![line];
    }
    let count = line.words.len();
    let middle = count as f64 / 2.0;
    // After which word to split: punctuation counts as a long pause, and
    // places near the middle are preferred.
    let split_after = (1..count - 1)
        .max_by(|a, b| {
            let score = |index: usize| {
                let word = &line.words[index];
                let pause = line.words[index + 1].start_seconds - word.end_seconds;
                let punctuation = if word.text.ends_with([',', '.', '!', '?', ';']) {
                    1.0
                } else {
                    0.0
                };
                let off_middle = (index as f64 + 1.0 - middle).abs() / count as f64;
                // A pause of a quarter second counts as much as a comma.
                pause.max(0.0) * 4.0 + punctuation - off_middle * 0.5
            };
            score(*a).total_cmp(&score(*b))
        })
        .unwrap_or(count / 2);
    let mut first = line.words;
    let second = first.split_off(split_after + 1);
    let part = |words: Vec<RecognizedWord>| RecognizedLine {
        start_seconds: words[0].start_seconds,
        end_seconds: words.last().map_or(0.0, |word| word.end_seconds),
        words,
    };
    let mut lines = split_long_line(part(first));
    lines.extend(split_long_line(part(second)));
    lines
}

/// The lines Whisper found in the piece it last heard, with their times
/// turned into times of the vocals by `to_vocals` (from seconds into the
/// piece).
fn collect_lines(
    state: &whisper_rs::WhisperState,
    end_of_text: whisper_rs::WhisperTokenId,
    to_vocals: impl Fn(f64) -> f64,
) -> Vec<RecognizedLine> {
    let mut lines = Vec::new();
    for segment in state.as_iter() {
        let mut words: Vec<RecognizedWord> = Vec::new();
        for index in 0..segment.n_tokens() {
            let Some(token) = segment.get_token(index) else {
                continue;
            };
            let data = token.token_data();
            // Timestamps, the end mark and other special tokens.
            if data.id >= end_of_text {
                continue;
            }
            let Ok(piece) = token.to_str_lossy() else {
                continue;
            };
            let (start, end) = (
                to_vocals(data.t0 as f64 / 100.0),
                to_vocals(data.t1 as f64 / 100.0),
            );
            // A piece that starts with a space starts a new word; the
            // others continue the word before ("bro" + "ken").
            match words.last_mut() {
                Some(word) if !piece.starts_with(' ') => {
                    word.text.push_str(&piece);
                    word.end_seconds = end.max(word.end_seconds);
                }
                _ => {
                    let text = piece.trim();
                    if !text.is_empty() {
                        words.push(RecognizedWord {
                            text: text.to_owned(),
                            start_seconds: start,
                            end_seconds: end.max(start),
                        });
                    }
                }
            }
        }
        words.retain(|word| word.text.chars().any(char::is_alphanumeric));
        if words.is_empty() {
            continue;
        }
        // The words' own times are finer than the line's, which Whisper
        // rounds; the line lasts from its first word to its last.
        let segment_end = to_vocals(segment.end_timestamp() as f64 / 100.0);
        let start_seconds = words[0].start_seconds;
        let end_seconds = words
            .last()
            .map_or(segment_end, |word| word.end_seconds)
            .max(start_seconds);
        lines.push(RecognizedLine {
            start_seconds,
            end_seconds,
            words,
        });
    }
    lines
}

/// Longest piece given to Whisper at once, in samples: a little under its
/// 30-second window.
const LONGEST_PIECE: usize = WHISPER_SAMPLE_RATE as usize * 28;
/// A piece is cut at the quietest moment of its last this many samples,
/// so the cut falls between words.
const CUT_SEARCH: usize = WHISPER_SAMPLE_RATE as usize * 6;

/// `samples` cut into pieces that each fill most of a Whisper window, cut
/// at quiet moments.
fn window_pieces(samples: &[f32]) -> Vec<std::ops::Range<usize>> {
    let mut pieces = Vec::new();
    let mut start = 0;
    while samples.len() - start > LONGEST_PIECE {
        let latest = start + LONGEST_PIECE;
        let earliest = latest - CUT_SEARCH;
        // The quietest tenth of a second between them.
        let cut = (earliest..latest - PIECE)
            .step_by(PIECE / 2)
            .min_by(|a, b| {
                let energy = |at: usize| samples[at..at + PIECE].iter().map(|s| s * s).sum::<f32>();
                energy(*a).total_cmp(&energy(*b))
            })
            .map_or(latest, |at| at + PIECE / 2);
        pieces.push(start..cut);
        start = cut;
    }
    if start < samples.len() {
        pieces.push(start..samples.len());
    }
    pieces
}

/// Analysed piece of the vocals when looking for singing, in samples
/// (a tenth of a second).
const PIECE: usize = WHISPER_SAMPLE_RATE as usize / 10;
/// Sung parts are widened by this many pieces on each side, so the first
/// and last sounds of a line are kept.
const WIDEN_PIECES: usize = 5;
/// Gaps shorter than this many pieces are kept, so a line is not cut up.
const KEEP_GAP_PIECES: usize = 15;
/// Silence put between the sung parts, in samples, so Whisper hears them
/// as separate phrases.
const PAUSE: usize = WHISPER_SAMPLE_RATE as usize * 2 / 5;

/// The sample ranges of `samples` where something is sung: louder than a
/// small share of the typical vocal loudness, widened a little, with short
/// gaps closed.
fn sung_stretches(samples: &[f32]) -> Vec<std::ops::Range<usize>> {
    let levels: Vec<f32> = samples
        .chunks(PIECE)
        .map(|piece| (piece.iter().map(|s| s * s).sum::<f32>() / piece.len() as f32).sqrt())
        .collect();
    let mut audible: Vec<f32> = levels
        .iter()
        .copied()
        .filter(|level| *level > 1e-4)
        .collect();
    if audible.is_empty() {
        return Vec::new();
    }
    audible.sort_by(f32::total_cmp);
    let threshold = audible[audible.len() / 2] * SILENT_SHARE;

    let mut sung = vec![false; levels.len()];
    for (index, level) in levels.iter().enumerate() {
        if *level >= threshold {
            let from = index.saturating_sub(WIDEN_PIECES);
            let to = (index + WIDEN_PIECES + 1).min(levels.len());
            sung[from..to].fill(true);
        }
    }
    let mut stretches: Vec<std::ops::Range<usize>> = Vec::new();
    for (index, is_sung) in sung.iter().enumerate() {
        if !is_sung {
            continue;
        }
        match stretches.last_mut() {
            Some(last) if index - last.end <= KEEP_GAP_PIECES => last.end = index + 1,
            _ => stretches.push(index..index + 1),
        }
    }
    stretches
        .into_iter()
        .map(|pieces| pieces.start * PIECE..(pieces.end * PIECE).min(samples.len()))
        .collect()
}

/// Where a part of the packed audio came from: `length` seconds starting
/// at `packed` in the packed audio and at `original` in the vocals.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Moved {
    packed: f64,
    original: f64,
    length: f64,
}

/// The sung stretches one after another, with a short pause between them,
/// and where each one came from.
fn pack(samples: &[f32], stretches: &[std::ops::Range<usize>]) -> (Vec<f32>, Vec<Moved>) {
    let rate = f64::from(WHISPER_SAMPLE_RATE);
    let mut packed = Vec::new();
    let mut map = Vec::new();
    for stretch in stretches {
        if !packed.is_empty() {
            packed.extend(std::iter::repeat_n(0.0, PAUSE));
        }
        map.push(Moved {
            packed: packed.len() as f64 / rate,
            original: stretch.start as f64 / rate,
            length: stretch.len() as f64 / rate,
        });
        packed.extend_from_slice(&samples[stretch.clone()]);
    }
    (packed, map)
}

/// The moment of the vocals that `packed_seconds` of the packed audio
/// stands for. A moment in a pause belongs to the end of the stretch
/// before it.
fn original_time(packed_seconds: f64, map: &[Moved]) -> f64 {
    let index = map
        .partition_point(|moved| moved.packed <= packed_seconds)
        .saturating_sub(1);
    map.get(index).map_or(packed_seconds, |moved| {
        moved.original + (packed_seconds - moved.packed).clamp(0.0, moved.length)
    })
}

/// How many threads Whisper uses: all the processor offers.
fn thread_count() -> c_int {
    std::thread::available_parallelism().map_or(4, |count| count.get()) as c_int
}

/// The lines whose vocals are loud enough to hold words.
fn drop_silent_lines(lines: Vec<RecognizedLine>, samples: &[f32]) -> Vec<RecognizedLine> {
    let loudness = |start: f64, end: f64| -> f32 {
        let rate = f64::from(WHISPER_SAMPLE_RATE);
        let first = ((start * rate) as usize).min(samples.len());
        let last = ((end * rate) as usize).clamp(first, samples.len());
        let part = &samples[first..last];
        if part.is_empty() {
            return 0.0;
        }
        (part.iter().map(|sample| sample * sample).sum::<f32>() / part.len() as f32).sqrt()
    };
    // The typical loudness of the sung parts: the median of one-second
    // stretches that are not nearly silent.
    let mut stretches: Vec<f32> = (0..samples.len() / WHISPER_SAMPLE_RATE as usize)
        .map(|second| loudness(second as f64, second as f64 + 1.0))
        .filter(|value| *value > 1e-4)
        .collect();
    if stretches.is_empty() {
        return Vec::new();
    }
    stretches.sort_by(f32::total_cmp);
    let typical = stretches[stretches.len() / 2];
    lines
        .into_iter()
        .filter(|line| loudness(line.start_seconds, line.end_seconds) >= typical * SILENT_SHARE)
        .collect()
}

/// `audio` as one channel at Whisper's rate. The channels are averaged; a
/// different rate is converted by averaging the samples that fall into
/// each new one (which also keeps out what is too high to be heard at
/// 16 kHz), or by straight-line interpolation when there are fewer.
pub(crate) fn mono_at_whisper_rate(audio: &DecodedAudio) -> Vec<f32> {
    let channels = audio.channel_count.max(1);
    let mono: Vec<f32> = audio
        .samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();
    if audio.sample_rate == WHISPER_SAMPLE_RATE || mono.is_empty() {
        return mono;
    }
    let step = f64::from(audio.sample_rate) / f64::from(WHISPER_SAMPLE_RATE);
    let count = (mono.len() as f64 / step).floor() as usize;
    (0..count)
        .map(|index| {
            let from = index as f64 * step;
            if step <= 1.0 {
                let before = from as usize;
                let after = (before + 1).min(mono.len() - 1);
                let fraction = (from - before as f64) as f32;
                return mono[before] * (1.0 - fraction) + mono[after] * fraction;
            }
            let first = from as usize;
            let last = (((index + 1) as f64 * step) as usize).clamp(first + 1, mono.len());
            mono[first..last].iter().sum::<f32>() / (last - first) as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_is_made_mono_at_sixteen_kilohertz() {
        let stereo = DecodedAudio {
            sample_rate: 48_000,
            channel_count: 2,
            // Left 1, right 0: averages to 0.5.
            samples: [1.0, 0.0].repeat(48_000),
        };
        let mono = mono_at_whisper_rate(&stereo);
        assert_eq!(mono.len(), 16_000);
        assert!(mono.iter().all(|sample| (sample - 0.5).abs() < 1e-6));

        let slow = DecodedAudio {
            sample_rate: 8_000,
            channel_count: 1,
            samples: vec![0.0, 1.0, 0.0, 1.0],
        };
        assert_eq!(mono_at_whisper_rate(&slow)[..3], [0.0, 0.5, 1.0]);
    }

    #[test]
    fn only_the_sung_parts_go_to_whisper_and_times_are_moved_back() {
        let rate = WHISPER_SAMPLE_RATE as usize;
        // 5 s of silence, 2 s of singing, 10 s of silence, 3 s of singing.
        let mut samples = vec![0.0f32; 5 * rate];
        samples.extend(vec![0.3; 2 * rate]);
        samples.extend(vec![0.0; 10 * rate]);
        samples.extend(vec![0.3; 3 * rate]);

        let stretches = sung_stretches(&samples);
        assert_eq!(stretches.len(), 2, "{stretches:?}");
        // Widened by half a second on each side.
        assert_eq!(stretches[0], 4 * rate + rate / 2..7 * rate + rate / 2);

        let (packed, map) = pack(&samples, &stretches);
        // 3 s and 3.5 s of sung parts, with a 0.4 s pause between them.
        assert_eq!(packed.len(), 3 * rate + PAUSE + 3 * rate + rate / 2);
        // 1 s into the packed audio is 5.5 s into the vocals; the start of
        // the second part is 16.5 s; a moment in the pause stays at the end
        // of the first part.
        assert!((original_time(1.0, &map) - 5.5).abs() < 1e-9);
        assert!((original_time(3.4, &map) - 16.5).abs() < 1e-9);
        assert!((original_time(3.2, &map) - 7.5).abs() < 1e-9);

        assert!(sung_stretches(&vec![0.0; 10 * rate]).is_empty());
    }

    #[test]
    fn long_vocals_are_cut_into_window_sized_pieces_at_quiet_moments() {
        let rate = WHISPER_SAMPLE_RATE as usize;
        // 70 s of sound with a quiet moment at 25 s and at 50 s.
        let mut samples = vec![0.3f32; 70 * rate];
        samples[25 * rate..25 * rate + rate / 5].fill(0.0);
        samples[50 * rate..50 * rate + rate / 5].fill(0.0);
        let pieces = window_pieces(&samples);
        assert_eq!(pieces.len(), 3, "{pieces:?}");
        let cut = pieces[0].end as f64 / rate as f64;
        assert!((25.0..25.2).contains(&cut), "cut at {cut}");
        assert_eq!(pieces[1].start, pieces[0].end);
        assert_eq!(pieces[2].end, samples.len());
        assert!(pieces.iter().all(|piece| piece.len() <= LONGEST_PIECE));
        // Short vocals are one piece.
        assert_eq!(window_pieces(&samples[..10 * rate]), vec![0..10 * rate]);
    }

    #[test]
    fn long_lines_are_split_at_punctuation_or_pauses() {
        let word = |text: &str, start: f64| RecognizedWord {
            text: text.to_owned(),
            start_seconds: start,
            end_seconds: start + 0.3,
        };
        let sentence =
            "Oh, their suits aren't made for me, but this isn't really quite my cup of tea.";
        let words: Vec<RecognizedWord> = sentence
            .split(' ')
            .enumerate()
            .map(|(index, text)| word(text, index as f64 * 0.4))
            .collect();
        let line = RecognizedLine {
            start_seconds: 0.0,
            end_seconds: 10.0,
            words,
        };
        let texts: Vec<String> = split_long_line(line)
            .iter()
            .map(RecognizedLine::text)
            .collect();
        assert_eq!(
            texts,
            [
                "Oh, their suits aren't made for me,",
                "but this isn't really quite my cup of tea."
            ]
        );
        // Short lines stay as they are.
        let short = RecognizedLine {
            start_seconds: 0.0,
            end_seconds: 1.0,
            words: vec![word("Hello", 0.0)],
        };
        assert_eq!(split_long_line(short.clone()), vec![short]);
    }

    #[test]
    fn lines_in_silent_vocals_are_dropped() {
        // One second of singing, one of silence.
        let mut samples = vec![0.3f32; 16_000];
        samples.extend(vec![0.0; 16_000]);
        let line = |start: f64| RecognizedLine {
            start_seconds: start,
            end_seconds: start + 0.8,
            words: vec![RecognizedWord {
                text: "word".to_owned(),
                start_seconds: start,
                end_seconds: start + 0.5,
            }],
        };
        let kept = drop_silent_lines(vec![line(0.1), line(1.1)], &samples);
        assert_eq!(kept, vec![line(0.1)]);
        assert_eq!(kept[0].text(), "word");
    }

    /// Runs the real model when it is installed: a few seconds of silence
    /// give no words (and nothing made up).
    #[test]
    fn silence_gives_no_words_when_the_model_is_installed() {
        let Ok(model) = find_whisper_model() else {
            eprintln!("skipped: the Whisper model is not installed");
            return;
        };
        let silence = DecodedAudio {
            sample_rate: 16_000,
            channel_count: 1,
            samples: vec![0.0; 16_000 * 3],
        };
        let lines = recognize_lyrics(
            &model,
            &silence,
            Some("en"),
            RecognitionQuality::Fast,
            &AtomicBool::new(false),
            &|_| {},
        )
        .unwrap();
        assert!(lines.is_empty(), "{lines:?}");
    }
}
