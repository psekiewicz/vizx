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

/// Recognizes the words sung in `vocals`. `language` is a code such as
/// "en" or "pl", or `None` to let the model tell. `progress` is told how
/// far it is (0 to 1); setting `cancel` stops it.
pub fn recognize_lyrics(
    model: &Path,
    vocals: &DecodedAudio,
    language: Option<&str>,
    cancel: &AtomicBool,
    progress: &dyn Fn(f32),
) -> Result<Vec<RecognizedLine>, String> {
    let samples = mono_at_whisper_rate(vocals);
    if samples.is_empty() {
        return Err("the vocals have no samples".to_owned());
    }
    progress(0.0);
    let context = WhisperContext::new_with_params(model, WhisperContextParameters::default())
        .map_err(|error| format!("could not load the Whisper model: {error}"))?;
    let mut state = context
        .create_state()
        .map_err(|error| format!("could not start Whisper: {error}"))?;

    // Greedy: the most likely word each time. A beam search of 5 is a
    // little more accurate but several times slower on a CPU (half an
    // hour for a song on a 4-core laptop). Where greedy decoding fails,
    // whisper.cpp retries the piece at a higher temperature by itself.
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 2 });
    params.set_language(Some(language.unwrap_or("auto")));
    params.set_token_timestamps(true);
    params.set_n_threads(
        std::thread::available_parallelism().map_or(4, |count| count.get()) as c_int,
    );
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_print_special(false);
    // Text in brackets ("[Music]") and other non-speech marks are left out.
    params.set_suppress_nst(true);

    // The callbacks get plain pointers to `cancel` and `progress`. Both
    // outlive `full`, which is the only time the callbacks are called.
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
        // SAFETY: `user_data` is the `&&dyn Fn(f32)` given below, alive for
        // the whole of `full`.
        let progress = unsafe { &*(user_data as *const &dyn Fn(f32)) };
        progress(percent as f32 / 100.0);
    }
    let progress_ref: &dyn Fn(f32) = progress;
    // SAFETY: the pointers stay valid while `state.full` runs (see above),
    // and the functions match the callback types of whisper.cpp.
    unsafe {
        params.set_abort_callback(Some(should_stop));
        params.set_abort_callback_user_data(cancel as *const AtomicBool as *mut c_void);
        params.set_progress_callback(Some(report));
        params.set_progress_callback_user_data(&progress_ref as *const &dyn Fn(f32) as *mut c_void);
    }

    let outcome = state.full(params, &samples);
    if cancel.load(Ordering::Relaxed) {
        return Err("the recognition was cancelled".to_owned());
    }
    outcome.map_err(|error| format!("Whisper failed: {error}"))?;
    progress(1.0);

    let end_of_text = context.token_eot();
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
            let (start, end) = (data.t0 as f64 / 100.0, data.t1 as f64 / 100.0);
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
        lines.push(RecognizedLine {
            start_seconds: segment.start_timestamp() as f64 / 100.0,
            end_seconds: segment.end_timestamp() as f64 / 100.0,
            words,
        });
    }
    Ok(drop_silent_lines(lines, &samples))
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
fn mono_at_whisper_rate(audio: &DecodedAudio) -> Vec<f32> {
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
            &AtomicBool::new(false),
            &|_| {},
        )
        .unwrap();
        assert!(lines.is_empty(), "{lines:?}");
    }
}
