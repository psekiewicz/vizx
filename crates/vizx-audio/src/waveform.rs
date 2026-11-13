use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::TrackType;

use crate::AudioFileError;
use crate::probe::open_format;

/// A compact summary of an audio file's loudness over time, for drawing a
/// waveform.
///
/// A song has millions of samples, far more than there are pixels to draw
/// them on. So the audio is cut into short, equal slices of time ("buckets")
/// and only the lowest and highest sample value of each slice is kept. A few
/// minutes of audio then take a few hundred kilobytes instead of tens of
/// megabytes, and drawing stays fast at any zoom level.
#[derive(Debug, Clone, PartialEq)]
pub struct WaveformPeaks {
    buckets_per_second: u32,
    /// Lowest sample value in each bucket, in the range -1.0..=1.0.
    minimums: Vec<f32>,
    /// Highest sample value in each bucket, in the range -1.0..=1.0.
    maximums: Vec<f32>,
}

impl WaveformPeaks {
    /// Resolution used by VizX: one bucket per 5 milliseconds of audio.
    pub const DEFAULT_BUCKETS_PER_SECOND: u32 = 200;

    pub fn bucket_count(&self) -> usize {
        self.minimums.len()
    }

    pub fn duration_seconds(&self) -> f64 {
        self.bucket_count() as f64 / f64::from(self.buckets_per_second)
    }

    /// Lowest and highest sample value between two points in time.
    ///
    /// This is what one pixel column of a waveform shows. Returns `None` if
    /// the range lies outside the audio.
    pub fn range(&self, start_seconds: f64, end_seconds: f64) -> Option<(f32, f32)> {
        if end_seconds <= 0.0 || start_seconds >= self.duration_seconds() {
            return None;
        }

        let buckets_per_second = f64::from(self.buckets_per_second);
        let last_bucket = self.bucket_count().checked_sub(1)?;
        let first = ((start_seconds.max(0.0) * buckets_per_second) as usize).min(last_bucket);
        // Always include at least one bucket, even for a very short range.
        let end = ((end_seconds * buckets_per_second).ceil() as usize)
            .clamp(first + 1, self.bucket_count());

        let minimum = self.minimums[first..end]
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min);
        let maximum = self.maximums[first..end]
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        Some((minimum, maximum))
    }
}

/// Decodes a whole audio file and summarizes it as waveform peaks.
///
/// This reads every sample of the file, so it can take seconds for a long
/// song. Call it from a background thread, never from the UI thread.
pub fn compute_waveform_peaks(path: &Path) -> Result<WaveformPeaks, AudioFileError> {
    let mut format = open_format(path)?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or(AudioFileError::NoAudioTrack)?;
    let track_id = track.id;
    let audio_parameters = track
        .codec_params
        .as_ref()
        .and_then(|parameters| parameters.audio())
        .ok_or(AudioFileError::NoAudioTrack)?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(audio_parameters, &AudioDecoderOptions::default())
        .map_err(|error| AudioFileError::UnsupportedFormat(error.to_string()))?;

    // Created when the first packet is decoded, because only then are the
    // real sample rate and channel count known for certain.
    let mut builder: Option<PeakBuilder> = None;
    // Reused for every packet to avoid allocating memory over and over.
    let mut samples: Vec<f32> = Vec::new();

    // An audio file is a sequence of "packets", each holding a short piece
    // of compressed audio. Decoding a packet yields its samples.
    while let Ok(Some(packet)) = format.next_packet() {
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let sample_rate = decoded.spec().rate();
                let channel_count = decoded.spec().channels().count();
                decoded.copy_to_vec_interleaved(&mut samples);

                builder
                    .get_or_insert_with(|| {
                        PeakBuilder::new(
                            sample_rate,
                            channel_count,
                            WaveformPeaks::DEFAULT_BUCKETS_PER_SECOND,
                        )
                    })
                    .push_interleaved(&samples);
            }
            // A damaged packet: skip it and keep going.
            Err(SymphoniaError::DecodeError(_)) => {}
            // Anything else means decoding cannot continue. Keep what was
            // decoded so far rather than showing nothing.
            Err(_) => break,
        }
    }

    builder
        .map(PeakBuilder::finish)
        .ok_or(AudioFileError::NoAudioTrack)
}

/// Turns a stream of samples into [`WaveformPeaks`].
struct PeakBuilder {
    sample_rate: u64,
    channel_count: usize,
    buckets_per_second: u32,
    /// Number of frames received so far. One frame is one sample for every
    /// channel at the same moment.
    frame_index: u64,
    /// Bucket the values below belong to.
    current_bucket: u64,
    current_minimum: f32,
    current_maximum: f32,
    minimums: Vec<f32>,
    maximums: Vec<f32>,
}

impl PeakBuilder {
    fn new(sample_rate: u32, channel_count: usize, buckets_per_second: u32) -> Self {
        Self {
            sample_rate: u64::from(sample_rate.max(1)),
            channel_count: channel_count.max(1),
            buckets_per_second,
            frame_index: 0,
            current_bucket: 0,
            current_minimum: f32::INFINITY,
            current_maximum: f32::NEG_INFINITY,
            minimums: Vec::new(),
            maximums: Vec::new(),
        }
    }

    /// Adds samples in interleaved order: for stereo that is left, right,
    /// left, right, ... All channels of a frame go into the same bucket, so
    /// the waveform shows the loudest channel.
    fn push_interleaved(&mut self, samples: &[f32]) {
        for frame in samples.chunks_exact(self.channel_count) {
            let bucket = self.frame_index * u64::from(self.buckets_per_second) / self.sample_rate;
            if bucket != self.current_bucket {
                self.finish_bucket();
                self.current_bucket = bucket;
            }

            for &sample in frame {
                self.current_minimum = self.current_minimum.min(sample);
                self.current_maximum = self.current_maximum.max(sample);
            }
            self.frame_index += 1;
        }
    }

    fn finish_bucket(&mut self) {
        if self.current_minimum <= self.current_maximum {
            self.minimums.push(self.current_minimum.clamp(-1.0, 1.0));
            self.maximums.push(self.current_maximum.clamp(-1.0, 1.0));
        }
        self.current_minimum = f32::INFINITY;
        self.current_maximum = f32::NEG_INFINITY;
    }

    fn finish(mut self) -> WaveformPeaks {
        self.finish_bucket();
        WaveformPeaks {
            buckets_per_second: self.buckets_per_second,
            minimums: self.minimums,
            maximums: self.maximums,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write_wav_file;

    #[test]
    fn samples_are_grouped_into_buckets_by_time() {
        // 100 samples per second and 10 buckets per second: 10 samples each.
        let mut builder = PeakBuilder::new(100, 1, 10);
        let mut samples = vec![0.0; 30];
        samples[3] = 0.5; // first bucket
        samples[12] = -0.25; // second bucket
        samples[29] = 1.0; // third bucket

        builder.push_interleaved(&samples);
        let peaks = builder.finish();

        assert_eq!(peaks.bucket_count(), 3);
        assert_eq!(peaks.maximums, [0.5, 0.0, 1.0]);
        assert_eq!(peaks.minimums, [0.0, -0.25, 0.0]);
        assert!((peaks.duration_seconds() - 0.3).abs() < 1e-9);
    }

    #[test]
    fn result_does_not_depend_on_how_samples_are_split_into_packets() {
        let samples: Vec<f32> = (0..1000).map(|index| (index as f32 * 0.37).sin()).collect();

        let mut all_at_once = PeakBuilder::new(100, 1, 10);
        all_at_once.push_interleaved(&samples);

        let mut in_pieces = PeakBuilder::new(100, 1, 10);
        for piece in samples.chunks(7) {
            in_pieces.push_interleaved(piece);
        }

        assert_eq!(all_at_once.finish(), in_pieces.finish());
    }

    #[test]
    fn stereo_frames_keep_the_loudest_channel() {
        let mut builder = PeakBuilder::new(100, 2, 10);
        // Ten stereo frames: left stays at 0.1, right at -0.8.
        let samples: Vec<f32> = (0..10).flat_map(|_| [0.1, -0.8]).collect();

        builder.push_interleaved(&samples);
        let peaks = builder.finish();

        assert_eq!(peaks.bucket_count(), 1);
        assert_eq!(peaks.range(0.0, 0.1), Some((-0.8, 0.1)));
    }

    #[test]
    fn range_combines_buckets_and_rejects_times_outside_the_audio() {
        let peaks = WaveformPeaks {
            buckets_per_second: 10,
            minimums: vec![-0.1, -0.5, -0.2, 0.0],
            maximums: vec![0.1, 0.3, 0.9, 0.0],
        };

        assert_eq!(peaks.range(0.0, 0.1), Some((-0.1, 0.1)));
        assert_eq!(peaks.range(0.1, 0.3), Some((-0.5, 0.9)));
        // A range narrower than one bucket still reports that bucket.
        assert_eq!(peaks.range(0.25, 0.26), Some((-0.2, 0.9)));
        assert_eq!(peaks.range(0.0, 100.0), Some((-0.5, 0.9)));
        assert_eq!(peaks.range(0.4, 0.5), None);
        assert_eq!(peaks.range(-1.0, 0.0), None);
    }

    #[test]
    fn wav_file_is_decoded_into_peaks_that_follow_its_loudness() {
        // One second of audio: silent first half, full-scale square wave in
        // the second half.
        let sample_rate = 8_000;
        let samples: Vec<i16> = (0..sample_rate)
            .map(|index| {
                if index < sample_rate / 2 {
                    0
                } else if index % 2 == 0 {
                    i16::MAX
                } else {
                    i16::MIN
                }
            })
            .collect();
        let path =
            std::env::temp_dir().join(format!("vizx-audio-peaks-{}.wav", std::process::id()));
        write_wav_file(&path, sample_rate as u32, 1, &samples).unwrap();

        let peaks = compute_waveform_peaks(&path);
        let _ = std::fs::remove_file(&path);
        let peaks = peaks.unwrap();

        assert!((peaks.duration_seconds() - 1.0).abs() < 0.01);
        assert_eq!(peaks.range(0.0, 0.45), Some((0.0, 0.0)));
        let (minimum, maximum) = peaks.range(0.55, 1.0).unwrap();
        assert!(minimum < -0.99, "minimum was {minimum}");
        assert!(maximum > 0.99, "maximum was {maximum}");
    }
}
