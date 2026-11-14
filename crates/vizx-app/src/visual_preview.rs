//! Shows the visualizers (MilkDrop presets and visualizer plugins) in the
//! preview, from pre-rendered frames.
//!
//! Each visual clip on the timeline is rendered ahead of time in the
//! background (see `Prerender`). The preview then only looks up the
//! frames that belong to the playhead, so scrubbing and seeking show the
//! right picture and playback is cheap.
//!
//! Several clips can be visible at once: every visual layer with a clip at
//! the playhead contributes one picture, drawn into the clip's placement.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use vizx_core::{
    AudioStore, EditorSession, FrameSource, Prerender, PrerenderJob, VisualizerCatalog,
    resolve_preset_path,
};
use vizx_project::{Clip, ClipId, MediaSource, Placement, TrackId};

use crate::transport::{audio_sources_for, project_audio_sources};

/// Width in pixels of a preview picture that fills the whole frame. The
/// height follows from the project's aspect ratio, and a smaller clip is
/// rendered proportionally smaller. Kept small so that rendering and
/// storing every frame of a song stays affordable on modest hardware;
/// export will render at full size.
const PREVIEW_WIDTH: u32 = 640;
/// Smallest and largest edge of a rendered picture, in pixels.
const RENDER_EDGE_RANGE: (u32, u32) = (16, 1920);
/// A render starts only after its settings have stayed the same for this
/// long. Dragging a size value changes them many times per second, and
/// each change would otherwise start and throw away a render.
const RENDER_START_DELAY: Duration = Duration::from_millis(350);

/// One visualizer in the picture at the playhead.
pub struct LayerView<'a> {
    pub clip_id: ClipId,
    /// The visual layer the clip is on.
    pub track_id: TrackId,
    pub placement: Placement,
    pub content: LayerContent<'a>,
}

/// What to draw for one visualizer right now.
pub enum LayerContent<'a> {
    /// The frame for the playhead.
    Picture(&'a egui::TextureHandle),
    /// The clip's frames are still being rendered and the playhead is ahead
    /// of them. `total` is 0 while the clip waits for its turn.
    Rendering { finished: u32, total: u32 },
    /// Rendering cannot start until the project's audio has been loaded.
    WaitingForAudio,
    /// The clip cannot be rendered; the text says why.
    Failed(String),
}

/// What `update` decided to show for a layer; turned into a
/// [`LayerContent`] by `layers`.
enum Shown {
    Picture,
    Rendering { finished: u32, total: u32 },
    WaitingForAudio,
    Failed(String),
}

struct ShownLayer {
    clip_id: ClipId,
    track_id: TrackId,
    placement: Placement,
    shown: Shown,
}

/// The frame of one clip that is currently uploaded to the graphics card.
struct LayerTexture {
    handle: egui::TextureHandle,
    /// Which frame it shows, and for which render (its key), so the same
    /// frame is not decoded and uploaded again every frame.
    frame_index: u32,
    render_key: String,
}

pub struct VisualPreview {
    /// Folder of the presets that ship with VizX, if this installation has
    /// one. Projects refer to those presets by their place in this folder.
    bundled_presets: Option<PathBuf>,
    /// Pre-rendered frames of every MilkDrop clip in the project.
    prerenders: HashMap<ClipId, Prerender>,
    /// Renders that are wanted but not started yet: the key of the wanted
    /// job and since when it has been wanted unchanged.
    waiting_since: HashMap<ClipId, (String, Instant)>,
    textures: HashMap<ClipId, LayerTexture>,
    /// The visualizers at the playhead, bottom layer first.
    layers: Vec<ShownLayer>,
}

impl VisualPreview {
    pub fn new(bundled_presets: Option<PathBuf>) -> Self {
        Self {
            bundled_presets,
            prerenders: HashMap::new(),
            waiting_since: HashMap::new(),
            textures: HashMap::new(),
            layers: Vec::new(),
        }
    }

    /// Call once per frame, before the UI is drawn.
    pub fn update(
        &mut self,
        ctx: &egui::Context,
        session: &EditorSession,
        audio_store: &AudioStore,
        catalog: &mut VisualizerCatalog,
    ) {
        let project = session.project();
        let settings = &project.settings;
        let frames_per_second = settings.frame_rate.as_f64().round().max(1.0) as u32;
        // Rendering waits until all audio is loaded, not only the audio a
        // clip reacts to, so that no clip is rendered twice.
        let (_all_sources, clips_still_loading) = project_audio_sources(session, audio_store);
        let is_audio_ready = clips_still_loading == 0;

        if is_audio_ready {
            // What should be rendered: one job per visual clip.
            let mut wanted: Vec<(ClipId, PrerenderJob)> = Vec::new();
            for (clip, media) in project.visual_clips() {
                // A clip that cannot be rendered at all (for example a
                // bundled preset this installation does not have) gets no
                // job; the preview says why further down.
                let Ok(source) =
                    frame_source(media, clip, self.bundled_presets.as_deref(), catalog)
                else {
                    continue;
                };
                let clip_seconds = clip.duration_milliseconds as f64 / 1000.0;
                let (width, height) = render_size(settings.width, settings.height, clip.placement);
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
                        audio_sources: audio_sources_for(session, audio_store, clip.audio_input).0,
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

        self.layers.clear();
        // The project lists layers from the top down; drawing goes from the
        // bottom up, so that higher layers cover lower ones.
        for (track, clip, media) in project
            .visual_layers_at(lookup_milliseconds)
            .into_iter()
            .rev()
        {
            let problem = frame_source(media, clip, self.bundled_presets.as_deref(), catalog).err();
            let shown = if let Some(problem) = problem {
                Shown::Failed(problem)
            } else if !is_audio_ready {
                Shown::WaitingForAudio
            } else {
                let seconds_into_clip = playhead_seconds - clip.start_milliseconds as f64 / 1000.0;
                let frame = (seconds_into_clip.max(0.0) * f64::from(frames_per_second)) as u32;
                self.show_frame(ctx, clip.id, frame)
            };
            self.layers.push(ShownLayer {
                clip_id: clip.id,
                track_id: track.id,
                placement: clip.placement,
                shown,
            });
        }
    }

    /// Forgets renders that no longer match the project and starts the
    /// missing ones.
    fn start_and_drop_renders(&mut self, ctx: &egui::Context, wanted: Vec<(ClipId, PrerenderJob)>) {
        // A render no longer matches when its clip is gone, or its preset,
        // size, length or audio changed. Dropping it also stops its thread.
        self.prerenders.retain(|clip_id, prerender| {
            wanted
                .iter()
                .any(|(wanted_id, job)| wanted_id == clip_id && job.key() == prerender.key())
        });
        self.textures
            .retain(|clip_id, _| wanted.iter().any(|(wanted_id, _)| wanted_id == clip_id));

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

    /// Makes sure the texture of a clip shows the given frame, and says
    /// what can be drawn for the clip.
    fn show_frame(&mut self, ctx: &egui::Context, clip_id: ClipId, frame: u32) -> Shown {
        let Some(prerender) = self.prerenders.get(&clip_id) else {
            // Queued behind another clip's render, or about to start.
            return Shown::Rendering {
                finished: 0,
                total: 0,
            };
        };
        if let Some(error) = prerender.error() {
            return Shown::Failed(error);
        }

        let frame_index = frame.min(prerender.frame_count().saturating_sub(1));
        if let Some(texture) = self.textures.get(&clip_id)
            && texture.frame_index == frame_index
            && texture.render_key == prerender.key()
        {
            return Shown::Picture;
        }
        let Some(frame) = prerender.frame(frame_index) else {
            return Shown::Rendering {
                finished: prerender.finished_frame_count(),
                total: prerender.frame_count(),
            };
        };

        let image = egui::ColorImage::from_rgba_unmultiplied(
            [frame.width as usize, frame.height as usize],
            &frame.pixels,
        );
        let render_key = prerender.key().to_owned();
        match self.textures.get_mut(&clip_id) {
            Some(texture) => {
                texture.handle.set(image, egui::TextureOptions::LINEAR);
                texture.frame_index = frame_index;
                texture.render_key = render_key;
            }
            None => {
                let handle = ctx.load_texture(
                    format!("visual_preview_{clip_id:?}"),
                    image,
                    egui::TextureOptions::LINEAR,
                );
                self.textures.insert(
                    clip_id,
                    LayerTexture {
                        handle,
                        frame_index,
                        render_key,
                    },
                );
            }
        }
        Shown::Picture
    }

    /// The visualizers at the playhead, in drawing order: the bottom layer
    /// first, the top layer last.
    pub fn layers(&self) -> Vec<LayerView<'_>> {
        self.layers
            .iter()
            .map(|layer| {
                let content = match (&layer.shown, self.textures.get(&layer.clip_id)) {
                    (Shown::Picture, Some(texture)) => LayerContent::Picture(&texture.handle),
                    (Shown::Picture, None) => LayerContent::Rendering {
                        finished: 0,
                        total: 0,
                    },
                    (Shown::Rendering { finished, total }, _) => LayerContent::Rendering {
                        finished: *finished,
                        total: *total,
                    },
                    (Shown::WaitingForAudio, _) => LayerContent::WaitingForAudio,
                    (Shown::Failed(reason), _) => LayerContent::Failed(reason.clone()),
                };
                LayerView {
                    clip_id: layer.clip_id,
                    track_id: layer.track_id,
                    placement: layer.placement,
                    content,
                }
            })
            .collect()
    }

    /// Overall pre-render progress for the status bar: finished and total
    /// frames of all clips, and the disk space the stored frames use. `None`
    /// when the project has no MilkDrop clips.
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

/// What draws the frames of a clip, or why it cannot be drawn.
fn frame_source(
    media: &MediaSource,
    clip: &Clip,
    bundled_presets: Option<&Path>,
    catalog: &mut VisualizerCatalog,
) -> Result<FrameSource, String> {
    if media.is_visualizer_plugin() {
        let plugin = catalog.plugin(&media.path);
        // A broken `visualizer.json` is reported here; a broken shader is
        // reported by the render itself.
        plugin.manifest.as_ref().map_err(Clone::clone)?;
        let folder = plugin
            .folder
            .clone()
            .ok_or("The visualizer's folder was not found.")?;
        return Ok(FrameSource::Plugin {
            folder,
            parameters: clip.parameters.clone(),
            version: plugin.version,
        });
    }

    let preset_file = resolve_preset_path(&media.path, bundled_presets).ok_or_else(|| {
        format!(
            "{} ships with VizX, but the bundled presets were not found on this computer.",
            media.file_name()
        )
    })?;
    Ok(FrameSource::Milkdrop {
        preset_path: preset_file.to_string_lossy().into_owned(),
    })
}

pub struct PrerenderProgress {
    pub finished_frames: u32,
    pub total_frames: u32,
    pub stored_bytes: u64,
}

/// Size in pixels at which a clip is rendered for the preview: its share of
/// a [`PREVIEW_WIDTH`] wide picture of the composition. The picture then has
/// the clip's own shape, so a preset is rendered for that shape instead of
/// being squeezed into it.
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
}
