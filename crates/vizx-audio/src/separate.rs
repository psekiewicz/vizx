//! Splitting a song into stems: vocals, drums, bass and the rest.
//!
//! This is the built-in, quick separation. It needs no model and no
//! download, and it is good enough to drive visualizers (a visualizer for
//! the drums sees mostly drums), though not clean enough to listen to on
//! its own. A separation model (Demucs and the like) gives cleaner stems;
//! it would produce the same four stems through the same steps.
//!
//! How it works, in the frequency domain (short-time Fourier transform):
//!
//! - **Drums vs. the rest**: harmonic-percussive separation. A note holds
//!   its pitch over time, so it forms horizontal lines in a spectrogram; a
//!   drum hit is short and covers many frequencies, a vertical line. A
//!   median across time keeps the horizontal lines, a median across
//!   frequencies keeps the vertical ones; comparing the two says how much of
//!   each bin is percussive.
//! - **Vocals**: lead vocals are usually mixed in the middle (the same in
//!   the left and right channel) and sit between about 150 Hz and 7 kHz, so
//!   the harmonic part that is centered and in that range counts as voice.
//! - **Bass**: the harmonic part below about 250 Hz.
//! - **Other**: whatever is left, so the four stems add up to the song.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

use std::path::{Path, PathBuf};

use crate::{DecodedAudio, write_wav_file};

/// Samples per analysis frame. About 93 ms at 44.1 kHz: long enough to tell
/// bass notes apart.
const FRAME: usize = 4096;
/// Samples between two frames (a quarter frame, so Hann windows add up to a
/// constant).
const HOP: usize = FRAME / 4;
/// Frames on each side looked at by the median across time (harmonic).
const TIME_REACH: usize = 8;
/// Bins on each side looked at by the median across frequencies
/// (percussive).
const FREQUENCY_REACH: usize = 8;

/// The four stems of a song, each stereo with the song's sample rate.
pub struct Stems {
    pub vocals: DecodedAudio,
    pub drums: DecodedAudio,
    pub bass: DecodedAudio,
    pub other: DecodedAudio,
}

impl Stems {
    /// The stems with their names, in a fixed order.
    pub fn named(&self) -> [(&'static str, &DecodedAudio); 4] {
        [
            ("Vocals", &self.vocals),
            ("Drums", &self.drums),
            ("Bass", &self.bass),
            ("Other", &self.other),
        ]
    }

    /// Writes each stem as a WAV file into `folder`, named after
    /// `song_name` and the stem (`Song - Vocals.wav`). Returns the stem names
    /// with their files.
    pub fn write_wav_files(
        &self,
        folder: &Path,
        song_name: &str,
    ) -> std::io::Result<Vec<(&'static str, PathBuf)>> {
        std::fs::create_dir_all(folder)?;
        let mut written = Vec::new();
        for (name, audio) in self.named() {
            let path = folder.join(format!("{song_name} - {name}.wav"));
            let samples: Vec<i16> = audio
                .samples
                .iter()
                .map(|sample| (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16)
                .collect();
            write_wav_file(
                &path,
                audio.sample_rate,
                audio.channel_count as u16,
                &samples,
            )?;
            written.push((name, path));
        }
        Ok(written)
    }
}

/// Why a separation did not finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeparationError {
    Cancelled,
    Empty,
}

impl std::fmt::Display for SeparationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(formatter, "the separation was cancelled"),
            Self::Empty => write!(formatter, "the audio has no samples"),
        }
    }
}

/// Splits `audio` into stems. `progress` is told how far it is (0 to 1)
/// now and then; setting `cancel` stops it.
pub fn separate_stems(
    audio: &DecodedAudio,
    cancel: &AtomicBool,
    progress: &dyn Fn(f32),
) -> Result<Stems, SeparationError> {
    let frames = audio.frame_count();
    if frames == 0 {
        return Err(SeparationError::Empty);
    }
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

    let separator = Separator::new(audio.sample_rate);
    // Padded by a frame on both sides, so the edges are analysed like the
    // middle.
    let padded = frames + 2 * FRAME;
    let frame_count = padded.div_ceil(HOP);
    let mut outputs = [(); 4].map(|_| [vec![0.0f32; padded + FRAME], vec![0.0; padded + FRAME]]);
    let sample = |signal: &[f32], index: usize| {
        index
            .checked_sub(FRAME)
            .and_then(|index| signal.get(index))
            .copied()
            .unwrap_or(0.0)
    };

    // Frames wait in the queue until the frames after them, needed by the
    // median across time, have been analysed.
    let mut queue: VecDeque<Analysed> = VecDeque::new();
    // Which frame `queue[0]` is.
    let mut queue_first = 0;
    let mut next_out = 0;
    for frame_index in 0..frame_count {
        if frame_index % 64 == 0 {
            if cancel.load(Ordering::Relaxed) {
                return Err(SeparationError::Cancelled);
            }
            progress(frame_index as f32 / frame_count as f32);
        }
        let start = frame_index * HOP;
        let left_frame: Vec<f32> = (start..start + FRAME).map(|i| sample(&left, i)).collect();
        let right_frame: Vec<f32> = (start..start + FRAME).map(|i| sample(&right, i)).collect();
        queue.push_back(separator.analyse(&left_frame, &right_frame));

        // A frame is split once the frames after it are known (at the end,
        // there are none to wait for).
        let is_last = frame_index + 1 == frame_count;
        while next_out < frame_count && (next_out + TIME_REACH <= frame_index || is_last) {
            let stems = separator.split(&queue, next_out - queue_first);
            let start = next_out * HOP;
            for (stem, [out_left, out_right]) in stems.into_iter().zip(outputs.iter_mut()) {
                for (offset, (l, r)) in stem[0].iter().zip(&stem[1]).enumerate() {
                    out_left[start + offset] += l;
                    out_right[start + offset] += r;
                }
            }
            next_out += 1;
            // Frames no later frame needs any more.
            while next_out > queue_first + TIME_REACH {
                queue.pop_front();
                queue_first += 1;
            }
        }
    }
    progress(1.0);

    let to_audio = |[left, right]: [Vec<f32>; 2]| {
        let mut samples = Vec::with_capacity(frames * 2);
        for index in FRAME..FRAME + frames {
            samples.push(left[index]);
            samples.push(right[index]);
        }
        DecodedAudio {
            sample_rate: audio.sample_rate,
            channel_count: 2,
            samples,
        }
    };
    let [vocals, drums, bass, other] = outputs.map(to_audio);
    Ok(Stems {
        vocals,
        drums,
        bass,
        other,
    })
}

/// One analysed frame: the spectrum of each channel and their loudness.
struct Analysed {
    left: Vec<Complex32>,
    right: Vec<Complex32>,
    /// Magnitude of the two channels together, per bin.
    magnitude: Vec<f32>,
}

struct Separator {
    forward: Arc<dyn Fft<f32>>,
    inverse: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    /// Frequency of each bin, in Hz.
    frequencies: Vec<f32>,
}

impl Separator {
    fn new(sample_rate: u32) -> Self {
        let mut planner = FftPlanner::new();
        let window = (0..FRAME)
            .map(|index| {
                let phase = std::f32::consts::TAU * index as f32 / FRAME as f32;
                0.5 - 0.5 * phase.cos()
            })
            .collect();
        let frequencies = (0..=FRAME / 2)
            .map(|bin| bin as f32 * sample_rate as f32 / FRAME as f32)
            .collect();
        Self {
            forward: planner.plan_fft_forward(FRAME),
            inverse: planner.plan_fft_inverse(FRAME),
            window,
            frequencies,
        }
    }

    fn analyse(&self, left: &[f32], right: &[f32]) -> Analysed {
        let spectrum = |signal: &[f32]| {
            let mut buffer: Vec<Complex32> = signal
                .iter()
                .zip(&self.window)
                .map(|(sample, weight)| Complex32::new(sample * weight, 0.0))
                .collect();
            self.forward.process(&mut buffer);
            buffer.truncate(FRAME / 2 + 1);
            buffer
        };
        let (left, right) = (spectrum(left), spectrum(right));
        let magnitude = left
            .iter()
            .zip(&right)
            .map(|(l, r)| (l.norm() + r.norm()) / 2.0)
            .collect();
        Analysed {
            left,
            right,
            magnitude,
        }
    }

    /// The four stems of frame `position` of `queue`, as windowed time
    /// signals to be added up (left and right each).
    fn split(&self, queue: &VecDeque<Analysed>, position: usize) -> [[Vec<f32>; 2]; 4] {
        let frame = &queue[position];
        let bins = frame.magnitude.len();
        let first = position.saturating_sub(TIME_REACH);
        let last = (position + TIME_REACH).min(queue.len() - 1);
        let mut around = Vec::with_capacity(2 * TIME_REACH.max(FREQUENCY_REACH) + 1);

        let mut masks = [(); 4].map(|_| vec![0.0f32; bins]);
        for bin in 0..bins {
            around.clear();
            around.extend((first..=last).map(|index| queue[index].magnitude[bin]));
            let harmonic = median(&mut around);
            around.clear();
            let low = bin.saturating_sub(FREQUENCY_REACH);
            let high = (bin + FREQUENCY_REACH).min(bins - 1);
            around.extend_from_slice(&frame.magnitude[low..=high]);
            let percussive = median(&mut around);

            let (h, p) = (harmonic * harmonic, percussive * percussive);
            let drums = p / (h + p + 1e-12);
            let tonal = 1.0 - drums;

            let frequency = self.frequencies[bin];
            let bass_share = 1.0 - smooth_step(200.0, 300.0, frequency);
            // How alike the two channels are: 1 in the middle of the mix.
            let (l, r) = (frame.left[bin], frame.right[bin]);
            let together = (l + r).norm_sqr();
            let apart = 2.0 * (l.norm_sqr() + r.norm_sqr()) + 1e-12;
            let centered = smooth_step(0.75, 0.97, together / apart);
            let voice_band = smooth_step(120.0, 250.0, frequency)
                * (1.0 - smooth_step(6000.0, 9000.0, frequency));

            let bass = tonal * bass_share;
            let vocals = tonal * (1.0 - bass_share) * voice_band * centered;
            masks[0][bin] = vocals;
            masks[1][bin] = drums;
            masks[2][bin] = bass;
            masks[3][bin] = (tonal - bass - vocals).max(0.0);
        }

        masks.map(|mask| {
            let signal = |spectrum: &[Complex32]| {
                let mut buffer = vec![Complex32::default(); FRAME];
                for (bin, value) in spectrum.iter().enumerate() {
                    buffer[bin] = value * mask[bin];
                    // The upper half mirrors the lower one, for a real signal.
                    if bin > 0 && bin < FRAME / 2 {
                        buffer[FRAME - bin] = (value * mask[bin]).conj();
                    }
                }
                self.inverse.process(&mut buffer);
                // Hann windows at a quarter-frame hop add up to 1.5 when
                // applied twice (analysis and synthesis).
                let scale = 1.0 / (FRAME as f32 * 1.5);
                buffer
                    .iter()
                    .zip(&self.window)
                    .map(|(value, weight)| value.re * weight * scale)
                    .collect::<Vec<f32>>()
            };
            [signal(&frame.left), signal(&frame.right)]
        })
    }
}

/// The middle value of `values` (which get reordered).
fn median(values: &mut [f32]) -> f32 {
    let middle = values.len() / 2;
    let (_, value, _) = values.select_nth_unstable_by(middle, f32::total_cmp);
    *value
}

/// 0 below `from`, 1 above `to`, a smooth curve in between.
fn smooth_step(from: f32, to: f32, value: f32) -> f32 {
    let t = ((value - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44_100;

    fn stereo(seconds: f64, sample: impl Fn(f64) -> (f32, f32)) -> DecodedAudio {
        let frames = (seconds * f64::from(RATE)) as usize;
        let mut samples = Vec::with_capacity(frames * 2);
        for index in 0..frames {
            let (left, right) = sample(index as f64 / f64::from(RATE));
            samples.push(left);
            samples.push(right);
        }
        DecodedAudio {
            sample_rate: RATE,
            channel_count: 2,
            samples,
        }
    }

    fn energy(audio: &DecodedAudio) -> f64 {
        // The edges are left out: the first and last frames see silence.
        let skip = audio.samples.len() / 8;
        audio.samples[skip..audio.samples.len() - skip]
            .iter()
            .map(|sample| f64::from(*sample).powi(2))
            .sum()
    }

    fn split(audio: &DecodedAudio) -> Stems {
        separate_stems(audio, &AtomicBool::new(false), &|_| {}).unwrap()
    }

    /// Share of the song's energy each stem got: vocals, drums, bass, other.
    fn shares(stems: &Stems) -> [f64; 4] {
        let energies = stems.named().map(|(_, audio)| energy(audio));
        let total: f64 = energies.iter().sum();
        energies.map(|part| part / total)
    }

    #[test]
    fn the_stems_add_up_to_the_song() {
        let song = stereo(1.0, |t| {
            let tone = (std::f64::consts::TAU * 440.0 * t).sin() as f32 * 0.3;
            let side = (std::f64::consts::TAU * 1000.0 * t).sin() as f32 * 0.2;
            (tone + side, tone - side)
        });
        let stems = split(&song);

        let rebuilt: Vec<f32> = (0..song.samples.len())
            .map(|index| {
                stems
                    .named()
                    .iter()
                    .map(|(_, audio)| audio.samples[index])
                    .sum()
            })
            .collect();
        let error = song
            .samples
            .iter()
            .zip(&rebuilt)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(error < 1e-3, "largest difference {error}");
    }

    #[test]
    fn a_low_note_is_bass_a_centered_voice_is_vocals_and_clicks_are_drums() {
        let bass = split(&stereo(2.0, |t| {
            let value = (std::f64::consts::TAU * 70.0 * t).sin() as f32 * 0.5;
            (value, value)
        }));
        let voice = split(&stereo(2.0, |t| {
            let value = (std::f64::consts::TAU * 600.0 * t).sin() as f32 * 0.5;
            (value, value)
        }));
        // A click every quarter second.
        let clicks = split(&stereo(2.0, |t| {
            let since = t % 0.25;
            let value = if since < 0.002 { 0.8 } else { 0.0 };
            (value, value)
        }));
        // The same tone as the voice, but only on the left: not centered.
        let wide = split(&stereo(2.0, |t| {
            ((std::f64::consts::TAU * 600.0 * t).sin() as f32 * 0.5, 0.0)
        }));

        assert!(shares(&bass)[2] > 0.9, "bass {:?}", shares(&bass));
        assert!(shares(&voice)[0] > 0.9, "voice {:?}", shares(&voice));
        assert!(shares(&clicks)[1] > 0.6, "clicks {:?}", shares(&clicks));
        assert!(shares(&wide)[3] > 0.9, "wide {:?}", shares(&wide));
    }

    #[test]
    fn a_separation_can_be_cancelled() {
        let song = stereo(1.0, |_| (0.0, 0.0));
        let cancel = AtomicBool::new(true);
        assert_eq!(
            separate_stems(&song, &cancel, &|_| {}).err(),
            Some(SeparationError::Cancelled)
        );
    }
}
