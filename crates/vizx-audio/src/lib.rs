//! Reading audio files for VizX.
//!
//! This crate knows about audio files and the sound card and nothing else:
//! not the project, not the GUI.
//!
//! It reads the basic facts about a file (for importing), decodes files
//! into memory, summarizes samples as waveform peaks (for drawing) and
//! plays audio through the sound card, and analyses audio into a spectrum
//! and levels for visualizers.
//!
//! Decoding is done by the `symphonia` library, which is written in Rust and
//! needs no system libraries, so it behaves the same on Windows and Linux.

mod analysis;
mod beats;
mod decode;
mod playback;
mod probe;
mod separate;
mod wav;
mod waveform;

pub use analysis::{ANALYSIS_WINDOW, AudioAnalyzer, AudioFeatures, SPECTRUM_BINS, WAVEFORM_POINTS};
pub use beats::{Beat, BeatAnalysis, detect_beats};
pub use decode::{DecodedAudio, decode_audio_file};
pub use playback::{AudioPlayer, PlaybackError, PlaybackSource, mix_to_stereo};
pub use probe::{AudioFileError, AudioFileInfo, probe_audio_file};
pub use separate::{SeparationError, Stems, separate_stems};
pub use wav::write_wav_file;
pub use waveform::{WaveformPeaks, compute_waveform_peaks};

/// File extensions of the audio formats VizX can read, in lower case.
pub const SUPPORTED_EXTENSIONS: [&str; 7] = ["wav", "mp3", "flac", "ogg", "m4a", "aac", "mp4"];

/// Whether a path has one of the [`SUPPORTED_EXTENSIONS`].
pub fn has_supported_extension(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            SUPPORTED_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn extension_check_ignores_case_and_rejects_other_files() {
        assert!(has_supported_extension(Path::new("song.mp3")));
        assert!(has_supported_extension(Path::new("C:/Music/Song.WAV")));
        assert!(!has_supported_extension(Path::new("cover.png")));
        assert!(!has_supported_extension(Path::new("no_extension")));
    }
}
