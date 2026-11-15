//! Writing the project as a video file.
//!
//! Export walks through the project frame by frame. For every frame it asks
//! each visual clip for its picture, lays the pictures over each other with
//! the same [`Compositor`] the preview uses, and hands the finished frame
//! to FFmpeg, which compresses it. The sound is the same mix that playback
//! plays.
//!
//! Time comes from the frame number (`frame / frame rate`), never from the
//! clock, so the video is the same however fast or slow this computer is.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use vizx_audio::PlaybackSource;
use vizx_project::{AudioReaction, Placement, Project, TextProperties};
use vizx_render::{Compositor, LayerDraw};

use crate::composition::{ReactionMeter, audio_sources, frame_source, layer_geometry};
use crate::ffmpeg::VideoWriter;
use crate::image_store::{DecodedImage, decode_image_file};
use crate::prerender::{FrameRenderer, apply_opacity_from_brightness};
use crate::text::render_text;
use crate::{AudioStore, FrameSource, PrerenderJob, VisualizerCatalog};

/// Sample rate of the exported sound.
const EXPORT_SAMPLE_RATE: u32 = 48_000;
/// Smallest and largest edge, in pixels, at which a clip is rendered.
const RENDER_EDGE_RANGE: (f64, f64) = (16.0, 4096.0);
/// Largest edge, in pixels, of an image used in an export. Far beyond any
/// video size, it only guards against absurd files.
const IMAGE_EDGE_LIMIT: u32 = 8192;
/// How often (in frames) the UI is told that progress was made.
const PROGRESS_INTERVAL: u32 = 10;

/// Everything needed to export, copied out of the project. The export runs
/// on its own thread from this snapshot, so the project can be edited (or
/// closed) meanwhile without affecting the video.
pub struct ExportJob {
    /// The video file to write (MP4).
    pub output: PathBuf,
    /// The `ffmpeg` program.
    pub ffmpeg: PathBuf,
    pub width: u32,
    pub height: u32,
    /// Frames per second as a fraction (frames, seconds).
    pub frame_rate: (u32, u32),
    pub duration_seconds: f64,
    /// The sound of the video: everything on the audio tracks.
    pub audio: Vec<PlaybackSource>,
    /// The visual clips, the bottom layer first.
    pub layers: Vec<ExportLayer>,
}

/// One visual clip of an [`ExportJob`].
pub struct ExportLayer {
    pub content: ExportContent,
    /// When the clip starts and how long it lasts, in project time.
    pub start_seconds: f64,
    pub duration_seconds: f64,
    pub placement: Placement,
    pub opacity: f64,
    pub reaction: AudioReaction,
    /// The audio the clip listens to: its visualizer draws from it and its
    /// reaction follows it.
    pub input_audio: Vec<PlaybackSource>,
}

pub enum ExportContent {
    /// A still image file.
    Image(PathBuf),
    /// A text element, drawn once.
    Text(TextProperties),
    /// Something drawn frame by frame: a visualizer or a video.
    Frames(FrameSource),
}

impl ExportJob {
    /// Collects what an export of `project` needs. Fails with a message for
    /// the user if something is not ready or cannot be drawn.
    pub fn from_project(
        project: &Project,
        audio_store: &AudioStore,
        catalog: &mut VisualizerCatalog,
        bundled_presets: Option<&Path>,
        ffmpeg: &Path,
        output: &Path,
    ) -> Result<Self, String> {
        let (audio, clips_still_loading) =
            audio_sources(project, audio_store, vizx_project::AudioInput::Mix);
        if clips_still_loading > 0 {
            return Err("the audio is still being loaded; try again in a moment".to_owned());
        }

        let mut layers = Vec::new();
        // The project lists layers from the top down; drawing goes from the
        // bottom up.
        for track in project.tracks().iter().rev() {
            for clip in &track.clips {
                let Some(media) = project.media_source(clip.media) else {
                    continue;
                };
                if !media.is_visual() {
                    continue;
                }
                let content = if media.image().is_some() {
                    ExportContent::Image(PathBuf::from(&media.path))
                } else if let Some(text) = media.text() {
                    ExportContent::Text(text.clone())
                } else {
                    let source = frame_source(media, clip, bundled_presets, catalog, Some(ffmpeg))
                        .map_err(|problem| format!("{}: {problem}", media.file_name()))?;
                    ExportContent::Frames(source)
                };
                layers.push(ExportLayer {
                    content,
                    start_seconds: clip.start_milliseconds as f64 / 1000.0,
                    duration_seconds: clip.duration_milliseconds as f64 / 1000.0,
                    placement: clip.placement,
                    opacity: clip.opacity,
                    reaction: clip.reaction,
                    input_audio: audio_sources(project, audio_store, clip.audio_input).0,
                });
            }
        }

        let settings = &project.settings;
        Ok(Self {
            output: output.to_owned(),
            ffmpeg: ffmpeg.to_owned(),
            width: settings.width,
            height: settings.height,
            frame_rate: (
                settings.frame_rate.numerator,
                settings.frame_rate.denominator.max(1),
            ),
            duration_seconds: settings.duration_seconds(),
            audio,
            layers,
        })
    }

    /// How many frames the video will have.
    pub fn frame_count(&self) -> u32 {
        let frames_per_second = f64::from(self.frame_rate.0) / f64::from(self.frame_rate.1.max(1));
        (self.duration_seconds * frames_per_second).ceil().max(1.0) as u32
    }
}

#[derive(Default)]
struct State {
    finished_frames: u32,
    /// `Some` once the export has ended: `Ok` if the file is complete.
    result: Option<Result<(), String>>,
}

/// An export that is running (or has ended) in the background.
pub struct Export {
    output: PathBuf,
    frame_count: u32,
    state: Arc<Mutex<State>>,
    is_cancelled: Arc<AtomicBool>,
}

impl Export {
    /// Starts exporting `job` on a background thread. `on_progress` runs on
    /// that thread from time to time and when the export ends; the UI uses
    /// it to ask for a repaint.
    pub fn start(job: ExportJob, on_progress: impl Fn() + Send + 'static) -> Self {
        let export = Self {
            output: job.output.clone(),
            frame_count: job.frame_count(),
            state: Arc::new(Mutex::new(State::default())),
            is_cancelled: Arc::new(AtomicBool::new(false)),
        };

        let state = Arc::clone(&export.state);
        let is_cancelled = Arc::clone(&export.is_cancelled);
        thread::spawn(move || {
            let result = run(&job, &state, &is_cancelled, &on_progress);
            if result.is_err() {
                // An unfinished video file is useless; do not leave it.
                let _ = std::fs::remove_file(&job.output);
            }
            if let Ok(mut state) = state.lock() {
                state.result = Some(result);
            }
            on_progress();
        });
        export
    }

    /// The file being written.
    pub fn output(&self) -> &Path {
        &self.output
    }

    pub fn frame_count(&self) -> u32 {
        self.frame_count
    }

    pub fn finished_frame_count(&self) -> u32 {
        self.state.lock().map_or(0, |state| state.finished_frames)
    }

    /// `None` while the export is running; afterwards whether it succeeded,
    /// or why not.
    pub fn result(&self) -> Option<Result<(), String>> {
        self.state.lock().ok()?.result.clone()
    }

    /// Asks the export to stop. It ends shortly afterwards with an error
    /// result, and the unfinished file is removed.
    pub fn cancel(&self) {
        self.is_cancelled.store(true, Ordering::Relaxed);
    }
}

impl Drop for Export {
    /// Nobody is waiting for the video any more.
    fn drop(&mut self) {
        self.cancel();
    }
}

/// What a layer is drawn from during the export.
enum LayerPicture {
    /// Not looked at yet; clips are prepared when they first appear.
    NotPrepared,
    Image(DecodedImage),
    Frames {
        // Boxed (kept behind a pointer) because it is much larger than
        // the other variants.
        renderer: Box<FrameRenderer>,
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    },
}

/// The export thread.
fn run(
    job: &ExportJob,
    state: &Mutex<State>,
    is_cancelled: &AtomicBool,
    on_progress: &dyn Fn(),
) -> Result<(), String> {
    let frame_count = job.frame_count();
    let frames_per_second = f64::from(job.frame_rate.0) / f64::from(job.frame_rate.1.max(1));

    let sound_file = write_sound_file(job)?;
    let result = (|| {
        let mut compositor = Compositor::new(job.width, job.height)?;
        let mut writer = VideoWriter::create(
            &job.ffmpeg,
            &job.output,
            (job.width, job.height),
            job.frame_rate,
            sound_file.as_deref(),
        )?;
        let mut pictures: Vec<LayerPicture> = job
            .layers
            .iter()
            .map(|_| LayerPicture::NotPrepared)
            .collect();
        let mut meter = ReactionMeter::default();
        let mut frame_pixels = Vec::new();

        for frame in 0..frame_count {
            if is_cancelled.load(Ordering::Relaxed) {
                return Err("the export was cancelled".to_owned());
            }
            let seconds = f64::from(frame) / frames_per_second;

            // First bring every visible layer's picture up to this frame...
            for (layer, picture) in job.layers.iter().zip(&mut pictures) {
                if is_visible(layer, seconds) {
                    advance(job, layer, picture, frames_per_second)?;
                }
            }
            // ...then describe where each one goes, and draw them all.
            let mut draws = Vec::new();
            for (index, (layer, picture)) in job.layers.iter().zip(&pictures).enumerate() {
                if !is_visible(layer, seconds) {
                    continue;
                }
                let (width, height, pixels) = match picture {
                    LayerPicture::Image(image) => (image.width, image.height, &image.pixels),
                    LayerPicture::Frames {
                        width,
                        height,
                        pixels,
                        ..
                    } => (*width, *height, pixels),
                    LayerPicture::NotPrepared => continue,
                };
                let effect = meter.effect(&layer.input_audio, &layer.reaction, seconds);
                let geometry = layer_geometry(layer.placement, layer.opacity, effect);
                draws.push(LayerDraw {
                    key: index as u64,
                    // An image never changes; a drawn clip changes every
                    // frame.
                    version: match picture {
                        LayerPicture::Image(_) => 0,
                        _ => u64::from(frame),
                    },
                    width,
                    height,
                    pixels,
                    center: geometry.center,
                    size: geometry.size,
                    rotation_degrees: geometry.rotation_degrees,
                    opacity: geometry.opacity,
                });
            }
            compositor.render(&draws, &mut frame_pixels)?;
            writer.write_frame(&frame_pixels)?;

            if let Ok(mut state) = state.lock() {
                state.finished_frames = frame + 1;
            }
            if frame.is_multiple_of(PROGRESS_INTERVAL) {
                on_progress();
            }
        }
        writer.finish()
    })();

    if let Some(sound_file) = sound_file {
        let _ = std::fs::remove_file(sound_file);
    }
    result
}

fn is_visible(layer: &ExportLayer, seconds: f64) -> bool {
    seconds >= layer.start_seconds && seconds < layer.start_seconds + layer.duration_seconds
}

/// Loads the layer's image, or draws the next frame of its visualizer or
/// video. Called once per frame while the layer is visible.
fn advance(
    job: &ExportJob,
    layer: &ExportLayer,
    picture: &mut LayerPicture,
    frames_per_second: f64,
) -> Result<(), String> {
    if matches!(picture, LayerPicture::NotPrepared) {
        *picture = match &layer.content {
            ExportContent::Image(path) => {
                let image = decode_image_file(path, IMAGE_EDGE_LIMIT)
                    .map_err(|problem| format!("{}: {problem}", path.display()))?;
                LayerPicture::Image(image)
            }
            // The same drawing as in the preview, for the video's height.
            ExportContent::Text(text) => LayerPicture::Image(render_text(text, job.height)?),
            ExportContent::Frames(source) => {
                // Rendered at the size the clip has in the video.
                let width = render_edge(layer.placement.width * f64::from(job.width));
                let height = render_edge(layer.placement.height * f64::from(job.height));
                let frames_per_second = frames_per_second.round().max(1.0) as u32;
                let renderer = FrameRenderer::new(&PrerenderJob {
                    source: source.clone(),
                    width,
                    height,
                    frames_per_second,
                    frame_count: (layer.duration_seconds * f64::from(frames_per_second)).ceil()
                        as u32,
                    start_seconds: layer.start_seconds,
                    audio_sources: layer.input_audio.clone(),
                })?;
                LayerPicture::Frames {
                    renderer: Box::new(renderer),
                    width,
                    height,
                    pixels: Vec::new(),
                }
            }
        };
    }

    if let LayerPicture::Frames {
        renderer, pixels, ..
    } = picture
    {
        renderer.next_frame(pixels)?;
        if renderer.opacity_from_brightness() {
            apply_opacity_from_brightness(pixels);
        }
    }
    Ok(())
}

/// An even edge length within the allowed range.
fn render_edge(pixels: f64) -> u32 {
    let (smallest, largest) = RENDER_EDGE_RANGE;
    let rounded = if pixels.is_finite() {
        pixels.round().clamp(smallest, largest) as u32
    } else {
        smallest as u32
    };
    (rounded / 2) * 2
}

/// Mixes the project's audio into a temporary WAV file for FFmpeg. `None`
/// if the project has no audio.
fn write_sound_file(job: &ExportJob) -> Result<Option<PathBuf>, String> {
    if job.audio.is_empty() {
        return Ok(None);
    }
    let frame_count = (job.duration_seconds * f64::from(EXPORT_SAMPLE_RATE)).ceil() as usize;
    let mix = vizx_audio::mix_to_stereo(&job.audio, EXPORT_SAMPLE_RATE, 0, frame_count);
    // WAV stores whole numbers; too loud a mix is limited rather than
    // wrapped around, which would crackle.
    let samples: Vec<i16> = mix
        .iter()
        .map(|sample| (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16)
        .collect();

    let path = std::env::temp_dir().join(format!("vizx-export-{}.wav", std::process::id()));
    vizx_audio::write_wav_file(&path, EXPORT_SAMPLE_RATE, 2, &samples)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use vizx_audio::DecodedAudio;

    use super::*;
    use crate::ffmpeg::{VideoReader, find_ffmpeg, probe_video};

    fn temporary_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("vizx-export-test-{}-{name}", std::process::id()))
    }

    fn tone(seconds: f64) -> Vec<PlaybackSource> {
        let rate = 44_100.0;
        let samples = (0..(seconds * rate) as usize)
            .map(|index| 0.5 * (std::f32::consts::TAU * 440.0 * index as f32 / rate as f32).sin())
            .collect();
        vec![PlaybackSource::whole(
            Arc::new(DecodedAudio {
                sample_rate: rate as u32,
                channel_count: 1,
                samples,
            }),
            0.0,
        )]
    }

    fn image_layer(path: &Path, placement: Placement) -> ExportLayer {
        ExportLayer {
            content: ExportContent::Image(path.to_owned()),
            start_seconds: 0.0,
            duration_seconds: 1.0,
            placement,
            opacity: 1.0,
            reaction: AudioReaction::default(),
            input_audio: Vec::new(),
        }
    }

    fn wait_for(export: &Export) -> Result<(), String> {
        for _ in 0..600 {
            if let Some(result) = export.result() {
                return result;
            }
            thread::sleep(Duration::from_millis(100));
        }
        Err("the export did not end".to_owned())
    }

    #[test]
    fn frame_count_follows_duration_and_frame_rate() {
        let job = |frame_rate, duration_seconds| ExportJob {
            output: PathBuf::new(),
            ffmpeg: PathBuf::new(),
            width: 64,
            height: 48,
            frame_rate,
            duration_seconds,
            audio: Vec::new(),
            layers: Vec::new(),
        };

        assert_eq!(job((30, 1), 2.0).frame_count(), 60);
        assert_eq!(job((30000, 1001), 10.0).frame_count(), 300); // 299.7 rounded up
        assert_eq!(job((30, 1), 0.0).frame_count(), 1);
        assert_eq!(render_edge(101.4), 100);
        assert_eq!(render_edge(1.0), 16);
    }

    /// Exports one second with a red image on the left half and sound, and
    /// looks at the resulting file.
    #[test]
    fn an_exported_video_shows_the_layers_and_has_the_right_size() {
        let (Some(ffmpeg), true) = (find_ffmpeg(), vizx_render::open_device("test").is_ok()) else {
            eprintln!("FFmpeg or a graphics adapter is missing: skipping");
            return;
        };
        let image_path = temporary_path("red.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255]))
            .save(&image_path)
            .unwrap();
        let output = temporary_path("out.mp4");
        let (progress_sender, progress_receiver) = mpsc::channel();

        let export = Export::start(
            ExportJob {
                output: output.clone(),
                ffmpeg: ffmpeg.clone(),
                width: 64,
                height: 48,
                frame_rate: (10, 1),
                duration_seconds: 1.0,
                audio: tone(1.0),
                layers: vec![image_layer(
                    &image_path,
                    Placement {
                        x: 0.0,
                        y: 0.0,
                        width: 0.5,
                        height: 1.0,
                    },
                )],
            },
            move || {
                let _ = progress_sender.send(());
            },
        );
        let result = wait_for(&export);
        let _ = std::fs::remove_file(&image_path);

        assert_eq!(result, Ok(()));
        assert_eq!(export.frame_count(), 10);
        assert_eq!(export.finished_frame_count(), 10);
        assert!(progress_receiver.try_recv().is_ok());

        let size = probe_video(&ffmpeg, &output);
        let mut reader = VideoReader::open(&ffmpeg, &output, (64, 48), 10, 0.0).unwrap();
        let mut pixels = Vec::new();
        reader.read_frame(&mut pixels).unwrap();
        drop(reader);
        let has_sound = std::process::Command::new(&ffmpeg)
            .arg("-hide_banner")
            .arg("-i")
            .arg(&output)
            .output()
            .is_ok_and(|description| {
                String::from_utf8_lossy(&description.stderr).contains("Audio:")
            });
        let _ = std::fs::remove_file(&output);

        assert_eq!(size, Ok((64, 48)));
        assert!(has_sound, "the video has no sound track");
        let pixel = |x: usize, y: usize| &pixels[(y * 64 + x) * 4..(y * 64 + x) * 4 + 3];
        let left = pixel(12, 24);
        let right = pixel(52, 24);
        assert!(left[0] > 180 && left[1] < 80, "left: {left:?}");
        assert!(right.iter().all(|part| *part < 50), "right: {right:?}");
    }

    #[test]
    fn a_failed_or_cancelled_export_leaves_no_file() {
        let (Some(ffmpeg), true) = (find_ffmpeg(), vizx_render::open_device("test").is_ok()) else {
            return;
        };
        let output = temporary_path("failed.mp4");

        // The image does not exist.
        let export = Export::start(
            ExportJob {
                output: output.clone(),
                ffmpeg,
                width: 64,
                height: 48,
                frame_rate: (10, 1),
                duration_seconds: 1.0,
                audio: Vec::new(),
                layers: vec![image_layer(
                    Path::new("vizx-export-image-that-does-not-exist.png"),
                    Placement::FULL,
                )],
            },
            || {},
        );

        let error = wait_for(&export).unwrap_err();
        assert!(error.contains("does-not-exist"), "{error}");
        assert!(!output.exists());
    }
}
