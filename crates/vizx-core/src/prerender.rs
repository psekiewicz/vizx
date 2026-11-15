use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use image::ImageFormat;
use jpeg_encoder::{ColorType, Encoder};
use vizx_audio::{ANALYSIS_WINDOW, AudioAnalyzer, PlaybackSource};
use vizx_milkdrop::MilkdropRenderer;
use vizx_visualizer::{FrameInput, ParameterValues, ShaderRenderer, VisualizerPlugin};

use crate::ffmpeg::VideoReader;

/// Sample rate of the audio handed to the visualizer.
const ANALYSIS_SAMPLE_RATE: u32 = 44_100;
/// JPEG quality of stored frames (1-100). High enough that the preview
/// shows no obvious artifacts, low enough to keep minutes of frames small.
const JPEG_QUALITY: u8 = 80;
/// How often (in frames) the UI is told that progress was made.
const PROGRESS_INTERVAL: u32 = 15;
/// How many drawn frames may wait to be compressed.
const PENDING_FRAME_LIMIT: usize = 4;
/// Frame files older than this are leftovers of earlier runs.
const OLD_FRAME_FILE_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// What draws the frames of a clip.
#[derive(Clone)]
pub enum FrameSource {
    /// A MilkDrop preset (`.milk` file), run by projectM.
    Milkdrop { preset_path: String },
    /// A visualizer plugin: a shader run on the graphics card.
    Plugin {
        folder: PathBuf,
        /// Values the user chose for the plugin's parameters.
        parameters: ParameterValues,
        /// Changes when the plugin's files are saved (see
        /// `VisualizerCatalog`), so that an edited plugin is rendered again.
        version: u64,
    },
    /// A video file, decoded by FFmpeg. It starts over when it ends.
    Video {
        file: PathBuf,
        /// The `ffmpeg` program to decode it with.
        ffmpeg: PathBuf,
        /// Where in the video the clip begins, in seconds.
        start_offset_seconds: f64,
    },
}

/// Where the opacity of a stored frame comes from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Opacity {
    /// The renderer delivers it with every pixel; it is stored.
    Stored,
    /// The renderer only knows colors. A pixel is as opaque as it is
    /// bright, so black is transparent and the picture lies on the layers
    /// below like light. Nothing extra is stored.
    FromBrightness,
}

impl FrameSource {
    /// MilkDrop draws on black and has no notion of transparency; plugins
    /// return an opacity for every pixel.
    fn opacity(&self) -> Opacity {
        match self {
            Self::Milkdrop { .. } => Opacity::FromBrightness,
            // A video is fully opaque, and for such frames nothing extra is
            // stored either.
            Self::Plugin { .. } | Self::Video { .. } => Opacity::Stored,
        }
    }
    /// A text that is equal exactly when two sources draw the same frames.
    fn description(&self) -> String {
        match self {
            Self::Milkdrop { preset_path } => preset_path.clone(),
            Self::Plugin {
                folder,
                parameters,
                version,
            } => format!("{} v{version} {parameters:?}", folder.display()),
            Self::Video {
                file,
                start_offset_seconds,
                ..
            } => format!("video {} from {start_offset_seconds:.3}s", file.display()),
        }
    }
}

/// Everything that determines the frames of one visual clip.
#[derive(Clone)]
pub struct PrerenderJob {
    pub source: FrameSource,
    pub width: u32,
    pub height: u32,
    pub frames_per_second: u32,
    pub frame_count: u32,
    /// Project time of the first frame (where the clip starts).
    pub start_seconds: f64,
    /// The project audio the preset reacts to.
    pub audio_sources: Vec<PlaybackSource>,
}

impl PrerenderJob {
    /// A text that is equal for two jobs exactly when they produce the same
    /// frames. Frames rendered for one key can be reused as long as the key
    /// stays the same; any change (another preset, other audio, a different
    /// length or size) gives a different key and requires rendering again.
    pub fn key(&self) -> String {
        let mut key = format!(
            "{}|{}x{}@{}|{} frames from {:.3}s",
            self.source.description(),
            self.width,
            self.height,
            self.frames_per_second,
            self.frame_count,
            self.start_seconds
        );
        for source in &self.audio_sources {
            // The address of the shared samples identifies the audio file
            // for as long as it stays loaded.
            key.push_str(&format!(
                "|audio {:p} at {:.3}s from {:.3}s for {:?}",
                Arc::as_ptr(&source.audio),
                source.start_seconds,
                source.offset_seconds,
                source.duration_seconds
            ));
        }
        key
    }
}

/// One decoded frame, ready to be shown.
pub struct PrerenderedFrame {
    pub width: u32,
    pub height: u32,
    /// RGBA bytes, top row first. The fourth byte is the opacity and the
    /// color bytes are already multiplied by it ("premultiplied alpha"),
    /// which is the form used for drawing one picture over another.
    pub pixels: Vec<u8>,
}

/// Where one stored frame is inside the frame file.
#[derive(Clone, Copy)]
struct FrameLocation {
    offset: u64,
    /// Size of the JPEG with the colors.
    length: u32,
    /// Size of the JPEG with the opacity, which follows the colors. 0 if
    /// the frame has none (it is fully opaque, or opacity is not stored).
    opacity_length: u32,
}

#[derive(Default)]
struct State {
    /// Finished frames in order.
    frames: Vec<FrameLocation>,
    /// Total size of the frame file so far.
    stored_bytes: u64,
    error: Option<String>,
}

/// The frames of one visual clip (a MilkDrop preset, a visualizer plugin or
/// a video),
/// rendered ahead of time in the background.
///
/// A MilkDrop picture builds on the previous one, so a frame is only
/// correct when all frames before it were rendered too. Rendering "live"
/// therefore cannot show the right picture after a jump of the playhead.
/// Rendering the whole clip once, in order, and keeping the frames solves
/// that: afterwards any frame can be shown instantly, in any order, and
/// playback costs almost nothing.
///
/// The frames of a whole song are far too large for memory (a detailed
/// preset needs over a gigabyte even when compressed), so each frame is
/// compressed as a JPEG image and appended to one temporary file on disk.
/// JPEG has no transparency, so a frame's opacity is stored as a second,
/// grey JPEG right after its colors.
/// Only the position of every frame in that file is kept in memory. The
/// file is deleted when the pre-render is dropped.
///
/// A plugin's shader may read its previous frame too, so the same holds for
/// plugins.
///
/// Two threads share the work: one draws frames, the other
/// compresses and stores them. Drawing uses the graphics card and
/// compressing uses the processor, so together they are much faster than
/// one thread doing both in turn.
pub struct Prerender {
    key: String,
    width: u32,
    height: u32,
    frame_count: u32,
    opacity: Opacity,
    state: Arc<Mutex<State>>,
    /// Set to make the threads stop early.
    is_cancelled: Arc<AtomicBool>,
    frame_file_path: PathBuf,
    /// Opened the first time a frame is read, then kept open.
    frame_reader: Mutex<Option<File>>,
}

impl Prerender {
    /// Starts rendering `job` in the background. `on_progress` runs on a
    /// background thread from time to time and when rendering ends; the UI
    /// uses it to ask for a repaint.
    pub fn start(job: PrerenderJob, on_progress: impl Fn() + Send + Sync + 'static) -> Self {
        let prerender = Self {
            key: job.key(),
            width: job.width,
            height: job.height,
            frame_count: job.frame_count,
            opacity: job.source.opacity(),
            state: Arc::new(Mutex::new(State::default())),
            is_cancelled: Arc::new(AtomicBool::new(false)),
            frame_file_path: new_frame_file_path(),
            frame_reader: Mutex::new(None),
        };

        let on_progress = Arc::new(on_progress);
        // A small buffer between the threads: the drawing thread waits when
        // compressing falls behind, instead of filling memory with frames.
        let (pixel_sender, pixel_receiver) = mpsc::sync_channel::<Vec<u8>>(PENDING_FRAME_LIMIT);

        // Thread 1: compresses frames and appends them to the file.
        {
            let state = Arc::clone(&prerender.state);
            let on_progress = Arc::clone(&on_progress);
            let path = prerender.frame_file_path.clone();
            let (width, height) = (job.width, job.height);
            let stores_opacity = prerender.opacity == Opacity::Stored;
            thread::spawn(move || {
                let size = (width, height);
                if let Err(message) = store_frames(
                    &pixel_receiver,
                    &path,
                    size,
                    stores_opacity,
                    &state,
                    &*on_progress,
                ) {
                    set_error(&state, message);
                }
                on_progress();
            });
        }

        // Thread 2: draws the frames.
        {
            let state = Arc::clone(&prerender.state);
            let is_cancelled = Arc::clone(&prerender.is_cancelled);
            thread::spawn(move || {
                if let Err(message) = draw_frames(&job, &pixel_sender, &is_cancelled) {
                    set_error(&state, message);
                }
                // Dropping `pixel_sender` here closes the channel, which
                // lets the storing thread finish.
                on_progress();
            });
        }

        prerender
    }

    /// The [`PrerenderJob::key`] these frames were rendered for.
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn frame_count(&self) -> u32 {
        self.frame_count
    }

    /// How many frames are finished so far.
    pub fn finished_frame_count(&self) -> u32 {
        self.state
            .lock()
            .map_or(0, |state| state.frames.len() as u32)
    }

    pub fn is_finished(&self) -> bool {
        self.finished_frame_count() >= self.frame_count || self.error().is_some()
    }

    /// Why rendering stopped, if it failed.
    pub fn error(&self) -> Option<String> {
        self.state.lock().ok()?.error.clone()
    }

    /// Disk space used by the stored frames, in bytes.
    pub fn stored_bytes(&self) -> u64 {
        self.state.lock().map_or(0, |state| state.stored_bytes)
    }

    /// Reads and decodes one frame. `None` if it is not rendered yet.
    pub fn frame(&self, index: u32) -> Option<PrerenderedFrame> {
        let location = *self.state.lock().ok()?.frames.get(index as usize)?;

        // The colors, directly followed by the opacity (if stored).
        let mut stored = vec![0; (location.length + location.opacity_length) as usize];
        {
            let mut reader = self.frame_reader.lock().ok()?;
            if reader.is_none() {
                *reader = File::open(&self.frame_file_path).ok();
            }
            let file = reader.as_mut()?;
            file.seek(SeekFrom::Start(location.offset)).ok()?;
            file.read_exact(&mut stored).ok()?;
        }
        let (color_jpeg, opacity_jpeg) = stored.split_at(location.length as usize);

        let mut pixels = image::load_from_memory_with_format(color_jpeg, ImageFormat::Jpeg)
            .ok()?
            .into_rgba8()
            .into_raw();
        if !opacity_jpeg.is_empty() {
            let opacity = image::load_from_memory_with_format(opacity_jpeg, ImageFormat::Jpeg)
                .ok()?
                .into_luma8()
                .into_raw();
            apply_stored_opacity(&mut pixels, &opacity);
        } else if self.opacity == Opacity::FromBrightness {
            apply_opacity_from_brightness(&mut pixels);
        }
        Some(PrerenderedFrame {
            width: self.width,
            height: self.height,
            pixels,
        })
    }
}

impl Drop for Prerender {
    /// Frames nobody will look at are not worth rendering or keeping:
    /// dropping the handle stops the threads and deletes the frame file.
    fn drop(&mut self) {
        self.is_cancelled.store(true, Ordering::Relaxed);
        if let Ok(mut reader) = self.frame_reader.lock() {
            *reader = None;
        }
        let _ = fs::remove_file(&self.frame_file_path);
    }
}

fn set_error(state: &Mutex<State>, message: String) {
    if let Ok(mut state) = state.lock() {
        // Keep the first error; later ones are usually just consequences.
        state.error.get_or_insert(message);
    }
}

/// A new, unique file in the system's temporary folder. Leftover files of
/// earlier runs (for example after a crash) are removed on the way.
fn new_frame_file_path() -> PathBuf {
    static NEXT_FILE_NUMBER: AtomicU64 = AtomicU64::new(0);

    let folder = std::env::temp_dir().join("vizx-prerender");
    let _ = fs::create_dir_all(&folder);
    remove_old_frame_files(&folder);
    folder.join(format!(
        "{}-{}.frames",
        std::process::id(),
        NEXT_FILE_NUMBER.fetch_add(1, Ordering::Relaxed)
    ))
}

fn remove_old_frame_files(folder: &Path) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let is_old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > OLD_FRAME_FILE_AGE);
        if is_old {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// The drawing thread: renders every frame in order and hands the pixels
/// to the storing thread.
fn draw_frames(
    job: &PrerenderJob,
    pixel_sender: &SyncSender<Vec<u8>>,
    is_cancelled: &AtomicBool,
) -> Result<(), String> {
    let mut renderer = FrameRenderer::new(job)?;
    let mut pixels = Vec::new();
    for _frame in 0..job.frame_count {
        if is_cancelled.load(Ordering::Relaxed) {
            return Ok(());
        }
        renderer.next_frame(&mut pixels)?;
        // Sending fails only if the storing thread has stopped (its error
        // is already recorded), so there is nothing left to do.
        if pixel_sender.send(pixels.clone()).is_err() {
            return Ok(());
        }
    }
    Ok(())
}

/// Draws the frames of one visual clip, one after another.
///
/// The preview's pre-render and the video export both get their frames
/// from this, at different sizes, so a clip looks the same in both.
///
/// A frame's time comes from its number, never from the clock, so the
/// result does not depend on how fast this computer renders. Frames must be
/// asked for in order: MilkDrop pictures and plugins with trails build on
/// the previous frame.
pub(crate) struct FrameRenderer {
    job: PrerenderJob,
    /// Number of the frame `next_frame` draws next.
    frame: u32,
    drawing: Drawing,
}

/// What actually draws, with the state it keeps between frames.
enum Drawing {
    Milkdrop {
        renderer: MilkdropRenderer,
        /// Position, in audio frames, up to which audio was handed over.
        audio_position: u64,
    },
    Plugin {
        // Boxed (kept behind a pointer) because it is much larger than
        // the other variants.
        renderer: Box<ShaderRenderer>,
        parameters: [f32; vizx_visualizer::PARAMETER_SLOTS],
        analyzer: AudioAnalyzer,
    },
    Video(VideoReader),
}

impl FrameRenderer {
    pub(crate) fn new(job: &PrerenderJob) -> Result<Self, String> {
        let drawing = match &job.source {
            FrameSource::Milkdrop { preset_path } => {
                let mut renderer =
                    MilkdropRenderer::new(job.width, job.height, job.frames_per_second)
                        .map_err(|error| error.to_string())?;
                renderer
                    .load_preset(Path::new(preset_path))
                    .map_err(|error| error.to_string())?;
                Drawing::Milkdrop {
                    renderer,
                    audio_position: seconds_to_audio_frames(job.start_seconds),
                }
            }
            FrameSource::Plugin {
                folder, parameters, ..
            } => {
                let plugin = VisualizerPlugin::load(folder)?;
                Drawing::Plugin {
                    renderer: Box::new(ShaderRenderer::new(&plugin, job.width, job.height)?),
                    parameters: plugin.pack_parameters(parameters),
                    analyzer: AudioAnalyzer::new(ANALYSIS_SAMPLE_RATE),
                }
            }
            FrameSource::Video {
                file,
                ffmpeg,
                start_offset_seconds,
            } => Drawing::Video(VideoReader::open(
                ffmpeg,
                file,
                (job.width, job.height),
                job.frames_per_second.max(1),
                *start_offset_seconds,
            )?),
        };
        Ok(Self {
            job: job.clone(),
            frame: 0,
            drawing,
        })
    }

    /// Whether the frames have no opacity of their own and get it from
    /// their brightness (see `Opacity::FromBrightness`).
    pub(crate) fn opacity_from_brightness(&self) -> bool {
        self.job.source.opacity() == Opacity::FromBrightness
    }

    /// Draws the next frame into `pixels` as RGBA bytes, top row first.
    pub(crate) fn next_frame(&mut self, pixels: &mut Vec<u8>) -> Result<(), String> {
        let frames_per_second = f64::from(self.job.frames_per_second.max(1));
        let seconds_into_clip = f64::from(self.frame) / frames_per_second;
        let project_seconds = self.job.start_seconds + seconds_into_clip;
        self.frame += 1;

        match &mut self.drawing {
            Drawing::Milkdrop {
                renderer,
                audio_position,
            } => {
                renderer
                    .set_frame_time(seconds_into_clip)
                    .map_err(|error| error.to_string())?;

                // Hand over the audio between the previous frame and this
                // one.
                let frame_audio_position = seconds_to_audio_frames(project_seconds);
                if frame_audio_position > *audio_position {
                    let audio = vizx_audio::mix_to_stereo(
                        &self.job.audio_sources,
                        ANALYSIS_SAMPLE_RATE,
                        *audio_position,
                        (frame_audio_position - *audio_position) as usize,
                    );
                    renderer.add_stereo_audio(&audio);
                    *audio_position = frame_audio_position;
                }
                renderer.render_frame(pixels);
                Ok(())
            }
            Drawing::Plugin {
                renderer,
                parameters,
                analyzer,
            } => {
                // Analyse the audio that leads up to this frame.
                let window_end = seconds_to_audio_frames(project_seconds);
                let window_start = window_end.saturating_sub(ANALYSIS_WINDOW as u64);
                let stereo = vizx_audio::mix_to_stereo(
                    &self.job.audio_sources,
                    ANALYSIS_SAMPLE_RATE,
                    window_start,
                    (window_end - window_start) as usize,
                );
                let mono: Vec<f32> = stereo
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|[left, right]| (left + right) * 0.5)
                    .collect();
                let audio = analyzer.analyze(&mono, (1.0 / frames_per_second) as f32);

                renderer.render(
                    &FrameInput {
                        time_seconds: seconds_into_clip as f32,
                        spectrum: &audio.spectrum,
                        waveform: &audio.waveform,
                        volume: audio.volume,
                        bass: audio.bass,
                        mid: audio.mid,
                        treble: audio.treble,
                        parameters: *parameters,
                    },
                    pixels,
                )
            }
            Drawing::Video(reader) => reader.read_frame(pixels),
        }
    }
}
/// The storing thread: compresses each frame and appends it to the file.
fn store_frames(
    pixel_receiver: &Receiver<Vec<u8>>,
    path: &Path,
    (width, height): (u32, u32),
    stores_opacity: bool,
    state: &Mutex<State>,
    on_progress: &dyn Fn(),
) -> Result<(), String> {
    let mut file = File::create(path)
        .map_err(|error| format!("could not create {}: {error}", path.display()))?;
    let mut offset = 0u64;
    let mut frame_number = 0u32;

    // `recv` fails once the drawing thread is done and has dropped its end
    // of the channel, which ends the loop.
    while let Ok(pixels) = pixel_receiver.recv() {
        let mut jpeg = encode_jpeg(&pixels, width, height, ColorType::Rgba)?;
        let color_length = jpeg.len();
        // A fully opaque frame needs no opacity picture.
        if stores_opacity
            && pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] != 255)
        {
            let opacity: Vec<u8> = pixels
                .as_chunks::<4>()
                .0
                .iter()
                .map(|pixel| pixel[3])
                .collect();
            jpeg.extend(encode_jpeg(&opacity, width, height, ColorType::Luma)?);
        }
        file.write_all(&jpeg)
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;

        let mut state = state
            .lock()
            .map_err(|_| "the frame store is no longer usable".to_owned())?;
        state.frames.push(FrameLocation {
            offset,
            length: color_length as u32,
            opacity_length: (jpeg.len() - color_length) as u32,
        });
        offset += jpeg.len() as u64;
        state.stored_bytes = offset;
        drop(state); // release the lock before calling back into the UI

        if frame_number.is_multiple_of(PROGRESS_INTERVAL) {
            on_progress();
        }
        frame_number += 1;
    }
    Ok(())
}

fn seconds_to_audio_frames(seconds: f64) -> u64 {
    (seconds.max(0.0) * f64::from(ANALYSIS_SAMPLE_RATE)) as u64
}

/// Puts a separately stored opacity into the fourth byte of each pixel.
///
/// JPEG changes values slightly. A color brighter than its opacity would
/// add light where the picture should be transparent, so each color byte is
/// limited to the opacity.
fn apply_stored_opacity(rgba: &mut [u8], opacity: &[u8]) {
    for (pixel, opacity) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(opacity) {
        for part in &mut pixel[..3] {
            *part = (*part).min(*opacity);
        }
        pixel[3] = *opacity;
    }
}

/// Makes each pixel as opaque as its brightest color part: black becomes
/// transparent, and the picture adds its light to what lies below.
pub(crate) fn apply_opacity_from_brightness(rgba: &mut [u8]) {
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel[3] = pixel[0].max(pixel[1]).max(pixel[2]);
    }
}

/// Compresses pixels as a JPEG image: `ColorType::Rgba` for colors (JPEG
/// has no transparency, so the fourth byte of each pixel is ignored) or
/// `ColorType::Luma` for one byte per pixel.
fn encode_jpeg(
    pixels: &[u8],
    width: u32,
    height: u32,
    color_type: ColorType,
) -> Result<Vec<u8>, String> {
    let (Ok(width), Ok(height)) = (u16::try_from(width), u16::try_from(height)) else {
        return Err("the picture is too large to store as a JPEG".to_owned());
    };

    let mut jpeg = Vec::new();
    Encoder::new(&mut jpeg, JPEG_QUALITY)
        .encode(pixels, width, height, color_type)
        .map_err(|error| format!("could not store a frame: {error}"))?;
    Ok(jpeg)
}

#[cfg(test)]
mod tests {
    use vizx_audio::DecodedAudio;

    use super::*;

    fn job(preset_path: &str) -> PrerenderJob {
        PrerenderJob {
            source: FrameSource::Milkdrop {
                preset_path: preset_path.to_owned(),
            },
            width: 64,
            height: 36,
            frames_per_second: 30,
            frame_count: 10,
            start_seconds: 0.0,
            audio_sources: Vec::new(),
        }
    }

    fn silent_source(start_seconds: f64) -> PlaybackSource {
        PlaybackSource::whole(
            Arc::new(DecodedAudio {
                sample_rate: 44_100,
                channel_count: 2,
                samples: vec![0.0; 16],
            }),
            start_seconds,
        )
    }

    #[test]
    fn key_changes_with_anything_that_changes_the_frames() {
        let base = job("a.milk");
        let key = base.key();
        assert_eq!(key, base.clone().key());

        let mut other_preset = base.clone();
        other_preset.source = FrameSource::Milkdrop {
            preset_path: "b.milk".to_owned(),
        };
        let mut longer = base.clone();
        longer.frame_count = 11;
        let mut other_size = base.clone();
        other_size.width = 128;
        let mut later = base.clone();
        later.start_seconds = 1.0;
        let mut with_audio = base.clone();
        with_audio.audio_sources.push(silent_source(0.0));

        for changed in [other_preset, longer, other_size, later, with_audio] {
            assert_ne!(changed.key(), key);
        }
    }

    #[test]
    fn key_of_a_plugin_follows_its_files_and_parameters() {
        let plugin_job = |version, bars: f64| {
            let mut parameters = ParameterValues::new();
            parameters.insert("bars".to_owned(), vec![bars]);
            PrerenderJob {
                source: FrameSource::Plugin {
                    folder: PathBuf::from("plugins/bars"),
                    parameters,
                    version,
                },
                ..job("unused")
            }
        };

        assert_eq!(plugin_job(1, 32.0).key(), plugin_job(1, 32.0).key());
        // The plugin's shader was edited and saved.
        assert_ne!(plugin_job(1, 32.0).key(), plugin_job(2, 32.0).key());
        // The user moved a slider.
        assert_ne!(plugin_job(1, 32.0).key(), plugin_job(1, 48.0).key());
    }

    #[test]
    fn a_plugin_that_does_not_exist_ends_with_an_error() {
        let (notified_sender, notified_receiver) = mpsc::channel();
        let prerender = Prerender::start(
            PrerenderJob {
                source: FrameSource::Plugin {
                    folder: PathBuf::from("vizx-core-plugin-that-does-not-exist"),
                    parameters: ParameterValues::new(),
                    version: 0,
                },
                ..job("unused")
            },
            move || {
                let _ = notified_sender.send(());
            },
        );

        assert!(
            notified_receiver
                .recv_timeout(Duration::from_secs(30))
                .is_ok(),
            "the render thread never answered"
        );
        assert!(prerender.error().unwrap().contains("visualizer.json"));
        assert_eq!(prerender.finished_frame_count(), 0);
    }

    #[test]
    fn key_follows_the_audio_file_and_where_it_is_placed() {
        let source = silent_source(0.0);
        let mut first = job("a.milk");
        first.audio_sources.push(source.clone());

        // The same loaded file again: same frames.
        let mut same_file = job("a.milk");
        same_file.audio_sources.push(source.clone());
        assert_eq!(first.key(), same_file.key());

        // The same file moved on the timeline: different frames.
        let mut moved = job("a.milk");
        moved.audio_sources.push(PlaybackSource {
            start_seconds: 2.0,
            ..source
        });
        assert_ne!(first.key(), moved.key());
    }

    #[test]
    fn stored_frames_decode_back_to_the_picture() {
        // A 4x2 picture: left half red, right half blue.
        let mut rgba = Vec::new();
        for _row in 0..2 {
            for column in 0..4 {
                rgba.extend_from_slice(if column < 2 {
                    &[255, 0, 0, 255]
                } else {
                    &[0, 0, 255, 255]
                });
            }
        }

        let jpeg = encode_jpeg(&rgba, 4, 2, ColorType::Rgba).unwrap();
        let decoded = image::load_from_memory_with_format(&jpeg, ImageFormat::Jpeg)
            .unwrap()
            .into_rgba8();

        assert_eq!((decoded.width(), decoded.height()), (4, 2));
        // JPEG is lossy, so colors are close rather than identical.
        let left = decoded.get_pixel(0, 0).0;
        let right = decoded.get_pixel(3, 1).0;
        assert!(left[0] > 150 && left[2] < 100, "left pixel was {left:?}");
        assert!(
            right[2] > 150 && right[0] < 100,
            "right pixel was {right:?}"
        );
        assert_eq!(left[3], 255);
    }

    #[test]
    fn opacity_comes_from_storage_or_from_brightness() {
        // Two pixels: one stored brighter than its opacity allows.
        let mut stored = vec![200, 100, 0, 255, 90, 90, 90, 255];
        apply_stored_opacity(&mut stored, &[255, 40]);
        assert_eq!(stored, [200, 100, 0, 255, 40, 40, 40, 40]);

        // Black is transparent, a bright pixel is opaque, colors stay.
        let mut lit = vec![0, 0, 0, 255, 10, 250, 30, 255];
        apply_opacity_from_brightness(&mut lit);
        assert_eq!(lit, [0, 0, 0, 0, 10, 250, 30, 250]);
    }

    #[test]
    fn a_frame_with_transparency_is_stored_with_its_opacity() {
        // 16x16: left half transparent, right half opaque white.
        let mut frame = Vec::new();
        for _row in 0..16 {
            for column in 0..16 {
                frame.extend_from_slice(if column < 8 { &[0; 4] } else { &[255; 4] });
            }
        }
        let path = std::env::temp_dir().join(format!(
            "vizx-prerender-opacity-test-{}.frames",
            std::process::id()
        ));
        let state = Mutex::new(State::default());
        let (sender, receiver) = mpsc::sync_channel(2);
        sender.send(frame).unwrap();
        sender.send(vec![255; 16 * 16 * 4]).unwrap(); // a fully opaque frame
        drop(sender);

        store_frames(&receiver, &path, (16, 16), true, &state, &|| {}).unwrap();
        let _ = fs::remove_file(&path);

        let frames = state.lock().unwrap().frames.clone();
        assert_eq!(frames.len(), 2);
        assert!(frames[0].opacity_length > 0);
        // Nothing extra is stored for a frame without transparency.
        assert_eq!(frames[1].opacity_length, 0);
        assert_eq!(
            frames[1].offset,
            u64::from(frames[0].length + frames[0].opacity_length)
        );
    }

    #[test]
    fn missing_preset_ends_with_an_error_and_no_frames() {
        let (notified_sender, notified_receiver) = mpsc::channel();
        let prerender = Prerender::start(
            job("vizx-core-preset-that-does-not-exist.milk"),
            move || {
                let _ = notified_sender.send(());
            },
        );

        // Works the same whether or not projectM is installed: either way
        // this job cannot produce frames.
        assert!(
            notified_receiver
                .recv_timeout(Duration::from_secs(30))
                .is_ok(),
            "the render thread never answered"
        );
        assert!(prerender.error().is_some());
        assert!(prerender.is_finished());
        assert_eq!(prerender.finished_frame_count(), 0);
        assert!(prerender.frame(0).is_none());
    }
}
