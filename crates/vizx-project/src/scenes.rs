//! Scenes and the transitions between them.
//!
//! Every timeline marker starts a scene that lasts until the next marker,
//! and can say how the picture changes there: a cut (nothing), a flash of
//! white, a fade through black, a zoom punch, a glitch, a blur, a spin or a
//! shake.
//!
//! Transitions are not drawn by special code: the project keeps a layer of
//! its own, "Scene transitions", with effects-layer clips around every
//! scene change, worked out from the markers whenever they change
//! (`rebuild_scene_transitions`). So the preview and the export show them
//! like any other effects layer, and they can be seen on the timeline. The
//! layer sits above the picture and below the subtitles, which stay
//! readable. Changes made to its clips by hand are replaced at the next
//! rebuild.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    AudioInput, AudioReaction, BlendMode, Clip, ClipEffects, Easing, Keyframe, MediaContent,
    MediaId, MediaSource, Placement, Project, TrackKind, Transform,
};

/// How the picture changes where a scene starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TransitionKind {
    /// Straight from one scene to the next.
    #[default]
    Cut,
    /// A flash of white light at the change.
    Flash,
    /// The picture goes dark and comes back.
    FadeBlack,
    /// A quick zoom in and out with motion blur.
    ZoomPunch,
    /// Bands jump and colors split.
    Glitch,
    /// Out of focus and back.
    Blur,
    /// The whole picture turns once.
    Spin,
    /// The camera shakes.
    Shake,
}

impl TransitionKind {
    pub const ALL: [Self; 8] = [
        Self::Cut,
        Self::Flash,
        Self::FadeBlack,
        Self::ZoomPunch,
        Self::Glitch,
        Self::Blur,
        Self::Spin,
        Self::Shake,
    ];

    /// Name shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Self::Cut => "Cut",
            Self::Flash => "Flash",
            Self::FadeBlack => "Fade through black",
            Self::ZoomPunch => "Zoom punch",
            Self::Glitch => "Glitch",
            Self::Blur => "Blur",
            Self::Spin => "Spin",
            Self::Shake => "Shake",
        }
    }
}

/// The transition at the start of a scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transition {
    pub kind: TransitionKind,
    /// How long it takes, centered on the scene change.
    pub duration_milliseconds: u64,
}

impl Default for Transition {
    fn default() -> Self {
        Self {
            kind: TransitionKind::Cut,
            duration_milliseconds: 600,
        }
    }
}

impl Transition {
    /// Shortest and longest allowed duration.
    pub const DURATION_RANGE: (u64, u64) = (100, 4000);

    pub fn new(kind: TransitionKind, duration_milliseconds: u64) -> Self {
        let (shortest, longest) = Self::DURATION_RANGE;
        Self {
            kind,
            duration_milliseconds: duration_milliseconds.clamp(shortest, longest),
        }
    }

    pub(crate) fn is_cut(&self) -> bool {
        self.kind == TransitionKind::Cut
    }
}

/// One scene: from its marker to the next one (or the end).
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    /// The marker that starts it; `None` for the part before the first
    /// marker.
    pub marker: Option<usize>,
    pub name: String,
    pub start_milliseconds: u64,
    pub end_milliseconds: u64,
    pub transition: Transition,
}

/// Name of the layer that holds the transitions.
pub const TRANSITIONS_LAYER_NAME: &str = "Scene transitions";

impl Project {
    /// The scenes, in order of time.
    pub fn scenes(&self) -> Vec<Scene> {
        let end = self.settings.duration_milliseconds;
        let markers = self.markers();
        let mut scenes = Vec::new();
        if markers.first().is_none_or(|first| first.milliseconds > 0) {
            scenes.push(Scene {
                marker: None,
                name: "Start".to_owned(),
                start_milliseconds: 0,
                end_milliseconds: markers.first().map_or(end, |first| first.milliseconds),
                transition: Transition::default(),
            });
        }
        for (index, marker) in markers.iter().enumerate() {
            scenes.push(Scene {
                marker: Some(index),
                name: marker.name.clone(),
                start_milliseconds: marker.milliseconds,
                end_milliseconds: markers
                    .get(index + 1)
                    .map_or(end, |next| next.milliseconds)
                    .max(marker.milliseconds),
                transition: marker.transition,
            });
        }
        scenes
    }

    /// Sets the transition at the start of the scene of marker `index` and
    /// lays the transitions out again. Returns `true` only if it changed.
    pub fn set_marker_transition(&mut self, index: usize, transition: Transition) -> bool {
        let transition = Transition::new(transition.kind, transition.duration_milliseconds);
        let changed = match self.markers_mut().get_mut(index) {
            Some(marker) if marker.transition != transition => {
                marker.transition = transition;
                true
            }
            _ => false,
        };
        if changed {
            self.rebuild_scene_transitions();
        }
        changed
    }

    /// Lays the transitions out again from the markers: the "Scene
    /// transitions" layer gets one run of effects-layer clips around every
    /// scene change that is not a cut, and goes away when there are none.
    pub fn rebuild_scene_transitions(&mut self) {
        let wanted: Vec<(u64, Transition)> = self
            .markers()
            .iter()
            .filter(|marker| !marker.transition.is_cut() && marker.milliseconds > 0)
            .map(|marker| (marker.milliseconds, marker.transition))
            .collect();
        let existing = self
            .tracks()
            .iter()
            .find(|track| track.scene_transitions)
            .map(|track| track.id);

        if wanted.is_empty() {
            if let Some(id) = existing {
                self.remove_track(id);
                // The effects layer it used is not needed any more.
                let used: Vec<MediaId> = self
                    .tracks()
                    .iter()
                    .flat_map(|track| track.clips.iter().map(|clip| clip.media))
                    .collect();
                self.media
                    .retain(|media| !media.is_effects_layer() || used.contains(&media.id));
            }
            return;
        }
        let track_id = match existing {
            Some(id) => id,
            None => {
                let id = self.add_track(TRANSITIONS_LAYER_NAME, TrackKind::Visual);
                let tracks = self.tracks_mut_vec();
                if let Some(from) = tracks.iter().position(|track| track.id == id) {
                    let mut track = tracks.remove(from);
                    track.scene_transitions = true;
                    // Above the picture, below the subtitles.
                    let at = tracks
                        .iter()
                        .position(|other| other.kind != TrackKind::Subtitles)
                        .unwrap_or(tracks.len());
                    tracks.insert(at, track);
                }
                id
            }
        };

        let media_id = self.transition_media(track_id);
        let end = self.settings.duration_milliseconds;
        let mut clips = Vec::new();
        for (at, transition) in wanted {
            let half = transition.duration_milliseconds / 2;
            let from = at.saturating_sub(half);
            let to = (at + half).min(end);
            if to <= from {
                continue;
            }
            let steps = (transition.duration_milliseconds / 50).clamp(4, 20);
            for step in 0..steps {
                let start = from + (to - from) * step / steps;
                let stop = from + (to - from) * (step + 1) / steps;
                if stop <= start {
                    continue;
                }
                let clip =
                    self.transition_step(media_id, transition, at, (from, to), (start, stop), step);
                clips.push(clip);
            }
        }
        if let Some(track) = self
            .tracks_mut_vec()
            .iter_mut()
            .find(|track| track.id == track_id)
        {
            track.clips = clips;
        }
    }

    /// The effects-layer media the transition clips share.
    fn transition_media(&mut self, track_id: crate::TrackId) -> MediaId {
        let used = self.track(track_id).and_then(|track| {
            track.clips.iter().map(|clip| clip.media).find(|media| {
                self.media_source(*media)
                    .is_some_and(MediaSource::is_effects_layer)
            })
        });
        if let Some(media) = used {
            return media;
        }
        let id = self.new_media_id();
        self.media.push(MediaSource {
            id,
            relative_path: None,
            path: String::new(),
            content: MediaContent::EffectsLayer,
            manual_beats: None,
            tempo_bpm: None,
        });
        id
    }

    /// One short clip of a transition: its effects at that moment, and its
    /// zoom, turn or shake gliding from its start to its end.
    fn transition_step(
        &mut self,
        media: MediaId,
        transition: Transition,
        at: u64,
        (from, to): (u64, u64),
        (start, stop): (u64, u64),
        step: u64,
    ) -> Clip {
        let half = (transition.duration_milliseconds as f64 / 2.0).max(1.0);
        // 0 far from the change, 1 right on it.
        let strength = |time: u64| (1.0 - (time as f64 - at as f64).abs() / half).clamp(0.0, 1.0);
        let middle = strength((start + stop) / 2);
        let progress =
            |time: u64| ((time - from) as f64 / (to - from).max(1) as f64).clamp(0.0, 1.0);
        let smooth = |value: f64| value * value * (3.0 - 2.0 * value);

        let none = ClipEffects::default();
        let effects = match transition.kind {
            TransitionKind::Cut => none,
            TransitionKind::Flash => ClipEffects {
                fill: middle.powf(1.5),
                fill_color: [1.0, 1.0, 1.0],
                glow: 0.5 * middle,
                ..none
            },
            TransitionKind::FadeBlack => ClipEffects {
                fill: smooth(middle),
                fill_color: [0.0, 0.0, 0.0],
                ..none
            },
            TransitionKind::ZoomPunch => ClipEffects {
                blur: 0.5 * middle,
                color_split: 0.3 * middle,
                ..none
            },
            TransitionKind::Glitch => ClipEffects {
                glitch: middle,
                color_split: middle,
                ..none
            },
            TransitionKind::Blur => ClipEffects {
                blur: middle,
                glow: 0.3 * middle,
                ..none
            },
            TransitionKind::Spin => ClipEffects {
                blur: 0.4 * middle,
                ..none
            },
            TransitionKind::Shake => ClipEffects {
                color_split: 0.3 * middle,
                ..none
            },
        };
        // Where the picture is at a moment: zoomed, turned, shaken.
        let place = |time: u64| -> (Placement, f64) {
            let amount = strength(time);
            let (scale, rotation, offset) = match transition.kind {
                TransitionKind::ZoomPunch => (1.0 + 0.35 * amount, 0.0, (0.0, 0.0)),
                TransitionKind::Glitch => (1.0 + 0.04 * amount, 0.0, (0.0, 0.0)),
                // A whole turn, enlarged enough that no corner shows.
                TransitionKind::Spin => (
                    1.0 + 1.0 * amount,
                    360.0 * smooth(progress(time)),
                    (0.0, 0.0),
                ),
                TransitionKind::Shake => {
                    let wobble = |seed: u64| {
                        let value = (step.wrapping_mul(2_654_435_761) ^ seed)
                            .wrapping_mul(0x9E37_79B9)
                            >> 7;
                        (value % 1000) as f64 / 1000.0 - 0.5
                    };
                    (
                        1.08,
                        0.0,
                        (wobble(17) * 0.06 * amount, wobble(91) * 0.06 * amount),
                    )
                }
                _ => (1.0, 0.0, (0.0, 0.0)),
            };
            (
                Placement {
                    x: (1.0 - scale) / 2.0 + offset.0,
                    y: (1.0 - scale) / 2.0 + offset.1,
                    width: scale,
                    height: scale,
                },
                rotation,
            )
        };
        let (first, first_turn) = place(start);
        let (last, last_turn) = place(stop);
        let key = |time: i64, placement: Placement, rotation_degrees: f64| Keyframe {
            time_milliseconds: time,
            transform: Transform {
                placement,
                rotation_degrees,
                opacity: 1.0,
            },
            easing: Easing::Linear,
        };
        let keyframes = if first == last && first_turn == last_turn {
            Vec::new()
        } else {
            vec![
                key(0, first, first_turn),
                key((stop - start) as i64, last, last_turn),
            ]
        };
        let id = self.new_clip_id();
        Clip {
            id,
            media,
            start_milliseconds: start,
            duration_milliseconds: stop - start,
            source_offset_milliseconds: 0,
            audio_gain: 1.0,
            audio_input: AudioInput::Mix,
            placement: first,
            parameters: BTreeMap::new(),
            opacity: 1.0,
            reaction: AudioReaction::default(),
            rotation_degrees: first_turn,
            spin_degrees_per_second: 0.0,
            keyframes,
            sharp_pixels: None,
            blend_mode: BlendMode::Normal,
            effects,
            entrance_exit: crate::EntranceExit::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_make_scenes_and_transitions_make_a_layer_below_the_subtitles() {
        let mut project = Project::with_starter_tracks();
        project.settings.duration_milliseconds = 60_000;
        let lyrics = project.add_subtitle_track("Lyrics");
        project.add_marker(10_000, "Verse");
        project.add_marker(30_000, "Chorus");

        let scenes = project.scenes();
        let names: Vec<&str> = scenes.iter().map(|scene| scene.name.as_str()).collect();
        assert_eq!(names, ["Start", "Verse", "Chorus"]);
        assert_eq!(scenes[1].end_milliseconds, 30_000);
        assert_eq!(scenes[2].end_milliseconds, 60_000);
        // Only cuts: no transition layer.
        assert!(!project.tracks().iter().any(|track| track.scene_transitions));

        let chorus = scenes[2].marker.unwrap();
        assert!(project.set_marker_transition(chorus, Transition::new(TransitionKind::Flash, 800)));
        let position = |project: &Project, id| {
            project
                .tracks()
                .iter()
                .position(|track| track.id == id)
                .unwrap()
        };
        let layer = project
            .tracks()
            .iter()
            .find(|track| track.scene_transitions)
            .expect("a transition layer")
            .clone();
        // Right below the subtitles.
        assert_eq!(position(&project, layer.id), position(&project, lyrics) + 1);
        // Clips cover 29.6 s to 30.4 s, strongest at 30 s.
        let first = layer
            .clips
            .iter()
            .map(|clip| clip.start_milliseconds)
            .min()
            .unwrap();
        let last = layer
            .clips
            .iter()
            .map(Clip::end_milliseconds)
            .max()
            .unwrap();
        assert_eq!((first, last), (29_600, 30_400));
        let strongest = layer
            .clips
            .iter()
            .max_by(|a, b| a.effects.fill.total_cmp(&b.effects.fill))
            .unwrap();
        assert!(strongest.start_milliseconds <= 30_000 && 30_000 <= strongest.end_milliseconds());

        // Moving the marker moves the transition; back to a cut removes it.
        project.move_marker(chorus, 40_000);
        let layer = project
            .tracks()
            .iter()
            .find(|track| track.scene_transitions)
            .unwrap();
        assert_eq!(
            layer.clips.iter().map(|clip| clip.start_milliseconds).min(),
            Some(39_600)
        );
        let chorus = project
            .markers()
            .iter()
            .position(|marker| marker.name == "Chorus")
            .unwrap();
        project.set_marker_transition(chorus, Transition::default());
        assert!(!project.tracks().iter().any(|track| track.scene_transitions));
        // Saved and loaded, the transition is kept.
        project.set_marker_transition(chorus, Transition::new(TransitionKind::Spin, 1000));
        let loaded = Project::from_vzx_str(&project.to_vzx_string().unwrap()).unwrap();
        assert_eq!(loaded.markers(), project.markers());
        let clips = |project: &Project| {
            project
                .tracks()
                .iter()
                .find(|track| track.scene_transitions)
                .map_or(0, |track| track.clips.len())
        };
        assert_eq!(clips(&loaded), clips(&project));
        // Only one effects layer is kept for the transitions.
        let effects_media = project
            .media()
            .iter()
            .filter(|media| media.is_effects_layer())
            .count();
        assert_eq!(effects_media, 1);
    }
}
