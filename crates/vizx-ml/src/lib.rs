//! Local machine learning for VizX: splitting a song into stems (vocals,
//! drums, bass, other) with the HTDemucs model, and recognizing the sung
//! words with the Whisper model (see [`transcribe`]).
//!
//! Everything runs on this computer. The model is an ONNX file and runs in
//! ONNX Runtime, a library that is loaded when it is first needed (it is not
//! linked into VizX and not downloaded by the build). Both are found in
//! folders that can be configured, see [`find_ml_setup`]. Without them, VizX
//! falls back to its built-in separation (`vizx_audio::separate_stems`).

pub mod speakers;
pub mod transcribe;

pub use speakers::{SPEAKER_MODEL_VARIABLE, SpeakerModel, find_speaker_model, identify_singers};

pub use transcribe::{
    RecognitionQuality, RecognizedLine, RecognizedWord, WHISPER_MODEL_VARIABLE, find_whisper_model,
    recognize_lyrics,
};

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use ort::environment::Environment;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Tensor;
use vizx_audio::{DecodedAudio, Stems};

/// Environment variable with ONNX Runtime's folder (the one with `lib/`,
/// as unpacked from its release archive). Used during development; an
/// installed VizX looks next to its executable.
pub const RUNTIME_DIR_VARIABLE: &str = "VIZX_ONNXRUNTIME_DIR";
/// Environment variable with the HTDemucs model file (`.onnx`).
pub const MODEL_VARIABLE: &str = "VIZX_DEMUCS_MODEL";

#[cfg(windows)]
const RUNTIME_FILE_NAME: &str = "onnxruntime.dll";
#[cfg(target_os = "macos")]
const RUNTIME_FILE_NAME: &str = "libonnxruntime.dylib";
#[cfg(all(not(windows), not(target_os = "macos")))]
const RUNTIME_FILE_NAME: &str = "libonnxruntime.so";

/// Model file names looked for next to the executable, in `models/`.
const MODEL_FILE_NAMES: [&str; 2] = ["htdemucs.onnx", "htdemucs_fp16weights.onnx"];

/// The sample rate HTDemucs works at.
pub const MODEL_SAMPLE_RATE: u32 = 44_100;
/// Samples per channel the model takes at once (7.8 seconds).
const SEGMENT: usize = 343_980;
/// How much neighbouring segments overlap; they are cross-faded there.
const OVERLAP: usize = SEGMENT / 4;

/// Where the files for machine learning are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MlSetup {
    /// The ONNX Runtime library.
    pub runtime_library: PathBuf,
    /// The HTDemucs model.
    pub model: PathBuf,
}

/// Finds ONNX Runtime and the HTDemucs model: next to the VizX executable
/// (`onnxruntime.dll` and `models/htdemucs.onnx`), or where the
/// environment variables [`RUNTIME_DIR_VARIABLE`] and [`MODEL_VARIABLE`]
/// point. Fails with a message saying what is missing.
pub fn find_ml_setup() -> Result<MlSetup, String> {
    let executable_folder = std::env::current_exe()
        .ok()
        .and_then(|path| Some(path.parent()?.to_path_buf()));
    let runtime_library = find_onnx_runtime()?;

    let model_candidates = executable_folder
        .iter()
        .flat_map(|folder| {
            MODEL_FILE_NAMES
                .iter()
                .map(move |name| folder.join("models").join(name))
        })
        .chain(std::env::var_os(MODEL_VARIABLE).map(PathBuf::from));
    let model = first_file(model_candidates).ok_or_else(|| {
        format!("the HTDemucs model was not found in models/ next to VizX or at %{MODEL_VARIABLE}%")
    })?;

    Ok(MlSetup {
        runtime_library,
        model,
    })
}

/// Finds the ONNX Runtime library: next to the VizX executable, or in
/// [`RUNTIME_DIR_VARIABLE`]`/lib`.
pub fn find_onnx_runtime() -> Result<PathBuf, String> {
    let executable_folder = std::env::current_exe()
        .ok()
        .and_then(|path| Some(path.parent()?.to_path_buf()));
    let candidates = executable_folder
        .iter()
        .map(|folder| folder.join(RUNTIME_FILE_NAME))
        .chain(
            std::env::var_os(RUNTIME_DIR_VARIABLE)
                .map(|folder| PathBuf::from(folder).join("lib").join(RUNTIME_FILE_NAME)),
        );
    first_file(candidates).ok_or_else(|| {
        format!(
            "ONNX Runtime ({RUNTIME_FILE_NAME}) was not found next to VizX or in \
             %{RUNTIME_DIR_VARIABLE}%/lib"
        )
    })
}

fn first_file(mut candidates: impl Iterator<Item = PathBuf>) -> Option<PathBuf> {
    candidates.find(|path| path.is_file())
}

/// ONNX Runtime, loaded once for the whole program.
fn environment(runtime_library: &Path) -> Result<&'static Environment, String> {
    static ENVIRONMENT: OnceLock<Result<Environment, String>> = OnceLock::new();
    ENVIRONMENT
        .get_or_init(|| {
            ort::init_from(runtime_library)
                .map_err(|error| format!("could not load ONNX Runtime: {error}"))?
                .with_name("vizx")
                .build()
                .map_err(|error| format!("could not start ONNX Runtime: {error}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// The HTDemucs model, ready to separate songs.
pub struct DemucsModel {
    session: Session,
}

impl DemucsModel {
    /// Loads the model (this takes a few seconds).
    pub fn load(setup: &MlSetup) -> Result<Self, String> {
        let environment = environment(&setup.runtime_library)?;
        let threads = std::thread::available_parallelism().map_or(4, |count| count.get());
        let session = Session::builder(environment)
            .and_then(|builder| {
                builder
                    .with_optimization_level(GraphOptimizationLevel::Level3)
                    .map_err(Into::into)
            })
            .and_then(|builder| builder.with_intra_threads(threads).map_err(Into::into))
            .and_then(|mut builder| builder.commit_from_file(&setup.model))
            .map_err(|error| format!("could not load the model: {error}"))?;
        Ok(Self { session })
    }

    /// Splits `audio` into stems. `progress` is told how far it is (0 to
    /// 1) after every piece; setting `cancel` stops it between pieces.
    ///
    /// The model works on 7.8-second pieces at 44.1 kHz. The song is
    /// converted to that rate, cut into pieces that overlap by a quarter,
    /// and the pieces' stems are cross-faded where they overlap. The stems
    /// come out at 44.1 kHz.
    pub fn separate(
        &mut self,
        audio: &DecodedAudio,
        cancel: &AtomicBool,
        progress: &dyn Fn(f32),
    ) -> Result<Stems, String> {
        let [left, right] = stereo_at_model_rate(audio);
        let total = left.len();
        if total == 0 {
            return Err("the audio has no samples".to_owned());
        }
        let stride = SEGMENT - OVERLAP;
        let pieces = total.div_ceil(stride).max(1);
        // A rising and falling edge for the cross-fade; never quite 0, so
        // the first and last samples keep some weight.
        let fade: Vec<f32> = (0..OVERLAP)
            .map(|index| (index + 1) as f32 / (OVERLAP + 1) as f32)
            .collect();

        // Model order: drums, bass, other, vocals; each left then right.
        let mut sums = vec![vec![0.0f32; total]; 8];
        let mut weights = vec![0.0f32; total];
        let mut input = vec![0.0f32; 2 * SEGMENT];
        for piece in 0..pieces {
            if cancel.load(Ordering::Relaxed) {
                return Err("the separation was cancelled".to_owned());
            }
            progress(piece as f32 / pieces as f32);
            let start = piece * stride;
            if start >= total {
                break;
            }
            let length = SEGMENT.min(total - start);
            input.fill(0.0);
            input[..length].copy_from_slice(&left[start..start + length]);
            input[SEGMENT..SEGMENT + length].copy_from_slice(&right[start..start + length]);

            let tensor = Tensor::from_array(([1usize, 2, SEGMENT], input.clone()))
                .map_err(|error| format!("could not prepare the audio: {error}"))?;
            let outputs = self
                .session
                .run(ort::inputs!["mix" => tensor])
                .map_err(|error| format!("the model failed: {error}"))?;
            let (shape, stems) = outputs["stems"]
                .try_extract_tensor::<f32>()
                .map_err(|error| format!("unexpected model output: {error}"))?;
            if stems.len() != 8 * SEGMENT {
                return Err(format!("unexpected model output shape {shape:?}"));
            }

            for offset in 0..length {
                let mut weight = 1.0;
                if piece > 0 && offset < OVERLAP {
                    weight = fade[offset];
                }
                if piece + 1 < pieces && offset >= SEGMENT - OVERLAP {
                    weight = fade[SEGMENT - 1 - offset];
                }
                let position = start + offset;
                weights[position] += weight;
                for (row, sum) in sums.iter_mut().enumerate() {
                    sum[position] += stems[row * SEGMENT + offset] * weight;
                }
            }
        }
        progress(1.0);

        let stem = |model_index: usize| {
            let (left, right) = (&sums[2 * model_index], &sums[2 * model_index + 1]);
            let mut samples = Vec::with_capacity(2 * total);
            for index in 0..total {
                let weight = weights[index].max(1e-6);
                samples.push(left[index] / weight);
                samples.push(right[index] / weight);
            }
            DecodedAudio {
                sample_rate: MODEL_SAMPLE_RATE,
                channel_count: 2,
                samples,
            }
        };
        Ok(Stems {
            drums: stem(0),
            bass: stem(1),
            other: stem(2),
            vocals: stem(3),
        })
    }
}

/// The left and right channel of `audio` at the model's sample rate (mono
/// is used for both; a different rate is converted by straight-line
/// interpolation, which is plenty for separating).
fn stereo_at_model_rate(audio: &DecodedAudio) -> [Vec<f32>; 2] {
    let frames = audio.frame_count();
    let channels = audio.channel_count.max(1);
    let channel = |index: usize| -> Vec<f32> {
        let index = index.min(channels - 1);
        audio
            .samples
            .chunks_exact(channels)
            .map(|frame| frame[index])
            .collect()
    };
    let (left, right) = (channel(0), channel(1));
    if audio.sample_rate == MODEL_SAMPLE_RATE || frames == 0 {
        return [left, right];
    }
    let step = f64::from(audio.sample_rate) / f64::from(MODEL_SAMPLE_RATE);
    let new_frames = ((frames as f64) / step).floor() as usize;
    let resample = |signal: &[f32]| -> Vec<f32> {
        (0..new_frames)
            .map(|index| {
                let position = index as f64 * step;
                let before = position as usize;
                let after = (before + 1).min(frames - 1);
                let fraction = (position - before as f64) as f32;
                signal[before] * (1.0 - fraction) + signal[after] * fraction
            })
            .collect()
    };
    [resample(&left), resample(&right)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_is_converted_to_the_model_rate_in_stereo() {
        let mono = DecodedAudio {
            sample_rate: 22_050,
            channel_count: 1,
            samples: vec![0.0, 1.0, 0.0, -1.0],
        };
        let [left, right] = stereo_at_model_rate(&mono);
        assert_eq!(left.len(), 8);
        assert_eq!(left, right);
        assert_eq!(left[..4], [0.0, 0.5, 1.0, 0.5]);
    }

    /// Runs the real model when it is installed (see `find_ml_setup`):
    /// a low tone in the middle should come out mostly as bass.
    #[test]
    fn the_model_separates_a_low_tone_as_bass_when_installed() {
        let Ok(setup) = find_ml_setup() else {
            eprintln!("skipped: the HTDemucs model is not installed");
            return;
        };
        let mut model = DemucsModel::load(&setup).unwrap();
        let frames = MODEL_SAMPLE_RATE as usize * 3;
        let samples: Vec<f32> = (0..frames)
            .flat_map(|index| {
                let t = index as f32 / MODEL_SAMPLE_RATE as f32;
                let value = (std::f32::consts::TAU * 55.0 * t).sin() * 0.4;
                [value, value]
            })
            .collect();
        let audio = DecodedAudio {
            sample_rate: MODEL_SAMPLE_RATE,
            channel_count: 2,
            samples,
        };

        let stems = model
            .separate(&audio, &AtomicBool::new(false), &|_| {})
            .unwrap();

        let energy = |audio: &DecodedAudio| -> f64 {
            audio.samples.iter().map(|s| f64::from(*s).powi(2)).sum()
        };
        let energies = stems.named().map(|(name, audio)| (name, energy(audio)));
        let total: f64 = energies.iter().map(|(_, value)| value).sum();
        let bass = energies[2].1 / total;
        assert_eq!(stems.bass.frame_count(), frames);
        assert!(bass > 0.5, "{energies:?}");
    }
}
