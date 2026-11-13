use std::fmt;
use std::fs::File;
use std::io;
use std::path::Path;

use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, Track, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Basic facts about an audio file, read from its header without decoding
/// the whole file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFileInfo {
    pub duration_milliseconds: u64,
    /// Samples per second, e.g. 44100.
    pub sample_rate: u32,
    /// 1 for mono, 2 for stereo.
    pub channel_count: u16,
}

/// Why an audio file could not be read.
#[derive(Debug)]
pub enum AudioFileError {
    /// The file could not be opened.
    Io(io::Error),
    /// The file is not in a format VizX can read, or it is damaged.
    UnsupportedFormat(String),
    /// The file contains no audio.
    NoAudioTrack,
    /// The file is readable, but a required property is not stated in it.
    MissingInformation(&'static str),
}

impl fmt::Display for AudioFileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "file access failed: {error}"),
            Self::UnsupportedFormat(details) => {
                write!(formatter, "unsupported or damaged audio file ({details})")
            }
            Self::NoAudioTrack => write!(formatter, "the file contains no audio"),
            Self::MissingInformation(what) => {
                write!(formatter, "the file does not state its {what}")
            }
        }
    }
}

impl std::error::Error for AudioFileError {}

impl From<io::Error> for AudioFileError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Opens an audio file and detects its container format.
///
/// The returned reader gives access to the file's tracks and, packet by
/// packet, to its compressed audio. Shared by probing and decoding.
pub(crate) fn open_format(path: &Path) -> Result<Box<dyn FormatReader>, AudioFileError> {
    let file = File::open(path)?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());

    // The extension is only a hint that speeds up format detection; the
    // actual format is determined from the file's content.
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
        hint.with_extension(extension);
    }

    symphonia::default::get_probe()
        .probe(
            &hint,
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|error| AudioFileError::UnsupportedFormat(error.to_string()))
}

/// Opens an audio file and reads its duration, sample rate and channels.
pub fn probe_audio_file(path: &Path) -> Result<AudioFileInfo, AudioFileError> {
    let format = open_format(path)?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or(AudioFileError::NoAudioTrack)?;
    let audio_parameters = track
        .codec_params
        .as_ref()
        .and_then(|parameters| parameters.audio())
        .ok_or(AudioFileError::NoAudioTrack)?;

    let sample_rate = audio_parameters
        .sample_rate
        .ok_or(AudioFileError::MissingInformation("sample rate"))?;
    let channel_count = audio_parameters
        .channels
        .as_ref()
        .map(|channels| channels.count() as u16)
        .ok_or(AudioFileError::MissingInformation("channel layout"))?;
    let duration_milliseconds = track_duration_milliseconds(track, sample_rate)
        .ok_or(AudioFileError::MissingInformation("duration"))?;

    Ok(AudioFileInfo {
        duration_milliseconds,
        sample_rate,
        channel_count,
    })
}

/// Works out the track length from whichever description the file provides.
fn track_duration_milliseconds(track: &Track, sample_rate: u32) -> Option<u64> {
    // Preferred: the exact number of audio frames (one frame is one sample
    // for every channel).
    if let Some(frame_count) = track.num_frames
        && sample_rate > 0
    {
        return Some(frames_to_milliseconds(frame_count, sample_rate));
    }

    // Otherwise: a duration in the container's own time units.
    let time = track.time_base?.calc_duration(track.duration?)?;
    Some((time.as_secs_f64() * 1000.0).round().max(0.0) as u64)
}

fn frames_to_milliseconds(frame_count: u64, sample_rate: u32) -> u64 {
    // u128 so that `frame_count * 1000` cannot overflow for very long files.
    let milliseconds =
        (u128::from(frame_count) * 1000 + u128::from(sample_rate) / 2) / u128::from(sample_rate);
    milliseconds as u64
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::write_wav_file;

    /// Writes a silent WAV file with the given number of frames.
    fn write_test_wav(name: &str, sample_rate: u32, channel_count: u16, frames: u32) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("vizx-audio-{name}-{}.wav", std::process::id()));
        let samples = vec![0i16; frames as usize * usize::from(channel_count)];
        write_wav_file(&path, sample_rate, channel_count, &samples).unwrap();
        path
    }

    #[test]
    fn wav_file_reports_duration_sample_rate_and_channels() {
        let path = write_test_wav("stereo", 44_100, 2, 44_100 * 3 / 2); // 1.5 s

        let info = probe_audio_file(&path);
        let _ = std::fs::remove_file(&path);

        assert_eq!(
            info.unwrap(),
            AudioFileInfo {
                duration_milliseconds: 1500,
                sample_rate: 44_100,
                channel_count: 2,
            }
        );
    }

    #[test]
    fn mono_file_reports_one_channel() {
        let path = write_test_wav("mono", 48_000, 1, 48_000);

        let info = probe_audio_file(&path);
        let _ = std::fs::remove_file(&path);

        let info = info.unwrap();
        assert_eq!(info.channel_count, 1);
        assert_eq!(info.duration_milliseconds, 1000);
    }

    #[test]
    fn file_that_is_not_audio_is_rejected() {
        let path = std::env::temp_dir().join(format!("vizx-audio-text-{}.wav", std::process::id()));
        std::fs::write(&path, "this is plain text, not audio").unwrap();

        let result = probe_audio_file(&path);
        let _ = std::fs::remove_file(&path);

        assert!(matches!(result, Err(AudioFileError::UnsupportedFormat(_))));
    }

    #[test]
    fn missing_file_is_an_io_error() {
        let path = std::env::temp_dir().join("vizx-audio-file-that-does-not-exist.wav");

        assert!(matches!(
            probe_audio_file(&path),
            Err(AudioFileError::Io(_))
        ));
    }

    #[test]
    fn frame_counts_convert_to_rounded_milliseconds() {
        assert_eq!(frames_to_milliseconds(44_100, 44_100), 1000);
        assert_eq!(frames_to_milliseconds(22_050, 44_100), 500);
        assert_eq!(frames_to_milliseconds(1, 44_100), 0);
        // Large enough that `frame_count * 1000` would overflow a u64.
        assert_eq!(
            frames_to_milliseconds(48_000 * 100_000_000_000_000, 48_000),
            100_000_000_000_000_000
        );
    }
}
