//! What the picture consists of at a moment of the project.
//!
//! The preview and the export both ask the same questions: which audio does
//! this clip listen to, how strongly does it react right now, where exactly
//! is it drawn, and what draws its frames. The answers live here, so the
//! two can never disagree.

use std::path::Path;

use vizx_audio::{ANALYSIS_WINDOW, AudioAnalyzer, Beat, PlaybackSource};
use vizx_project::{
    AudioInput, AudioReaction, Clip, MediaSource, Placement, Project, ReactionBand, ReactionEffect,
    TrackKind,
};

use crate::{AudioStore, FrameSource, VisualizerCatalog, resolve_preset_path};

/// Sample rate at which audio is analysed for clips that react to it.
const REACTION_SAMPLE_RATE: u32 = 44_100;
/// For a reaction's release, the sound is measured at earlier moments this
/// far apart, and at most this many of them.
const RELEASE_STEP_SECONDS: f64 = 1.0 / 30.0;
const MAX_RELEASE_STEPS: u32 = 15;

/// The loaded audio clips that `input` selects: all audio tracks for the
/// mix, or the clips of one audio track, each placed at its position on the
/// timeline. Also returns how many of the selected clips are not loaded
/// yet.
///
/// An input naming a track that no longer exists selects the mix, so a
/// visualizer never goes silent because of a stale reference.
pub fn audio_sources(
    project: &Project,
    audio_store: &AudioStore,
    input: AudioInput,
) -> (Vec<PlaybackSource>, usize) {
    let only_track = match input {
        AudioInput::Track(id) if project.track(id).is_some() => Some(id),
        _ => None,
    };

    let mut sources = Vec::new();
    let mut clips_still_loading = 0;
    for track in project.tracks() {
        if track.kind != TrackKind::Audio || only_track.is_some_and(|id| id != track.id) {
            continue;
        }
        for clip in &track.clips {
            let loaded = project
                .media_source(clip.media)
                .and_then(|source| audio_store.loaded(&source.path));
            match loaded {
                Some(loaded) => sources.push(PlaybackSource {
                    audio: loaded.audio.clone(),
                    start_seconds: clip.start_milliseconds as f64 / 1000.0,
                    // A trimmed clip plays only its part of the file.
                    offset_seconds: clip.source_offset_milliseconds as f64 / 1000.0,
                    duration_seconds: Some(clip.duration_milliseconds as f64 / 1000.0),
                }),
                None => clips_still_loading += 1,
            }
        }
    }
    (sources, clips_still_loading)
}

/// Two beats from different files closer than this are one beat.
const SAME_BEAT_SECONDS: f64 = 0.05;
/// A beat is felt for at least this long, even with no release time.
const SHORTEST_BEAT_SECONDS: f64 = 0.08;

/// The beats `input` selects, in project time and in order: the beats of
/// every loaded audio clip involved, where that clip plays them (a trimmed
/// clip only brings the beats of its own part). Also the tempo of the first
/// of those files that has one.
pub fn beat_times(
    project: &Project,
    audio_store: &AudioStore,
    input: AudioInput,
) -> (Vec<Beat>, Option<f64>) {
    let only_track = match input {
        AudioInput::Track(id) if project.track(id).is_some() => Some(id),
        _ => None,
    };

    let mut beats = Vec::new();
    let mut tempo = None;
    for track in project.tracks() {
        if track.kind != TrackKind::Audio || only_track.is_some_and(|id| id != track.id) {
            continue;
        }
        for clip in &track.clips {
            let Some(loaded) = project
                .media_source(clip.media)
                .and_then(|source| audio_store.loaded(&source.path))
            else {
                continue;
            };
            let start = clip.start_milliseconds as f64 / 1000.0;
            let offset = clip.source_offset_milliseconds as f64 / 1000.0;
            let duration = clip.duration_milliseconds as f64 / 1000.0;
            beats.extend(
                loaded
                    .beats
                    .beats
                    .iter()
                    .filter(|beat| beat.seconds >= offset && beat.seconds < offset + duration)
                    .map(|beat| Beat {
                        seconds: start + beat.seconds - offset,
                        strength: beat.strength,
                    }),
            );
            tempo = tempo.or(loaded.beats.tempo_bpm);
        }
    }

    beats.sort_by(|a, b| a.seconds.total_cmp(&b.seconds));
    // The same drum on two stems is one beat, as strong as the stronger.
    let mut merged: Vec<Beat> = Vec::with_capacity(beats.len());
    for beat in beats {
        match merged.last_mut() {
            Some(last) if beat.seconds - last.seconds < SAME_BEAT_SECONDS => {
                last.strength = last.strength.max(beat.strength);
            }
            _ => merged.push(beat),
        }
    }
    (merged, tempo)
}

/// How strongly the beats are felt at `seconds`: the last beat's strength
/// right on it, falling to 0 over `decay_seconds`.
pub fn beat_pulse(beats: &[Beat], seconds: f64, decay_seconds: f64) -> f32 {
    let Some(beat) = last_beat(beats, seconds) else {
        return 0.0;
    };
    let decay = decay_seconds.max(SHORTEST_BEAT_SECONDS);
    let since = seconds - beat.seconds;
    if since >= decay {
        0.0
    } else {
        beat.strength * (1.0 - since / decay) as f32
    }
}

/// The last beat at or before `seconds`.
pub fn last_beat(beats: &[Beat], seconds: f64) -> Option<Beat> {
    let count = beat_count(beats, seconds);
    count.checked_sub(1).map(|index| beats[index])
}

/// How many beats have happened up to `seconds` (inclusive).
pub fn beat_count(beats: &[Beat], seconds: f64) -> usize {
    beats.partition_point(|beat| beat.seconds <= seconds)
}

/// What draws the frames of a visualizer or video clip, or why it cannot be
/// drawn. Not for images, which are not drawn frame by frame.
pub fn frame_source(
    media: &MediaSource,
    clip: &Clip,
    bundled_presets: Option<&Path>,
    catalog: &mut VisualizerCatalog,
    ffmpeg: Option<&Path>,
) -> Result<FrameSource, String> {
    if media.video().is_some() {
        let ffmpeg = ffmpeg.ok_or(
            "Video files need FFmpeg, which was not found. Put ffmpeg next to VizX or install it.",
        )?;
        return Ok(FrameSource::Video {
            file: media.path.clone().into(),
            ffmpeg: ffmpeg.to_owned(),
            start_offset_seconds: clip.source_offset_milliseconds as f64 / 1000.0,
        });
    }
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

/// Where and how a layer is drawn into the frame at one moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerGeometry {
    /// Middle of the layer, as fractions of the frame's width and height.
    pub center: [f32; 2],
    /// Width and height, as fractions of the frame's.
    pub size: [f32; 2],
    /// Turn around the middle, in degrees clockwise.
    pub rotation_degrees: f32,
    /// 0 to 1.
    pub opacity: f32,
}

/// Combines a clip's placement and opacity with what its reaction to the
/// sound does right now.
pub fn layer_geometry(placement: Placement, opacity: f64, effect: ReactionEffect) -> LayerGeometry {
    LayerGeometry {
        center: [
            (placement.x + placement.width / 2.0 + effect.offset_x) as f32,
            (placement.y + placement.height / 2.0 + effect.offset_y) as f32,
        ],
        // Growing happens around the layer's own middle.
        size: [
            (placement.width * effect.scale) as f32,
            (placement.height * effect.scale) as f32,
        ],
        rotation_degrees: effect.rotation_degrees as f32,
        opacity: (opacity * effect.opacity).clamp(0.0, 1.0) as f32,
    }
}

/// Measures how loud the sound is for clips that react to it.
pub struct ReactionMeter {
    analyzer: AudioAnalyzer,
}

impl Default for ReactionMeter {
    fn default() -> Self {
        Self {
            analyzer: AudioAnalyzer::new(REACTION_SAMPLE_RATE),
        }
    }
}

impl ReactionMeter {
    /// What a reaction does to its element at a moment of the project.
    /// `audio_gain` is the clip's sensitivity: the sound is multiplied by it
    /// before it is measured.
    pub fn effect(
        &mut self,
        sources: &[PlaybackSource],
        beats: &[Beat],
        reaction: &AudioReaction,
        audio_gain: f64,
        seconds: f64,
    ) -> ReactionEffect {
        if reaction.is_active() {
            reaction.at_level(self.level(sources, beats, reaction, audio_gain, seconds))
        } else {
            ReactionEffect::NONE
        }
    }

    /// How loud the part of the sound is that a reaction follows, at a
    /// moment of the project, from 0 to 1, with the reaction's release time
    /// applied: the value jumps up with the sound and then falls back
    /// gradually.
    ///
    /// It is worked out from the audio leading up to that moment and from
    /// nothing else, so the same moment of the song always gives the same
    /// value, whether it is reached by playing, by jumping there, or during
    /// export. For the release that means looking at a few earlier moments
    /// too: a loud moment shortly before still counts, the less the longer
    /// ago it was.
    pub fn level(
        &mut self,
        sources: &[PlaybackSource],
        beats: &[Beat],
        reaction: &AudioReaction,
        audio_gain: f64,
        seconds: f64,
    ) -> f64 {
        // Beats are known in advance: a push on each, fading over the
        // release time.
        if reaction.band == ReactionBand::Beat {
            let pulse = beat_pulse(beats, seconds, reaction.release_seconds);
            return (f64::from(pulse) * audio_gain).min(1.0);
        }
        let release = reaction.release_seconds;
        let steps_back = if release > 0.0 {
            ((release / RELEASE_STEP_SECONDS).ceil() as u32).min(MAX_RELEASE_STEPS)
        } else {
            0
        };

        let mut level: f64 = 0.0;
        for step in 0..=steps_back {
            let seconds_ago = f64::from(step) * RELEASE_STEP_SECONDS;
            let moment = seconds - seconds_ago;
            if moment < 0.0 {
                break;
            }
            // How much of a level from that moment is left by now.
            let remaining = if step == 0 {
                1.0
            } else {
                (1.0 - seconds_ago / release).max(0.0)
            };
            level =
                level.max(self.level_at(sources, reaction.band, audio_gain, moment) * remaining);
        }
        level
    }

    /// How loud one part of the sound is at one moment, from 0 to 1.
    fn level_at(
        &mut self,
        sources: &[PlaybackSource],
        band: ReactionBand,
        audio_gain: f64,
        seconds: f64,
    ) -> f64 {
        let window_end = (seconds.max(0.0) * f64::from(REACTION_SAMPLE_RATE)) as u64;
        let window_start = window_end.saturating_sub(ANALYSIS_WINDOW as u64);
        let stereo = vizx_audio::mix_to_stereo(
            sources,
            REACTION_SAMPLE_RATE,
            window_start,
            (window_end - window_start) as usize,
        );
        let mono: Vec<f32> = stereo
            .as_chunks::<2>()
            .0
            .iter()
            // The clip's sensitivity is applied to the sound itself.
            .map(|[left, right]| (left + right) * 0.5 * audio_gain as f32)
            .collect();

        // "Infinitely long ago": the analyzer's memory of earlier calls has
        // faded completely, so only this moment counts.
        let sound = self.analyzer.analyze(&mono, f32::MAX);
        f64::from(match band {
            ReactionBand::Volume => sound.volume,
            ReactionBand::Bass => sound.bass,
            ReactionBand::Mid => sound.mid,
            ReactionBand::Treble => sound.treble,
            // Handled in `level`.
            ReactionBand::Beat => 0.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use vizx_audio::DecodedAudio;

    use super::*;

    /// One second of a loud 440 Hz tone followed by one second of silence.
    fn tone_then_silence() -> Vec<PlaybackSource> {
        let rate = 44_100;
        let samples = (0..rate * 2)
            .map(|index| {
                if index < rate {
                    0.8 * (std::f32::consts::TAU * 440.0 * index as f32 / rate as f32).sin()
                } else {
                    0.0
                }
            })
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

    #[test]
    fn geometry_combines_placement_with_the_reaction() {
        let placement = Placement {
            x: 0.2,
            y: 0.1,
            width: 0.4,
            height: 0.6,
        };

        let still = layer_geometry(placement, 0.5, ReactionEffect::NONE);
        assert_eq!(still.center, [0.4, 0.4]);
        assert_eq!(still.size, [0.4, 0.6]);
        assert_eq!((still.rotation_degrees, still.opacity), (0.0, 0.5));

        let moved = layer_geometry(
            placement,
            0.5,
            ReactionEffect {
                scale: 2.0,
                opacity: 0.5,
                offset_x: 0.1,
                offset_y: -0.1,
                rotation_degrees: 45.0,
            },
        );
        assert_eq!(moved.center, [0.5, 0.3]);
        assert_eq!(moved.size, [0.8, 1.2]);
        assert_eq!((moved.rotation_degrees, moved.opacity), (45.0, 0.25));
    }

    #[test]
    fn level_follows_the_sound_and_falls_back_over_the_release_time() {
        let sources = tone_then_silence();
        let mut meter = ReactionMeter::default();
        let immediate = AudioReaction {
            release_seconds: 0.0,
            ..AudioReaction::default()
        };
        let releasing = AudioReaction {
            release_seconds: 0.4,
            ..AudioReaction::default()
        };

        let during_tone = meter.level(&sources, &[], &immediate, 1.0, 0.5);
        assert!(during_tone > 0.7, "{during_tone}");
        // 0.2 s after the tone ended.
        assert_eq!(meter.level(&sources, &[], &immediate, 1.0, 1.2), 0.0);

        // With a release the level is on its way down: about half is left
        // after half of the release time.
        let falling = meter.level(&sources, &[], &releasing, 1.0, 1.2);
        assert!(
            falling > during_tone * 0.3 && falling < during_tone * 0.7,
            "{falling}"
        );
        assert_eq!(meter.level(&sources, &[], &releasing, 1.0, 1.8), 0.0);

        // A quarter of the sensitivity: a quieter sound, a lower level.
        let quiet = meter.level(&sources, &[], &immediate, 0.25, 0.5);
        assert!(quiet < during_tone * 0.4 && quiet > 0.0, "{quiet}");

        // The same moment gives the same value, whatever was asked before.
        assert_eq!(meter.level(&sources, &[], &releasing, 1.0, 1.2), falling);
    }

    #[test]
    fn a_beat_is_felt_right_on_it_and_fades_out() {
        let beats = [
            Beat {
                seconds: 1.0,
                strength: 1.0,
            },
            Beat {
                seconds: 2.0,
                strength: 0.5,
            },
        ];

        assert_eq!(beat_pulse(&beats, 0.5, 0.2), 0.0);
        assert_eq!(beat_pulse(&beats, 1.0, 0.2), 1.0);
        assert!((beat_pulse(&beats, 1.1, 0.2) - 0.5).abs() < 1e-6);
        assert_eq!(beat_pulse(&beats, 1.3, 0.2), 0.0);
        assert_eq!(beat_pulse(&beats, 2.0, 0.2), 0.5);
        // Even without a release time a beat is felt for a moment.
        assert!(beat_pulse(&beats, 1.04, 0.0) > 0.0);
        assert_eq!((beat_count(&beats, 0.9), beat_count(&beats, 1.0)), (0, 1));
        assert_eq!(last_beat(&beats, 5.0).unwrap().seconds, 2.0);

        // As an animation: the "Beat" band follows the beats, not the sound.
        let reaction = AudioReaction {
            band: ReactionBand::Beat,
            scale: 1.0,
            release_seconds: 0.2,
            ..AudioReaction::default()
        };
        let mut meter = ReactionMeter::default();
        assert_eq!(meter.level(&[], &beats, &reaction, 1.0, 1.0), 1.0);
        assert_eq!(meter.level(&[], &beats, &reaction, 0.5, 1.0), 0.5);
        assert_eq!(meter.level(&[], &beats, &reaction, 1.0, 1.5), 0.0);
    }

    #[test]
    fn an_inactive_reaction_has_no_effect() {
        let mut meter = ReactionMeter::default();

        let effect = meter.effect(
            &tone_then_silence(),
            &[],
            &AudioReaction::default(),
            1.0,
            0.5,
        );

        assert_eq!(effect, ReactionEffect::NONE);
    }
}
