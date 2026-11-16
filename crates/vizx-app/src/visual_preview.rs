//! Builds the picture shown in the preview.
//!
//! Visualizers (MilkDrop presets, visualizer plugins) and videos are
//! rendered ahead of time in the background (see `Prerender`), so the
//! preview only looks up the frames that belong to the playhead. Images are
//! decoded once. Whatever is visible at the playhead is then laid over each
//! other by the `Compositor`, the same one the video export uses, so the
//! preview shows what an export will contain.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use vizx_core::{
    AnimationReader, AudioStore, Compositor, DecodedImage, EditorSession, ImageState, ImageStore,
    LayerDraw, LayerGeometry, Prerender, PrerenderJob, ReactionMeter, VisualizerCatalog,
    audio_sources, beat_count, beat_times, frame_source, layer_geometry,
};
use vizx_project::{
    AudioInput, ClipId, FrameAnimation, Placement, ReactionEffect, TextProperties, TrackId,
};

/// Width in pixels at which a clip that fills the whole frame is
/// pre-rendered. The height follows from the project's aspect ratio, and a
/// smaller clip is rendered proportionally smaller. Kept small so that
/// rendering and storing every frame of a song stays affordable on modest
/// hardware; export renders at full size.
const PREVIEW_WIDTH: u32 = 640;
/// Smallest and largest edge of a pre-rendered picture, in pixels.
const RENDER_EDGE_RANGE: (u32, u32) = (16, 1920);
/// Largest width, in pixels, of the composed preview picture.
const MAX_COMPOSITION_WIDTH: u32 = 1280;
/// Longest edge, in pixels, of a frame of a frame animation in the preview.
const ANIMATION_EDGE_LIMIT: u32 = 1024;
/// At most this many bytes of decoded animation frames are kept per clip.
const ANIMATION_BYTE_BUDGET: usize = 128 * 1024 * 1024;
/// A render starts only after its settings have stayed the same for this
/// long. Dragging a size value changes them many times per second, and
/// each change would otherwise start and throw away a render.
const RENDER_START_DELAY: Duration = Duration::from_millis(350);

/// Names under which the preview panel leaves notes for this module in
/// egui's temporary storage.
const SHOWN_WIDTH_ID: &str = "vizx_preview_shown_width";
const DRAGGED_PLACEMENT_ID: &str = "vizx_preview_dragged_placement";

/// Called by the preview panel: the composition is currently shown this
/// many screen pixels wide. The next frame is composed at that size, so it
/// is neither blurry nor larger than needed.
pub fn set_shown_width(ctx: &egui::Context, pixels: f32) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(SHOWN_WIDTH_ID), pixels));
}

/// Called by the preview panel while a clip is being moved or resized with
/// the mouse: draw that clip at this placement instead of the one stored in
/// the project, so the picture follows the mouse. `None` ends it.
pub fn set_dragged_placement(ctx: &egui::Context, dragged: Option<(ClipId, Placement, f64)>) {
    let id = egui::Id::new(DRAGGED_PLACEMENT_ID);
    ctx.data_mut(|data| match dragged {
        Some(dragged) => {
            data.insert_temp(id, dragged);
        }
        None => data.remove::<(ClipId, Placement, f64)>(id),
    });
}

/// One visual clip at the playhead, for the preview panel: where it is, and
/// whether it could be drawn.
pub struct LayerView {
    pub clip_id: ClipId,
    /// The visual layer the clip is on.
    pub track_id: TrackId,
    pub placement: Placement,
    /// Its keyed rotation at the playhead, in degrees clockwise.
    pub rotation_degrees: f64,
    /// Enlarged, its picture keeps sharp pixels instead of being smoothed.
    pub sharp_pixels: bool,
    /// How its colors combine with the layers below.
    pub blend: vizx_core::LayerBlend,
    /// Its effects at the playhead.
    pub effects: vizx_core::LayerEffects,
    /// It is an effects layer: it changes the layers below it and has no
    /// picture of its own.
    pub shows_below: bool,
    pub state: LayerState,
}

/// Whether a clip is part of the composed picture right now.
pub enum LayerState {
    /// It is in the picture.
    Shown,
    /// The clip's frames are still being rendered and the playhead is ahead
    /// of them. `total` is 0 while the clip waits for its turn.
    Rendering { finished: u32, total: u32 },
    /// Rendering cannot start until the project's audio has been loaded.
    WaitingForAudio,
    /// The image file is still being read.
    LoadingImage,
    /// The clip cannot be drawn; the text says why.
    Failed(String),
}

/// The pixels of one clip as they are right now.
struct LayerPicture {
    width: u32,
    height: u32,
    pixels: Pixels,
    /// Which frame this is, and of which render (its key), so the same
    /// frame is not decoded again every frame. For an image, the key is its
    /// file and the frame is always 0.
    frame_index: u32,
    render_key: String,
    /// Counts up whenever the pixels change, which tells the compositor to
    /// send them to the graphics card again.
    version: u64,
}

enum Pixels {
    /// A decoded pre-rendered frame.
    Frame(Vec<u8>),
    /// A decoded image, shared with the image store.
    Image(Arc<DecodedImage>),
}

impl LayerPicture {
    fn bytes(&self) -> &[u8] {
        match &self.pixels {
            Pixels::Frame(pixels) => pixels,
            Pixels::Image(image) => &image.pixels,
        }
    }
}

pub struct VisualPreview {
    /// Folder of the presets that ship with VizX, if this installation has
    /// one. Projects refer to those presets by their place in this folder.
    bundled_presets: Option<PathBuf>,
    /// The FFmpeg program, if it was found. Needed for video clips.
    ffmpeg: Option<PathBuf>,
    /// Pre-rendered frames of every visualizer and video clip.
    prerenders: HashMap<ClipId, Prerender>,
    /// Renders that are wanted but not started yet: the key of the wanted
    /// job and since when it has been wanted unchanged.
    waiting_since: HashMap<ClipId, (String, Instant)>,
    pictures: HashMap<ClipId, LayerPicture>,
    /// Readers of the frame animations, by clip.
    animations: HashMap<ClipId, AnimationReader>,
    last_picture_version: u64,
    /// The visual clips at the playhead, bottom layer first.
    layers: Vec<LayerView>,
    /// Measures the sound at the playhead for clips that react to it.
    meter: ReactionMeter,
    /// Created when the first picture is composed. `Err` if the graphics
    /// card could not be used; the text says why.
    compositor: Option<Result<Compositor, String>>,
    /// The composed picture, as egui shows it.
    composition: Option<egui::TextureHandle>,
    /// Why the picture could not be composed, if it could not.
    composition_error: Option<String>,
    frame_pixels: Vec<u8>,
}

impl VisualPreview {
    pub fn new(bundled_presets: Option<PathBuf>, ffmpeg: Option<PathBuf>) -> Self {
        Self {
            bundled_presets,
            ffmpeg,
            prerenders: HashMap::new(),
            waiting_since: HashMap::new(),
            pictures: HashMap::new(),
            animations: HashMap::new(),
            last_picture_version: 0,
            layers: Vec::new(),
            meter: ReactionMeter::default(),
            compositor: None,
            composition: None,
            composition_error: None,
            frame_pixels: Vec::new(),
        }
    }

    /// Call once per frame, before the UI is drawn.
    pub fn update(
        &mut self,
        ctx: &egui::Context,
        session: &EditorSession,
        audio_store: &AudioStore,
        catalog: &mut VisualizerCatalog,
        image_store: &ImageStore,
    ) {
        let project = session.project();
        let settings = &project.settings;
        let frames_per_second = settings.frame_rate.as_f64().round().max(1.0) as u32;
        // Pictures of clips that no longer exist are not needed any more.
        self.pictures
            .retain(|clip_id, _| project.visual_clips().any(|(clip, _)| clip.id == *clip_id));
        self.animations
            .retain(|clip_id, _| project.visual_clips().any(|(clip, _)| clip.id == *clip_id));

        // Rendering waits until all audio is loaded, not only the audio a
        // clip reacts to, so that no clip is rendered twice.
        let (_all_sources, clips_still_loading) =
            audio_sources(project, audio_store, AudioInput::Mix);
        let is_audio_ready = clips_still_loading == 0;

        if is_audio_ready {
            // What should be rendered: one job per visualizer or video clip.
            let mut wanted: Vec<(ClipId, PrerenderJob)> = Vec::new();
            for (clip, media) in project.visual_clips() {
                // An image or a text is one picture, and a frame animation
                // is read frame by frame as it is shown; nothing is
                // rendered ahead for them.
                if media.is_still() || media.animation().is_some() || media.is_effects_layer() {
                    continue;
                }
                // A clip that cannot be rendered at all (for example a
                // bundled preset this installation does not have) gets no
                // job; the preview says why further down.
                let Ok(source) = frame_source(
                    media,
                    clip,
                    self.bundled_presets.as_deref(),
                    catalog,
                    self.ffmpeg.as_deref(),
                ) else {
                    continue;
                };
                let clip_seconds = clip.duration_milliseconds as f64 / 1000.0;
                let (beats, tempo) = beat_times(project, audio_store, clip.audio_input);
                // Rendered at the largest size the clip reaches.
                let (largest_width, largest_height) = clip.motion().largest_size();
                let largest = Placement {
                    width: largest_width,
                    height: largest_height,
                    ..Placement::FULL
                };
                let (width, height) = render_size(settings.width, settings.height, largest);
                wanted.push((
                    clip.id,
                    PrerenderJob {
                        source,
                        width,
                        height,
                        frames_per_second,
                        frame_count: (clip_seconds * f64::from(frames_per_second)).ceil() as u32,
                        start_seconds: clip.start_milliseconds as f64 / 1000.0,
                        // Each visualizer hears only the audio it was told
                        // to react to: the mix, or one audio track.
                        audio_sources: audio_sources(project, audio_store, clip.audio_input).0,
                        audio_gain: clip.audio_gain as f32,
                        beats: Arc::new(beats),
                        tempo_bpm: tempo.unwrap_or(0.0),
                    },
                ));
            }
            self.start_and_drop_renders(ctx, wanted);
        }

        // What to show at the playhead.
        // A clip does not include its own end, so with the playhead parked
        // exactly at the end of the project nothing would be shown. Look up
        // the last moment of the project instead, to keep its final frame.
        let last_millisecond = settings.duration_milliseconds.saturating_sub(1);
        let playhead_seconds = session.playhead_seconds();
        let lookup_milliseconds = ((playhead_seconds * 1000.0) as u64).min(last_millisecond);
        // A clip the user is dragging right now is drawn where the mouse
        // has taken it.
        let dragged: Option<(ClipId, Placement, f64)> =
            ctx.data(|data| data.get_temp(egui::Id::new(DRAGGED_PLACEMENT_ID)));

        self.layers.clear();
        // Where each layer goes, in the same order as `self.layers`.
        let mut geometries = Vec::new();
        // The project lists layers from the top down; drawing goes from the
        // bottom up, so that higher layers cover lower ones.
        for (track, clip, media) in project
            .visual_layers_at(lookup_milliseconds)
            .into_iter()
            .rev()
        {
            let state = if media.is_effects_layer() {
                // Drawn from the layers below it, which are already there.
                LayerState::Shown
            } else if media.image().is_some() {
                self.show_image(clip.id, &media.path, image_store)
            } else if let Some(animation) = media.animation() {
                let seconds_into_clip = playhead_seconds - clip.start_milliseconds as f64 / 1000.0;
                let (beats, _) = beat_times(project, audio_store, clip.audio_input);
                let frame =
                    animation.frame_at(seconds_into_clip, beat_count(&beats, playhead_seconds));
                self.show_animation_frame(clip.id, &media.path, animation, frame)
            } else if let Some(text) = media.text() {
                self.show_text(clip.id, text, settings.height)
            } else {
                let problem = frame_source(
                    media,
                    clip,
                    self.bundled_presets.as_deref(),
                    catalog,
                    self.ffmpeg.as_deref(),
                )
                .err();
                if let Some(problem) = problem {
                    LayerState::Failed(problem)
                } else if !is_audio_ready {
                    LayerState::WaitingForAudio
                } else {
                    let seconds_into_clip =
                        playhead_seconds - clip.start_milliseconds as f64 / 1000.0;
                    let frame = (seconds_into_clip.max(0.0) * f64::from(frames_per_second)) as u32;
                    self.show_frame(clip.id, frame)
                }
            };

            let effect = if clip.reaction.is_active() {
                let (sources, _) = audio_sources(project, audio_store, clip.audio_input);
                let (beats, _) = beat_times(project, audio_store, clip.audio_input);
                self.meter.effect(
                    &sources,
                    &beats,
                    &clip.reaction,
                    clip.audio_gain,
                    playhead_seconds,
                    f64::from(settings.width) / f64::from(settings.height.max(1)),
                )
            } else {
                ReactionEffect::NONE
            };
            // The clip's state at the playhead: keyframes and spin.
            let seconds_into_clip = playhead_seconds - clip.start_milliseconds as f64 / 1000.0;
            let mut transform = clip.motion().animated_at(seconds_into_clip.max(0.0));
            let keyed = clip.transform_at(seconds_into_clip.max(0.0) * 1000.0);
            if let Some((dragged_clip, placement, rotation)) = dragged
                && dragged_clip == clip.id
            {
                transform.placement = placement;
                // The dragged rotation replaces the keyed one; spin stays.
                transform.rotation_degrees += rotation - keyed.rotation_degrees;
            }
            geometries.push(if media.is_effects_layer() {
                vizx_core::effects_layer_geometry(&transform, effect, &clip.reaction)
            } else {
                layer_geometry(&transform, effect)
            });
            self.layers.push(LayerView {
                clip_id: clip.id,
                track_id: track.id,
                // The frame and handles in the preview show the clip where
                // it is at the playhead.
                placement: keyed.placement,
                rotation_degrees: keyed.rotation_degrees,
                sharp_pixels: project.clip_sharp_pixels(clip),
                blend: vizx_core::layer_blend(clip.blend_mode),
                effects: vizx_core::layer_effects(
                    &clip.effects,
                    &clip.reaction,
                    effect,
                    playhead_seconds,
                ),
                shows_below: media.is_effects_layer(),
                state,
            });
        }

        self.compose(ctx, settings.width, settings.height, &geometries);
    }

    /// Lays the layers that can be shown over each other and hands the
    /// result to egui as one texture.
    fn compose(
        &mut self,
        ctx: &egui::Context,
        composition_width: u32,
        composition_height: u32,
        geometries: &[LayerGeometry],
    ) {
        // As large as the panel shows it, in the composition's shape.
        let shown_width: f32 = ctx
            .data(|data| data.get_temp(egui::Id::new(SHOWN_WIDTH_ID)))
            .unwrap_or(PREVIEW_WIDTH as f32);
        let width = even_edge(f64::from(shown_width)).min(MAX_COMPOSITION_WIDTH);
        let height = even_edge(
            f64::from(width) * f64::from(composition_height) / f64::from(composition_width.max(1)),
        );

        let compositor = match self
            .compositor
            .get_or_insert_with(|| Compositor::new(width, height))
        {
            Ok(compositor) => compositor,
            Err(problem) => {
                self.composition_error = Some(problem.clone());
                return;
            }
        };
        compositor.resize(width, height);

        let draws: Vec<LayerDraw<'_>> = self
            .layers
            .iter()
            .zip(geometries)
            .filter(|(layer, _)| matches!(layer.state, LayerState::Shown))
            .filter_map(|(layer, geometry)| {
                let (version, width, height, pixels) = if layer.shows_below {
                    (0, 0, 0, &[][..])
                } else {
                    let picture = self.pictures.get(&layer.clip_id)?;
                    (
                        picture.version,
                        picture.width,
                        picture.height,
                        picture.bytes(),
                    )
                };
                Some(LayerDraw {
                    key: clip_key(layer.clip_id),
                    version,
                    width,
                    height,
                    pixels,
                    center: geometry.center,
                    size: geometry.size,
                    rotation_degrees: geometry.rotation_degrees,
                    opacity: geometry.opacity,
                    sharp_pixels: layer.sharp_pixels,
                    blend: layer.blend,
                    effects: layer.effects,
                    shows_below: layer.shows_below,
                })
            })
            .collect();

        match compositor.render(&draws, &mut self.frame_pixels) {
            Ok(()) => {
                let image = egui::ColorImage::from_rgba_premultiplied(
                    [width as usize, height as usize],
                    &self.frame_pixels,
                );
                match &mut self.composition {
                    Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                    None => {
                        self.composition = Some(ctx.load_texture(
                            "vizx_composition",
                            image,
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                }
                self.composition_error = None;
            }
            Err(problem) => self.composition_error = Some(problem),
        }
    }

    /// Forgets renders that no longer match the project and starts the
    /// missing ones.
    fn start_and_drop_renders(&mut self, ctx: &egui::Context, wanted: Vec<(ClipId, PrerenderJob)>) {
        // A render no longer matches when its clip is gone, or its source,
        // size, length or audio changed. Dropping it also stops its thread.
        self.prerenders.retain(|clip_id, prerender| {
            wanted
                .iter()
                .any(|(wanted_id, job)| wanted_id == clip_id && job.key() == prerender.key())
        });

        // Note since when each missing render has been wanted unchanged.
        let now = Instant::now();
        let mut still_waiting = HashMap::new();
        for (clip_id, job) in &wanted {
            if self.prerenders.contains_key(clip_id) {
                continue;
            }
            let key = job.key();
            let since = match self.waiting_since.get(clip_id) {
                Some((waiting_key, since)) if *waiting_key == key => *since,
                _ => now,
            };
            still_waiting.insert(*clip_id, (key, since));
        }
        self.waiting_since = still_waiting;
        if !self.waiting_since.is_empty() {
            // Look again when the delay is over, even without user input.
            ctx.request_repaint_after(RENDER_START_DELAY);
        }

        // Start one render at a time: each one uses the graphics card and a
        // processor core.
        let is_rendering = self
            .prerenders
            .values()
            .any(|prerender| !prerender.is_finished());
        if is_rendering {
            return;
        }
        let ready_to_start = wanted.into_iter().find(|(clip_id, _)| {
            self.waiting_since
                .get(clip_id)
                .is_some_and(|(_, since)| now.duration_since(*since) >= RENDER_START_DELAY)
        });
        if let Some((clip_id, job)) = ready_to_start {
            let ctx = ctx.clone();
            // Called on the render thread as frames get finished, so the
            // window redraws and shows progress and new frames.
            let prerender = Prerender::start(job, move || ctx.request_repaint());
            self.prerenders.insert(clip_id, prerender);
            self.waiting_since.remove(&clip_id);
        }
    }

    /// Makes sure the picture of a clip is the given pre-rendered frame,
    /// and says whether the clip can be shown.
    fn show_frame(&mut self, clip_id: ClipId, frame: u32) -> LayerState {
        let Some(prerender) = self.prerenders.get(&clip_id) else {
            // Queued behind another clip's render, or about to start.
            return LayerState::Rendering {
                finished: 0,
                total: 0,
            };
        };
        if let Some(error) = prerender.error() {
            return LayerState::Failed(error);
        }

        let frame_index = frame.min(prerender.frame_count().saturating_sub(1));
        if let Some(picture) = self.pictures.get(&clip_id)
            && picture.frame_index == frame_index
            && picture.render_key == prerender.key()
        {
            return LayerState::Shown;
        }
        let Some(frame) = prerender.frame(frame_index) else {
            return LayerState::Rendering {
                finished: prerender.finished_frame_count(),
                total: prerender.frame_count(),
            };
        };

        let render_key = prerender.key().to_owned();
        self.last_picture_version += 1;
        self.pictures.insert(
            clip_id,
            LayerPicture {
                width: frame.width,
                height: frame.height,
                pixels: Pixels::Frame(frame.pixels),
                frame_index,
                render_key,
                version: self.last_picture_version,
            },
        );
        LayerState::Shown
    }

    /// Makes sure the picture of an image clip is its file, and says
    /// whether the clip can be shown.
    fn show_image(&mut self, clip_id: ClipId, path: &str, image_store: &ImageStore) -> LayerState {
        let key = format!("image:{path}");
        if self
            .pictures
            .get(&clip_id)
            .is_some_and(|picture| picture.render_key == key)
        {
            return LayerState::Shown;
        }
        let image = match image_store.state(path) {
            Some(ImageState::Ready(image)) => Arc::clone(image),
            Some(ImageState::Failed(problem)) => {
                return LayerState::Failed(format!("Could not read {path}: {problem}"));
            }
            Some(ImageState::Loading) | None => return LayerState::LoadingImage,
        };

        self.last_picture_version += 1;
        self.pictures.insert(
            clip_id,
            LayerPicture {
                width: image.width,
                height: image.height,
                pixels: Pixels::Image(image),
                frame_index: 0,
                render_key: key,
                version: self.last_picture_version,
            },
        );
        LayerState::Shown
    }

    /// Makes sure the picture of a frame animation clip is frame `frame`,
    /// and says whether the clip can be shown.
    fn show_animation_frame(
        &mut self,
        clip_id: ClipId,
        path: &str,
        animation: &FrameAnimation,
        frame: u32,
    ) -> LayerState {
        let key = format!("animation:{path}:{animation:?}:{frame}");
        if self
            .pictures
            .get(&clip_id)
            .is_some_and(|picture| picture.render_key == key)
        {
            return LayerState::Shown;
        }
        let is_current = self
            .animations
            .get(&clip_id)
            .is_some_and(|reader| reader.reads(path, animation));
        if !is_current {
            self.animations.insert(
                clip_id,
                AnimationReader::new(path, animation, ANIMATION_EDGE_LIMIT, ANIMATION_BYTE_BUDGET),
            );
        }
        let Some(reader) = self.animations.get_mut(&clip_id) else {
            return LayerState::Failed("the animation could not be opened".to_owned());
        };
        let image = match reader.frame(frame) {
            Ok(image) => image,
            Err(problem) => return LayerState::Failed(problem),
        };

        self.last_picture_version += 1;
        self.pictures.insert(
            clip_id,
            LayerPicture {
                width: image.width,
                height: image.height,
                pixels: Pixels::Image(image),
                frame_index: frame,
                render_key: key,
                version: self.last_picture_version,
            },
        );
        LayerState::Shown
    }

    /// Makes sure the picture of a text clip shows its current text, and
    /// says whether the clip can be shown.
    fn show_text(
        &mut self,
        clip_id: ClipId,
        text: &TextProperties,
        frame_height: u32,
    ) -> LayerState {
        // Everything that decides how the text looks.
        let key = format!("text:{text:?}@{frame_height}");
        if self
            .pictures
            .get(&clip_id)
            .is_some_and(|picture| picture.render_key == key)
        {
            return LayerState::Shown;
        }
        // Drawing a text takes milliseconds, so it is done right here
        // whenever the text changes.
        let image = match vizx_core::render_text(text, frame_height) {
            Ok(image) => image,
            Err(problem) => return LayerState::Failed(problem),
        };

        self.last_picture_version += 1;
        self.pictures.insert(
            clip_id,
            LayerPicture {
                width: image.width,
                height: image.height,
                pixels: Pixels::Image(Arc::new(image)),
                frame_index: 0,
                render_key: key,
                version: self.last_picture_version,
            },
        );
        LayerState::Shown
    }

    /// The composed picture, once there is one.
    pub fn composition(&self) -> Option<&egui::TextureHandle> {
        self.composition.as_ref()
    }

    /// Why the picture could not be composed, if it could not.
    pub fn composition_error(&self) -> Option<&str> {
        self.composition_error.as_deref()
    }

    /// The visual clips at the playhead, the bottom layer first.
    pub fn layers(&self) -> &[LayerView] {
        &self.layers
    }

    /// Overall pre-render progress for the status bar: finished and total
    /// frames of all clips, and the disk space the stored frames use. `None`
    /// when the project has nothing to pre-render.
    pub fn progress(&self) -> Option<PrerenderProgress> {
        if self.prerenders.is_empty() {
            return None;
        }
        Some(PrerenderProgress {
            finished_frames: self
                .prerenders
                .values()
                .map(Prerender::finished_frame_count)
                .sum(),
            total_frames: self.prerenders.values().map(Prerender::frame_count).sum(),
            stored_bytes: self.prerenders.values().map(Prerender::stored_bytes).sum(),
        })
    }
}

pub struct PrerenderProgress {
    pub finished_frames: u32,
    pub total_frames: u32,
    pub stored_bytes: u64,
}

/// A number that identifies a clip's picture for the compositor.
fn clip_key(clip_id: ClipId) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    clip_id.hash(&mut hasher);
    hasher.finish()
}

/// Size in pixels at which a clip is pre-rendered for the preview: its
/// share of a [`PREVIEW_WIDTH`] wide picture of the composition. The
/// picture then has the clip's own shape, so a preset is rendered for that
/// shape instead of being squeezed into it.
fn render_size(
    composition_width: u32,
    composition_height: u32,
    placement: Placement,
) -> (u32, u32) {
    let full_height = preview_height(composition_width, composition_height);
    (
        even_edge(f64::from(PREVIEW_WIDTH) * placement.width),
        even_edge(f64::from(full_height) * placement.height),
    )
}

/// An edge length within the allowed range, rounded to an even number (some
/// graphics drivers dislike odd sizes).
fn even_edge(pixels: f64) -> u32 {
    let (smallest, largest) = RENDER_EDGE_RANGE;
    let rounded = if pixels.is_finite() {
        pixels
            .round()
            .clamp(f64::from(smallest), f64::from(largest)) as u32
    } else {
        smallest
    };
    (rounded / 2) * 2
}

/// Height matching the composition's aspect ratio at [`PREVIEW_WIDTH`],
/// rounded to an even number.
fn preview_height(composition_width: u32, composition_height: u32) -> u32 {
    let exact = u64::from(PREVIEW_WIDTH) * u64::from(composition_height)
        / u64::from(composition_width.max(1));
    ((exact as u32).max(2) / 2) * 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_height_keeps_the_aspect_ratio_and_is_even() {
        assert_eq!(preview_height(1920, 1080), 360);
        assert_eq!(preview_height(1080, 1920), 1136); // 1137.7 rounded to even
        assert_eq!(preview_height(1000, 1000), 640);
        assert_eq!(preview_height(1920, 1), 2);
        assert_eq!(preview_height(0, 1080) % 2, 0);
    }

    #[test]
    fn a_clip_is_rendered_at_its_share_of_the_preview_picture() {
        assert_eq!(render_size(1920, 1080, Placement::FULL), (640, 360));

        let left_half = Placement {
            x: 0.0,
            y: 0.0,
            width: 0.5,
            height: 1.0,
        };
        assert_eq!(render_size(1920, 1080, left_half), (320, 360));

        // Moving a clip does not change its size, so nothing is re-rendered.
        let moved = Placement {
            x: 0.4,
            y: -0.2,
            ..left_half
        };
        assert_eq!(render_size(1920, 1080, moved), (320, 360));

        // Sizes stay even and within the allowed range.
        let tiny = Placement {
            width: 0.001,
            height: 0.001,
            ..Placement::FULL
        };
        assert_eq!(render_size(1920, 1080, tiny), (16, 16));
        let huge = Placement {
            width: 100.0,
            height: 100.0,
            ..Placement::FULL
        };
        assert_eq!(render_size(1920, 1080, huge), (1920, 1920));
    }

    #[test]
    fn clips_get_different_stable_keys() {
        let mut project = vizx_project::Project::with_starter_tracks();
        let (_, first_layer) = project.add_milkdrop_preset("a.milk");
        let (_, second_layer) = project.add_milkdrop_preset("b.milk");
        let first = project.track(first_layer).unwrap().clips[0].id;
        let second = project.track(second_layer).unwrap().clips[0].id;

        assert_eq!(clip_key(first), clip_key(first));
        assert_ne!(clip_key(first), clip_key(second));
    }
}
