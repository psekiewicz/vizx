//! Turns audio samples into the numbers a visualizer draws: a frequency
//! spectrum, the waveform and a few overall levels.

use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

/// How many values the spectrum has, from the lowest to the highest
/// frequency.
pub const SPECTRUM_BINS: usize = 512;
/// How many values the waveform has, from oldest to newest.
pub const WAVEFORM_POINTS: usize = 512;
/// How many samples one analysis looks at (about 46 ms at 44.1 kHz). Give
/// [`AudioAnalyzer::analyze`] the most recent samples, this many.
pub const ANALYSIS_WINDOW: usize = 2048;

/// Lowest and highest frequency shown in the spectrum, in hertz.
const LOWEST_FREQUENCY: f32 = 30.0;
const HIGHEST_FREQUENCY: f32 = 16_000.0;
/// Loudness range mapped onto 0..1, in decibels below full scale.
const FLOOR_DECIBELS: f32 = -66.0;
const CEILING_DECIBELS: f32 = -6.0;
/// Music has much more energy in the bass than in the treble. Raising the
/// spectrum by this many decibels per octave (around 1 kHz) makes a typical
/// song fill the whole width instead of only the left side.
const TILT_DECIBELS_PER_OCTAVE: f32 = 3.0;
/// How fast a spectrum value falls after the sound is gone, in full-scale
/// units per second. Rising is immediate.
const FALL_PER_SECOND: f32 = 2.5;
/// Upper edges of the "bass" and "mid" ranges, in hertz.
const BASS_LIMIT: f32 = 250.0;
const MID_LIMIT: f32 = 4_000.0;

/// What a visualizer gets to know about the sound at one moment.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioFeatures {
    /// [`SPECTRUM_BINS`] loudness values from 0 (silent) to 1 (loud). The
    /// frequencies are spaced like notes on a piano (logarithmically), from
    /// 30 Hz on the left to 16 kHz on the right.
    pub spectrum: Vec<f32>,
    /// [`WAVEFORM_POINTS`] sample values from -1 to 1, oldest first: the
    /// shape of the sound wave during the last few milliseconds.
    pub waveform: Vec<f32>,
    /// Overall loudness, 0 to 1.
    pub volume: f32,
    /// Average of the spectrum below 250 Hz, 0 to 1.
    pub bass: f32,
    /// Average of the spectrum from 250 Hz to 4 kHz, 0 to 1.
    pub mid: f32,
    /// Average of the spectrum above 4 kHz, 0 to 1.
    pub treble: f32,
}

/// Analyses a stream of audio, one moment at a time.
///
/// It remembers the previous spectrum so that values fall smoothly instead
/// of flickering. Call it for consecutive frames, in order, to get the same
/// result every time.
pub struct AudioAnalyzer {
    fft: Arc<dyn Fft<f32>>,
    /// The "window": a curve that fades the analysed samples in and out, so
    /// the abrupt start and end of the excerpt do not show up as false
    /// frequencies.
    window: Vec<f32>,
    /// Scales FFT results so that a full-volume tone gives 1.0.
    amplitude_scale: f32,
    /// For each spectrum value: the range of FFT results it is made from,
    /// as fractional positions.
    bin_ranges: Vec<(f32, f32)>,
    /// Decibels added to each spectrum value (see the tilt above).
    bin_tilt: Vec<f32>,
    /// Spectrum indices where the mid and treble ranges begin.
    mid_start: usize,
    treble_start: usize,
    spectrum: Vec<f32>,
    buffer: Vec<Complex32>,
}

impl AudioAnalyzer {
    pub fn new(sample_rate: u32) -> Self {
        let sample_rate = sample_rate.max(1) as f32;
        let window: Vec<f32> = (0..ANALYSIS_WINDOW)
            .map(|index| {
                let phase = index as f32 / (ANALYSIS_WINDOW - 1) as f32;
                0.5 - 0.5 * (std::f32::consts::TAU * phase).cos() // Hann window
            })
            .collect();
        let window_sum: f32 = window.iter().sum();

        // One FFT result covers this many hertz.
        let hertz_per_result = sample_rate / ANALYSIS_WINDOW as f32;
        let highest = HIGHEST_FREQUENCY.min(sample_rate / 2.0);
        let frequency_at = |position: f32| {
            LOWEST_FREQUENCY * (highest / LOWEST_FREQUENCY).powf(position / SPECTRUM_BINS as f32)
        };
        let bin_ranges = (0..SPECTRUM_BINS)
            .map(|bin| {
                (
                    frequency_at(bin as f32) / hertz_per_result,
                    frequency_at(bin as f32 + 1.0) / hertz_per_result,
                )
            })
            .collect();
        let bin_tilt = (0..SPECTRUM_BINS)
            .map(|bin| {
                let octaves_from_1khz = (frequency_at(bin as f32 + 0.5) / 1000.0).log2();
                TILT_DECIBELS_PER_OCTAVE * octaves_from_1khz
            })
            .collect();
        let first_bin_at = |frequency: f32| {
            (0..SPECTRUM_BINS)
                .find(|bin| frequency_at(*bin as f32) >= frequency)
                .unwrap_or(SPECTRUM_BINS)
        };

        Self {
            fft: FftPlanner::new().plan_fft_forward(ANALYSIS_WINDOW),
            window,
            amplitude_scale: 2.0 / window_sum,
            bin_ranges,
            bin_tilt,
            mid_start: first_bin_at(BASS_LIMIT),
            treble_start: first_bin_at(MID_LIMIT),
            spectrum: vec![0.0; SPECTRUM_BINS],
            buffer: vec![Complex32::default(); ANALYSIS_WINDOW],
        }
    }

    /// Analyses the most recent samples (one channel, -1 to 1).
    ///
    /// `samples` should hold [`ANALYSIS_WINDOW`] values ending at the moment
    /// of interest; a shorter slice is treated as preceded by silence.
    /// `seconds_since_last` is the time since the previous call and decides
    /// how far the old spectrum has fallen meanwhile.
    pub fn analyze(&mut self, samples: &[f32], seconds_since_last: f32) -> AudioFeatures {
        let samples = &samples[samples.len().saturating_sub(ANALYSIS_WINDOW)..];
        let silence_before = ANALYSIS_WINDOW - samples.len();

        for (index, value) in self.buffer.iter_mut().enumerate() {
            let sample = index
                .checked_sub(silence_before)
                .map_or(0.0, |index| samples[index]);
            *value = Complex32::new(sample * self.window[index], 0.0);
        }
        self.fft.process(&mut self.buffer);

        let fall = FALL_PER_SECOND * seconds_since_last.max(0.0);
        for bin in 0..SPECTRUM_BINS {
            let amplitude = self.amplitude_in(self.bin_ranges[bin]) * self.amplitude_scale;
            let decibels = 20.0 * amplitude.max(1e-9).log10() + self.bin_tilt[bin];
            let level =
                ((decibels - FLOOR_DECIBELS) / (CEILING_DECIBELS - FLOOR_DECIBELS)).clamp(0.0, 1.0);
            // Rise at once, fall gradually.
            self.spectrum[bin] = level.max(self.spectrum[bin] - fall);
        }

        let mean_square =
            samples.iter().map(|sample| sample * sample).sum::<f32>() / ANALYSIS_WINDOW as f32;
        let newest = &samples[samples.len().saturating_sub(WAVEFORM_POINTS)..];
        let mut waveform = vec![0.0; WAVEFORM_POINTS - newest.len()];
        waveform.extend_from_slice(newest);

        AudioFeatures {
            spectrum: self.spectrum.clone(),
            waveform,
            // A full-volume tone has a root mean square of 0.707.
            volume: (mean_square.sqrt() / std::f32::consts::FRAC_1_SQRT_2).clamp(0.0, 1.0),
            bass: mean(&self.spectrum[..self.mid_start]),
            mid: mean(&self.spectrum[self.mid_start..self.treble_start]),
            treble: mean(&self.spectrum[self.treble_start..]),
        }
    }

    /// Strength of the FFT results within a range of fractional positions.
    ///
    /// Low notes are closer together than FFT results are, so there the
    /// value is read between two results. High up, one spectrum value spans
    /// many results and takes the strongest of them.
    fn amplitude_in(&self, (start, end): (f32, f32)) -> f32 {
        let last_result = ANALYSIS_WINDOW / 2 - 1;
        let amplitude_of = |index: usize| self.buffer[index.min(last_result)].norm();

        if end - start < 1.0 {
            let center = (start + end) / 2.0;
            let below = center.floor();
            let share_of_upper = center - below;
            return amplitude_of(below as usize) * (1.0 - share_of_upper)
                + amplitude_of(below as usize + 1) * share_of_upper;
        }
        (start.round() as usize..end.round() as usize)
            .map(amplitude_of)
            .fold(0.0, f32::max)
    }
}

fn mean(values: &[f32]) -> f32 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f32>() / values.len() as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: u32 = 44_100;

    fn tone(frequency: f32, amplitude: f32) -> Vec<f32> {
        (0..ANALYSIS_WINDOW)
            .map(|index| {
                let seconds = index as f32 / SAMPLE_RATE as f32;
                amplitude * (std::f32::consts::TAU * frequency * seconds).sin()
            })
            .collect()
    }

    /// Where a frequency lies in the spectrum, from 0 to 1.
    fn position_of(frequency: f32) -> f32 {
        (frequency / LOWEST_FREQUENCY).ln() / (HIGHEST_FREQUENCY / LOWEST_FREQUENCY).ln()
    }

    fn loudest_bin(spectrum: &[f32]) -> usize {
        (0..spectrum.len())
            .max_by(|a, b| spectrum[*a].total_cmp(&spectrum[*b]))
            .unwrap()
    }

    #[test]
    fn silence_gives_an_empty_spectrum() {
        let features = AudioAnalyzer::new(SAMPLE_RATE).analyze(&[0.0; ANALYSIS_WINDOW], 0.0);

        assert_eq!(features.spectrum.len(), SPECTRUM_BINS);
        assert_eq!(features.waveform.len(), WAVEFORM_POINTS);
        assert!(features.spectrum.iter().all(|value| *value == 0.0));
        assert_eq!(features.volume, 0.0);
        assert_eq!(features.bass + features.mid + features.treble, 0.0);
    }

    #[test]
    fn a_tone_shows_up_where_its_frequency_lies() {
        for frequency in [100.0, 1000.0, 8000.0] {
            let features = AudioAnalyzer::new(SAMPLE_RATE).analyze(&tone(frequency, 0.5), 0.0);

            let found = loudest_bin(&features.spectrum) as f32 / SPECTRUM_BINS as f32;
            let expected = position_of(frequency);
            assert!(
                (found - expected).abs() < 0.03,
                "{frequency} Hz was found at {found}, expected {expected}"
            );
        }
    }

    #[test]
    fn levels_follow_the_part_of_the_spectrum_that_sounds() {
        let bass = AudioAnalyzer::new(SAMPLE_RATE).analyze(&tone(80.0, 0.8), 0.0);
        let treble = AudioAnalyzer::new(SAMPLE_RATE).analyze(&tone(9000.0, 0.8), 0.0);

        assert!(bass.bass > bass.mid && bass.bass > bass.treble);
        assert!(treble.treble > treble.mid && treble.treble > treble.bass);
        // A sine at 0.8 of full volume.
        assert!((bass.volume - 0.8).abs() < 0.05, "volume {}", bass.volume);
    }

    #[test]
    fn the_spectrum_falls_gradually_after_the_sound_stops() {
        let mut analyzer = AudioAnalyzer::new(SAMPLE_RATE);
        let loud = analyzer.analyze(&tone(1000.0, 0.8), 0.0);
        let peak_bin = loudest_bin(&loud.spectrum);
        let peak = loud.spectrum[peak_bin];

        let shortly_after = analyzer.analyze(&[0.0; ANALYSIS_WINDOW], 0.1);
        let much_later = analyzer.analyze(&[0.0; ANALYSIS_WINDOW], 10.0);

        let expected = peak - FALL_PER_SECOND * 0.1;
        assert!((shortly_after.spectrum[peak_bin] - expected).abs() < 1e-4);
        assert_eq!(much_later.spectrum[peak_bin], 0.0);
    }

    #[test]
    fn fewer_samples_than_a_window_count_as_preceded_by_silence() {
        let features = AudioAnalyzer::new(SAMPLE_RATE).analyze(&[0.5; 100], 0.0);

        assert_eq!(features.waveform.len(), WAVEFORM_POINTS);
        assert_eq!(features.waveform[0], 0.0);
        assert_eq!(features.waveform[WAVEFORM_POINTS - 1], 0.5);
    }
}
