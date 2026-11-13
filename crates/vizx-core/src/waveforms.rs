use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use vizx_audio::WaveformPeaks;

/// Progress of the waveform of one audio file.
#[derive(Debug, Clone)]
pub enum WaveformState {
    /// The file is being decoded in the background.
    Loading,
    /// `Arc` is a shared pointer: the peaks can be handed to the UI without
    /// copying them.
    Ready(Arc<WaveformPeaks>),
    /// The file could not be decoded; the text says why.
    Failed(String),
}

/// Computes and remembers the waveforms of audio files.
///
/// Decoding a whole song takes seconds, far too long to do while drawing a
/// frame. Each file is therefore decoded once on its own background thread,
/// and the result is sent back through a channel. The UI never waits: it
/// asks for the current state and draws whatever is available.
///
/// Waveforms are derived from the audio files, so they are not project data
/// and are not saved; they are recomputed when a project is opened.
pub struct WaveformStore {
    /// Keyed by file path, so the same file is only ever decoded once, even
    /// across different projects opened in one session.
    states: HashMap<String, WaveformState>,
    /// Given (as a copy) to every background thread to send its result.
    result_sender: Sender<(String, WaveformState)>,
    result_receiver: Receiver<(String, WaveformState)>,
}

impl Default for WaveformStore {
    fn default() -> Self {
        let (result_sender, result_receiver) = mpsc::channel();
        Self {
            states: HashMap::new(),
            result_sender,
            result_receiver,
        }
    }
}

impl WaveformStore {
    /// The state of a file's waveform, or `None` if it was never requested.
    pub fn state(&self, path: &str) -> Option<&WaveformState> {
        self.states.get(path)
    }

    /// Starts computing the waveform of a file unless that already happened.
    ///
    /// `on_finished` runs on the background thread once the result is ready
    /// to be collected. The UI uses it to ask for a repaint.
    pub fn request(&mut self, path: &str, on_finished: impl FnOnce() + Send + 'static) {
        if self.states.contains_key(path) {
            return;
        }
        self.states.insert(path.to_owned(), WaveformState::Loading);

        let path = path.to_owned();
        let result_sender = self.result_sender.clone();
        // `move` gives the new thread ownership of the values it uses.
        thread::spawn(move || {
            let state = match vizx_audio::compute_waveform_peaks(&PathBuf::from(&path)) {
                Ok(peaks) => WaveformState::Ready(Arc::new(peaks)),
                Err(error) => WaveformState::Failed(error.to_string()),
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
    fn wait_until_finished(store: &mut WaveformStore, path: &str) -> WaveformState {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            store.collect_finished();
            match store.state(path) {
                Some(WaveformState::Loading) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(5));
                }
                Some(state) => return state.clone(),
                None => panic!("waveform was never requested"),
            }
        }
    }

    #[test]
    fn requested_file_is_loading_and_then_ready() {
        let path =
            std::env::temp_dir().join(format!("vizx-core-waveform-{}.wav", std::process::id()));
        vizx_audio::write_wav_file(&path, 8_000, 1, &vec![1000i16; 4_000]).unwrap();
        let path_text = path.to_string_lossy().into_owned();
        let mut store = WaveformStore::default();

        store.request(&path_text, || {});
        assert!(matches!(
            store.state(&path_text),
            Some(WaveformState::Loading)
        ));

        let state = wait_until_finished(&mut store, &path_text);
        let _ = std::fs::remove_file(&path);

        let WaveformState::Ready(peaks) = state else {
            panic!("expected a ready waveform, got {state:?}");
        };
        assert!((peaks.duration_seconds() - 0.5).abs() < 0.01);
    }

    #[test]
    fn unreadable_file_ends_as_failed() {
        let mut store = WaveformStore::default();
        let path = "vizx-core-waveform-file-that-does-not-exist.wav";

        store.request(path, || {});
        let state = wait_until_finished(&mut store, path);

        assert!(matches!(state, WaveformState::Failed(_)));
    }

    #[test]
    fn finish_notification_is_called() {
        let (notified_sender, notified_receiver) = mpsc::channel();
        let mut store = WaveformStore::default();

        store.request("vizx-core-waveform-missing.wav", move || {
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
        assert!(
            WaveformStore::default()
                .state("never-requested.wav")
                .is_none()
        );
    }
}
