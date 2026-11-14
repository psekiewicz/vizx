//! Shows the MilkDrop visualizer in the preview, from pre-rendered frames.
//!
//! Each MilkDrop clip on the timeline is rendered ahead of time in the
//! background (see `MilkdropPrerender`). The preview then only looks up the
//! frame that belongs to the playhead, so scrubbing and seeking show the
//! right picture and playback is cheap.

use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui;
use vizx_core::{AudioStore, EditorSession, MilkdropPrerender, PrerenderJob, resolve_preset_path};
use vizx_project::ClipId;

use crate::transport::{audio_sources_for, project_audio_sources};

/// Width of the preview picture in pixels. The height follows from the
/// project's aspect ratio. Kept small so that rendering and storing every
/// frame of a song stays affordable on modest hardware; export will render
/// at full size.
const PREVIEW_WIDTH: u32 = 640;

/// What the preview panel should draw for the visualizer right now.
pub enum MilkdropView<'a> {
    /// No MilkDrop clip is on the timeline at the playhead.
    NotActive,
    /// The frame for the playhead.
    Picture(&'a egui::TextureHandle),
    /// The clip's frames are still being rendered and the playhead is ahead
    /// of them.
    Rendering { finished: u32, total: u32 },
    /// Rendering cannot start until the project's audio has been loaded.
    WaitingForAudio,
    /// The clip cannot be rendered; the text says why.
    Failed(String),
}

/// What `update` decided to show; turned into a [`MilkdropView`] by `view`.
#[derive(Default)]
enum Shown {
    #[default]
    NotActive,
    Picture,
    Rendering {
        finished: u32,
        total: u32,
    },
    WaitingForAudio,
    Failed(String),
}

pub struct MilkdropPreview {
    /// Folder of the presets that ship with VizX, if this installation has
    /// one. Projects refer to those presets by their place in this folder.
    bundled_presets: Option<PathBuf>,
    /// Pre-rendered frames of every MilkDrop clip in the project.
    prerenders: HashMap<ClipId, MilkdropPrerender>,
    /// The frame currently uploaded to the graphics card.
    texture: Option<egui::TextureHandle>,
    /// Which clip and frame `texture` shows, and for which render (its key),
    /// so the same frame is not decoded and uploaded again every frame.
    texture_content: Option<(ClipId, u32, String)>,
    shown: Shown,
}

impl MilkdropPreview {
    pub fn new(bundled_presets: Option<PathBuf>) -> Self {
        Self {
            bundled_presets,
            prerenders: HashMap::new(),
            texture: None,
            texture_content: None,
            shown: Shown::default(),
        }
    }

    /// Call once per frame, before the UI is drawn.
    pub fn update(
        &mut self,
        ctx: &egui::Context,
        session: &EditorSession,
        audio_store: &AudioStore,
    ) {
        let project = session.project();
        let settings = &project.settings;
        let frames_per_second = settings.frame_rate.as_f64().round().max(1.0) as u32;
        // Rendering waits until all audio is loaded, not only the audio a
        // clip reacts to, so that no clip is rendered twice.
        let (_all_sources, clips_still_loading) = project_audio_sources(session, audio_store);
        let is_audio_ready = clips_still_loading == 0;

        // What should be rendered: one job per MilkDrop clip.
        let mut wanted: Vec<(ClipId, PrerenderJob)> = Vec::new();
        if is_audio_ready {
            for (clip, preset) in project.milkdrop_clips() {
                // A bundled preset this installation does not have cannot
                // be rendered; the preview says so further down.
                let Some(preset_file) =
                    resolve_preset_path(&preset.path, self.bundled_presets.as_deref())
                else {
                    continue;
                };
                let clip_seconds = clip.duration_milliseconds as f64 / 1000.0;
                wanted.push((
                    clip.id,
                    PrerenderJob {
                        preset_path: preset_file.to_string_lossy().into_owned(),
                        width: PREVIEW_WIDTH,
                        height: preview_height(settings.width, settings.height),
                        frames_per_second,
                        frame_count: (clip_seconds * f64::from(frames_per_second)).ceil() as u32,
                        start_seconds: clip.start_milliseconds as f64 / 1000.0,
                        // Each visualizer hears only the audio it was told
                        // to react to: the mix, or one audio track.
                        audio_sources: audio_sources_for(session, audio_store, clip.audio_input).0,
                    },
                ));
            }

            // Forget renders that no longer match the project: the clip is
            // gone, or its preset, length or audio changed. Dropping a
            // render also stops its thread.
            self.prerenders.retain(|clip_id, prerender| {
                wanted
                    .iter()
                    .any(|(wanted_id, job)| wanted_id == clip_id && job.key() == prerender.key())
            });

            // Start missing renders, one at a time: each one uses the
            // graphics card and a processor core.
            let is_rendering = self
                .prerenders
                .values()
                .any(|prerender| !prerender.is_finished());
            if !is_rendering
                && let Some((clip_id, job)) = wanted
                    .into_iter()
                    .find(|(clip_id, _)| !self.prerenders.contains_key(clip_id))
            {
                let ctx = ctx.clone();
                // Called on the render thread as frames get finished, so
                // the window redraws and shows progress and new frames.
                let prerender = MilkdropPrerender::start(job, move || ctx.request_repaint());
                self.prerenders.insert(clip_id, prerender);
            }
        }

        // What to show at the playhead.
        // A clip does not include its own end, so with the playhead parked
        // exactly at the end of the project nothing would be shown. Look up
        // the last moment of the project instead, to keep its final frame.
        let last_millisecond = settings.duration_milliseconds.saturating_sub(1);
        let playhead_seconds = session.playhead_seconds();
        let lookup_milliseconds = ((playhead_seconds * 1000.0) as u64).min(last_millisecond);
        let Some((clip, preset)) = project.milkdrop_clip_at(lookup_milliseconds) else {
            self.shown = Shown::NotActive;
            return;
        };
        if resolve_preset_path(&preset.path, self.bundled_presets.as_deref()).is_none() {
            self.shown = Shown::Failed(format!(
                "{} ships with VizX, but the bundled presets were not found on this computer.",
                preset.file_name()
            ));
            return;
        }
        let Some(prerender) = self.prerenders.get(&clip.id) else {
            self.shown = if is_audio_ready {
                // Queued behind another clip's render.
                Shown::Rendering {
                    finished: 0,
                    total: 0,
                }
            } else {
                Shown::WaitingForAudio
            };
            return;
        };
        if let Some(error) = prerender.error() {
            self.shown = Shown::Failed(error);
            return;
        }

        let seconds_into_clip = playhead_seconds - clip.start_milliseconds as f64 / 1000.0;
        let frame_index = ((seconds_into_clip * f64::from(frames_per_second)) as u32)
            .min(prerender.frame_count().saturating_sub(1));

        let content = (clip.id, frame_index, prerender.key().to_owned());
        if self.texture_content.as_ref() == Some(&content) {
            self.shown = Shown::Picture;
            return;
        }
        let Some(frame) = prerender.frame(frame_index) else {
            self.shown = Shown::Rendering {
                finished: prerender.finished_frame_count(),
                total: prerender.frame_count(),
            };
            return;
        };

        let image = egui::ColorImage::from_rgba_unmultiplied(
            [frame.width as usize, frame.height as usize],
            &frame.pixels,
        );
        match &mut self.texture {
            Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
            None => {
                self.texture =
                    Some(ctx.load_texture("milkdrop_preview", image, egui::TextureOptions::LINEAR));
            }
        }
        self.texture_content = Some(content);
        self.shown = Shown::Picture;
    }

    /// What the preview panel should show.
    pub fn view(&self) -> MilkdropView<'_> {
        match (&self.shown, &self.texture) {
            (Shown::Picture, Some(texture)) => MilkdropView::Picture(texture),
            (Shown::Picture, None) | (Shown::NotActive, _) => MilkdropView::NotActive,
            (Shown::Rendering { finished, total }, _) => MilkdropView::Rendering {
                finished: *finished,
                total: *total,
            },
            (Shown::WaitingForAudio, _) => MilkdropView::WaitingForAudio,
            (Shown::Failed(reason), _) => MilkdropView::Failed(reason.clone()),
        }
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
                .map(MilkdropPrerender::finished_frame_count)
                .sum(),
            total_frames: self
                .prerenders
                .values()
                .map(MilkdropPrerender::frame_count)
                .sum(),
            stored_bytes: self
                .prerenders
                .values()
                .map(MilkdropPrerender::stored_bytes)
                .sum(),
        })
    }
}

pub struct PrerenderProgress {
    pub finished_frames: u32,
    pub total_frames: u32,
    pub stored_bytes: u64,
}

/// Height matching the composition's aspect ratio at [`PREVIEW_WIDTH`],
/// rounded to an even number (some graphics drivers dislike odd sizes).
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
}
