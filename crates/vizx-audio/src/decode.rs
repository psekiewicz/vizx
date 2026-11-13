use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::TrackType;

use crate::AudioFileError;
use crate::probe::open_format;

/// The complete, uncompressed content of an audio file, held in memory.
///
/// Keeping a song decoded costs memory (roughly 20 MB per minute of stereo
/// audio), but it makes playback, seeking and analysis simple and instant.
/// Streaming from disk can replace this later without changing its users.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedAudio {
    /// Samples per second for each channel, e.g. 44100.
    pub sample_rate: u32,
    pub channel_count: usize,
    /// Sample values from -1.0 to 1.0 in interleaved order: for stereo that
    /// is left, right, left, right, ...
    pub samples: Vec<f32>,
}

impl DecodedAudio {
    /// Number of frames. One frame is one sample for every channel at the
    /// same moment.
    pub fn frame_count(&self) -> usize {
        self.samples.len() / self.channel_count.max(1)
    }

    pub fn duration_seconds(&self) -> f64 {
        self.frame_count() as f64 / f64::from(self.sample_rate.max(1))
    }

    /// The left and right sample value at a position given in frames, which
    /// may fall between two frames.
    ///
    /// The value between two frames is estimated by drawing a straight line
    /// between them ("linear interpolation"). This is what lets a file be
    /// played on a sound card that runs at a different sample rate. Mono
    /// audio is returned on both sides. Outside the audio the result is
    /// silence.
    pub fn stereo_at(&self, frame_position: f64) -> [f32; 2] {
        if frame_position < 0.0 {
            return [0.0, 0.0];
        }
        let index = frame_position as usize;
        let fraction = (frame_position - index as f64) as f32;

        let current = self.stereo_frame(index);
        let next = self.stereo_frame(index + 1);
        [
            current[0] + (next[0] - current[0]) * fraction,
            current[1] + (next[1] - current[1]) * fraction,
        ]
    }

    fn stereo_frame(&self, index: usize) -> [f32; 2] {
        let start = index * self.channel_count;
        match self.samples.get(start..start + self.channel_count) {
            Some([mono]) => [*mono, *mono],
            Some([left, right, ..]) => [*left, *right],
            _ => [0.0, 0.0],
        }
    }
}

/// Decodes a whole audio file into memory.
///
/// This reads every sample of the file, so it can take seconds for a long
/// song. Call it from a background thread, never from the UI thread.
pub fn decode_audio_file(path: &Path) -> Result<DecodedAudio, AudioFileError> {
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

    // Filled in when the first packet is decoded, because only then are the
    // real sample rate and channel count known for certain.
    let mut audio: Option<DecodedAudio> = None;
    // Reused for every packet to avoid allocating memory over and over.
    let mut packet_samples: Vec<f32> = Vec::new();

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
                decoded.copy_to_vec_interleaved(&mut packet_samples);

                let audio = audio.get_or_insert_with(|| DecodedAudio {
                    sample_rate,
                    channel_count,
                    samples: Vec::new(),
                });
                // A file that changes its layout midway is not supported;
                // the differing part is left out rather than played wrong.
                if channel_count == audio.channel_count {
                    audio.samples.extend_from_slice(&packet_samples);
                }
            }
            // A damaged packet: skip it and keep going.
            Err(SymphoniaError::DecodeError(_)) => {}
            // Anything else means decoding cannot continue. Keep what was
            // decoded so far rather than nothing.
            Err(_) => break,
        }
    }

    audio
        .filter(|audio| audio.channel_count > 0 && !audio.samples.is_empty())
        .ok_or(AudioFileError::NoAudioTrack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write_wav_file;

    fn audio(channel_count: usize, samples: &[f32]) -> DecodedAudio {
        DecodedAudio {
            sample_rate: 100,
            channel_count,
            samples: samples.to_vec(),
        }
    }

    #[test]
    fn frames_and_duration_account_for_channels() {
        let stereo = audio(2, &[0.0; 400]);

        assert_eq!(stereo.frame_count(), 200);
        assert_eq!(stereo.duration_seconds(), 2.0);
    }

    #[test]
    fn positions_between_frames_are_interpolated() {
        let mono = audio(1, &[0.0, 1.0, 0.0]);

        assert_eq!(mono.stereo_at(0.0), [0.0, 0.0]);
        assert_eq!(mono.stereo_at(0.5), [0.5, 0.5]);
        assert_eq!(mono.stereo_at(1.0), [1.0, 1.0]);
        assert_eq!(mono.stereo_at(1.25), [0.75, 0.75]);
    }

    #[test]
    fn stereo_channels_stay_separate_and_outside_is_silent() {
        let stereo = audio(2, &[0.2, -0.4, 0.6, -0.8]);

        assert_eq!(stereo.stereo_at(0.0), [0.2, -0.4]);
        assert_eq!(stereo.stereo_at(1.0), [0.6, -0.8]);
        assert_eq!(stereo.stereo_at(5.0), [0.0, 0.0]);
        assert_eq!(stereo.stereo_at(-1.0), [0.0, 0.0]);
    }

    #[test]
    fn wav_file_decodes_to_the_samples_that_were_written() {
        let path =
            std::env::temp_dir().join(format!("vizx-audio-decode-{}.wav", std::process::id()));
        // Two stereo frames: (max, min) then (0, half).
        write_wav_file(&path, 8_000, 2, &[i16::MAX, i16::MIN, 0, 16_384]).unwrap();

        let decoded = decode_audio_file(&path);
        let _ = std::fs::remove_file(&path);
        let decoded = decoded.unwrap();

        assert_eq!(decoded.sample_rate, 8_000);
        assert_eq!(decoded.channel_count, 2);
        assert_eq!(decoded.frame_count(), 2);
        let expected = [1.0, -1.0, 0.0, 0.5];
        for (sample, expected) in decoded.samples.iter().zip(expected) {
            assert!((sample - expected).abs() < 0.001, "{sample} vs {expected}");
        }
    }
}
