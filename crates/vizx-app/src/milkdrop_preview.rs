//! Shows the MilkDrop visualizer in the preview.
//!
//! This is a *live* preview: whenever the playhead moves, the render thread
//! is asked for a frame that reacts to the audio leading up to the playhead.
//! It follows playback in real time and updates while scrubbing; when the
//! playhead stands still, nothing is rendered.

use eframe::egui;
use vizx_core::{AudioStore, EditorSession, MilkdropFrameRequest, MilkdropWorker};

use crate::transport::project_audio_sources;

/// Width of the preview picture in pixels. The height follows from the
/// project's aspect ratio. Kept small so that rendering and copying each
/// frame stay cheap on modest hardware; export will render at full size.
const PREVIEW_WIDTH: u32 = 640;
/// Sample rate of the audio handed to the visualizer.
const ANALYSIS_SAMPLE_RATE: u32 = 44_100;
/// Audio given to a frame after a jump of the playhead (seek, first frame).
const FRAMES_AFTER_JUMP: u64 = 1024;
/// A playhead step longer than this (in seconds) counts as a jump rather
/// than as continuous playback.
const LONGEST_CONTINUOUS_STEP: f64 = 0.25;

/// What the preview panel should draw for the visualizer right now.
pub enum MilkdropView<'a> {
    /// No MilkDrop preset is on the timeline at the playhead.
    NotActive,
    /// A preset is active and this is its latest picture.
    Picture(&'a egui::TextureHandle),
    /// A preset is active but its first frame is not ready yet.
    Waiting,
    /// A preset is active but cannot be rendered; the text says why.
    Failed(String),
}

#[derive(Default)]
pub struct MilkdropPreview {
    /// Started the first time a preset needs to be drawn.
    worker: Option<MilkdropWorker>,
    /// The latest frame, uploaded to the graphics card for egui to draw.
    texture: Option<egui::TextureHandle>,
    /// Preset the current `texture` belongs to.
    shown_preset: Option<String>,
    /// Playhead position and preset of the last frame that was requested.
    last_request: Option<(f64, String)>,
    is_active: bool,
}

impl MilkdropPreview {
    /// Call once per frame, before the UI is drawn.
    pub fn update(
        &mut self,
        ctx: &egui::Context,
        session: &EditorSession,
        audio_store: &AudioStore,
    ) {
        let playhead_seconds = session.playhead_seconds();
        let playhead_milliseconds = (playhead_seconds * 1000.0) as u64;
        let Some(preset) = session.project().milkdrop_preset_at(playhead_milliseconds) else {
            self.is_active = false;
            return;
        };
        let preset_path = preset.path.clone();
        self.is_active = true;

        // A picture of a different preset must not be shown for this one.
        if self.shown_preset.as_deref() != Some(preset_path.as_str()) {
            self.texture = None;
        }

        let worker = self.worker.get_or_insert_with(|| {
            let ctx = ctx.clone();
            // Called on the render thread when a frame is ready, so the
            // window redraws and shows it.
            MilkdropWorker::spawn(move || ctx.request_repaint())
        });

        // Take over a finished frame.
        if let Some(frame) = worker.take_frame() {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [frame.width as usize, frame.height as usize],
                &frame.pixels,
            );
            match &mut self.texture {
                Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                None => {
                    self.texture = Some(ctx.load_texture(
                        "milkdrop_preview",
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
            self.shown_preset = Some(preset_path.clone());
        }

        // Request a new frame only when something changed.
        let previous = self.last_request.as_ref();
        let is_same_preset = previous.is_some_and(|(_, path)| *path == preset_path);
        let previous_seconds = previous.map(|(seconds, _)| *seconds);
        if is_same_preset && previous_seconds == Some(playhead_seconds) {
            return;
        }

        // The audio this frame reacts to: what was played since the last
        // frame, or a short stretch before the playhead after a jump.
        let playhead_frame = (playhead_seconds * f64::from(ANALYSIS_SAMPLE_RATE)) as u64;
        let step = previous_seconds.map_or(f64::INFINITY, |previous| playhead_seconds - previous);
        let frame_count = if is_same_preset && step > 0.0 && step <= LONGEST_CONTINUOUS_STEP {
            (step * f64::from(ANALYSIS_SAMPLE_RATE)) as u64
        } else {
            FRAMES_AFTER_JUMP
        }
        .clamp(1, playhead_frame.max(1));
        let (sources, _still_loading) = project_audio_sources(session, audio_store);
        let audio = vizx_audio::mix_to_stereo(
            &sources,
            ANALYSIS_SAMPLE_RATE,
            playhead_frame.saturating_sub(frame_count),
            frame_count as usize,
        );

        let settings = &session.project().settings;
        worker.request_frame(MilkdropFrameRequest {
            preset_path: preset_path.clone(),
            width: PREVIEW_WIDTH,
            height: preview_height(settings.width, settings.height),
            frames_per_second: settings.frame_rate.as_f64().round().max(1.0) as u32,
            audio,
        });
        self.last_request = Some((playhead_seconds, preset_path));
    }

    /// What the preview panel should show.
    pub fn view(&self) -> MilkdropView<'_> {
        if !self.is_active {
            return MilkdropView::NotActive;
        }
        if let Some(error) = self.worker.as_ref().and_then(MilkdropWorker::error) {
            return MilkdropView::Failed(error);
        }
        match &self.texture {
            Some(texture) => MilkdropView::Picture(texture),
            None => MilkdropView::Waiting,
        }
    }
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
