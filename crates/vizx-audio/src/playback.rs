//! Playing audio through the sound card.
//!
//! The sound card asks for audio in small pieces, many times per second, on
//! its own thread (the "audio callback"). That thread must never wait: if it
//! is late, the result is an audible click. So the UI thread and the audio
//! thread only talk through a few values that can be read and written
//! without waiting (atomics), plus one lock the audio thread only *tries* to
//! take.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

use crate::DecodedAudio;

/// Far below the length of one sample; see its use in `mix_sources`.
const END_TOLERANCE_SECONDS: f64 = 1e-9;

/// One piece of audio placed on the project timeline.
#[derive(Debug, Clone)]
pub struct PlaybackSource {
    pub audio: Arc<DecodedAudio>,
    /// Project time at which this audio begins.
    pub start_seconds: f64,
    /// Where in the audio it begins, in seconds: 0 plays it from its start,
    /// more skips the beginning (a clip trimmed at its left edge).
    pub offset_seconds: f64,
    /// How long it plays, in seconds. `None` plays to the end of the audio
    /// (a clip trimmed at its right edge stops earlier).
    pub duration_seconds: Option<f64>,
}

impl PlaybackSource {
    /// The whole of `audio`, beginning at `start_seconds` of project time.
    pub fn whole(audio: Arc<DecodedAudio>, start_seconds: f64) -> Self {
        Self {
            audio,
            start_seconds,
            offset_seconds: 0.0,
            duration_seconds: None,
        }
    }
}

/// Why audio could not be played.
#[derive(Debug)]
pub struct PlaybackError(String);

impl fmt::Display for PlaybackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "audio output failed: {}", self.0)
    }
}

impl std::error::Error for PlaybackError {}

impl From<cpal::Error> for PlaybackError {
    fn from(error: cpal::Error) -> Self {
        Self(error.to_string())
    }
}

/// State shared between the UI thread and the audio thread.
struct SharedState {
    is_playing: AtomicBool,
    /// Playback position in frames at the sound card's sample rate. While
    /// playing, this is the clock of the whole project: it advances by
    /// exactly the audio that was handed to the sound card.
    position_frames: AtomicU64,
    /// Position at which playback stops by itself (the end of the project).
    end_frame: AtomicU64,
    /// Loudness of what is heard, 0 to 1, as the bits of an `f32`. It only
    /// changes the output, not what the visualizers or the export get.
    volume_bits: AtomicU32,
    /// How far, in microseconds, the position runs ahead of what can be
    /// heard: one buffer plus the device's delay. Measured by the audio
    /// thread.
    heard_delay_micros: AtomicU64,
    sources: Mutex<Arc<Vec<PlaybackSource>>>,
}

/// Plays [`PlaybackSource`]s on the default sound output device.
///
/// It must stay on the thread that created it: on some systems the audio
/// stream cannot be moved between threads.
pub struct AudioPlayer {
    stream: cpal::Stream,
    shared: Arc<SharedState>,
    output_sample_rate: u32,
    device_description: String,
}

impl AudioPlayer {
    /// Opens the system's default output device. The stream starts paused.
    pub fn new() -> Result<Self, PlaybackError> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or_else(|| PlaybackError("no audio output device found".to_owned()))?;
        let supported_config = device.default_output_config()?;
        let sample_format = supported_config.sample_format();
        let config: cpal::StreamConfig = supported_config.into();

        let shared = Arc::new(SharedState {
            is_playing: AtomicBool::new(false),
            position_frames: AtomicU64::new(0),
            end_frame: AtomicU64::new(u64::MAX),
            volume_bits: AtomicU32::new(1.0_f32.to_bits()),
            heard_delay_micros: AtomicU64::new(0),
            sources: Mutex::new(Arc::new(Vec::new())),
        });

        // Sound cards differ in the number format they want. The mixing is
        // always done in f32 and converted at the very end.
        let stream = match sample_format {
            SampleFormat::F32 => build_stream::<f32>(&device, config, &shared),
            SampleFormat::I16 => build_stream::<i16>(&device, config, &shared),
            SampleFormat::U16 => build_stream::<u16>(&device, config, &shared),
            SampleFormat::I32 => build_stream::<i32>(&device, config, &shared),
            other => Err(PlaybackError(format!(
                "unsupported output sample format {other}"
            ))),
        }?;
        // Some systems start a new stream immediately; VizX starts paused.
        stream.pause()?;

        Ok(Self {
            stream,
            shared,
            output_sample_rate: config.sample_rate,
            device_description: format!(
                "{} Hz, {} channels, {sample_format}",
                config.sample_rate, config.channels
            ),
        })
    }

    /// Sample rate, channel count and sample format of the output device.
    pub fn device_description(&self) -> &str {
        &self.device_description
    }

    /// Replaces what is played. Safe to call during playback.
    pub fn set_sources(&self, sources: Vec<PlaybackSource>) {
        // The lock is only held for the instant it takes to swap a pointer.
        if let Ok(mut current) = self.shared.sources.lock() {
            *current = Arc::new(sources);
        }
    }

    /// Sets the project time at which playback stops by itself.
    pub fn set_end_seconds(&self, seconds: f64) {
        self.shared
            .end_frame
            .store(self.seconds_to_frames(seconds), Ordering::Relaxed);
    }

    /// Sets how loud playback is: 0 is silent, 1 is as mixed.
    pub fn set_volume(&self, volume: f32) {
        let volume = if volume.is_finite() { volume.clamp(0.0, 1.0) } else { 1.0 };
        self.shared
            .volume_bits
            .store(volume.to_bits(), Ordering::Relaxed);
    }

    pub fn play(&self) -> Result<(), PlaybackError> {
        self.shared.is_playing.store(true, Ordering::Relaxed);
        self.stream.play()?;
        Ok(())
    }

    /// Stops the sound and the sound card stream, keeping the position.
    pub fn pause(&self) {
        self.shared.is_playing.store(false, Ordering::Relaxed);
        // Pausing the stream stops the audio thread while nothing plays.
        // A failure here only means the device keeps running silently.
        let _ = self.stream.pause();
    }

    /// `false` when paused, and also once playback has reached the end.
    pub fn is_playing(&self) -> bool {
        self.shared.is_playing.load(Ordering::Relaxed)
    }

    pub fn position_seconds(&self) -> f64 {
        self.shared.position_frames.load(Ordering::Relaxed) as f64
            / f64::from(self.output_sample_rate)
    }

    /// How far [`Self::position_seconds`] runs ahead of what is heard while
    /// playing. Something done "on the beat one hears" (tapping along)
    /// happened this much earlier in the project.
    pub fn heard_delay_seconds(&self) -> f64 {
        // Capped, in case a device reports nonsense.
        (self.shared.heard_delay_micros.load(Ordering::Relaxed) as f64 / 1e6).min(0.5)
    }

    pub fn seek(&self, seconds: f64) {
        self.shared
            .position_frames
            .store(self.seconds_to_frames(seconds), Ordering::Relaxed);
    }

    fn seconds_to_frames(&self, seconds: f64) -> u64 {
        (seconds.max(0.0) * f64::from(self.output_sample_rate)).round() as u64
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    shared: &Arc<SharedState>,
) -> Result<cpal::Stream, PlaybackError>
where
    T: SizedSample + FromSample<f32>,
{
    let channel_count = usize::from(config.channels);
    let sample_rate = config.sample_rate;
    let shared = Arc::clone(shared);
    // Reused by every callback, so the audio thread never allocates memory
    // after the first call.
    let mut mix_buffer: Vec<f32> = Vec::new();

    let stream = device.build_output_stream(
        config,
        move |output: &mut [T], info: &cpal::OutputCallbackInfo| {
            let timestamp = info.timestamp();
            let device_delay = timestamp.playback.duration_since(timestamp.callback);
            let buffer_seconds =
                (output.len() / channel_count.max(1)) as f64 / f64::from(sample_rate.max(1));
            let delay = device_delay.as_secs_f64() + buffer_seconds;
            shared
                .heard_delay_micros
                .store((delay * 1e6) as u64, Ordering::Relaxed);
            mix_buffer.clear();
            mix_buffer.resize(output.len(), 0.0);
            fill_from_shared(&mut mix_buffer, channel_count, sample_rate, &shared);

            for (output_sample, mixed) in output.iter_mut().zip(&mix_buffer) {
                *output_sample = T::from_sample(*mixed);
            }
        },
        // Stream errors (e.g. the device was unplugged) cannot be shown from
        // the audio thread; playback simply goes silent.
        |_error| {},
        None,
    )?;
    Ok(stream)
}

/// The body of the audio callback: writes the next piece of the project's
/// audio into `output`, which must already be silent, and advances the
/// playback position.
fn fill_from_shared(
    output: &mut [f32],
    channel_count: usize,
    sample_rate: u32,
    shared: &SharedState,
) {
    if !shared.is_playing.load(Ordering::Relaxed) {
        return;
    }
    // `try_lock` never waits. In the rare case the UI thread is swapping the
    // sources at this very moment, this one short piece stays silent.
    let Ok(sources) = shared.sources.try_lock() else {
        return;
    };

    let position = shared.position_frames.load(Ordering::Relaxed);
    let end = shared.end_frame.load(Ordering::Relaxed);
    let requested_frames = (output.len() / channel_count.max(1)) as u64;
    let playable_frames = requested_frames.min(end.saturating_sub(position));

    let playable_samples = playable_frames as usize * channel_count;
    mix_sources(
        &mut output[..playable_samples],
        channel_count,
        sample_rate,
        position,
        &sources,
    );
    let volume = f32::from_bits(shared.volume_bits.load(Ordering::Relaxed));
    if volume < 1.0 {
        for sample in &mut output[..playable_samples] {
            *sample *= volume;
        }
    }

    // Advance the position, unless the UI thread moved it (a seek) while
    // this piece was being mixed; in that case the seek wins.
    let _ = shared.position_frames.compare_exchange(
        position,
        position + playable_frames,
        Ordering::Relaxed,
        Ordering::Relaxed,
    );
    if playable_frames < requested_frames {
        shared.is_playing.store(false, Ordering::Relaxed);
    }
}

/// Mixes the sources into stereo samples (left, right, left, right, ...)
/// for a stretch of project time, without playing anything.
///
/// This is the same mix the sound card receives. Anything that needs to
/// "hear" the project, such as a visualizer, uses it so that what it reacts
/// to is exactly what is audible.
pub fn mix_to_stereo(
    sources: &[PlaybackSource],
    sample_rate: u32,
    start_frame: u64,
    frame_count: usize,
) -> Vec<f32> {
    let mut samples = vec![0.0; frame_count * 2];
    mix_sources(&mut samples, 2, sample_rate, start_frame, sources);
    samples
}

/// Adds every source into `output`, starting at project frame `position`.
///
/// `output` is interleaved with `channel_count` channels at `sample_rate`.
/// Sources may have any sample rate; they are resampled on the fly.
fn mix_sources(
    output: &mut [f32],
    channel_count: usize,
    sample_rate: u32,
    position: u64,
    sources: &[PlaybackSource],
) {
    if channel_count == 0 {
        return;
    }
    let output_rate = f64::from(sample_rate);

    for (frame_offset, frame) in output.chunks_exact_mut(channel_count).enumerate() {
        let project_seconds = (position + frame_offset as u64) as f64 / output_rate;

        let mut left = 0.0f32;
        let mut right = 0.0f32;
        for source in sources {
            let seconds_into_source = project_seconds - source.start_seconds;
            // Times like 0.3 - 0.1 are not exactly 0.2 in floating point;
            // the tiny tolerance keeps a clip from playing one sample past
            // its end because of that.
            let has_ended = source
                .duration_seconds
                .is_some_and(|duration| seconds_into_source + END_TOLERANCE_SECONDS >= duration);
            if seconds_into_source < 0.0 || has_ended {
                continue; // not started yet, or already over
            }
            let seconds_into_audio = seconds_into_source + source.offset_seconds;
            let [source_left, source_right] = source
                .audio
                .stereo_at(seconds_into_audio * f64::from(source.audio.sample_rate));
            left += source_left;
            right += source_right;
        }
        // Several sources added together can exceed the valid range; limit
        // the result so it cannot wrap around into loud noise.
        let left = left.clamp(-1.0, 1.0);
        let right = right.clamp(-1.0, 1.0);

        match frame {
            [mono] => *mono = (left + right) * 0.5,
            [first, second, rest @ ..] => {
                *first = left;
                *second = right;
                rest.fill(0.0); // surround channels stay silent
            }
            [] => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(sample_rate: u32, samples: &[f32], start_seconds: f64) -> PlaybackSource {
        PlaybackSource::whole(
            Arc::new(DecodedAudio {
                sample_rate,
                channel_count: 1,
                samples: samples.to_vec(),
            }),
            start_seconds,
        )
    }

    fn shared(sources: Vec<PlaybackSource>, position: u64, end: u64) -> SharedState {
        SharedState {
            is_playing: AtomicBool::new(true),
            position_frames: AtomicU64::new(position),
            end_frame: AtomicU64::new(end),
            volume_bits: AtomicU32::new(1.0_f32.to_bits()),
            heard_delay_micros: AtomicU64::new(0),
            sources: Mutex::new(Arc::new(sources)),
        }
    }

    #[test]
    fn volume_scales_what_is_heard() {
        let audio = Arc::new(DecodedAudio {
            sample_rate: 100,
            channel_count: 1,
            samples: vec![0.8; 100],
        });
        let source = PlaybackSource {
            audio,
            start_seconds: 0.0,
            offset_seconds: 0.0,
            duration_seconds: None,
        };
        let state = shared(vec![source], 0, u64::MAX);
        state.volume_bits.store(0.5_f32.to_bits(), Ordering::Relaxed);
        let mut output = vec![0.0; 10];
        fill_from_shared(&mut output, 1, 100, &state);
        assert!(output.iter().all(|sample| (sample - 0.4).abs() < 1e-6), "{output:?}");
    }

    #[test]
    fn source_at_the_output_rate_is_copied_to_both_channels() {
        let sources = [source(10, &[0.1, 0.2, 0.3, 0.4], 0.0)];
        let mut output = [0.0; 8];

        mix_sources(&mut output, 2, 10, 0, &sources);

        assert_eq!(output, [0.1, 0.1, 0.2, 0.2, 0.3, 0.3, 0.4, 0.4]);
    }

    #[test]
    fn playback_position_selects_the_part_of_the_source() {
        let sources = [source(10, &[0.1, 0.2, 0.3, 0.4], 0.0)];
        let mut output = [0.0; 2];

        mix_sources(&mut output, 1, 10, 2, &sources);

        assert_eq!(output, [0.3, 0.4]);
    }

    #[test]
    fn source_with_a_later_start_is_silent_until_then() {
        // Starts 0.2 s into the project, which is 2 frames at 10 Hz.
        let sources = [source(10, &[0.5, 0.5], 0.2)];
        let mut output = [0.0; 5];

        mix_sources(&mut output, 1, 10, 0, &sources);

        assert_eq!(output, [0.0, 0.0, 0.5, 0.5, 0.0]);
    }

    #[test]
    fn a_trimmed_source_plays_only_its_middle_part() {
        // Skip the first 0.2 s of the audio and play 0.2 s of it, starting
        // 0.1 s into the project.
        let sources = [PlaybackSource {
            offset_seconds: 0.2,
            duration_seconds: Some(0.2),
            ..source(10, &[0.1, 0.2, 0.3, 0.4, 0.5, 0.6], 0.1)
        }];
        let mut output = [0.0; 5];

        mix_sources(&mut output, 1, 10, 0, &sources);

        assert_eq!(output, [0.0, 0.3, 0.4, 0.0, 0.0]);
    }

    #[test]
    fn source_at_half_the_output_rate_is_stretched() {
        // 5 Hz audio played at 10 Hz: every source frame covers two output
        // frames, with interpolated values in between.
        let sources = [source(5, &[0.0, 1.0], 0.0)];
        let mut output = [0.0; 3];

        mix_sources(&mut output, 1, 10, 0, &sources);

        assert_eq!(output, [0.0, 0.5, 1.0]);
    }

    #[test]
    fn sources_are_added_and_limited_to_the_valid_range() {
        let sources = [source(10, &[0.3, 0.8], 0.0), source(10, &[0.2, 0.8], 0.0)];
        let mut output = [0.0; 2];

        mix_sources(&mut output, 1, 10, 0, &sources);

        assert_eq!(output, [0.5, 1.0]);
    }

    #[test]
    fn stereo_mixdown_matches_what_playback_produces() {
        let sources = [source(10, &[0.1, 0.2, 0.3, 0.4], 0.0)];

        let samples = mix_to_stereo(&sources, 10, 1, 2);

        assert_eq!(samples, [0.2, 0.2, 0.3, 0.3]);
    }

    #[test]
    fn callback_advances_the_position_by_what_it_played() {
        let state = shared(vec![source(10, &[0.1; 100], 0.0)], 20, u64::MAX);
        let mut output = [0.0; 8];

        fill_from_shared(&mut output, 2, 10, &state);

        assert_eq!(state.position_frames.load(Ordering::Relaxed), 24);
        assert!(state.is_playing.load(Ordering::Relaxed));
        assert_eq!(output, [0.1; 8]);
    }

    #[test]
    fn callback_stops_at_the_end_and_leaves_the_rest_silent() {
        let state = shared(vec![source(10, &[0.1; 100], 0.0)], 8, 10);
        let mut output = [0.0; 4];

        fill_from_shared(&mut output, 1, 10, &state);

        assert_eq!(output, [0.1, 0.1, 0.0, 0.0]);
        assert_eq!(state.position_frames.load(Ordering::Relaxed), 10);
        assert!(!state.is_playing.load(Ordering::Relaxed));
    }

    #[test]
    fn paused_callback_outputs_silence_and_does_not_move() {
        let state = shared(vec![source(10, &[0.1; 100], 0.0)], 5, u64::MAX);
        state.is_playing.store(false, Ordering::Relaxed);
        let mut output = [0.0; 4];

        fill_from_shared(&mut output, 1, 10, &state);

        assert_eq!(output, [0.0; 4]);
        assert_eq!(state.position_frames.load(Ordering::Relaxed), 5);
    }
}
