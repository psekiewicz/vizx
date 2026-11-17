//! Telling singers apart: which lines of a song are sung by the same
//! person.
//!
//! Every line (its stretch of the separated vocals) is turned into a "voice
//! print" by a speaker-embedding model (WeSpeaker ResNet34, trained on
//! VoxCeleb; an ONNX file run by ONNX Runtime), and lines whose prints are
//! alike are grouped. The model learned from speech, so singing is harder:
//! a man and a woman are told apart well, two similar voices less so, and
//! lines sung together get the voice that is louder. The user can always
//! move a line to another singer afterwards.
//!
//! The model takes Kaldi-style filter-bank features (80 mel bands, 25 ms
//! frames every 10 ms, 16 kHz), computed here.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Tensor;
use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use vizx_audio::DecodedAudio;

/// Environment variable with the speaker-embedding model file (`.onnx`).
pub const SPEAKER_MODEL_VARIABLE: &str = "VIZX_SPEAKER_MODEL";
/// Model file names looked for next to the executable, in `models/`.
const SPEAKER_MODEL_FILE_NAMES: [&str; 1] = ["voxceleb_resnet34_LM.onnx"];

const RATE: usize = 16_000;
const FRAME: usize = 400;
const SHIFT: usize = 160;
const FFT_SIZE: usize = 512;
const MEL_BANDS: usize = 80;
/// Shortest stretch a voice print is taken from; shorter lines are widened
/// around their middle.
const SHORTEST_SECONDS: f64 = 1.5;
/// Longest stretch used per line (a long line costs time and says no more).
const LONGEST_SECONDS: f64 = 8.0;
/// Lines whose vocals are quieter than this share of the song's typical
/// vocal loudness have no reliable print; they join the singer of the line
/// next to them.
const QUIET: f32 = 0.08;
/// Without a number of singers, groups are joined while their prints are at
/// least this alike (cosine similarity).
pub const SAME_SINGER_SIMILARITY: f32 = 0.45;

/// Finds the speaker-embedding model: in `models/` next to the VizX
/// executable, or where [`SPEAKER_MODEL_VARIABLE`] points.
pub fn find_speaker_model() -> Result<PathBuf, String> {
    let executable_folder = std::env::current_exe()
        .ok()
        .and_then(|path| Some(path.parent()?.to_path_buf()));
    executable_folder
        .iter()
        .flat_map(|folder| {
            SPEAKER_MODEL_FILE_NAMES
                .iter()
                .map(move |name| folder.join("models").join(name))
        })
        .chain(std::env::var_os(SPEAKER_MODEL_VARIABLE).map(PathBuf::from))
        .find(|path| path.is_file())
        .ok_or_else(|| {
            format!(
                "the singer model (voxceleb_resnet34_LM.onnx) was not found in models/ next to \
                 VizX or at %{SPEAKER_MODEL_VARIABLE}%; install it with Install models"
            )
        })
}

/// The speaker-embedding model, ready to make voice prints.
pub struct SpeakerModel {
    session: Session,
    input: String,
    output: String,
    fbank: Fbank,
}

impl SpeakerModel {
    pub fn load(runtime_library: &Path, model: &Path) -> Result<Self, String> {
        let environment = crate::environment(runtime_library)?;
        let threads = std::thread::available_parallelism().map_or(4, |count| count.get());
        let session = Session::builder(environment)
            .and_then(|builder| {
                builder
                    .with_optimization_level(GraphOptimizationLevel::Level3)
                    .map_err(Into::into)
            })
            .and_then(|builder| builder.with_intra_threads(threads).map_err(Into::into))
            .and_then(|mut builder| builder.commit_from_file(model))
            .map_err(|error| format!("could not load the singer model: {error}"))?;
        let input = session
            .inputs()
            .first()
            .map(|input| input.name().to_owned())
            .ok_or("the singer model has no input")?;
        let output = session
            .outputs()
            .first()
            .map(|output| output.name().to_owned())
            .ok_or("the singer model has no output")?;
        Ok(Self {
            session,
            input,
            output,
            fbank: Fbank::new(),
        })
    }

    /// The voice print of `samples` (mono, 16 kHz, -1 to 1), of length 1.
    pub fn voice_print(&mut self, samples: &[f32]) -> Result<Vec<f32>, String> {
        let features = self.fbank.features(samples);
        let frames = features.len() / MEL_BANDS;
        if frames == 0 {
            return Err("the stretch is too short".to_owned());
        }
        let tensor = Tensor::from_array(([1usize, frames, MEL_BANDS], features))
            .map_err(|error| format!("could not prepare the audio: {error}"))?;
        let outputs = self
            .session
            .run(ort::inputs![self.input.as_str() => tensor])
            .map_err(|error| format!("the singer model failed: {error}"))?;
        let (_, print) = outputs[self.output.as_str()]
            .try_extract_tensor::<f32>()
            .map_err(|error| format!("unexpected singer model output: {error}"))?;
        Ok(normalized(print.to_vec()))
    }
}

/// Which singer sings each of `lines` (start and end in seconds of
/// `vocals`): numbers from 0, in the order the singers are first heard.
/// `singers` fixes how many there are; `None` decides from how alike the
/// voices are. `progress` hears how far it is (0 to 1).
pub fn identify_singers(
    model: &mut SpeakerModel,
    vocals: &DecodedAudio,
    lines: &[(f64, f64)],
    singers: Option<usize>,
    cancel: &AtomicBool,
    progress: &dyn Fn(f32),
) -> Result<Vec<usize>, String> {
    if lines.is_empty() {
        return Ok(Vec::new());
    }
    let samples = crate::transcribe::mono_at_whisper_rate(vocals);
    let typical = typical_loudness(&samples);
    let mut prints: Vec<Option<Vec<f32>>> = Vec::with_capacity(lines.len());
    for (index, (start, end)) in lines.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err("stopped".to_owned());
        }
        progress(index as f32 / lines.len() as f32);
        let stretch = stretch(&samples, *start, *end);
        let loudness = rms(stretch);
        prints.push(if loudness < typical * QUIET || stretch.len() < FRAME {
            None
        } else {
            Some(model.voice_print(stretch)?)
        });
    }
    progress(1.0);
    Ok(group(&prints, singers, SAME_SINGER_SIMILARITY))
}

/// The samples of a line, at least [`SHORTEST_SECONDS`] around its middle
/// and at most [`LONGEST_SECONDS`].
fn stretch(samples: &[f32], start: f64, end: f64) -> &[f32] {
    let (start, end) = (start.max(0.0), end.max(start));
    let middle = (start + end) / 2.0;
    let half = ((end - start).clamp(SHORTEST_SECONDS, LONGEST_SECONDS)) / 2.0;
    let to_index = |seconds: f64| ((seconds * RATE as f64) as usize).min(samples.len());
    &samples[to_index(middle - half)..to_index(middle + half)]
}

/// Groups voice prints into singers. Lines without a print take the singer
/// of the nearest line that has one. Singers are numbered in the order they
/// are first heard.
///
/// 1. Prints are joined into groups while they are at least `alike`
///    (agglomerative clustering on the cosine similarity of the groups'
///    average prints).
/// 2. Groups of only a line or two are noise (a shout, a short word, a
///    backing voice); only the larger ones are singers. With `singers`
///    given, the most alike singers are joined until that many are left
///    (or the largest small groups count as singers when there are too
///    few).
/// 3. Every line then goes to the singer whose average print is most like
///    its own.
pub fn group(prints: &[Option<Vec<f32>>], singers: Option<usize>, alike: f32) -> Vec<usize> {
    let known: Vec<usize> = (0..prints.len())
        .filter(|index| prints[*index].is_some())
        .collect();
    if known.is_empty() {
        return vec![0; prints.len()];
    }
    let print = |index: usize| prints[index].as_deref().unwrap_or(&[]);

    // Every group: its members and the sum of their prints.
    let mut groups: Vec<(Vec<usize>, Vec<f32>)> = known
        .iter()
        .map(|index| (vec![*index], print(*index).to_vec()))
        .collect();
    let join = |groups: &mut Vec<(Vec<usize>, Vec<f32>)>, a: usize, b: usize| {
        let (members, sum) = groups.remove(b);
        groups[a].0.extend(members);
        for (total, value) in groups[a].1.iter_mut().zip(sum) {
            *total += value;
        }
    };
    let most_alike = |groups: &[(Vec<usize>, Vec<f32>)], among: &dyn Fn(usize) -> bool| {
        let mut best: Option<(usize, usize, f32)> = None;
        for a in (0..groups.len()).filter(|a| among(*a)) {
            for b in (a + 1..groups.len()).filter(|b| among(*b)) {
                let similarity = cosine(&groups[a].1, &groups[b].1);
                if best.is_none_or(|(_, _, most)| similarity > most) {
                    best = Some((a, b, similarity));
                }
            }
        }
        best
    };
    while let Some((a, b, similarity)) = most_alike(&groups, &|_| true) {
        if similarity < alike {
            break;
        }
        join(&mut groups, a, b);
    }

    // The singers: groups large enough, largest first.
    groups.sort_by_key(|(members, _)| std::cmp::Reverse(members.len()));
    let smallest_singer = (known.len() / 20).max(3).min(known.len());
    let mut singer_count = groups
        .iter()
        .filter(|(members, _)| members.len() >= smallest_singer)
        .count()
        .max(1);
    if let Some(wanted) = singers {
        let wanted = wanted.clamp(1, groups.len());
        singer_count = singer_count.max(wanted);
        while singer_count > wanted {
            let Some((a, b, _)) = most_alike(&groups, &|index| index < singer_count) else {
                break;
            };
            join(&mut groups, a, b);
            singer_count -= 1;
            groups.sort_by_key(|(members, _)| std::cmp::Reverse(members.len()));
        }
    }
    let centers: Vec<Vec<f32>> = groups[..singer_count]
        .iter()
        .map(|(_, sum)| normalized(sum.clone()))
        .collect();

    let mut label: Vec<Option<usize>> = vec![None; prints.len()];
    for index in &known {
        label[*index] = (0..centers.len()).max_by(|a, b| {
            cosine(print(*index), &centers[*a]).total_cmp(&cosine(print(*index), &centers[*b]))
        });
    }
    // Lines without a print: the singer of the nearest line before, or
    // after.
    let filled: Vec<usize> = (0..prints.len())
        .map(|index| {
            label[index].unwrap_or_else(|| {
                let nearest = known
                    .iter()
                    .min_by_key(|other| other.abs_diff(index))
                    .copied()
                    .unwrap_or(0);
                label[nearest].unwrap_or(0)
            })
        })
        .collect();
    // Number the singers in the order they are first heard.
    let mut order: Vec<usize> = Vec::new();
    for group in &filled {
        if !order.contains(group) {
            order.push(*group);
        }
    }
    filled
        .iter()
        .map(|group| order.iter().position(|first| first == group).unwrap_or(0))
        .collect()
}
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let length = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (length(a) * length(b)).max(1e-12)
}

fn normalized(mut values: Vec<f32>) -> Vec<f32> {
    let length = values.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
    for value in &mut values {
        *value /= length;
    }
    values
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt()
}

/// The loudness of the louder half of 0.1-second pieces: what "singing"
/// sounds like in this song.
fn typical_loudness(samples: &[f32]) -> f32 {
    let mut pieces: Vec<f32> = samples.chunks(RATE / 10).map(rms).collect();
    if pieces.is_empty() {
        return 0.0;
    }
    pieces.sort_by(f32::total_cmp);
    pieces[pieces.len() * 3 / 4]
}

/// Kaldi-style log mel filter-bank features, as WeSpeaker was trained on:
/// 25 ms Hamming frames every 10 ms, DC removed, pre-emphasis 0.97, a
/// 512-point power spectrum, 80 triangular mel bands from 20 Hz to 8 kHz,
/// natural logarithm; then the mean of every band is subtracted.
struct Fbank {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    /// For every band, its first spectrum bin and the weights from there.
    bands: Vec<(usize, Vec<f32>)>,
}

impl Fbank {
    fn new() -> Self {
        let window = (0..FRAME)
            .map(|index| {
                0.54 - 0.46 * (2.0 * std::f32::consts::PI * index as f32 / (FRAME - 1) as f32).cos()
            })
            .collect();
        let mel = |hertz: f32| 1127.0 * (1.0 + hertz / 700.0).ln();
        let (low, high) = (mel(20.0), mel(RATE as f32 / 2.0));
        let step = (high - low) / (MEL_BANDS + 1) as f32;
        let bands = (0..MEL_BANDS)
            .map(|band| {
                let left = low + band as f32 * step;
                let (center, right) = (left + step, left + 2.0 * step);
                let weights: Vec<(usize, f32)> = (0..=FFT_SIZE / 2)
                    .filter_map(|bin| {
                        let at = mel(bin as f32 * RATE as f32 / FFT_SIZE as f32);
                        let weight = if at > left && at <= center {
                            (at - left) / (center - left)
                        } else if at > center && at < right {
                            (right - at) / (right - center)
                        } else {
                            return None;
                        };
                        Some((bin, weight))
                    })
                    .collect();
                let first = weights.first().map_or(0, |(bin, _)| *bin);
                (
                    first,
                    weights.into_iter().map(|(_, weight)| weight).collect(),
                )
            })
            .collect();
        Self {
            fft: FftPlanner::new().plan_fft_forward(FFT_SIZE),
            window,
            bands,
        }
    }

    /// Features of `samples` (mono 16 kHz, -1 to 1), frame after frame,
    /// with the mean of every band subtracted.
    fn features(&self, samples: &[f32]) -> Vec<f32> {
        let mut features = self.log_mel(samples);
        let frames = features.len() / MEL_BANDS;
        for band in 0..MEL_BANDS {
            let mean = (0..frames)
                .map(|frame| features[frame * MEL_BANDS + band])
                .sum::<f32>()
                / frames.max(1) as f32;
            for frame in 0..frames {
                features[frame * MEL_BANDS + band] -= mean;
            }
        }
        features
    }

    /// The log mel energies, frame after frame.
    fn log_mel(&self, samples: &[f32]) -> Vec<f32> {
        if samples.len() < FRAME {
            return Vec::new();
        }
        let frames = 1 + (samples.len() - FRAME) / SHIFT;
        let mut features = Vec::with_capacity(frames * MEL_BANDS);
        let mut frame = [0.0f32; FRAME];
        let mut spectrum = vec![Complex32::new(0.0, 0.0); FFT_SIZE];
        for index in 0..frames {
            let start = index * SHIFT;
            for (target, sample) in frame.iter_mut().zip(&samples[start..start + FRAME]) {
                // Kaldi works with 16-bit sample values.
                *target = sample * 32_768.0;
            }
            let mean = frame.iter().sum::<f32>() / FRAME as f32;
            for value in &mut frame {
                *value -= mean;
            }
            for position in (1..FRAME).rev() {
                frame[position] -= 0.97 * frame[position - 1];
            }
            frame[0] -= 0.97 * frame[0];
            spectrum.fill(Complex32::new(0.0, 0.0));
            for ((target, value), weight) in spectrum.iter_mut().zip(&frame).zip(&self.window) {
                target.re = value * weight;
            }
            self.fft.process(&mut spectrum);
            for (first, weights) in &self.bands {
                let energy: f32 = weights
                    .iter()
                    .enumerate()
                    .map(|(offset, weight)| spectrum[first + offset].norm_sqr() * weight)
                    .sum();
                features.push(energy.max(f32::EPSILON).ln());
            }
        }
        features
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn features_have_eighty_bands_every_ten_milliseconds() {
        let fbank = Fbank::new();
        let second: Vec<f32> = (0..RATE)
            .map(|index| (index as f32 * 440.0 * std::f32::consts::TAU / RATE as f32).sin() * 0.5)
            .collect();
        assert_eq!(fbank.features(&second).len(), 98 * MEL_BANDS);
        let features = fbank.log_mel(&second);
        // A 440 Hz tone is strongest in a low band, not the highest one.
        let first_frame = &features[..MEL_BANDS];
        let loudest = (0..MEL_BANDS)
            .max_by(|a, b| first_frame[*a].total_cmp(&first_frame[*b]))
            .unwrap();
        assert!(loudest < 30, "{loudest}");
        assert!(fbank.features(&second[..100]).is_empty());
    }

    #[test]
    fn alike_prints_are_grouped_and_numbered_in_order_of_appearance() {
        let voice = |x: f32, y: f32| Some(normalized(vec![x, y, 0.1]));
        // Lines: A A B A (quiet) B A B, and one odd shout.
        let prints = vec![
            voice(0.0, 1.0),
            voice(0.1, 1.0),
            voice(1.0, 0.0),
            voice(0.05, 1.0),
            None,
            voice(1.0, 0.1),
            voice(0.0, 0.9),
            voice(0.9, 0.05),
            Some(normalized(vec![-1.0, 0.2, 1.0])),
        ];
        // The shout is not a singer of its own.
        let expected = [0, 0, 1, 0, 0, 1, 0, 1];
        let found = group(&prints, None, SAME_SINGER_SIMILARITY);
        assert_eq!(found[..8], expected);
        assert!(found[8] < 2);
        // Asked for two, the same two; asked for one, everything is one.
        assert_eq!(
            group(&prints, Some(2), SAME_SINGER_SIMILARITY)[..8],
            expected
        );
        assert_eq!(group(&prints, Some(1), SAME_SINGER_SIMILARITY), [0; 9]);
        assert_eq!(group(&[None, None], None, 0.5), [0, 0]);
    }

    /// Needs the model and ONNX Runtime; run with `--ignored`.
    #[test]
    #[ignore = "needs the singer model and ONNX Runtime"]
    fn the_model_makes_prints_of_length_one() {
        let runtime = crate::find_onnx_runtime().unwrap();
        let mut model = SpeakerModel::load(&runtime, &find_speaker_model().unwrap()).unwrap();
        let sound: Vec<f32> = (0..RATE * 2)
            .map(|index| ((index as f32 * 0.0137).sin() * (index as f32 * 0.00071).cos()) * 0.3)
            .collect();
        let print = model.voice_print(&sound).unwrap();
        assert_eq!(print.len(), 256);
        assert!((print.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-3);
        // The same sound gives the same print.
        assert!(cosine(&print, &model.voice_print(&sound).unwrap()) > 0.999);
    }
}
