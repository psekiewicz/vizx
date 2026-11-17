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

use vizx_audio::{Beat, PlaybackSource};
use vizx_project::{AudioReaction, FrameAnimation, Motion, Project, TextProperties};
use vizx_render::{Compositor, LayerDraw};

use crate::animation::AnimationReader;
use crate::composition::{
    ReactionMeter, audio_sources, beat_count, beat_times, effects_layer_geometry, frame_source,
    layer_effects, layer_geometry,
};
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
/// At most this many bytes of decoded animation frames are kept per layer.
const ANIMATION_BYTE_BUDGET: usize = 256 * 1024 * 1024;
/// How often (in frames) the UI is told that progress was made.
const PROGRESS_INTERVAL: u32 = 10;

/// The kind of file an export writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum VideoFormat {
    /// MP4 with H.264 and AAC: plays everywhere.
    #[default]
    Mp4H264,
    /// MP4 with H.265 (HEVC): smaller files, newer players.
    Mp4H265,
    /// WebM with VP9 and Opus: for the web.
    WebmVp9,
    /// QuickTime with Apple ProRes 422 HQ: large, for further editing.
    MovProres,
    /// QuickTime with Apple ProRes 4444 and an alpha channel: the empty
    /// parts of the frame stay transparent, for laying the video over
    /// other pictures in another program.
    MovProresTransparent,
    /// One PNG file per frame, with transparency, without sound.
    PngSequence,
    /// Animated GIF, without sound.
    Gif,
}

impl VideoFormat {
    pub const ALL: [Self; 7] = [
        Self::Mp4H264,
        Self::Mp4H265,
        Self::WebmVp9,
        Self::MovProres,
        Self::MovProresTransparent,
        Self::PngSequence,
        Self::Gif,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Mp4H264 => "MP4 (H.264)",
            Self::Mp4H265 => "MP4 (H.265 / HEVC)",
            Self::WebmVp9 => "WebM (VP9)",
            Self::MovProres => "MOV (ProRes 422 HQ)",
            Self::MovProresTransparent => "MOV (ProRes 4444, transparent)",
            Self::PngSequence => "PNG sequence (transparent, no sound)",
            Self::Gif => "GIF (no sound)",
        }
    }

    /// File extension, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4H264 | Self::Mp4H265 => "mp4",
            Self::WebmVp9 => "webm",
            Self::MovProres | Self::MovProresTransparent => "mov",
            Self::PngSequence => "png",
            Self::Gif => "gif",
        }
    }

    pub fn has_sound(self) -> bool {
        !matches!(self, Self::Gif | Self::PngSequence)
    }

    /// Whether the empty parts of the frame are left transparent instead
    /// of black.
    pub fn is_transparent(self) -> bool {
        matches!(self, Self::MovProresTransparent | Self::PngSequence)
    }
}

/// The files of a PNG sequence exported to `output` ("clip.png"): FFmpeg's
/// pattern for them, "clip_00001.png" and on.
pub fn png_sequence_pattern(output: &Path) -> PathBuf {
    output.with_file_name(format!("{}_%05d.png", sequence_stem(output)))
}

/// The file of frame `number` (from 1) of a PNG sequence exported to
/// `output`.
pub fn png_sequence_frame(output: &Path, number: u32) -> PathBuf {
    output.with_file_name(format!("{}_{number:05}.png", sequence_stem(output)))
}

fn sequence_stem(output: &Path) -> String {
    output
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "frame".to_owned())
}

/// Turns premultiplied RGBA (what the compositor makes) into the plain
/// RGBA video files store: each color divided by the pixel's opacity.
fn unpremultiply(pixels: &mut [u8]) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let opacity = u32::from(pixel[3]);
        match opacity {
            255 => {}
            0 => pixel[..3].fill(0),
            _ => {
                for part in &mut pixel[..3] {
                    // Added light (the Add blend) can be brighter than the
                    // opacity allows; it stays at full brightness.
                    *part = ((u32::from(*part) * 255 + opacity / 2) / opacity).min(255) as u8;
                }
            }
        }
    }
}

/// How much an export is compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ExportQuality {
    #[default]
    High,
    Medium,
    /// Smallest files.
    Low,
}

impl ExportQuality {
    pub const ALL: [Self; 3] = [Self::High, Self::Medium, Self::Low];

    pub fn label(self) -> &'static str {
        match self {
            Self::High => "High",
            Self::Medium => "Medium",
            Self::Low => "Low",
        }
    }
}

/// What the user chose in the export window.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ExportSettings {
    pub format: VideoFormat,
    pub quality: ExportQuality,
    /// Height of the video in pixels, the width following the project's
    /// shape. `None`: the project's size.
    pub height: Option<u32>,
    /// Only this stretch of the project (start, end in seconds). `None`:
    /// the whole project.
    pub range: Option<(f64, f64)>,
}

impl ExportSettings {
    /// Heights offered besides the project's own.
    pub const HEIGHTS: [u32; 5] = [2160, 1440, 1080, 720, 480];

    /// The video size for a project of `width` x `height`: even numbers,
    /// in the project's shape.
    pub fn video_size(&self, width: u32, height: u32) -> (u32, u32) {
        let even = |value: f64| ((value.round() as u32) / 2 * 2).max(2);
        match self.height {
            Some(wanted) if wanted != height && height > 0 => (
                even(f64::from(width) * f64::from(wanted) / f64::from(height)),
                even(f64::from(wanted)),
            ),
            _ => (even(f64::from(width)), even(f64::from(height))),
        }
    }
}

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
    /// Project time of the first frame (0 unless only a part is exported).
    pub start_seconds: f64,
    /// Length of the video.
    pub duration_seconds: f64,
    pub format: VideoFormat,
    pub quality: ExportQuality,
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
    /// Where and how the clip is drawn over time (keyframes, spin).
    pub motion: Motion,
    pub reaction: AudioReaction,
    /// The audio the clip listens to: its visualizer draws from it and its
    /// reaction follows it.
    pub input_audio: Vec<PlaybackSource>,
    /// The clip's sensitivity: its audio is multiplied by this.
    pub audio_gain: f64,
    /// The beats of that audio, in project time, and its tempo (0 if not
    /// known).
    pub beats: Arc<Vec<Beat>>,
    pub tempo_bpm: f64,
    /// Enlarged, the picture keeps sharp pixels instead of being smoothed.
    pub sharp_pixels: bool,
    /// How its colors combine with the layers below.
    pub blend: vizx_render::LayerBlend,
    pub effects: vizx_project::ClipEffects,
}

/// The compositor's blend for a clip's blend mode.
pub fn layer_blend(mode: vizx_project::BlendMode) -> vizx_render::LayerBlend {
    use vizx_project::BlendMode;
    use vizx_render::LayerBlend;
    match mode {
        BlendMode::Normal => LayerBlend::Normal,
        BlendMode::Add => LayerBlend::Add,
        BlendMode::Screen => LayerBlend::Screen,
        BlendMode::Multiply => LayerBlend::Multiply,
        BlendMode::Lighten => LayerBlend::Lighten,
    }
}

pub enum ExportContent {
    /// A frame animation (image sequence or sprite sheet) stored at the path.
    Animation(PathBuf, FrameAnimation),
    /// A still image file.
    Image(PathBuf),
    /// A text element, drawn once.
    Text(TextProperties),
    /// Something drawn frame by frame: a visualizer or a video.
    Frames(FrameSource),
    /// An effects layer: it changes everything drawn below it.
    LayersBelow,
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
        export: &ExportSettings,
    ) -> Result<Self, String> {
        let (audio, clips_still_loading) =
            audio_sources(project, audio_store, vizx_project::AudioInput::Mix);
        if clips_still_loading > 0 {
            return Err("audio is still loading".to_owned());
        }

        let mut layers = Vec::new();
        let (_, video_height) = export.video_size(project.settings.width, project.settings.height);
        let subtitles = crate::subtitle_layout(project, video_height);
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
                let mut motion = clip.motion();
                let content = if track.kind == vizx_project::TrackKind::Subtitles {
                    // The layer decides the look and the place.
                    let Some(line) = subtitles.get(&clip.id) else {
                        continue;
                    };
                    motion = line.motion.clone();
                    ExportContent::Text(line.text.clone())
                } else if media.image().is_some() {
                    ExportContent::Image(PathBuf::from(&media.path))
                } else if let Some(animation) = media.animation() {
                    ExportContent::Animation(PathBuf::from(&media.path), animation.clone())
                } else if let Some(text) = media.text() {
                    ExportContent::Text(text.clone())
                } else if media.is_effects_layer() {
                    ExportContent::LayersBelow
                } else {
                    let source = frame_source(media, clip, bundled_presets, catalog, Some(ffmpeg))
                        .map_err(|problem| format!("{}: {problem}", media.file_name()))?;
                    ExportContent::Frames(source)
                };
                let (beats, tempo) = beat_times(project, audio_store, clip.audio_input);
                layers.push(ExportLayer {
                    content,
                    start_seconds: clip.start_milliseconds as f64 / 1000.0,
                    duration_seconds: clip.duration_milliseconds as f64 / 1000.0,
                    motion,
                    reaction: clip.reaction,
                    sharp_pixels: project.clip_sharp_pixels(clip),
                    blend: layer_blend(clip.blend_mode),
                    effects: clip.effects,
                    input_audio: audio_sources(project, audio_store, clip.audio_input).0,
                    audio_gain: clip.audio_gain,
                    beats: Arc::new(beats),
                    tempo_bpm: tempo.unwrap_or(0.0),
                });
            }
        }

        let settings = &project.settings;
        let project_seconds = settings.duration_seconds();
        let (start_seconds, end_seconds) = match export.range {
            Some((start, end)) => (
                start.clamp(0.0, project_seconds),
                end.clamp(0.0, project_seconds),
            ),
            None => (0.0, project_seconds),
        };
        if end_seconds - start_seconds < 0.01 {
            return Err("the range to export is empty".to_owned());
        }
        let (width, height) = export.video_size(settings.width, settings.height);
        Ok(Self {
            output: output.to_owned(),
            ffmpeg: ffmpeg.to_owned(),
            width,
            height,
            frame_rate: (
                settings.frame_rate.numerator,
                settings.frame_rate.denominator.max(1),
            ),
            start_seconds,
            duration_seconds: end_seconds - start_seconds,
            format: export.format,
            quality: export.quality,
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
                if job.format == VideoFormat::PngSequence {
                    // Only the frames this export wrote (FFmpeg may be one
                    // ahead of the count).
                    let written = state.lock().map_or(0, |state| state.finished_frames);
                    for number in 1..=written + 1 {
                        let _ = std::fs::remove_file(png_sequence_frame(&job.output, number));
                    }
                } else {
                    let _ = std::fs::remove_file(&job.output);
                }
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
    /// A frame animation, with the frame shown now and its number.
    Animation {
        reader: Box<AnimationReader>,
        frame: Arc<DecodedImage>,
        index: u32,
    },
    /// Not looked at yet; clips are prepared when they first appear.
    NotPrepared,
    /// An effects layer: what is below it.
    Below,
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
        let transparent = job.format.is_transparent();
        compositor.set_transparent(transparent);
        let mut writer = VideoWriter::create(
            &job.ffmpeg,
            &job.output,
            (job.width, job.height),
            job.frame_rate,
            sound_file.as_deref(),
            (job.format, job.quality),
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
            let seconds = job.start_seconds + f64::from(frame) / frames_per_second;

            // First bring every visible layer's picture up to this frame...
            for (layer, picture) in job.layers.iter().zip(&mut pictures) {
                if is_visible(layer, seconds) {
                    advance(job, layer, picture, frames_per_second, seconds)?;
                }
            }
            // ...then describe where each one goes, and draw them all.
            let no_pixels = Vec::new();
            let mut draws = Vec::new();
            for (index, (layer, picture)) in job.layers.iter().zip(&pictures).enumerate() {
                if !is_visible(layer, seconds) {
                    continue;
                }
                let (width, height, pixels) = match picture {
                    LayerPicture::Image(image) => (image.width, image.height, &image.pixels),
                    LayerPicture::Animation { frame, .. } => {
                        (frame.width, frame.height, &frame.pixels)
                    }
                    LayerPicture::Frames {
                        width,
                        height,
                        pixels,
                        ..
                    } => (*width, *height, pixels),
                    LayerPicture::Below => (0, 0, &no_pixels),
                    LayerPicture::NotPrepared => continue,
                };
                let shows_below = matches!(picture, LayerPicture::Below);
                let effect = meter.effect(
                    &layer.input_audio,
                    &layer.beats,
                    &layer.reaction,
                    layer.audio_gain,
                    seconds,
                    f64::from(job.width) / f64::from(job.height.max(1)),
                );
                let transform = layer.motion.animated_at(seconds - layer.start_seconds);
                let geometry = if shows_below {
                    effects_layer_geometry(&transform, effect, &layer.reaction)
                } else {
                    layer_geometry(&transform, effect)
                };
                draws.push(LayerDraw {
                    key: index as u64,
                    // An image never changes; a drawn clip changes every
                    // frame.
                    version: match picture {
                        LayerPicture::Image(_) => 0,
                        LayerPicture::Animation { index, .. } => u64::from(*index),
                        _ => u64::from(frame),
                    },
                    width,
                    height,
                    pixels,
                    center: geometry.center,
                    size: geometry.size,
                    rotation_degrees: geometry.rotation_degrees,
                    opacity: geometry.opacity,
                    sharp_pixels: layer.sharp_pixels,
                    blend: layer.blend,
                    effects: layer_effects(&layer.effects, &layer.reaction, effect, seconds),
                    shows_below,
                });
            }
            compositor.render(&draws, &mut frame_pixels)?;
            if transparent {
                unpremultiply(&mut frame_pixels);
            }
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
    seconds: f64,
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
            ExportContent::LayersBelow => LayerPicture::Below,
            ExportContent::Animation(path, animation) => {
                let mut reader = AnimationReader::new(
                    &path.to_string_lossy(),
                    animation,
                    IMAGE_EDGE_LIMIT,
                    ANIMATION_BYTE_BUDGET,
                );
                let frame = reader.frame(0)?;
                LayerPicture::Animation {
                    reader: Box::new(reader),
                    frame,
                    index: 0,
                }
            }
            ExportContent::Frames(source) => {
                // Rendered at the size the clip has in the video.
                // Rendered at the largest size the clip reaches in the video.
                let (largest_width, largest_height) = layer.motion.largest_size();
                let width = render_edge(largest_width * f64::from(job.width));
                let height = render_edge(largest_height * f64::from(job.height));
                let frames_per_second = frames_per_second.round().max(1.0) as u32;
                let mut renderer = FrameRenderer::new(&PrerenderJob {
                    source: source.clone(),
                    width,
                    height,
                    frames_per_second,
                    frame_count: (layer.duration_seconds * f64::from(frames_per_second)).ceil()
                        as u32,
                    start_seconds: layer.start_seconds,
                    audio_sources: layer.input_audio.clone(),
                    audio_gain: layer.audio_gain as f32,
                    beats: Arc::clone(&layer.beats),
                    tempo_bpm: layer.tempo_bpm,
                })?;
                // A clip that began before the exported part is drawn from
                // its start, so it looks as it does there: the frames
                // before are drawn and dropped.
                let skipped = ((seconds - layer.start_seconds) * f64::from(frames_per_second))
                    .round()
                    .max(0.0) as u32;
                let mut pixels = Vec::new();
                for _ in 0..skipped {
                    renderer.next_frame(&mut pixels)?;
                }
                LayerPicture::Frames {
                    renderer: Box::new(renderer),
                    width,
                    height,
                    pixels,
                }
            }
        };
    }

    if let (
        LayerPicture::Animation {
            reader,
            frame,
            index,
        },
        ExportContent::Animation(_, animation),
    ) = (&mut *picture, &layer.content)
    {
        let wanted = animation.frame_at(
            seconds - layer.start_seconds,
            beat_count(&layer.beats, seconds),
        );
        if wanted != *index {
            *frame = reader.frame(wanted)?;
            *index = wanted;
        }
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
    if job.audio.is_empty() || !job.format.has_sound() {
        return Ok(None);
    }
    let frame_count = (job.duration_seconds * f64::from(EXPORT_SAMPLE_RATE)).ceil() as usize;
    let start_frame = (job.start_seconds * f64::from(EXPORT_SAMPLE_RATE)).round() as u64;
    let mix = vizx_audio::mix_to_stereo(&job.audio, EXPORT_SAMPLE_RATE, start_frame, frame_count);
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

    use vizx_project::Placement;

    use super::*;
    use crate::ffmpeg::{VideoReader, find_ffmpeg, probe_video};

    #[test]
    fn export_size_keeps_the_project_shape_in_even_numbers() {
        let at = |height| ExportSettings {
            height,
            ..ExportSettings::default()
        };
        assert_eq!(at(None).video_size(1920, 1080), (1920, 1080));
        assert_eq!(at(Some(720)).video_size(1920, 1080), (1280, 720));
        assert_eq!(at(Some(480)).video_size(1920, 1080), (852, 480));
        // A square project stays square.
        assert_eq!(at(Some(1080)).video_size(800, 800), (1080, 1080));
    }

    #[test]
    fn a_range_exports_only_its_part() {
        let mut project = Project::new();
        project.settings.duration_milliseconds = 10_000;
        let settings = ExportSettings {
            range: Some((2.0, 5.0)),
            ..ExportSettings::default()
        };
        let job = ExportJob::from_project(
            &project,
            &AudioStore::default(),
            &mut VisualizerCatalog::with_bundled_root(None),
            None,
            Path::new("ffmpeg"),
            Path::new("out.mp4"),
            &settings,
        )
        .unwrap();
        assert_eq!(job.start_seconds, 2.0);
        assert_eq!(job.duration_seconds, 3.0);
        assert_eq!(job.frame_count(), 90);
    }

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
            motion: Motion::still(placement),
            reaction: AudioReaction::default(),
            input_audio: Vec::new(),
            audio_gain: 1.0,
            beats: Arc::new(Vec::new()),
            tempo_bpm: 0.0,
            sharp_pixels: false,
            blend: vizx_render::LayerBlend::Normal,
            effects: vizx_project::ClipEffects::default(),
        }
    }

    /// An effects layer over a red left half moves it to the right and
    /// tints it, in the exported video as in the preview.
    #[test]
    fn an_effects_layer_changes_the_layers_below_it_in_the_export() {
        let (Some(ffmpeg), true) = (find_ffmpeg(), vizx_render::open_device("test").is_ok()) else {
            eprintln!("FFmpeg or a graphics adapter is missing: skipping");
            return;
        };
        let image_path = temporary_path("camera-source.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255]))
            .save(&image_path)
            .unwrap();
        let output = temporary_path("camera-source.png").with_file_name("camera-out.png");
        let red = image_layer(
            &image_path,
            Placement {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            },
        );
        let mut camera = image_layer(
            &image_path,
            Placement {
                x: 0.5,
                ..Placement::FULL
            },
        );
        camera.content = ExportContent::LayersBelow;
        camera.effects = vizx_project::ClipEffects {
            tint: 1.0,
            tint_color: [0.0, 0.0, 1.0],
            ..vizx_project::ClipEffects::default()
        };

        let export = Export::start(
            ExportJob {
                output: output.clone(),
                ffmpeg,
                width: 32,
                height: 16,
                start_seconds: 0.0,
                frame_rate: (10, 1),
                duration_seconds: 0.1,
                format: VideoFormat::PngSequence,
                quality: ExportQuality::Low,
                audio: Vec::new(),
                layers: vec![red, camera],
            },
            || {},
        );
        let result = wait_for(&export);
        let _ = std::fs::remove_file(&image_path);
        assert_eq!(result, Ok(()));
        let frame = png_sequence_frame(&output, 1);
        let picture = image::open(&frame).unwrap().to_rgba8();
        let _ = std::fs::remove_file(&frame);
        // A PNG sequence is transparent where nothing is drawn.
        assert_eq!(picture.get_pixel(4, 8).0[3], 0, "left is empty");
        let right = picture.get_pixel(24, 8).0;
        assert!(
            right[0] < 10 && right[2] > 60,
            "right is the tinted red: {right:?}"
        );
    }

    /// A red picture tinted blue by its effects, which follow the beats:
    /// red until the beat at 0.5 s, blue on it.
    #[test]
    fn effects_that_follow_the_beats_appear_on_them() {
        let (Some(ffmpeg), true) = (find_ffmpeg(), vizx_render::open_device("test").is_ok()) else {
            eprintln!("FFmpeg or a graphics adapter is missing: skipping");
            return;
        };
        let image_path = temporary_path("beat-tint.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255]))
            .save(&image_path)
            .unwrap();
        let output = temporary_path("beat-tint.png").with_file_name("beat-tint-out.png");
        let mut layer = image_layer(&image_path, Placement::FULL);
        layer.reaction = AudioReaction {
            band: vizx_project::ReactionBand::Beat,
            drives_effects: true,
            release_seconds: 0.15,
            ..AudioReaction::default()
        };
        layer.effects = vizx_project::ClipEffects {
            tint: 1.0,
            tint_color: [0.0, 0.0, 1.0],
            ..vizx_project::ClipEffects::default()
        };
        layer.beats = Arc::new(vec![Beat {
            seconds: 0.5,
            strength: 1.0,
        }]);

        let export = Export::start(
            ExportJob {
                output: output.clone(),
                ffmpeg,
                width: 32,
                height: 32,
                start_seconds: 0.0,
                frame_rate: (10, 1),
                duration_seconds: 0.7,
                format: VideoFormat::PngSequence,
                quality: ExportQuality::Low,
                audio: Vec::new(),
                layers: vec![layer],
            },
            || {},
        );
        let result = wait_for(&export);
        let _ = std::fs::remove_file(&image_path);
        assert_eq!(result, Ok(()));
        let middle = |number: u32| {
            let frame = png_sequence_frame(&output, number);
            let pixel = *image::open(&frame).unwrap().to_rgba8().get_pixel(16, 16);
            pixel.0
        };
        let colors: Vec<[u8; 4]> = (1..=7).map(middle).collect();
        for number in 1..=7 {
            let _ = std::fs::remove_file(png_sequence_frame(&output, number));
        }
        // Frame 6 is at 0.5 s.
        assert_eq!(colors[4], [255, 0, 0, 255], "before the beat");
        assert!(
            colors[5][0] < 10 && colors[5][2] > 60,
            "on the beat: {:?}",
            colors[5]
        );
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
            start_seconds: 0.0,
            frame_rate,
            duration_seconds,
            format: VideoFormat::Mp4H264,
            quality: ExportQuality::High,
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
                start_seconds: 0.0,
                frame_rate: (10, 1),
                duration_seconds: 1.0,
                format: VideoFormat::Mp4H264,
                quality: ExportQuality::High,
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

    /// A red square that moves from the left edge to the right edge with
    /// keyframes: the video shows it on the left first and on the right
    /// at the end.
    #[test]
    fn keyframes_move_a_layer_in_the_exported_video() {
        let (Some(ffmpeg), true) = (find_ffmpeg(), vizx_render::open_device("test").is_ok()) else {
            eprintln!("FFmpeg or a graphics adapter is missing: skipping");
            return;
        };
        let image_path = temporary_path("moving.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255]))
            .save(&image_path)
            .unwrap();
        let output = temporary_path("moving.mp4");
        let square = |x: f64| vizx_project::Transform {
            placement: Placement {
                x,
                y: 0.25,
                width: 0.25,
                height: 0.5,
            },
            rotation_degrees: 0.0,
            opacity: 1.0,
        };
        let keyframe = |time_milliseconds, x| vizx_project::Keyframe {
            time_milliseconds,
            transform: square(x),
            easing: vizx_project::Easing::Linear,
        };
        let mut layer = image_layer(&image_path, Placement::FULL);
        layer.motion.keyframes = vec![keyframe(0, 0.0), keyframe(900, 0.75)];

        let export = Export::start(
            ExportJob {
                output: output.clone(),
                ffmpeg: ffmpeg.clone(),
                width: 64,
                height: 48,
                start_seconds: 0.0,
                frame_rate: (10, 1),
                duration_seconds: 1.0,
                format: VideoFormat::Mp4H264,
                quality: ExportQuality::High,
                audio: Vec::new(),
                layers: vec![layer],
            },
            || {},
        );
        let result = wait_for(&export);
        let _ = std::fs::remove_file(&image_path);
        assert_eq!(result, Ok(()));

        let mut reader = VideoReader::open(&ffmpeg, &output, (64, 48), 10, 0.0).unwrap();
        let mut frames = Vec::new();
        for _ in 0..10 {
            let mut pixels = Vec::new();
            reader.read_frame(&mut pixels).unwrap();
            frames.push(pixels);
        }
        drop(reader);
        let _ = std::fs::remove_file(&output);

        let red_at = |pixels: &[u8], x: usize| pixels[(24 * 64 + x) * 4] > 150;
        assert!(
            red_at(&frames[0], 8) && !red_at(&frames[0], 56),
            "first frame"
        );
        assert!(
            red_at(&frames[9], 56) && !red_at(&frames[9], 8),
            "last frame"
        );
        // Halfway it is in the middle.
        assert!(red_at(&frames[5], 32) && !red_at(&frames[5], 4));
    }

    /// A sprite sheet of two frames, red and blue, at 5 frames per second:
    /// the video changes color every 0.2 s.
    #[test]
    fn a_sprite_sheet_plays_in_the_exported_video() {
        let (Some(ffmpeg), true) = (find_ffmpeg(), vizx_render::open_device("test").is_ok()) else {
            eprintln!("FFmpeg or a graphics adapter is missing: skipping");
            return;
        };
        let sheet_path = temporary_path("sheet.png");
        let mut sheet = image::RgbaImage::from_pixel(16, 8, image::Rgba([255, 0, 0, 255]));
        image::imageops::replace(
            &mut sheet,
            &image::RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 255, 255])),
            8,
            0,
        );
        sheet.save(&sheet_path).unwrap();
        let output = temporary_path("sheet.mp4");
        let mut layer = image_layer(&sheet_path, Placement::FULL);
        layer.content = ExportContent::Animation(
            sheet_path.clone(),
            FrameAnimation::sprite_sheet((16, 8), 2, 1, 5.0),
        );

        let export = Export::start(
            ExportJob {
                output: output.clone(),
                ffmpeg: ffmpeg.clone(),
                width: 64,
                height: 48,
                start_seconds: 0.0,
                frame_rate: (10, 1),
                duration_seconds: 1.0,
                format: VideoFormat::Mp4H264,
                quality: ExportQuality::High,
                audio: Vec::new(),
                layers: vec![layer],
            },
            || {},
        );
        let result = wait_for(&export);
        let _ = std::fs::remove_file(&sheet_path);
        assert_eq!(result, Ok(()));

        let mut reader = VideoReader::open(&ffmpeg, &output, (64, 48), 10, 0.0).unwrap();
        let mut colors = Vec::new();
        for _ in 0..10 {
            let mut pixels = Vec::new();
            reader.read_frame(&mut pixels).unwrap();
            let middle = (24 * 64 + 32) * 4;
            colors.push(if pixels[middle] > 150 {
                'R'
            } else if pixels[middle + 2] > 150 {
                'B'
            } else {
                '?'
            });
        }
        drop(reader);
        let _ = std::fs::remove_file(&output);

        assert_eq!(colors.iter().collect::<String>(), "RRBBRRBBRR");
    }

    #[test]
    fn premultiplied_colors_become_plain_colors() {
        let mut pixels = vec![
            128, 0, 64, 128, // half-transparent red-purple
            0, 0, 0, 0, // empty
            9, 9, 9, 0, // empty with stray light
            200, 0, 0, 100, // added light brighter than its opacity
            10, 20, 30, 255, // opaque
        ];
        unpremultiply(&mut pixels);
        assert_eq!(
            pixels,
            [
                255, 0, 128, 128, 0, 0, 0, 0, 0, 0, 0, 0, 255, 0, 0, 100, 10, 20, 30, 255
            ]
        );
    }

    /// A red square on the left half, the right half empty, the square at
    /// half opacity: both transparent formats keep exactly that.
    #[test]
    fn transparent_formats_keep_the_empty_parts_transparent() {
        let (Some(ffmpeg), true) = (find_ffmpeg(), vizx_render::open_device("test").is_ok()) else {
            eprintln!("FFmpeg or a graphics adapter is missing: skipping");
            return;
        };
        let image_path = temporary_path("transparent-source.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255]))
            .save(&image_path)
            .unwrap();
        let half_red_square = || {
            let mut layer = image_layer(
                &image_path,
                Placement {
                    x: 0.0,
                    y: 0.0,
                    width: 0.5,
                    height: 1.0,
                },
            );
            layer.motion.resting.opacity = 0.5;
            layer
        };

        for format in [VideoFormat::PngSequence, VideoFormat::MovProresTransparent] {
            let output = temporary_path(&format!("transparent.{}", format.extension()));
            let export = Export::start(
                ExportJob {
                    output: output.clone(),
                    ffmpeg: ffmpeg.clone(),
                    width: 64,
                    height: 48,
                    start_seconds: 0.0,
                    frame_rate: (10, 1),
                    duration_seconds: 0.3,
                    format,
                    quality: ExportQuality::Medium,
                    audio: Vec::new(),
                    layers: vec![half_red_square()],
                },
                || {},
            );
            assert_eq!(wait_for(&export), Ok(()), "{format:?}");

            let pixels = if format == VideoFormat::PngSequence {
                let first = png_sequence_frame(&output, 1);
                let pixels = image::open(&first).unwrap().to_rgba8().into_raw();
                for number in 1..=3 {
                    let frame = png_sequence_frame(&output, number);
                    assert!(frame.is_file(), "{}", frame.display());
                    let _ = std::fs::remove_file(frame);
                }
                pixels
            } else {
                let mut reader = VideoReader::open(&ffmpeg, &output, (64, 48), 10, 0.0).unwrap();
                let mut pixels = Vec::new();
                reader.read_frame(&mut pixels).unwrap();
                drop(reader);
                let _ = std::fs::remove_file(&output);
                pixels
            };
            let at = |x: usize| {
                let start = (24 * 64 + x) * 4;
                [pixels[start], pixels[start + 1], pixels[start + 3]]
            };
            let [red, green, opacity] = at(16);
            assert!(
                red > 240 && green < 15 && (118..=138).contains(&opacity),
                "{format:?} left: {:?}",
                at(16)
            );
            assert_eq!(at(48)[2], 0, "{format:?} right half is transparent");
        }
        let _ = std::fs::remove_file(&image_path);
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
                start_seconds: 0.0,
                frame_rate: (10, 1),
                duration_seconds: 1.0,
                format: VideoFormat::Mp4H264,
                quality: ExportQuality::High,
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
