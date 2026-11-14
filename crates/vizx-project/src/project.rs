use serde::{Deserialize, Serialize};

use crate::{
    AudioProperties, CURRENT_FORMAT_VERSION, Clip, ClipId, MediaContent, MediaId, MediaSource,
    ProjectMetadata, ProjectSettings, Track, TrackId, TrackKind,
};

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
    /// Files used by the project (the "media library").
    #[serde(default)]
    media: Vec<MediaSource>,
    #[serde(default)]
    next_media_id: u64,
    #[serde(default)]
    next_clip_id: u64,
}

impl Default for Project {
    fn default() -> Self {
        Self::new()
    }
}

impl Project {
    /// Creates an empty project with default settings and no tracks.
    pub fn new() -> Self {
        Self {
            format_version: CURRENT_FORMAT_VERSION,
            metadata: ProjectMetadata::default(),
            settings: ProjectSettings::default(),
            tracks: Vec::new(),
            next_track_id: 0,
            media: Vec::new(),
            next_media_id: 0,
            next_clip_id: 0,
        }
    }

    /// Creates a project with the smallest useful timeline: one audio track
    /// for the song and one visual layer to put things on.
    pub fn with_starter_tracks() -> Self {
        let mut project = Self::new();
        project.add_track("Audio", TrackKind::Audio);
        project.add_track("Layer 1", TrackKind::Visual);
        project
    }

    /// Tracks from top to bottom. Visual layers come first, in drawing order
    /// (the first one is drawn on top), followed by the audio tracks.
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|track| track.id == id)
    }

    /// Adds a track and returns its id.
    ///
    /// A visual layer is added at the top, so it is drawn over the existing
    /// layers, as a new layer is in a video or image editor. An audio track
    /// is added at the bottom, below all visual layers.
    pub fn add_track(&mut self, name: impl Into<String>, kind: TrackKind) -> TrackId {
        // Guard against a hand-edited file whose counter is behind its ids.
        let highest_existing_id = self.tracks.iter().map(|track| track.id.0 + 1).max();
        let id = TrackId(self.next_track_id.max(highest_existing_id.unwrap_or(0)));

        self.next_track_id = id.0 + 1;
        let track = Track {
            id,
            name: name.into(),
            kind,
            clips: Vec::new(),
        };
        match kind {
            TrackKind::Visual => self.tracks.insert(0, track),
            TrackKind::Audio => self.tracks.push(track),
        }
        id
    }

    /// Adds a track with an automatic name such as `Layer 2` or `Audio 2`.
    pub fn add_default_named_track(&mut self, kind: TrackKind) -> TrackId {
        let number = self
            .tracks
            .iter()
            .filter(|track| track.kind == kind)
            .count()
            + 1;
        self.add_track(format!("{} {number}", kind.default_name_prefix()), kind)
    }

    /// Removes a track. Returns `false` if no track has this id.
    pub fn remove_track(&mut self, id: TrackId) -> bool {
        let count_before = self.tracks.len();
        self.tracks.retain(|track| track.id != id);
        self.tracks.len() != count_before
    }

    /// All media sources, in the order they were imported.
    pub fn media(&self) -> &[MediaSource] {
        &self.media
    }

    pub fn media_source(&self, id: MediaId) -> Option<&MediaSource> {
        self.media.iter().find(|source| source.id == id)
    }

    /// Adds an audio file to the media library and places it on the
    /// timeline, starting at the beginning of the project.
    ///
    /// It goes onto the first audio track that is still empty. If every
    /// audio track is in use, a new one is created, so importing never
    /// overlaps or replaces existing audio. Returns the new media source and
    /// the track that received the clip.
    pub fn import_audio(
        &mut self,
        path: impl Into<String>,
        properties: AudioProperties,
    ) -> (MediaId, TrackId) {
        let has_no_clips_yet = self.tracks.iter().all(|track| track.clips.is_empty());

        let media_id = MediaId(self.next_media_id);
        self.next_media_id += 1;
        self.media.push(MediaSource {
            id: media_id,
            path: path.into(),
            content: MediaContent::Audio(properties),
        });

        let free_audio_track = self
            .tracks
            .iter()
            .find(|track| track.kind == TrackKind::Audio && track.clips.is_empty())
            .map(|track| track.id);
        let track_id =
            free_audio_track.unwrap_or_else(|| self.add_default_named_track(TrackKind::Audio));

        let clip = Clip {
            id: ClipId(self.next_clip_id),
            media: media_id,
            start_milliseconds: 0,
            duration_milliseconds: properties.duration_milliseconds,
        };
        self.next_clip_id += 1;
        let clip_end = clip.end_milliseconds();
        if let Some(track) = self.tracks.iter_mut().find(|track| track.id == track_id) {
            track.clips.push(clip);
        }

        // The project length follows the audio: the first file sets it, and
        // a later, longer file extends it. It is never shortened here.
        self.settings.duration_milliseconds = if has_no_clips_yet {
            clip_end
        } else {
            self.settings.duration_milliseconds.max(clip_end)
        };

        (media_id, track_id)
    }

    /// Adds a MilkDrop preset to the media library and places it on a
    /// visual layer, covering the whole project.
    ///
    /// Like audio, it goes onto the first layer that is still empty, and a
    /// new layer is created only if all of them are in use.
    pub fn add_milkdrop_preset(&mut self, path: impl Into<String>) -> (MediaId, TrackId) {
        let media_id = MediaId(self.next_media_id);
        self.next_media_id += 1;
        self.media.push(MediaSource {
            id: media_id,
            path: path.into(),
            content: MediaContent::MilkdropPreset,
        });

        let free_layer = self
            .tracks
            .iter()
            .find(|track| track.kind == TrackKind::Visual && track.clips.is_empty())
            .map(|track| track.id);
        let track_id =
            free_layer.unwrap_or_else(|| self.add_default_named_track(TrackKind::Visual));

        let clip = Clip {
            id: ClipId(self.next_clip_id),
            media: media_id,
            start_milliseconds: 0,
            duration_milliseconds: self.settings.duration_milliseconds,
        };
        self.next_clip_id += 1;
        if let Some(track) = self.tracks.iter_mut().find(|track| track.id == track_id) {
            track.clips.push(clip);
        }

        (media_id, track_id)
    }

    /// The MilkDrop preset shown at a point in time: the one on the topmost
    /// visual layer that has a preset clip covering that time.
    pub fn milkdrop_preset_at(&self, milliseconds: u64) -> Option<&MediaSource> {
        self.tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Visual)
            .flat_map(|track| &track.clips)
            .filter(|clip| {
                milliseconds >= clip.start_milliseconds && milliseconds < clip.end_milliseconds()
            })
            .filter_map(|clip| self.media_source(clip.media))
            .find(|source| source.is_milkdrop_preset())
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
        let project = Project::new();

        assert_eq!(project.format_version, CURRENT_FORMAT_VERSION);
        assert_eq!(project.settings, ProjectSettings::default());
        assert!(project.tracks().is_empty());
    }

    #[test]
    fn project_survives_a_serialization_round_trip() {
        let mut project = Project::with_starter_tracks();
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
    fn new_project_starts_with_one_layer_above_one_audio_track() {
        let project = Project::with_starter_tracks();

        let tracks: Vec<(&str, TrackKind)> = project
            .tracks()
            .iter()
            .map(|track| (track.name.as_str(), track.kind))
            .collect();
        assert_eq!(
            tracks,
            [("Layer 1", TrackKind::Visual), ("Audio", TrackKind::Audio)]
        );
        assert_ne!(project.tracks()[0].id, project.tracks()[1].id);
    }

    #[test]
    fn new_layers_go_on_top_and_new_audio_tracks_to_the_bottom() {
        let mut project = Project::with_starter_tracks();

        project.add_default_named_track(TrackKind::Audio);
        project.add_default_named_track(TrackKind::Visual);

        let names: Vec<&str> = project
            .tracks()
            .iter()
            .map(|track| track.name.as_str())
            .collect();
        assert_eq!(names, ["Layer 2", "Layer 1", "Audio", "Audio 2"]);
    }

    #[test]
    fn track_ids_are_not_reused_after_removal() {
        let mut project = Project::new();
        let first = project.add_track("First", TrackKind::Audio);
        let second = project.add_track("Second", TrackKind::Audio);

        assert!(project.remove_track(second));
        let third = project.add_track("Third", TrackKind::Visual);

        assert_ne!(third, first);
        assert_ne!(third, second);
        assert!(project.track(second).is_none());
        assert!(!project.remove_track(second));
    }

    fn audio(duration_milliseconds: u64) -> AudioProperties {
        AudioProperties {
            duration_milliseconds,
            sample_rate: 44_100,
            channel_count: 2,
        }
    }

    #[test]
    fn first_imported_audio_fills_the_audio_track_and_sets_the_project_length() {
        let mut project = Project::with_starter_tracks();

        let (media_id, track_id) = project.import_audio("C:/Music/song.mp3", audio(215_000));

        let track = project.track(track_id).unwrap();
        assert_eq!(track.name, "Audio");
        assert_eq!(track.clips.len(), 1);
        assert_eq!(track.clips[0].media, media_id);
        assert_eq!(track.clips[0].start_milliseconds, 0);
        assert_eq!(track.clips[0].duration_milliseconds, 215_000);
        assert_eq!(project.settings.duration_milliseconds, 215_000);
        assert_eq!(
            project.media_source(media_id).unwrap().file_name(),
            "song.mp3"
        );
        // No extra track was needed for a single audio file.
        assert_eq!(project.tracks().len(), 2);
    }

    #[test]
    fn further_audio_gets_its_own_track_and_only_ever_extends_the_project() {
        let mut project = Project::with_starter_tracks();
        project.import_audio("song.wav", audio(200_000));

        let (_, shorter_track) = project.import_audio("vocals.wav", audio(150_000));
        assert_eq!(project.settings.duration_milliseconds, 200_000);

        let (_, longer_track) = project.import_audio("extended.wav", audio(260_000));
        assert_eq!(project.settings.duration_milliseconds, 260_000);

        let audio_tracks: Vec<&str> = project
            .tracks()
            .iter()
            .filter(|track| track.kind == TrackKind::Audio)
            .map(|track| track.name.as_str())
            .collect();
        assert_eq!(audio_tracks, ["Audio", "Audio 2", "Audio 3"]);
        assert_ne!(shorter_track, longer_track);
        assert_eq!(project.media().len(), 3);
    }

    #[test]
    fn media_file_name_handles_both_path_separators() {
        let mut project = Project::new();
        let (windows, _) = project.import_audio(r"D:\Music\Album\track 01.flac", audio(1000));
        let (unix, _) = project.import_audio("/home/user/music/track 02.ogg", audio(1000));

        assert_eq!(
            project.media_source(windows).unwrap().file_name(),
            "track 01.flac"
        );
        assert_eq!(
            project.media_source(unix).unwrap().file_name(),
            "track 02.ogg"
        );
    }

    #[test]
    fn milkdrop_preset_fills_the_first_empty_layer_for_the_whole_project() {
        let mut project = Project::with_starter_tracks();
        project.settings.duration_milliseconds = 90_000;

        let (media_id, track_id) = project.add_milkdrop_preset("D:/presets/Geiss - Swirl.milk");

        let track = project.track(track_id).unwrap();
        assert_eq!(track.name, "Layer 1");
        assert_eq!(track.clips[0].media, media_id);
        assert_eq!(track.clips[0].start_milliseconds, 0);
        assert_eq!(track.clips[0].duration_milliseconds, 90_000);
        let source = project.media_source(media_id).unwrap();
        assert!(source.is_milkdrop_preset());
        assert!(source.audio().is_none());
        assert_eq!(source.file_name(), "Geiss - Swirl.milk");
    }

    #[test]
    fn preset_at_a_time_comes_from_the_topmost_layer_covering_it() {
        let mut project = Project::with_starter_tracks();
        project.settings.duration_milliseconds = 10_000;
        project.import_audio("song.wav", audio(10_000));
        assert!(project.milkdrop_preset_at(0).is_none());

        project.add_milkdrop_preset("bottom.milk"); // goes onto "Layer 1"
        project.add_milkdrop_preset("top.milk"); // new layer, added on top

        assert_eq!(project.milkdrop_preset_at(5_000).unwrap().path, "top.milk");
        // The clip covers 0..10000; its end is not part of it.
        assert!(project.milkdrop_preset_at(10_000).is_none());
    }

    #[test]
    fn rename_reports_whether_anything_changed() {
        let mut project = Project::new();
        let id = project.add_track("Old", TrackKind::Audio);

        assert!(project.rename_track(id, "New"));
        assert!(!project.rename_track(id, "New"));
        assert_eq!(project.track(id).unwrap().name, "New");
    }
}
