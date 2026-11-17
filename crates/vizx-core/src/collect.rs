//! "Collect All and Save" in the background: the project and copies of all
//! its files go into one new folder (see `vizx_project::CollectionPlan`).
//! Copying large videos takes a while, so it runs on its own thread, with
//! progress, and can be stopped; a stopped or failed collection removes
//! the folder it started.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

use vizx_project::{PROJECT_FILE_EXTENSION, Project, new_collection_folder};

/// A finished collection.
#[derive(Debug, Clone)]
pub struct Collected {
    /// The new project file.
    pub project_path: PathBuf,
    /// The project as saved, pointing at the copies.
    pub project: Project,
    /// Where each file the project used now is (old path, new path).
    pub moved_paths: Vec<(String, String)>,
    pub file_count: usize,
    pub bytes: u64,
    /// Files that could not be found and were not copied.
    pub missing: Vec<String>,
}

/// A collection running in the background.
pub struct CollectJob {
    folder: PathBuf,
    /// Progress in thousandths.
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<Result<Collected, String>>,
}

impl CollectJob {
    /// Starts collecting `project` into a new folder named `name` inside
    /// `parent` (`name (2)` if that exists), saved there as `name.vzx`.
    /// `on_progress` is called from the background thread when there is
    /// news.
    pub fn start(
        project: Project,
        parent: &Path,
        name: &str,
        on_progress: impl Fn() + Send + 'static,
    ) -> Self {
        let folder = new_collection_folder(parent, name);
        let project_path = folder.join(format!("{name}.{PROJECT_FILE_EXTENSION}"));
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, result) = mpsc::channel();
        let (thread_folder, thread_progress, thread_cancel) =
            (folder.clone(), Arc::clone(&progress), Arc::clone(&cancel));
        thread::spawn(move || {
            let report = |share: f64| {
                thread_progress.store((share.clamp(0.0, 1.0) * 1000.0) as u32, Ordering::Relaxed);
                on_progress();
            };
            let outcome = collect(
                &project,
                &thread_folder,
                &project_path,
                &thread_cancel,
                &report,
            );
            if outcome.is_err() {
                // Only this job made the folder, so nothing else is lost.
                let _ = fs::remove_dir_all(&thread_folder);
            }
            // The window may have been closed in the meantime.
            let _ = sender.send(outcome);
            on_progress();
        });
        Self {
            folder,
            progress,
            cancel,
            result,
        }
    }

    /// The folder being filled.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// How far it is, from 0 to 1.
    pub fn progress(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.0
    }

    /// The outcome once it has ended. `None` while it is still running.
    pub fn finished(&self) -> Option<Result<Collected, String>> {
        match self.result.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("collecting stopped unexpectedly".to_owned()))
            }
        }
    }
}

impl Drop for CollectJob {
    /// Dropping the job stops it.
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn collect(
    project: &Project,
    folder: &Path,
    project_path: &Path,
    cancel: &AtomicBool,
    report: &dyn Fn(f64),
) -> Result<Collected, String> {
    let plan = project.plan_collection(folder);
    fs::create_dir_all(folder)
        .map_err(|error| format!("could not create {}: {error}", folder.display()))?;
    let total = plan.total_bytes().max(1);
    let mut done = 0;
    for copy in &plan.copies {
        if let Some(parent) = copy.to.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        copy_file(&copy.from, &copy.to, cancel, &mut |bytes| {
            done += bytes;
            report(done as f64 / total as f64);
        })
        .map_err(|error| format!("could not copy {}: {error}", copy.from.display()))?;
    }
    plan.project
        .save_to_file(project_path)
        .map_err(|error| format!("could not save {}: {error}", project_path.display()))?;
    Ok(Collected {
        project_path: project_path.to_owned(),
        file_count: plan.copies.len(),
        bytes: plan.total_bytes(),
        project: plan.project,
        moved_paths: plan.moved_paths,
        missing: plan.missing,
    })
}

/// Copies in pieces, so a large video shows progress and can be stopped.
fn copy_file(
    from: &Path,
    to: &Path,
    cancel: &AtomicBool,
    copied: &mut dyn FnMut(u64),
) -> std::io::Result<()> {
    const PIECE: usize = 4 << 20;
    let mut source = fs::File::open(from)?;
    let mut target = fs::File::create(to)?;
    let mut buffer = vec![0; PIECE];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("stopped"));
        }
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        target.write_all(&buffer[..read])?;
        copied(read as u64);
    }
    target.flush()?;
    // The copy keeps the original's date, as Explorer's copy does.
    if let Ok(modified) = fs::metadata(from).and_then(|metadata| metadata.modified()) {
        let _ = target.set_modified(modified);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_collected_project_opens_with_its_copied_files() {
        let root = std::env::temp_dir().join(format!("vizx-collect-job-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let sources = root.join("sources");
        fs::create_dir_all(&sources).unwrap();
        let song = sources.join("song.wav");
        fs::write(&song, vec![7_u8; 9 << 20]).unwrap();

        let mut project = Project::new();
        let audio = vizx_project::AudioProperties {
            duration_milliseconds: 1000,
            sample_rate: 44_100,
            channel_count: 2,
        };
        project.import_audio(song.to_string_lossy(), audio);

        let job = CollectJob::start(project, &root, "My Song", || {});
        let collected = loop {
            if let Some(outcome) = job.finished() {
                break outcome.unwrap();
            }
            thread::sleep(std::time::Duration::from_millis(10));
        };
        assert_eq!(job.progress(), 1.0);
        assert_eq!(
            collected.project_path,
            root.join("My Song").join("My Song.vzx")
        );
        assert_eq!((collected.file_count, collected.bytes), (1, 9 << 20));

        // The originals go away; the collected project does not need them.
        fs::remove_dir_all(&sources).unwrap();
        let opened = Project::load_from_file(&collected.project_path).unwrap();
        let copy = root.join("My Song").join("Media").join("song.wav");
        assert_eq!(opened.media()[0].path, copy.to_string_lossy());
        assert_eq!(fs::read(&copy).unwrap().len(), 9 << 20);
        assert!(opened.missing_media().is_empty());

        // The next collection does not mix with this one.
        let again = CollectJob::start(opened, &root, "My Song", || {});
        assert_eq!(again.folder(), root.join("My Song (2)"));
        drop(again);
        let _ = fs::remove_dir_all(&root);
    }
}
