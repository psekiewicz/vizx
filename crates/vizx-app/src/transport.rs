//! Playback control: connects the playhead in the editing session with the
//! audio player.

use eframe::egui;
use vizx_audio::{AudioPlayer, PlaybackError, PlaybackSource};
use vizx_core::{AudioStore, EditorSession};
use vizx_project::{AudioInput, TrackKind};

/// Everything on the project's audio tracks that can be heard right now:
/// every audio clip whose file has finished loading, placed at its position
/// on the timeline. Also returns how many clips are not loaded yet.
///
/// Playback and the visualizer both use this, so the visualizer reacts to
/// exactly what is audible.
pub fn project_audio_sources(
    session: &EditorSession,
    audio_store: &AudioStore,
) -> (Vec<PlaybackSource>, usize) {
    audio_sources_for(session, audio_store, AudioInput::Mix)
}

/// The loaded audio clips that `input` selects: all audio tracks for the
/// mix, or the clips of one audio track. Also returns how many of the
/// selected clips are not loaded yet.
///
/// An input naming a track that no longer exists selects the mix, so a
/// visualizer never goes silent because of a stale reference.
pub fn audio_sources_for(
    session: &EditorSession,
    audio_store: &AudioStore,
    input: AudioInput,
) -> (Vec<PlaybackSource>, usize) {
    let only_track = match input {
        AudioInput::Track(id) if session.project().track(id).is_some() => Some(id),
        _ => None,
    };

    let mut sources = Vec::new();
    let mut clips_still_loading = 0;
    for track in session.project().tracks() {
        if track.kind != TrackKind::Audio || only_track.is_some_and(|id| id != track.id) {
            continue;
        }
        for clip in &track.clips {
            let loaded = session
                .project()
                .media_source(clip.media)
                .and_then(|source| audio_store.loaded(&source.path));
            match loaded {
                Some(loaded) => sources.push(PlaybackSource {
                    audio: loaded.audio.clone(),
                    start_seconds: clip.start_milliseconds as f64 / 1000.0,
                }),
                None => clips_still_loading += 1,
            }
        }
    }
    (sources, clips_still_loading)
}

/// Starts and stops playback and keeps the playhead in step with the sound.
///
/// While playing, the audio player is the clock: the playhead is set from
/// the amount of audio actually handed to the sound card, so picture and
/// sound cannot drift apart. While stopped, the playhead simply stays where
/// the user put it.
#[derive(Default)]
pub struct Transport {
    /// Created the first time playback starts, so VizX does not touch the
    /// sound card until it is needed.
    player: Option<AudioPlayer>,
    is_playing: bool,
    /// Where the playhead was after this frame's update from the player.
    /// Used to notice when the UI moved it during the frame.
    playhead_at_frame_start: f64,
}

impl Transport {
    pub fn is_playing(&self) -> bool {
        self.is_playing
    }

    /// Call at the start of every frame, before the UI is drawn.
    pub fn begin_frame(&mut self, session: &mut EditorSession, ctx: &egui::Context) {
        if let Some(player) = self.player.as_ref().filter(|_| self.is_playing) {
            session.set_playhead_seconds(player.position_seconds());

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

        player.set_sources(sources);
        player.set_end_seconds(project_end);
        player.seek(session.playhead_seconds());
        let result = player.play();

        let note = if clips_still_loading > 0 {
            format!("Playing ({clips_still_loading} audio file(s) not loaded yet)")
        } else {
            format!("Playing - output: {}", player.device_description())
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
