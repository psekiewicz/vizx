//! Reading audio files for VizX.
//!
//! This crate knows about audio file formats and nothing else: not the
//! project, not the GUI. It currently answers one question, "what is in
//! this audio file?", which is what importing needs. Decoding samples for
//! waveforms, analysis and playback will be added here later.
//!
//! Decoding is done by the `symphonia` library, which is written in Rust and
//! needs no system libraries, so it behaves the same on Windows and Linux.

mod probe;

pub use probe::{AudioFileError, AudioFileInfo, probe_audio_file};

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
