//! Finding the beats of a piece of music.
//!
//! "Beats" here are the moments where something new and strong starts: a
//! kick drum, a snare, a chord. They are found by watching how the
//! frequency content changes from one moment to the next ("spectral flux"):
//! a sudden rise of energy is a beat. The bass and the higher frequencies
//! are judged separately and the bass weighs more, so the kick drum stands
//! out over cymbals and a voice gliding from note to note.
//!
//! Not every such moment is a beat: a hi-hat between two kicks is not. So
//! the tempo is worked out first, and then the beats are chosen as the
//! strong moments that follow each other at that tempo ("beat tracking",
//! after D. Ellis, 2007). Without a tempo (a short or irregular file), the
//! strong moments themselves are used.
//!
//! The whole file is analysed once, when it is loaded, so the result is
//! the same every time and does not depend on playback.

use rustfft::FftPlanner;
use rustfft::num_complex::Complex32;

use crate::DecodedAudio;

/// Distance between two analysed moments, in seconds.
const HOP_SECONDS: f64 = 0.01;
/// How many samples each moment looks back at.
const WINDOW: usize = 1024;
/// Two beats are at least this far apart; of two candidates closer than
/// this, the stronger one is kept.
const MIN_BEAT_GAP_SECONDS: f64 = 0.12;
/// Highest frequency taken into account, and the upper end of the "bass"
/// that counts double, in hertz.
const HIGHEST_FREQUENCY: f64 = 8_000.0;
const BASS_LIMIT: f64 = 200.0;
/// How much of the beat signal comes from the bass (the rest comes from the
/// frequencies above it): the kick drum is what most people feel as the
/// beat.
const BASS_SHARE: f32 = 0.65;
/// The steady level of change (noise, a sustained sound) is measured over
/// this many moments before and after, and taken away, so that only real
/// hits are left.
const BACKGROUND_REACH: usize = 20;
/// A beat must stand out this much above the average change around it
/// (on a scale where the strongest change of the file is 1).
const THRESHOLD_ABOVE_AVERAGE: f32 = 0.06;
/// How far around a moment the average is taken, in analysed moments.
const AVERAGE_REACH: usize = 10;
/// How far around a moment it must be the largest change, in moments.
const PEAK_REACH: usize = 3;
/// Tempos considered, in beats per minute.
const TEMPO_RANGE: (f64, f64) = (60.0, 200.0);

/// One beat.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Beat {
    /// When it happens, in seconds from the start of the file.
    pub seconds: f64,
    /// How strong it is, from 0 to 1, compared with the other beats of the
    /// same file (the strongest ones are 1).
    pub strength: f32,
}

/// The beats of a file and its tempo.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BeatAnalysis {
    /// In order of time.
    pub beats: Vec<Beat>,
    /// The most likely tempo in beats per minute, if the file is long and
    /// regular enough to tell.
    pub tempo_bpm: Option<f64>,
}

/// Finds the beats of `audio` (all channels mixed together).
pub fn detect_beats(audio: &DecodedAudio) -> BeatAnalysis {
    let sample_rate = f64::from(audio.sample_rate.max(1));
    let hop = ((sample_rate * HOP_SECONDS).round() as usize).max(1);
    let mono: Vec<f32> = (0..audio.frame_count())
        .map(|frame| {
            let [left, right] = audio.stereo_at(frame as f64);
            (left + right) * 0.5
        })
        .collect();

    let envelope = change_envelope(&mono, sample_rate, hop);
    let seconds_per_step = hop as f64 / sample_rate;
    let tempo_bpm = estimate_tempo(&envelope, seconds_per_step);
    let beats = match tempo_bpm {
        Some(tempo) => track_beats(&envelope, seconds_per_step, tempo),
        None => pick_beats(&envelope, seconds_per_step),
    };
    BeatAnalysis { beats, tempo_bpm }
}

/// Chooses beats that are both strong and spaced like the tempo.
///
/// Every analysed moment gets a score: how much happens there, plus the
/// best score of an earlier moment roughly one beat before it, minus a
/// penalty for being off that distance. The best chain of such moments,
/// followed back from the end, are the beats.
fn track_beats(envelope: &[f32], seconds_per_step: f64, tempo_bpm: f64) -> Vec<Beat> {
    // How strongly a beat is held to the tempo.
    const TIGHTNESS: f64 = 100.0;
    // Around a beat, at least this much must happen for it to count; the
    // chain keeps going through quiet passages, but no beat is shown there.
    const QUIET: f32 = 0.05;
    // A beat must have at least this share of a typical strong beat.
    const WEAKEST_SHARE: f32 = 0.2;

    let period = 60.0 / tempo_bpm / seconds_per_step;
    if envelope.is_empty() || period < 2.0 {
        return Vec::new();
    }
    let mut score = vec![0.0f64; envelope.len()];
    let mut previous_beat: Vec<Option<usize>> = vec![None; envelope.len()];
    for step in 0..envelope.len() {
        let earliest = step as f64 - 2.0 * period;
        let latest = step as f64 - period / 2.0;
        let mut best: Option<(f64, usize)> = None;
        if latest >= 0.0 {
            for candidate in (earliest.max(0.0).ceil() as usize)..=(latest as usize) {
                let off_tempo = ((step - candidate) as f64 / period).ln();
                let value = score[candidate] - TIGHTNESS * off_tempo * off_tempo;
                if best.is_none_or(|(best_value, _)| value > best_value) {
                    best = Some((value, candidate));
                }
            }
        }
        score[step] = f64::from(envelope[step]) + best.map_or(0.0, |(value, _)| value.max(0.0));
        previous_beat[step] = best.filter(|(value, _)| *value > 0.0).map(|(_, step)| step);
    }

    // The chain ends at the best moment within the last beat.
    let last_period = (envelope.len() as f64 - period).max(0.0) as usize;
    let mut step = (last_period..envelope.len())
        .max_by(|a, b| score[*a].total_cmp(&score[*b]))
        .unwrap_or(0);
    let mut steps = vec![step];
    while let Some(earlier) = previous_beat[step] {
        steps.push(earlier);
        step = earlier;
    }
    steps.reverse();

    // How much happens around each beat.
    let reach = PEAK_REACH;
    let activity: Vec<(usize, f32)> = steps
        .into_iter()
        .map(|step| {
            let around =
                &envelope[step.saturating_sub(reach)..(step + reach + 1).min(envelope.len())];
            (step, around.iter().copied().fold(0.0, f32::max))
        })
        .collect();
    let mut sorted: Vec<f32> = activity.iter().map(|(_, value)| *value).collect();
    sorted.sort_by(f32::total_cmp);
    let reference = sorted
        .get(sorted.len().saturating_sub(1) * 9 / 10)
        .copied()
        .unwrap_or(1.0)
        .max(1e-6);
    // Beats in quiet passages (and the chain running on past the end of
    // the music) are left out: nothing should pulse where nothing plays.
    let quietest = QUIET.max(WEAKEST_SHARE * reference);
    activity
        .into_iter()
        .filter(|(_, value)| *value >= quietest)
        .map(|(step, value)| Beat {
            seconds: step as f64 * seconds_per_step,
            strength: (value / reference).min(1.0),
        })
        .collect()
}

/// How much new energy appears at each analysed moment, scaled so the
/// largest value is 1. Empty or all zero for silence.
fn change_envelope(mono: &[f32], sample_rate: f64, hop: usize) -> Vec<f32> {
    let fft = FftPlanner::new().plan_fft_forward(WINDOW);
    let window: Vec<f32> = (0..WINDOW)
        .map(|index| {
            let phase = index as f32 / (WINDOW - 1) as f32;
            0.5 - 0.5 * (std::f32::consts::TAU * phase).cos()
        })
        .collect();
    let hertz_per_bin = sample_rate / WINDOW as f64;
    let top_bin = ((HIGHEST_FREQUENCY / hertz_per_bin) as usize).clamp(2, WINDOW / 2);
    let bass_bin = (BASS_LIMIT / hertz_per_bin).ceil() as usize;

    let mut buffer = vec![Complex32::default(); WINDOW];
    let mut previous = vec![0.0f32; top_bin];
    let mut current = vec![0.0f32; top_bin];
    // The change in the bass and the change above it, kept apart.
    let mut bass = Vec::with_capacity(mono.len() / hop + 1);
    let mut rest = Vec::with_capacity(mono.len() / hop + 1);

    // Moment `step` looks at the samples just before it, so a beat is
    // found when it starts, not before.
    for step in 0..=mono.len() / hop {
        let end = step * hop;
        for (index, value) in buffer.iter_mut().enumerate() {
            let sample = (end + index)
                .checked_sub(WINDOW)
                .and_then(|position| mono.get(position))
                .copied()
                .unwrap_or(0.0);
            *value = Complex32::new(sample * window[index], 0.0);
        }
        fft.process(&mut buffer);

        let (mut bass_change, mut rest_change) = (0.0f32, 0.0f32);
        for bin in 1..top_bin {
            // Logarithmic, like hearing: a quiet sound that doubles counts
            // about as much as a loud one that doubles.
            current[bin] = (1.0 + 100.0 * buffer[bin].norm()).ln();
            let rise = (current[bin] - previous[bin]).max(0.0);
            if bin <= bass_bin {
                bass_change += rise;
            } else {
                rest_change += rise;
            }
        }
        std::mem::swap(&mut previous, &mut current);
        bass.push(bass_change);
        rest.push(rest_change);
    }

    // Each band is measured against its own strongest hit, so a loud
    // cymbal cannot drown out the kick drum, and then the two are mixed
    // with the bass in front.
    let bass = normalized(&hits_only(&bass));
    let rest = normalized(&hits_only(&rest));
    let mixed: Vec<f32> = bass
        .iter()
        .zip(&rest)
        .map(|(bass, rest)| BASS_SHARE * bass + (1.0 - BASS_SHARE) * rest)
        .collect();
    normalized(&mixed)
}

/// Takes away the steady background (noise, a held sound) and keeps the
/// hits that rise above it.
fn hits_only(changes: &[f32]) -> Vec<f32> {
    (0..changes.len())
        .map(|step| {
            let around = &changes[step.saturating_sub(BACKGROUND_REACH)
                ..(step + BACKGROUND_REACH + 1).min(changes.len())];
            let background = around.iter().sum::<f32>() / around.len() as f32;
            (changes[step] - background).max(0.0)
        })
        .collect()
}

/// Scaled so the largest value is 1; all zero if there is (next to)
/// nothing.
fn normalized(values: &[f32]) -> Vec<f32> {
    let largest = values.iter().copied().fold(0.0f32, f32::max);
    if largest <= 1e-3 {
        return vec![0.0; values.len()];
    }
    values.iter().map(|value| value / largest).collect()
}
/// The moments that are clearly larger than what surrounds them.
fn pick_beats(envelope: &[f32], seconds_per_step: f64) -> Vec<Beat> {
    let min_gap = (MIN_BEAT_GAP_SECONDS / seconds_per_step).round() as usize;
    let mut candidates: Vec<(usize, f32)> = Vec::new();

    for (step, &value) in envelope.iter().enumerate() {
        let around = |reach: usize| {
            &envelope[step.saturating_sub(reach)..(step + reach + 1).min(envelope.len())]
        };
        let is_peak = around(PEAK_REACH).iter().all(|&other| other <= value);
        let nearby = around(AVERAGE_REACH);
        let average = nearby.iter().sum::<f32>() / nearby.len() as f32;
        if !is_peak || value < average + THRESHOLD_ABOVE_AVERAGE {
            continue;
        }
        match candidates.last_mut() {
            // Too close to the previous beat: keep the stronger of the two.
            Some(last) if step - last.0 < min_gap => {
                if value > last.1 {
                    *last = (step, value);
                }
            }
            _ => candidates.push((step, value)),
        }
    }

    // Strength relative to the typical strong beat of the file.
    let mut strengths: Vec<f32> = candidates.iter().map(|(_, value)| *value).collect();
    strengths.sort_by(f32::total_cmp);
    let reference = strengths
        .get(strengths.len().saturating_sub(1) * 9 / 10)
        .copied()
        .unwrap_or(1.0)
        .max(1e-6);
    candidates
        .into_iter()
        .map(|(step, value)| Beat {
            seconds: step as f64 * seconds_per_step,
            strength: (value / reference).min(1.0),
        })
        .collect()
}

/// The tempo at which the change envelope repeats itself best.
fn estimate_tempo(envelope: &[f32], seconds_per_step: f64) -> Option<f64> {
    let (slowest, fastest) = TEMPO_RANGE;
    let shortest_lag = (60.0 / fastest / seconds_per_step).floor() as usize;
    let longest_lag = (60.0 / slowest / seconds_per_step).ceil() as usize;
    // At least a few beats at the slowest tempo are needed.
    if envelope.len() < longest_lag * 4 {
        return None;
    }

    let mean = envelope.iter().sum::<f32>() / envelope.len() as f32;
    let centered: Vec<f32> = envelope.iter().map(|value| value - mean).collect();
    let mut best: Option<(f64, f64)> = None;
    for lag in shortest_lag.max(1)..=longest_lag {
        let correlation: f32 = centered
            .iter()
            .zip(&centered[lag..])
            .map(|(a, b)| a * b)
            .sum();
        let bpm = 60.0 / (lag as f64 * seconds_per_step);
        // Of two tempos that fit equally well (one double the other),
        // the one nearer to 120 is the more likely beat.
        let likelihood = (-0.5 * (bpm / 120.0).log2().powi(2)).exp();
        let score = f64::from(correlation) * likelihood;
        if score > 0.0 && best.is_none_or(|(best_score, _)| score > best_score) {
            best = Some((score, bpm));
        }
    }
    best.map(|(_, bpm)| bpm)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 22_050;

    /// A kick drum every `interval` seconds over quiet noise.
    fn kicks(interval: f64, seconds: f64) -> DecodedAudio {
        let mut noise_state = 12345u32;
        let samples = (0..(seconds * f64::from(RATE)) as usize)
            .map(|index| {
                let time = index as f64 / f64::from(RATE);
                let since_kick = time % interval;
                let kick = (-since_kick * 30.0).exp()
                    * (std::f64::consts::TAU * 60.0 * since_kick).sin()
                    * 0.9;
                // A little deterministic noise, as in real recordings.
                noise_state = noise_state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let noise = (noise_state >> 16) as f64 / 65_536.0 - 0.5;
                (kick + noise * 0.02) as f32
            })
            .collect();
        DecodedAudio {
            sample_rate: RATE,
            channel_count: 1,
            samples,
        }
    }

    #[test]
    fn regular_kicks_are_found_on_time_with_their_tempo() {
        let analysis = detect_beats(&kicks(0.5, 8.0));

        let count = analysis.beats.len();
        assert!(
            (15..=17).contains(&count),
            "{count} beats: {:?}",
            analysis.beats
        );
        for beat in &analysis.beats {
            let off_grid = (beat.seconds + 0.25) % 0.5 - 0.25;
            assert!(off_grid.abs() < 0.03, "beat at {} s", beat.seconds);
            assert!(beat.strength > 0.5, "{beat:?} in {:?}", analysis.beats);
        }
        let tempo = analysis.tempo_bpm.unwrap();
        assert!((115.0..125.0).contains(&tempo), "tempo {tempo}");
    }

    #[test]
    fn hi_hats_between_the_kicks_are_not_beats() {
        // Kicks every 0.5 s, and a short, high hi-hat halfway between two
        // kicks.
        let mut audio = kicks(0.5, 10.0);
        for (index, sample) in audio.samples.iter_mut().enumerate() {
            let time = index as f64 / f64::from(RATE);
            let since_hat = (time + 0.25) % 0.5;
            if since_hat < 0.03 {
                let tone = (std::f64::consts::TAU * 6_000.0 * time).sin()
                    + (std::f64::consts::TAU * 7_300.0 * time).sin();
                *sample += (tone * 0.2 * (1.0 - since_hat / 0.03)) as f32;
            }
        }

        let analysis = detect_beats(&audio);

        let tempo = analysis.tempo_bpm.unwrap();
        assert!((115.0..125.0).contains(&tempo), "tempo {tempo}");
        let count = analysis.beats.len();
        assert!(
            (18..=21).contains(&count),
            "{count} beats: {:?}",
            analysis.beats
        );
        let on_kicks = analysis
            .beats
            .iter()
            .filter(|beat| ((beat.seconds + 0.25) % 0.5 - 0.25).abs() < 0.04)
            .count();
        assert!(
            on_kicks >= count - 1,
            "{on_kicks} of {count} beats on the kicks: {:?}",
            analysis.beats
        );
    }

    #[test]
    fn a_faster_pattern_gives_a_faster_tempo() {
        let tempo = detect_beats(&kicks(60.0 / 150.0, 10.0)).tempo_bpm.unwrap();

        assert!((145.0..155.0).contains(&tempo), "tempo {tempo}");
    }

    #[test]
    fn silence_and_a_short_click_have_no_tempo() {
        let silence = DecodedAudio {
            sample_rate: RATE,
            channel_count: 2,
            samples: vec![0.0; RATE as usize * 6],
        };
        assert_eq!(detect_beats(&silence), BeatAnalysis::default());

        let short = detect_beats(&kicks(0.5, 1.0));
        assert!(short.tempo_bpm.is_none());
        assert!(!short.beats.is_empty());
    }
}
