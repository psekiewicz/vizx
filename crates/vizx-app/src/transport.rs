//! Playback control: connects the playhead in the editing session with the
//! audio player.

use eframe::egui;
use vizx_audio::{AudioPlayer, PlaybackError, PlaybackSource};
use vizx_core::{AudioStore, EditorSession};
use vizx_project::AudioInput;

/// Everything on the project's audio tracks that can be heard right now:
/// every audio clip whose file has finished loading, placed at its position
/// on the timeline. Also returns how many clips are not loaded yet.
///
/// Playback, the visualizers and the export all take their audio from the
/// same function (`vizx_core::audio_sources`), so they hear the same thing.
pub fn project_audio_sources(
    session: &EditorSession,
    audio_store: &AudioStore,
) -> (Vec<PlaybackSource>, usize) {
    vizx_core::audio_sources(session.project(), audio_store, AudioInput::Mix)
}
/// Starts and stops playback and keeps the playhead in step with the sound.
///
/// While playing, the audio player is the clock: the playhead is set from
/// the amount of audio actually handed to the sound card, so picture and
/// sound cannot drift apart. While stopped, the playhead simply stays where
/// the user put it.
pub struct Transport {
    /// Created the first time playback starts, so VizX does not touch the
    /// sound card until it is needed.
    player: Option<AudioPlayer>,
    is_playing: bool,
    /// Where the playhead was after this frame's update from the player.
    /// Used to notice when the UI moved it during the frame.
    playhead_at_frame_start: f64,
    /// What the player was last given, to notice when the audio changes
    /// during playback (a file finished loading, a clip was moved or muted).
    sources_signature: SourcesSignature,
    /// Loudness of playback, 0 to 1.
    volume: f32,
}

/// Every source's audio (by address), placement and length, and the end of
/// the project: if this changes, the player needs the new sources.
type SourcesSignature = (Vec<(usize, u64, u64, Option<u64>)>, u64);

fn signature(sources: &[PlaybackSource], end_seconds: f64) -> SourcesSignature {
    let placed = sources
        .iter()
        .map(|source| {
            (
                std::sync::Arc::as_ptr(&source.audio) as usize,
                source.start_seconds.to_bits(),
                source.offset_seconds.to_bits(),
                source.duration_seconds.map(f64::to_bits),
            )
        })
        .collect();
    (placed, end_seconds.to_bits())
}

impl Default for Transport {
    fn default() -> Self {
        Self {
            player: None,
            is_playing: false,
            playhead_at_frame_start: 0.0,
            sources_signature: (Vec::new(), 0),
            volume: 1.0,
        }
    }
}

impl Transport {
    pub fn is_playing(&self) -> bool {
        self.is_playing
    }

    /// The project time of what is being heard right now: while playing,
    /// slightly behind the playhead (the sound card's delay); otherwise the
    /// playhead. Taps along with the music are placed here.
    pub fn heard_seconds(&self, session: &EditorSession) -> f64 {
        let playhead = session.playhead_seconds();
        match self.player.as_ref().filter(|_| self.is_playing) {
            Some(player) => (playhead - player.heard_delay_seconds()).max(0.0),
            None => playhead,
        }
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// Sets how loud playback is, 0 to 1. It does not change the export.
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        if let Some(player) = &self.player {
            player.set_volume(self.volume);
        }
    }

    /// Call at the start of every frame, before the UI is drawn.
    pub fn begin_frame(
        &mut self,
        session: &mut EditorSession,
        audio_store: &AudioStore,
        ctx: &egui::Context,
    ) {
        if let Some(player) = self.player.as_ref().filter(|_| self.is_playing) {
            // The audio changed while playing: hand over the new sources
            // right away instead of waiting for the next Play.
            let (sources, _) = project_audio_sources(session, audio_store);
            let project_end = session.project().settings.duration_seconds();
            let current = signature(&sources, project_end);
            if current != self.sources_signature {
                player.set_sources(sources);
                player.set_end_seconds(project_end);
                self.sources_signature = current;
            }

            session.set_playhead_seconds(player.position_seconds());

            // Looping: past the loop's end (or at the end of the project),
            // back to its start.
            let loop_region = session.loop_region().filter(|_| session.is_looping());
            if let Some((start, end)) = loop_region
                && (player.position_seconds() >= end || !player.is_playing())
            {
                player.seek(start);
                session.set_playhead_seconds(start);
                if !player.is_playing() {
                    let _ = player.play();
                }
            }

            if player.is_playing() {
                // Keep redrawing so the playhead moves smoothly, even when
                // the mouse is still.
                ctx.request_repaint();
            } else {
                // The player reached the end of the project by itself.
                player.pause();
                self.is_playing = false;
            }
        }
        self.playhead_at_frame_start = session.playhead_seconds();
    }

    /// Call at the end of every frame. If the UI moved the playhead during
    /// the frame (a click on the ruler, "go to start", ...), the player is
    /// told to continue from the new position.
    pub fn end_frame(&mut self, session: &EditorSession) {
        if session.playhead_seconds() != self.playhead_at_frame_start
            && let Some(player) = &self.player
        {
            player.seek(session.playhead_seconds());
        }
    }

    /// Starts playback from the playhead. Returns a note for the status bar.
    pub fn play(
        &mut self,
        session: &mut EditorSession,
        audio_store: &AudioStore,
    ) -> Result<String, PlaybackError> {
        let player = match self.player.take() {
            Some(player) => player,
            None => AudioPlayer::new()?,
        };

        let (sources, clips_still_loading) = project_audio_sources(session, audio_store);

        // Pressing play at the very end starts over from the beginning.
        let project_end = session.project().settings.duration_seconds();
        if session.playhead_seconds() >= project_end {
            session.set_playhead_seconds(0.0);
        }
        // While looping, playing from past the loop starts at the loop.
        if let Some((start, end)) = session.loop_region().filter(|_| session.is_looping())
            && session.playhead_seconds() >= end
        {
            session.set_playhead_seconds(start);
        }

        self.sources_signature = signature(&sources, project_end);
        player.set_sources(sources);
        player.set_end_seconds(project_end);
        player.set_volume(self.volume);
        player.seek(session.playhead_seconds());
        let result = player.play();

        let note = if clips_still_loading > 0 {
            format!("Playing - {clips_still_loading} audio file(s) still loading")
        } else {
            format!("Playing ({})", player.device_description())
        };
        self.player = Some(player);
        result?;
        self.is_playing = true;
        Ok(note)
    }

    /// Stops the sound and leaves the playhead where it is.
    pub fn pause(&mut self) {
        if let Some(player) = &self.player {
            player.pause();
        }
        self.is_playing = false;
    }
}
