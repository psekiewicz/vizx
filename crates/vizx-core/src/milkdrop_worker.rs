use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use vizx_milkdrop::MilkdropRenderer;

/// At most this much audio (in stereo samples) is handed to projectM for
/// one frame. If rendering falls behind and requests pile up, older audio
/// is dropped: projectM only looks at the most recent fraction of a second.
const MAX_SAMPLES_PER_FRAME: usize = 2 * 8192;

/// One finished picture from the MilkDrop renderer.
pub struct MilkdropFrame {
    pub width: u32,
    pub height: u32,
    /// RGBA bytes, top row first, fully opaque.
    pub pixels: Vec<u8>,
}

/// What to draw, sent from the UI to the render thread.
pub struct MilkdropFrameRequest {
    /// Path of the `.milk` preset to show.
    pub preset_path: String,
    pub width: u32,
    pub height: u32,
    pub frames_per_second: u32,
    /// The project audio leading up to the frame, as stereo samples.
    pub audio: Vec<f32>,
}

/// State the render thread shares with the UI.
#[derive(Default)]
struct Shared {
    /// The newest finished frame, until the UI takes it.
    frame: Option<MilkdropFrame>,
    /// Why rendering is not working, if it is not.
    error: Option<String>,
}

/// Renders MilkDrop frames on a background thread.
///
/// projectM draws with OpenGL, and an OpenGL context belongs to one thread.
/// So the renderer lives entirely on its own thread: the UI sends requests
/// through a channel and picks up finished frames from a shared slot. The
/// UI never waits for a frame; it shows the newest one available.
///
/// Dropping the worker closes the channel, which ends the thread.
pub struct MilkdropWorker {
    request_sender: Sender<MilkdropFrameRequest>,
    shared: Arc<Mutex<Shared>>,
}

impl MilkdropWorker {
    /// Starts the render thread. `on_frame` runs on that thread every time
    /// a frame (or an error) is ready; the UI uses it to ask for a repaint.
    pub fn spawn(on_frame: impl Fn() + Send + 'static) -> Self {
        let (request_sender, request_receiver) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Shared::default()));

        let thread_shared = Arc::clone(&shared);
        thread::spawn(move || render_loop(&request_receiver, &thread_shared, &on_frame));

        Self {
            request_sender,
            shared,
        }
    }

    /// Asks for a frame. Requests are never queued up behind each other: if
    /// the renderer is still busy, pending requests are merged into one.
    pub fn request_frame(&self, request: MilkdropFrameRequest) {
        // Sending fails only if the thread has ended, which is reported
        // through `error` instead.
        let _ = self.request_sender.send(request);
    }

    /// Takes the newest finished frame, if there is one that was not taken
    /// yet.
    pub fn take_frame(&self) -> Option<MilkdropFrame> {
        self.shared.lock().ok()?.frame.take()
    }

    /// Why no frames are being produced, if something is wrong.
    pub fn error(&self) -> Option<String> {
        self.shared.lock().ok()?.error.clone()
    }
}

/// What the render thread currently has set up.
struct ActiveRenderer {
    renderer: MilkdropRenderer,
    frames_per_second: u32,
    /// Preset currently loaded, so it is only loaded again when it changes.
    preset_path: String,
}

fn render_loop(
    requests: &Receiver<MilkdropFrameRequest>,
    shared: &Mutex<Shared>,
    on_frame: &dyn Fn(),
) {
    let mut active: Option<ActiveRenderer> = None;
    let mut pixels = Vec::new();

    // `recv` waits for the next request and fails once the worker has been
    // dropped, which ends the loop and the thread. The renderer is dropped
    // here, on the thread that owns its OpenGL context.
    while let Ok(mut request) = requests.recv() {
        // Merge everything that queued up while the last frame was drawn:
        // keep the newest settings, and all of the audio in order.
        while let Ok(newer) = requests.try_recv() {
            let mut audio = std::mem::take(&mut request.audio);
            audio.extend_from_slice(&newer.audio);
            request = newer;
            request.audio = audio;
        }
        if request.audio.len() > MAX_SAMPLES_PER_FRAME {
            let excess = request.audio.len() - MAX_SAMPLES_PER_FRAME;
            request.audio.drain(..excess);
        }

        let result = render_one(&mut active, &request, &mut pixels);

        if let Ok(mut shared) = shared.lock() {
            match result {
                Ok(()) => {
                    shared.error = None;
                    shared.frame = Some(MilkdropFrame {
                        width: request.width,
                        height: request.height,
                        pixels: pixels.clone(),
                    });
                }
                Err(message) => shared.error = Some(message),
            }
        }
        on_frame();
    }
}

fn render_one(
    active: &mut Option<ActiveRenderer>,
    request: &MilkdropFrameRequest,
    pixels: &mut Vec<u8>,
) -> Result<(), String> {
    // (Re)create the renderer when the picture size or frame rate changed.
    let needs_new_renderer = active.as_ref().is_none_or(|active| {
        active.renderer.width() != request.width
            || active.renderer.height() != request.height
            || active.frames_per_second != request.frames_per_second
    });
    if needs_new_renderer {
        // Drop the old one first: only one OpenGL context per thread.
        *active = None;
        let renderer =
            MilkdropRenderer::new(request.width, request.height, request.frames_per_second)
                .map_err(|error| error.to_string())?;
        *active = Some(ActiveRenderer {
            renderer,
            frames_per_second: request.frames_per_second,
            preset_path: String::new(),
        });
    }
    let active = active.as_mut().expect("renderer was just created");

    if active.preset_path != request.preset_path {
        active
            .renderer
            .load_preset(Path::new(&request.preset_path))
            .map_err(|error| error.to_string())?;
        active.preset_path.clone_from(&request.preset_path);
    }

    active.renderer.add_stereo_audio(&request.audio);
    active.renderer.render_frame(pixels);

    // What the graphics card leaves in the transparency channel is not
    // meaningful here; the picture is always fully opaque.
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel[3] = 255;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn missing_preset_is_reported_as_an_error_and_produces_no_frame() {
        let (notified_sender, notified_receiver) = mpsc::channel();
        let worker = MilkdropWorker::spawn(move || {
            let _ = notified_sender.send(());
        });

        worker.request_frame(MilkdropFrameRequest {
            preset_path: "vizx-core-preset-that-does-not-exist.milk".to_owned(),
            width: 64,
            height: 36,
            frames_per_second: 30,
            audio: vec![0.0; 64],
        });

        // Works the same whether or not projectM is installed: either way
        // this request cannot produce a picture.
        assert!(
            notified_receiver
                .recv_timeout(Duration::from_secs(30))
                .is_ok(),
            "the render thread never answered"
        );
        assert!(worker.error().is_some());
        assert!(worker.take_frame().is_none());
    }

    #[test]
    fn dropping_the_worker_ends_its_thread() {
        let finished = Arc::new(Mutex::new(false));
        let (request_sender, request_receiver) = mpsc::channel::<MilkdropFrameRequest>();
        let shared = Arc::new(Mutex::new(Shared::default()));

        let thread_finished = Arc::clone(&finished);
        let thread_shared = Arc::clone(&shared);
        let handle = thread::spawn(move || {
            render_loop(&request_receiver, &thread_shared, &|| {});
            *thread_finished.lock().unwrap() = true;
        });
        drop(request_sender);

        let deadline = Instant::now() + Duration::from_secs(10);
        while !handle.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(*finished.lock().unwrap());
    }
}
