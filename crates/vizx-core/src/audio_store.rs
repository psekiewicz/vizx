use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use vizx_audio::{BeatAnalysis, DecodedAudio, WaveformPeaks};

/// An audio file that has been decoded and is ready to be drawn and played.
#[derive(Debug)]
pub struct LoadedAudio {
    /// Summary of the samples for drawing the waveform.
    pub peaks: WaveformPeaks,
    /// Where the beats of the file are, and its tempo.
    pub beats: BeatAnalysis,
    /// The samples themselves, for playback. `Arc` is a shared pointer: the
    /// audio player can use the same samples without copying them.
    pub audio: Arc<DecodedAudio>,
}

/// Progress of loading one audio file.
#[derive(Debug, Clone)]
pub enum AudioState {
    /// The file is being decoded in the background.
    Loading,
    Ready(Arc<LoadedAudio>),
    /// The file could not be decoded; the text says why.
    Failed(String),
}

/// Decodes and remembers the audio files a project uses.
///
/// Decoding a whole song takes seconds, far too long to do while drawing a
/// frame. Each file is therefore decoded once on its own background thread,
/// and the result is sent back through a channel. The UI never waits: it
/// asks for the current state and uses whatever is available.
///
/// Decoded audio is derived from the files, so it is not project data and
/// is not saved; it is loaded again when a project is opened.
pub struct AudioStore {
    /// Keyed by file path, so the same file is only ever decoded once, even
    /// across different projects opened in one session.
    states: HashMap<String, AudioState>,
    /// Given (as a copy) to every background thread to send its result.
    result_sender: Sender<(String, AudioState)>,
    result_receiver: Receiver<(String, AudioState)>,
}

impl Default for AudioStore {
    fn default() -> Self {
        let (result_sender, result_receiver) = mpsc::channel();
        Self {
            states: HashMap::new(),
            result_sender,
            result_receiver,
        }
    }
}

impl AudioStore {
    /// The state of a file, or `None` if it was never requested.
    pub fn state(&self, path: &str) -> Option<&AudioState> {
        self.states.get(path)
    }

    /// The decoded file, if it has finished loading.
    pub fn loaded(&self, path: &str) -> Option<&Arc<LoadedAudio>> {
        match self.states.get(path) {
            Some(AudioState::Ready(loaded)) => Some(loaded),
            _ => None,
        }
    }

    /// Starts loading a file unless that already happened.
    ///
    /// `on_finished` runs on the background thread once the result is ready
    /// to be collected. The UI uses it to ask for a repaint.
    pub fn request(&mut self, path: &str, on_finished: impl FnOnce() + Send + 'static) {
        if self.states.contains_key(path) {
            return;
        }
        self.states.insert(path.to_owned(), AudioState::Loading);

        let path = path.to_owned();
        let result_sender = self.result_sender.clone();
        // `move` gives the new thread ownership of the values it uses.
        thread::spawn(move || {
            let state = match vizx_audio::decode_audio_file(&PathBuf::from(&path)) {
                Ok(audio) => AudioState::Ready(Arc::new(LoadedAudio {
                    peaks: WaveformPeaks::from_audio(&audio),
                    beats: vizx_audio::detect_beats(&audio),
                    audio: Arc::new(audio),
                })),
                Err(error) => AudioState::Failed(error.to_string()),
            };
            // Sending fails only if the store no longer exists (the app is
            // closing), in which case nobody needs the result.
            if result_sender.send((path, state)).is_ok() {
                on_finished();
            }
        });
    }

    /// Takes over the results that background threads have finished since
    /// the last call. Never waits. Call it once per frame.
    pub fn collect_finished(&mut self) {
        while let Ok((path, state)) = self.result_receiver.try_recv() {
            self.states.insert(path, state);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    /// Waits until the background thread has delivered a result.
    fn wait_until_finished(store: &mut AudioStore, path: &str) -> AudioState {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            store.collect_finished();
            match store.state(path) {
                Some(AudioState::Loading) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(5));
                }
                Some(state) => return state.clone(),
                None => panic!("audio was never requested"),
            }
        }
    }

    #[test]
    fn requested_file_is_loading_and_then_ready_with_samples_and_peaks() {
        let path = std::env::temp_dir().join(format!("vizx-core-audio-{}.wav", std::process::id()));
        vizx_audio::write_wav_file(&path, 8_000, 1, &vec![1000i16; 4_000]).unwrap();
        let path_text = path.to_string_lossy().into_owned();
        let mut store = AudioStore::default();

        store.request(&path_text, || {});
        assert!(matches!(store.state(&path_text), Some(AudioState::Loading)));
        assert!(store.loaded(&path_text).is_none());

        let state = wait_until_finished(&mut store, &path_text);
        let _ = std::fs::remove_file(&path);

        let AudioState::Ready(loaded) = state else {
            panic!("expected loaded audio, got {state:?}");
        };
        assert!((loaded.peaks.duration_seconds() - 0.5).abs() < 0.01);
        assert_eq!(loaded.audio.frame_count(), 4_000);
        assert!(store.loaded(&path_text).is_some());
    }

    #[test]
    fn unreadable_file_ends_as_failed() {
        let mut store = AudioStore::default();
        let path = "vizx-core-audio-file-that-does-not-exist.wav";

        store.request(path, || {});
        let state = wait_until_finished(&mut store, path);

        assert!(matches!(state, AudioState::Failed(_)));
    }

    #[test]
    fn finish_notification_is_called() {
        let (notified_sender, notified_receiver) = mpsc::channel();
        let mut store = AudioStore::default();

        store.request("vizx-core-audio-missing.wav", move || {
            let _ = notified_sender.send(());
        });

        assert!(
            notified_receiver
                .recv_timeout(Duration::from_secs(20))
                .is_ok()
        );
    }

    #[test]
    fn unknown_file_has_no_state() {
        assert!(AudioStore::default().state("never-requested.wav").is_none());
    }
}
