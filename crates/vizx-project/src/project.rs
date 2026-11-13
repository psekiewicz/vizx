use serde::{Deserialize, Serialize};

use crate::{CURRENT_FORMAT_VERSION, ProjectMetadata, ProjectSettings, Track, TrackId, TrackKind};

/// Root of all data that belongs to one VizX project.
///
/// Everything that is saved into a `.vzx` file hangs off this struct. Future
/// milestones add media sources, clips, keyframes and effects here, so that
/// every subsystem works on the same shared data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// Data layout version this project was written with.
    pub format_version: u32,
    pub metadata: ProjectMetadata,
    pub settings: ProjectSettings,
    /// Timeline tracks, from top to bottom. Private so that ids can only be
    /// handed out by [`Project::add_track`].
    ///
    /// `#[serde(default)]` lets files written before tracks existed (format
    /// version 1) still load: a missing field becomes an empty list.
    #[serde(default)]
    tracks: Vec<Track>,
    /// Id the next new track receives. Ids are never reused, so a stale
    /// reference to a removed track can never point at a different track.
    #[serde(default)]
    next_track_id: u64,
}

impl Project {
    /// Creates an empty project with default settings and no tracks.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            format_version: CURRENT_FORMAT_VERSION,
            metadata: ProjectMetadata::new(name),
            settings: ProjectSettings::default(),
            tracks: Vec::new(),
            next_track_id: 0,
        }
    }

    /// Creates a project with a typical set of tracks for a music video, so
    /// a new project does not start with an empty timeline.
    pub fn with_starter_tracks(name: impl Into<String>) -> Self {
        let mut project = Self::new(name);
        for (track_name, kind) in [
            ("Vocals", TrackKind::Audio),
            ("Music", TrackKind::Audio),
            ("Beat", TrackKind::Audio),
            ("Visualizer", TrackKind::Visualizer),
            ("Animation", TrackKind::Animation),
            ("Lyrics", TrackKind::Text),
        ] {
            project.add_track(track_name, kind);
        }
        project
    }

    /// Tracks from top to bottom.
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|track| track.id == id)
    }

    /// Appends a track at the bottom and returns its id.
    pub fn add_track(&mut self, name: impl Into<String>, kind: TrackKind) -> TrackId {
        // Guard against a hand-edited file whose counter is behind its ids.
        let highest_existing_id = self.tracks.iter().map(|track| track.id.0 + 1).max();
        let id = TrackId(self.next_track_id.max(highest_existing_id.unwrap_or(0)));

        self.next_track_id = id.0 + 1;
        self.tracks.push(Track {
            id,
            name: name.into(),
            kind,
        });
        id
    }

    /// Appends a track with an automatic name such as `Audio 7`.
    pub fn add_default_named_track(&mut self, kind: TrackKind) -> TrackId {
        let number = self.tracks.len() + 1;
        self.add_track(format!("{} {number}", kind.label()), kind)
    }

    /// Removes a track. Returns `false` if no track has this id.
    pub fn remove_track(&mut self, id: TrackId) -> bool {
        let count_before = self.tracks.len();
        self.tracks.retain(|track| track.id != id);
        self.tracks.len() != count_before
    }

    /// Renames a track. Returns `true` only if the name actually changed.
    pub fn rename_track(&mut self, id: TrackId, name: &str) -> bool {
        match self.tracks.iter_mut().find(|track| track.id == id) {
            Some(track) if track.name != name => {
                track.name = name.to_owned();
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FrameRate;

    #[test]
    fn new_project_uses_current_format_and_defaults() {
        let project = Project::new("Demo");

        assert_eq!(project.format_version, CURRENT_FORMAT_VERSION);
        assert_eq!(project.metadata.name, "Demo");
        assert_eq!(project.settings, ProjectSettings::default());
        assert!(project.tracks().is_empty());
    }

    #[test]
    fn project_survives_a_serialization_round_trip() {
        let mut project = Project::with_starter_tracks("Round trip");
        project.metadata.author = "Tester".to_owned();
        project.settings.frame_rate = FrameRate {
            numerator: 30000,
            denominator: 1001,
        };

        let encoded = serde_json::to_string(&project).expect("project should serialize");
        let decoded: Project = serde_json::from_str(&encoded).expect("project should deserialize");

        assert_eq!(decoded, project);
    }

    #[test]
    fn frame_rate_fraction_converts_to_expected_value() {
        let ntsc = FrameRate {
            numerator: 30000,
            denominator: 1001,
        };

        assert!((ntsc.as_f64() - 29.97).abs() < 0.001);
        assert_eq!(FrameRate::FPS_60.as_f64(), 60.0);
    }

    #[test]
    fn starter_tracks_have_unique_ids_in_order() {
        let project = Project::with_starter_tracks("Starter");

        let names: Vec<&str> = project
            .tracks()
            .iter()
            .map(|track| track.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "Vocals",
                "Music",
                "Beat",
                "Visualizer",
                "Animation",
                "Lyrics"
            ]
        );

        let mut ids: Vec<TrackId> = project.tracks().iter().map(|track| track.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), 6);
    }

    #[test]
    fn track_ids_are_not_reused_after_removal() {
        let mut project = Project::new("Ids");
        let first = project.add_track("First", TrackKind::Audio);
        let second = project.add_track("Second", TrackKind::Audio);

        assert!(project.remove_track(second));
        let third = project.add_track("Third", TrackKind::Text);

        assert_ne!(third, first);
        assert_ne!(third, second);
        assert!(project.track(second).is_none());
        assert!(!project.remove_track(second));
    }

    #[test]
    fn rename_reports_whether_anything_changed() {
        let mut project = Project::new("Rename");
        let id = project.add_track("Old", TrackKind::Audio);

        assert!(project.rename_track(id, "New"));
        assert!(!project.rename_track(id, "New"));
        assert_eq!(project.track(id).unwrap().name, "New");
    }
}
