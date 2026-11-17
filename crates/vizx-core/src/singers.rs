//! Telling the singers of a song apart in the background: the vocals are
//! taken from the song's stems (separated first if there are none), every
//! subtitle line's stretch of them gets a voice print, and lines with alike
//! prints are given to the same singer (see `vizx_ml::speakers`).
//!
//! The result says which singer each line belongs to; the lines are then
//! moved to one subtitle layer per singer
//! (`Project::split_subtitle_layer`).

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

use vizx_audio::DecodedAudio;
use vizx_project::ClipId;

use crate::SongPlacement;
use crate::stems::{StemEngine, song_name, stem_folder};

/// Which singer (from 0) sings each line.
pub type SingerAssignment = Vec<(ClipId, usize)>;

/// A separation of singers running in the background.
pub struct SingerSeparation {
    song_name: String,
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<Result<SingerAssignment, String>>,
}

impl SingerSeparation {
    /// Starts telling apart who sings `lines` (a clip and its start and end
    /// in project time) of the song `audio`, the decoded file at
    /// `song_path`, placed on the timeline as `placement`. `singers` fixes
    /// how many there are; `None` decides from the voices. Fails at once if
    /// the singer model or ONNX Runtime is missing.
    pub fn start(
        audio: Arc<DecodedAudio>,
        song_path: &Path,
        placement: SongPlacement,
        lines: Vec<(ClipId, f64, f64)>,
        singers: Option<usize>,
        on_progress: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let runtime = vizx_ml::find_onnx_runtime()?;
        let model_path = vizx_ml::find_speaker_model()?;
        let engine = StemEngine::best_available();
        let folder = stem_folder(song_path, &engine);
        let name = song_name(song_path);
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, result) = mpsc::channel();

        let shared = (Arc::clone(&progress), Arc::clone(&cancel), name.clone());
        thread::spawn(move || {
            let (progress, cancel, name) = shared;
            let report = |base: f32, share: f32| {
                let progress = Arc::clone(&progress);
                let on_progress = &on_progress;
                move |part: f32| {
                    let total = base + share * part.clamp(0.0, 1.0);
                    progress.store((total * 1000.0) as u32, Ordering::Relaxed);
                    on_progress();
                }
            };
            let outcome = (|| {
                let vocals = crate::lyrics_recognition::vocals(
                    &audio,
                    &folder,
                    &name,
                    &engine,
                    &cancel,
                    &report(0.0, 0.6),
                )?;
                let mut model = vizx_ml::SpeakerModel::load(&runtime, &model_path)?;
                // From the timeline's time to the audio's.
                let shift = placement.source_offset_seconds - placement.clip_start_seconds;
                let times: Vec<(f64, f64)> = lines
                    .iter()
                    .map(|(_, start, end)| (start + shift, end + shift))
                    .collect();
                let found = vizx_ml::identify_singers(
                    &mut model,
                    &vocals,
                    &times,
                    singers,
                    &cancel,
                    &report(0.6, 0.4),
                )?;
                Ok(lines
                    .iter()
                    .zip(found)
                    .map(|((clip, _, _), singer)| (*clip, singer))
                    .collect())
            })();
            let _ = sender.send(outcome);
            on_progress();
        });

        Ok(Self {
            song_name: name,
            progress,
            cancel,
            result,
        })
    }

    pub fn song_name(&self) -> &str {
        &self.song_name
    }

    /// How far it is, from 0 to 1.
    pub fn progress(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.0
    }

    /// Which singer sings each line once it has ended, or why it failed.
    /// `None` while it is still running.
    pub fn finished(&self) -> Option<Result<SingerAssignment, String>> {
        match self.result.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                "telling the singers apart stopped unexpectedly".to_owned(),
            )),
        }
    }
}

impl Drop for SingerSeparation {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
