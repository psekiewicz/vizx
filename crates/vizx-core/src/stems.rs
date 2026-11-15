//! Separating a song into stems in the background, for visuals that follow
//! only the vocals, the drums or the bass.
//!
//! The stems are written as WAV files into a folder next to the song
//! (`Song stems/Song - Vocals.wav`, ...), so they are made once: when the
//! files are already there, they are used as they are.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

use vizx_audio::DecodedAudio;
use vizx_project::ClipId;

/// The names of the stems, in the order they are added.
pub const STEM_NAMES: [&str; 4] = ["Vocals", "Drums", "Bass", "Other"];

/// The stem files: each stem's name and its WAV file.
pub type StemFiles = Vec<(String, PathBuf)>;

/// A separation running in the background.
pub struct StemJob {
    clip_id: ClipId,
    song_name: String,
    /// Progress in thousandths.
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<Result<StemFiles, String>>,
}

impl StemJob {
    /// Starts separating `audio`, the decoded file at `song_path` that
    /// `clip_id` plays. `on_progress` is called from the background thread
    /// whenever there is news (to repaint the window).
    pub fn start(
        clip_id: ClipId,
        audio: Arc<DecodedAudio>,
        song_path: &Path,
        on_progress: impl Fn() + Send + 'static,
    ) -> Self {
        let folder = stem_folder(song_path);
        let song_name = song_name(song_path);
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, result) = mpsc::channel();

        let (thread_progress, thread_cancel, thread_name) = (
            Arc::clone(&progress),
            Arc::clone(&cancel),
            song_name.clone(),
        );
        thread::spawn(move || {
            let outcome = existing_stems(&folder, &thread_name).map_or_else(
                || {
                    let report = |share: f32| {
                        thread_progress.store((share * 1000.0) as u32, Ordering::Relaxed);
                        on_progress();
                    };
                    let stems = vizx_audio::separate_stems(&audio, &thread_cancel, &report)
                        .map_err(|error| error.to_string())?;
                    let written = stems
                        .write_wav_files(&folder, &thread_name)
                        .map_err(|error| format!("could not write the stems: {error}"))?;
                    Ok(written
                        .into_iter()
                        .map(|(name, path)| (name.to_owned(), path))
                        .collect())
                },
                Ok,
            );
            // The window may have been closed in the meantime.
            let _ = sender.send(outcome);
        });

        Self {
            clip_id,
            song_name,
            progress,
            cancel,
            result,
        }
    }

    /// The audio clip being separated.
    pub fn clip_id(&self) -> ClipId {
        self.clip_id
    }

    /// The song's name, for messages.
    pub fn song_name(&self) -> &str {
        &self.song_name
    }

    /// How far it is, from 0 to 1.
    pub fn progress(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.0
    }

    /// The stem files once the separation has ended, or why it failed.
    /// `None` while it is still running.
    pub fn finished(&self) -> Option<Result<StemFiles, String>> {
        match self.result.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("the separation stopped unexpectedly".to_owned()))
            }
        }
    }
}

impl Drop for StemJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// The folder a song's stems are written to: next to the song, named after
/// it.
pub fn stem_folder(song_path: &Path) -> PathBuf {
    let folder = song_path.parent().unwrap_or(Path::new("."));
    folder.join(format!("{} stems", song_name(song_path)))
}

fn song_name(song_path: &Path) -> String {
    song_path.file_stem().map_or_else(
        || "song".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// The stem files in `folder` if all of them are there already.
fn existing_stems(folder: &Path, song_name: &str) -> Option<StemFiles> {
    STEM_NAMES
        .iter()
        .map(|name| {
            let path = folder.join(format!("{song_name} - {name}.wav"));
            path.is_file().then(|| ((*name).to_owned(), path))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_are_written_next_to_the_song_and_reused() {
        let folder = std::env::temp_dir().join(format!("vizx-stems-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let song = folder.join("My Song.mp3");
        let audio = Arc::new(DecodedAudio {
            sample_rate: 8000,
            channel_count: 1,
            samples: (0..8000)
                .map(|index| (index as f32 * 0.05).sin() * 0.5)
                .collect(),
        });

        let mut project = vizx_project::Project::new();
        let properties = vizx_project::AudioProperties {
            duration_milliseconds: 1000,
            sample_rate: 8000,
            channel_count: 1,
        };
        let (_, track_id) = project.import_audio("song.wav", properties);
        let clip_id = project.track(track_id).unwrap().clips[0].id;
        let run = || {
            let job = StemJob::start(clip_id, Arc::clone(&audio), &song, || {});
            loop {
                if let Some(outcome) = job.finished() {
                    return outcome.unwrap();
                }
                thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        let first = run();
        let modified = std::fs::metadata(&first[0].1).unwrap().modified().unwrap();
        let second = run();
        let modified_again = std::fs::metadata(&second[0].1).unwrap().modified().unwrap();
        let _ = std::fs::remove_dir_all(&folder);

        let names: Vec<&str> = first.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, STEM_NAMES);
        assert!(first[0].1.ends_with("My Song stems/My Song - Vocals.wav"));
        // The second time, the files were only found, not written again.
        assert_eq!(first, second);
        assert_eq!(modified, modified_again);
    }
}
