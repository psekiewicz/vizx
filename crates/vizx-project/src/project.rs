use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    AudioInput, AudioProperties, AudioReaction, CURRENT_FORMAT_VERSION, Clip, ClipId,
    ImageProperties, MediaContent, MediaId, MediaSource, Placement, ProjectMetadata,
    ProjectSettings, TextProperties, Track, TrackId, TrackKind, Transform,
};
use crate::{Easing, FrameAnimation, Keyframe};

/// Root of all data that belongs to one VizX project.
///
/// Everything that is saved into a `.vzx` file hangs off this struct. Future
/// milestones add media sources, clips, keyframes and effects here, so that
/// every subsystem works on the same shared data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Named moments on the timeline (`Chorus`, `Drop`), in order of time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    markers: Vec<Marker>,
}

/// A named moment on the timeline, to find one's way around a song.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    pub milliseconds: u64,
    pub name: String,
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
            markers: Vec::new(),
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

    /// The timeline's markers, in order of time.
    pub fn markers(&self) -> &[Marker] {
        &self.markers
    }

    /// Adds a marker at `milliseconds` (named `Marker 3` and so on if
    /// `name` is empty) and returns its place in [`Self::markers`]. A marker
    /// already at that moment is returned instead of a second one.
    pub fn add_marker(&mut self, milliseconds: u64, name: &str) -> usize {
        if let Some(index) = self
            .markers
            .iter()
            .position(|marker| marker.milliseconds.abs_diff(milliseconds) < 10)
        {
            return index;
        }
        let name = if name.trim().is_empty() {
            format!("Marker {}", self.markers.len() + 1)
        } else {
            name.trim().to_owned()
        };
        let index = self
            .markers
            .partition_point(|marker| marker.milliseconds <= milliseconds);
        self.markers.insert(index, Marker { milliseconds, name });
        index
    }

    /// Renames marker `index`. Returns `true` only if this changed
    /// something.
    pub fn rename_marker(&mut self, index: usize, name: &str) -> bool {
        match self.markers.get_mut(index) {
            Some(marker) if marker.name != name && !name.trim().is_empty() => {
                marker.name = name.trim().to_owned();
                true
            }
            _ => false,
        }
    }

    /// Moves marker `index` to `milliseconds`, keeping the markers in
    /// order. Returns its new place.
    pub fn move_marker(&mut self, index: usize, milliseconds: u64) -> Option<usize> {
        if index >= self.markers.len() {
            return None;
        }
        let mut marker = self.markers.remove(index);
        marker.milliseconds = milliseconds;
        let new_index = self
            .markers
            .partition_point(|other| other.milliseconds <= milliseconds);
        self.markers.insert(new_index, marker);
        Some(new_index)
    }

    /// Removes marker `index`. Returns `false` if there is no such marker.
    pub fn remove_marker(&mut self, index: usize) -> bool {
        if index < self.markers.len() {
            self.markers.remove(index);
            true
        } else {
            false
        }
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
            muted: false,
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
    ///
    /// Clips that reacted to the audio of the removed track go back to
    /// reacting to the whole mix, so nothing is left pointing at a track
    /// that no longer exists.
    pub fn remove_track(&mut self, id: TrackId) -> bool {
        let count_before = self.tracks.len();
        self.tracks.retain(|track| track.id != id);
        let was_removed = self.tracks.len() != count_before;

        if was_removed {
            for clip in self.tracks.iter_mut().flat_map(|track| &mut track.clips) {
                if clip.audio_input == AudioInput::Track(id) {
                    clip.audio_input = AudioInput::Mix;
                }
            }
        }
        was_removed
    }

    /// Moves a visual layer one step up or down among the visual layers,
    /// or all the way to the front or the back. A layer higher up is drawn
    /// over the ones below it. Returns `true` only if the layer moved.
    pub fn move_layer(&mut self, id: TrackId, to: LayerMove) -> bool {
        let layer_count = self
            .tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Visual)
            .count();
        // Visual layers come first in the list, so a layer's position in
        // the list is also its position among the layers.
        let Some(from) = self.tracks[..layer_count]
            .iter()
            .position(|track| track.id == id)
        else {
            return false;
        };
        let last = layer_count - 1;
        let target = match to {
            LayerMove::Up => from.saturating_sub(1),
            LayerMove::Down => (from + 1).min(last),
            LayerMove::ToFront => 0,
            LayerMove::ToBack => last,
        };
        if target == from {
            return false;
        }
        let layer = self.tracks.remove(from);
        self.tracks.insert(target, layer);
        true
    }

    /// A clip and the track it is on.
    pub fn clip(&self, id: ClipId) -> Option<(&Track, &Clip)> {
        self.tracks.iter().find_map(|track| {
            let clip = track.clips.iter().find(|clip| clip.id == id)?;
            Some((track, clip))
        })
    }

    /// How long the media of a clip is, in milliseconds, if it has a length
    /// of its own. Only audio has: a visualizer, an image or a text lasts as
    /// long as its clip, and a video starts over when it ends.
    fn media_length_milliseconds(&self, clip: &Clip) -> Option<u64> {
        let audio = self.media_source(clip.media)?.audio()?;
        Some(audio.duration_milliseconds)
    }

    /// Whether trimming a clip at its left edge skips the beginning of its
    /// media (audio, video) rather than just starting the clip later.
    fn has_source_time(&self, clip: &Clip) -> bool {
        self.media_source(clip.media)
            .is_some_and(|media| media.audio().is_some() || media.video().is_some())
    }

    /// Moves a clip along the timeline so that it starts at
    /// `start_milliseconds`. Returns `true` only if the clip moved.
    pub fn move_clip(&mut self, id: ClipId, start_milliseconds: u64) -> bool {
        let has_moved = match self.clip_mut(id) {
            Some(clip) if clip.start_milliseconds != start_milliseconds => {
                clip.start_milliseconds = start_milliseconds;
                true
            }
            _ => false,
        };
        self.grow_to_fit_clips();
        has_moved
    }

    /// Moves a clip onto another track of the same kind (a visual clip to
    /// another layer, an audio clip to another audio track), keeping its
    /// time. Returns `true` only if the clip changed tracks.
    pub fn move_clip_to_track(&mut self, id: ClipId, track_id: TrackId) -> bool {
        let Some((from, _)) = self.clip(id) else {
            return false;
        };
        let (from_id, from_kind) = (from.id, from.kind);
        let is_allowed = self
            .track(track_id)
            .is_some_and(|to| to.id != from_id && to.kind == from_kind);
        if !is_allowed {
            return false;
        }

        let mut moved = None;
        if let Some(from) = self.tracks.iter_mut().find(|track| track.id == from_id)
            && let Some(index) = from.clips.iter().position(|clip| clip.id == id)
        {
            moved = Some(from.clips.remove(index));
        }
        match (
            moved,
            self.tracks.iter_mut().find(|track| track.id == track_id),
        ) {
            (Some(clip), Some(to)) => {
                to.clips.push(clip);
                true
            }
            _ => false,
        }
    }

    /// Moves the left edge of a clip to `start_milliseconds`, keeping its
    /// right edge where it is. For audio and video this also changes where
    /// in the file the clip begins, so the rest of it stays in place.
    ///
    /// The edge stops where the clip would become too short, and where
    /// there is no more media before it. Returns `true` only if this
    /// changed something.
    pub fn trim_clip_start(&mut self, id: ClipId, start_milliseconds: u64) -> bool {
        let Some((_, clip)) = self.clip(id) else {
            return false;
        };
        let has_source_time = self.has_source_time(clip);
        let end = clip.end_milliseconds();
        // With media before the clip's start, the edge can go left by as
        // much as was skipped so far; otherwise it is free.
        let earliest = if has_source_time {
            clip.start_milliseconds
                .saturating_sub(clip.source_offset_milliseconds)
        } else {
            0
        };
        let latest = end.saturating_sub(Clip::MIN_DURATION_MILLISECONDS);
        let new_start = start_milliseconds.clamp(earliest.min(latest), latest);

        let Some(clip) = self.clip_mut(id) else {
            return false;
        };
        if new_start == clip.start_milliseconds {
            return false;
        }
        // Keyframes keep their moments in project time.
        clip.keyframes = clip.shifted_keyframes(clip.start_milliseconds as i64 - new_start as i64);
        if has_source_time {
            // Later start: more is skipped. Earlier start: less.
            clip.source_offset_milliseconds = (clip.source_offset_milliseconds + new_start)
                .saturating_sub(clip.start_milliseconds);
        }
        clip.start_milliseconds = new_start;
        clip.duration_milliseconds = end - new_start;
        true
    }

    /// Moves the right edge of a clip to `end_milliseconds`, keeping its
    /// left edge where it is. The edge stops where the clip would become
    /// too short, and (for audio) at the end of the file. Returns `true`
    /// only if this changed something.
    pub fn trim_clip_end(&mut self, id: ClipId, end_milliseconds: u64) -> bool {
        let Some((_, clip)) = self.clip(id) else {
            return false;
        };
        let start = clip.start_milliseconds;
        let shortest = Clip::MIN_DURATION_MILLISECONDS;
        let longest = self
            .media_length_milliseconds(clip)
            .map_or(u64::MAX, |length| {
                length.saturating_sub(clip.source_offset_milliseconds)
            })
            .max(shortest);
        let new_duration = end_milliseconds
            .saturating_sub(start)
            .clamp(shortest, longest);

        let has_changed = match self.clip_mut(id) {
            Some(clip) if clip.duration_milliseconds != new_duration => {
                clip.duration_milliseconds = new_duration;
                true
            }
            _ => false,
        };
        self.grow_to_fit_clips();
        has_changed
    }

    /// Cuts a clip in two at `at_milliseconds` of project time. The first
    /// part keeps the clip's id; the id of the new second part is returned.
    /// `None` if the moment is not inside the clip, or too close to one of
    /// its edges.
    ///
    /// Both parts keep the clip's settings. A text gets a copy of its own,
    /// so the two parts can say different things afterwards.
    pub fn split_clip(&mut self, id: ClipId, at_milliseconds: u64) -> Option<ClipId> {
        let (track, clip) = self.clip(id)?;
        let track_id = track.id;
        let shortest = Clip::MIN_DURATION_MILLISECONDS;
        if at_milliseconds < clip.start_milliseconds + shortest
            || at_milliseconds + shortest > clip.end_milliseconds()
        {
            return None;
        }
        let first_duration = at_milliseconds - clip.start_milliseconds;
        let has_source_time = self.has_source_time(clip);

        // Both parts keep all keyframes, so each continues the motion.
        let second_keyframes = clip.shifted_keyframes(-(first_duration as i64));
        let mut second = clip.clone();
        second.keyframes = second_keyframes;
        second.id = ClipId(self.next_clip_id);
        self.next_clip_id += 1;
        second.start_milliseconds = at_milliseconds;
        second.duration_milliseconds -= first_duration;
        if has_source_time {
            second.source_offset_milliseconds += first_duration;
        }
        second.media = self.own_media_for_copy(second.media);
        let second_id = second.id;

        let first = self.clip_mut(id)?;
        first.duration_milliseconds = first_duration;
        self.tracks
            .iter_mut()
            .find(|track| track.id == track_id)?
            .clips
            .push(second);
        Some(second_id)
    }

    /// Adds a copy of a clip right after it on the same track and returns
    /// the copy's id. A text gets a copy of its own to edit.
    pub fn duplicate_clip(&mut self, id: ClipId) -> Option<ClipId> {
        let (track, clip) = self.clip(id)?;
        let track_id = track.id;

        let mut copy = clip.clone();
        copy.start_milliseconds = clip.end_milliseconds();
        copy.id = ClipId(self.next_clip_id);
        self.next_clip_id += 1;
        copy.media = self.own_media_for_copy(copy.media);
        let copy_id = copy.id;

        self.tracks
            .iter_mut()
            .find(|track| track.id == track_id)?
            .clips
            .push(copy);
        self.grow_to_fit_clips();
        Some(copy_id)
    }

    /// Removes a clip. Media that no clip uses any more leaves the media
    /// library too. Returns `false` if no clip has this id.
    pub fn remove_clip(&mut self, id: ClipId) -> bool {
        let Some((_, clip)) = self.clip(id) else {
            return false;
        };
        let media_id = clip.media;
        for track in &mut self.tracks {
            track.clips.retain(|clip| clip.id != id);
        }

        let is_still_used = self
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .any(|clip| clip.media == media_id);
        if !is_still_used {
            self.media.retain(|source| source.id != media_id);
        }
        true
    }

    /// Pastes a copy of `clip`, copied earlier together with its media
    /// `source` (perhaps from another project), onto the track `track_id`
    /// starting at `start_milliseconds`. A file already in the media library
    /// is shared; a text, or a file the project does not have (any more),
    /// gets a media entry of its own. `None` if the track does not exist or
    /// is of the wrong kind (audio on a visual layer or the other way).
    pub fn paste_clip(
        &mut self,
        clip: &Clip,
        source: &MediaSource,
        track_id: TrackId,
        start_milliseconds: u64,
    ) -> Option<ClipId> {
        let wanted_kind = if source.audio().is_some() {
            TrackKind::Audio
        } else {
            TrackKind::Visual
        };
        if self.track(track_id)?.kind != wanted_kind {
            return None;
        }
        let shared = self
            .media
            .iter()
            .find(|media| {
                source.text().is_none()
                    && media.path == source.path
                    && media.content == source.content
            })
            .map(|media| media.id);
        let media_id = shared.unwrap_or_else(|| {
            let mut copy = source.clone();
            copy.id = MediaId(self.next_media_id);
            self.next_media_id += 1;
            let id = copy.id;
            self.media.push(copy);
            id
        });

        let mut copy = clip.clone();
        copy.id = ClipId(self.next_clip_id);
        self.next_clip_id += 1;
        copy.media = media_id;
        copy.start_milliseconds = start_milliseconds;
        let copy_id = copy.id;
        self.tracks
            .iter_mut()
            .find(|track| track.id == track_id)?
            .clips
            .push(copy);
        self.grow_to_fit_clips();
        Some(copy_id)
    }

    /// The media a copied clip should use: files are shared between clips,
    /// but a text is duplicated, because each copy is edited on its own.
    fn own_media_for_copy(&mut self, media_id: MediaId) -> MediaId {
        let Some(source) = self
            .media_source(media_id)
            .filter(|source| source.text().is_some())
        else {
            return media_id;
        };
        let mut copy = source.clone();
        copy.id = MediaId(self.next_media_id);
        self.next_media_id += 1;
        let copy_id = copy.id;
        self.media.push(copy);
        copy_id
    }

    /// Makes the project long enough to contain every clip. It never gets
    /// shorter this way.
    fn grow_to_fit_clips(&mut self) {
        let last_end = self
            .tracks
            .iter()
            .map(Track::end_milliseconds)
            .max()
            .unwrap_or(0);
        self.settings.duration_milliseconds = self.settings.duration_milliseconds.max(last_end);
    }

    /// The audio tracks, from top to bottom.
    pub fn audio_tracks(&self) -> impl Iterator<Item = &Track> {
        self.tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Audio)
    }

    /// Chooses which audio a clip reacts to. Returns `true` only if this
    /// changed something. An input naming a track that is not an audio
    /// track of this project is refused.
    pub fn set_clip_audio_input(&mut self, clip_id: ClipId, input: AudioInput) -> bool {
        if let AudioInput::Track(track_id) = input
            && !self.audio_tracks().any(|track| track.id == track_id)
        {
            return false;
        }

        let clip = self
            .tracks
            .iter_mut()
            .flat_map(|track| &mut track.clips)
            .find(|clip| clip.id == clip_id);
        match clip {
            Some(clip) if clip.audio_input != input => {
                clip.audio_input = input;
                true
            }
            _ => false,
        }
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
            manual_beats: None,
            tempo_bpm: None,
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
            source_offset_milliseconds: 0,
            audio_gain: 1.0,
            audio_input: AudioInput::Mix,
            placement: Placement::FULL,
            parameters: BTreeMap::new(),
            opacity: 1.0,
            reaction: AudioReaction::default(),
            rotation_degrees: 0.0,
            spin_degrees_per_second: 0.0,
            keyframes: Vec::new(),
            sharp_pixels: None,
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

    /// Adds a stem of the song in `like_clip` (its vocals, say) on a new,
    /// muted audio track named `name`, timed exactly like that clip, so
    /// visuals can follow just that part of the song. Returns the track.
    pub fn add_stem(
        &mut self,
        path: impl Into<String>,
        properties: AudioProperties,
        name: &str,
        like_clip: ClipId,
    ) -> Option<TrackId> {
        let (_, original) = self.clip(like_clip)?;
        let timing = (
            original.start_milliseconds,
            original.duration_milliseconds,
            original.source_offset_milliseconds,
        );
        let media_id = MediaId(self.next_media_id);
        self.next_media_id += 1;
        self.media.push(MediaSource {
            id: media_id,
            path: path.into(),
            content: MediaContent::Audio(properties),
            manual_beats: None,
            tempo_bpm: None,
        });
        let track_id = self.add_track(name, TrackKind::Audio);
        let clip = Clip {
            id: ClipId(self.next_clip_id),
            media: media_id,
            start_milliseconds: timing.0,
            duration_milliseconds: timing.1,
            source_offset_milliseconds: timing.2,
            audio_gain: 1.0,
            audio_input: AudioInput::Mix,
            placement: Placement::FULL,
            parameters: BTreeMap::new(),
            opacity: 1.0,
            reaction: AudioReaction::default(),
            rotation_degrees: 0.0,
            spin_degrees_per_second: 0.0,
            keyframes: Vec::new(),
            sharp_pixels: None,
        };
        self.next_clip_id += 1;
        let track = self.tracks.iter_mut().find(|track| track.id == track_id)?;
        track.muted = true;
        track.clips.push(clip);
        Some(track_id)
    }

    /// Mutes or unmutes an audio track (see `Track::muted`). Returns
    /// `true` only if this changed something.
    pub fn set_track_muted(&mut self, track_id: TrackId, muted: bool) -> bool {
        match self.tracks.iter_mut().find(|track| track.id == track_id) {
            Some(track) if track.kind == TrackKind::Audio && track.muted != muted => {
                track.muted = muted;
                true
            }
            _ => false,
        }
    }

    /// Adds a MilkDrop preset to the media library and places it on a
    /// visual layer, covering the whole project.
    ///
    /// Like audio, it goes onto the first layer that is still empty, and a
    /// new layer is created only if all of them are in use.
    pub fn add_milkdrop_preset(&mut self, path: impl Into<String>) -> (MediaId, TrackId) {
        self.add_visual(path.into(), MediaContent::MilkdropPreset)
    }

    /// Adds a visualizer plugin (its folder) to the media library and
    /// places it on a visual layer, like [`Project::add_milkdrop_preset`].
    pub fn add_visualizer_plugin(&mut self, path: impl Into<String>) -> (MediaId, TrackId) {
        self.add_visual(path.into(), MediaContent::VisualizerPlugin)
    }

    /// Adds an image to the media library and places it on a visual layer
    /// for the whole project. It is shown as large as fits into the frame
    /// without changing its shape, in the middle.
    pub fn add_image(
        &mut self,
        path: impl Into<String>,
        properties: ImageProperties,
    ) -> (MediaId, TrackId) {
        let (media_id, track_id) = self.add_visual(path.into(), MediaContent::Image(properties));
        let placement = fitted_placement(
            (properties.width, properties.height),
            (self.settings.width, self.settings.height),
        );
        let new_clip = self
            .tracks
            .iter_mut()
            .flat_map(|track| &mut track.clips)
            .find(|clip| clip.media == media_id);
        if let Some(clip) = new_clip {
            clip.placement = placement;
        }
        (media_id, track_id)
    }

    /// Adds an image as the background of the picture: on a new layer below
    /// all other layers, enlarged until it covers the whole frame without
    /// changing its shape (what sticks out is not visible).
    pub fn add_background_image(
        &mut self,
        path: impl Into<String>,
        properties: ImageProperties,
    ) -> (MediaId, TrackId) {
        self.add_background(path.into(), MediaContent::Image(properties), properties)
    }

    /// Adds a video as the background, like
    /// [`Project::add_background_image`].
    pub fn add_background_video(
        &mut self,
        path: impl Into<String>,
        properties: ImageProperties,
    ) -> (MediaId, TrackId) {
        self.add_background(path.into(), MediaContent::Video(properties), properties)
    }

    /// Adds a video to the media library and places it on a visual layer
    /// for the whole project, fitted into the frame like an image.
    pub fn add_video(
        &mut self,
        path: impl Into<String>,
        properties: ImageProperties,
    ) -> (MediaId, TrackId) {
        let (media_id, track_id) = self.add_visual(path.into(), MediaContent::Video(properties));
        let placement = fitted_placement(
            (properties.width, properties.height),
            (self.settings.width, self.settings.height),
        );
        let new_clip = self
            .tracks
            .iter_mut()
            .flat_map(|track| &mut track.clips)
            .find(|clip| clip.media == media_id);
        if let Some(clip) = new_clip {
            clip.placement = placement;
        }
        (media_id, track_id)
    }

    /// Adds a frame animation (image sequence or sprite sheet) on a visual
    /// layer for the whole project, fitted into the frame like an image.
    pub fn add_animation(
        &mut self,
        path: impl Into<String>,
        animation: FrameAnimation,
    ) -> (MediaId, TrackId) {
        let animation = animation.sanitized();
        let frame_size = (animation.frame_width, animation.frame_height);
        let (media_id, track_id) = self.add_visual(path.into(), MediaContent::Animation(animation));
        let placement = fitted_placement(frame_size, (self.settings.width, self.settings.height));
        if let Some(clip) = self.clip_of_media_mut(media_id) {
            clip.placement = placement;
        }
        (media_id, track_id)
    }

    /// Changes how a frame animation is laid out or played. Returns `true`
    /// only if this changed something.
    pub fn set_animation(&mut self, media_id: MediaId, animation: FrameAnimation) -> bool {
        let animation = animation.sanitized();
        let Some(source) = self.media.iter_mut().find(|source| source.id == media_id) else {
            return false;
        };
        let MediaContent::Animation(current) = &mut source.content else {
            return false;
        };
        let has_changed = *current != animation;
        *current = animation;
        has_changed
    }

    fn add_background(
        &mut self,
        path: String,
        content: MediaContent,
        properties: ImageProperties,
    ) -> (MediaId, TrackId) {
        let media_id = MediaId(self.next_media_id);
        self.next_media_id += 1;
        self.media.push(MediaSource {
            id: media_id,
            path,
            content,
            manual_beats: None,
            tempo_bpm: None,
        });

        let track_id = self.add_track("Background", TrackKind::Visual);
        self.move_layer(track_id, LayerMove::ToBack);

        let clip = Clip {
            id: ClipId(self.next_clip_id),
            media: media_id,
            start_milliseconds: 0,
            duration_milliseconds: self.settings.duration_milliseconds,
            source_offset_milliseconds: 0,
            audio_gain: 1.0,
            audio_input: AudioInput::Mix,
            placement: covering_placement(
                (properties.width, properties.height),
                (self.settings.width, self.settings.height),
            ),
            parameters: BTreeMap::new(),
            opacity: 1.0,
            reaction: AudioReaction::default(),
            rotation_degrees: 0.0,
            spin_degrees_per_second: 0.0,
            keyframes: Vec::new(),
            sharp_pixels: None,
        };
        self.next_clip_id += 1;
        if let Some(track) = self.tracks.iter_mut().find(|track| track.id == track_id) {
            track.clips.push(clip);
        }
        (media_id, track_id)
    }

    fn add_visual(&mut self, path: String, content: MediaContent) -> (MediaId, TrackId) {
        let media_id = MediaId(self.next_media_id);
        self.next_media_id += 1;
        self.media.push(MediaSource {
            id: media_id,
            path,
            content,
            manual_beats: None,
            tempo_bpm: None,
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
            source_offset_milliseconds: 0,
            audio_gain: 1.0,
            audio_input: AudioInput::Mix,
            placement: Placement::FULL,
            parameters: BTreeMap::new(),
            opacity: 1.0,
            reaction: AudioReaction::default(),
            rotation_degrees: 0.0,
            spin_degrees_per_second: 0.0,
            keyframes: Vec::new(),
            sharp_pixels: None,
        };
        self.next_clip_id += 1;
        if let Some(track) = self.tracks.iter_mut().find(|track| track.id == track_id) {
            track.clips.push(clip);
        }

        (media_id, track_id)
    }

    /// Every clip that produces a picture (MilkDrop preset or visualizer
    /// plugin) with its media source, from the topmost visual layer down.
    pub fn visual_clips(&self) -> impl Iterator<Item = (&Clip, &MediaSource)> {
        self.tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Visual)
            .flat_map(|track| &track.clips)
            .filter_map(|clip| Some((clip, self.media_source(clip.media)?)))
            .filter(|(_, source)| source.is_visual())
    }

    /// Every MilkDrop preset clip with its preset, from the topmost visual
    /// layer down.
    pub fn milkdrop_clips(&self) -> impl Iterator<Item = (&Clip, &MediaSource)> {
        self.visual_clips()
            .filter(|(_, source)| source.is_milkdrop_preset())
    }

    /// The MilkDrop clip shown at a point in time: the one on the topmost
    /// visual layer that covers that time.
    pub fn milkdrop_clip_at(&self, milliseconds: u64) -> Option<(&Clip, &MediaSource)> {
        self.milkdrop_clips().find(|(clip, _)| {
            milliseconds >= clip.start_milliseconds && milliseconds < clip.end_milliseconds()
        })
    }

    /// Every visual clip that covers a point in time, with its layer and
    /// media source, from the topmost visual layer down. All of them are
    /// part of the picture at that time.
    pub fn visual_layers_at(&self, milliseconds: u64) -> Vec<(&Track, &Clip, &MediaSource)> {
        self.tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Visual)
            .flat_map(|track| track.clips.iter().map(move |clip| (track, clip)))
            .filter(|(_, clip)| {
                milliseconds >= clip.start_milliseconds && milliseconds < clip.end_milliseconds()
            })
            .filter_map(|(track, clip)| Some((track, clip, self.media_source(clip.media)?)))
            .filter(|(_, _, source)| source.is_visual())
            .collect()
    }

    /// Moves or resizes a clip in the picture. The placement is corrected
    /// to a valid one first. Returns `true` only if this changed something.
    pub fn set_clip_placement(&mut self, clip_id: ClipId, placement: Placement) -> bool {
        let placement = placement.sanitized();
        let clip = self
            .tracks
            .iter_mut()
            .flat_map(|track| &mut track.clips)
            .find(|clip| clip.id == clip_id);
        match clip {
            Some(clip) if clip.placement != placement => {
                clip.placement = placement;
                true
            }
            _ => false,
        }
    }

    /// Adds a text element on a visual layer for the whole project, at the
    /// given place in the picture.
    pub fn add_text(
        &mut self,
        properties: TextProperties,
        placement: Placement,
    ) -> (MediaId, TrackId) {
        let (media_id, track_id) =
            self.add_visual(String::new(), MediaContent::Text(properties.sanitized()));
        if let Some(clip) = self.clip_of_media_mut(media_id) {
            clip.placement = placement.sanitized();
        }
        (media_id, track_id)
    }

    /// Adds a text element on the visual layer `track_id`, from
    /// `start_milliseconds` for `duration_milliseconds` (lines of lyrics
    /// are added this way, one after another on one layer). `None` if there
    /// is no such visual layer.
    pub fn add_text_clip(
        &mut self,
        track_id: TrackId,
        properties: TextProperties,
        placement: Placement,
        start_milliseconds: u64,
        duration_milliseconds: u64,
    ) -> Option<ClipId> {
        if self.track(track_id)?.kind != TrackKind::Visual {
            return None;
        }
        let media_id = MediaId(self.next_media_id);
        self.next_media_id += 1;
        self.media.push(MediaSource {
            id: media_id,
            path: String::new(),
            content: MediaContent::Text(properties.sanitized()),
            manual_beats: None,
            tempo_bpm: None,
        });
        let clip_id = ClipId(self.next_clip_id);
        self.next_clip_id += 1;
        let clip = Clip {
            id: clip_id,
            media: media_id,
            start_milliseconds,
            duration_milliseconds: duration_milliseconds.max(Clip::MIN_DURATION_MILLISECONDS),
            source_offset_milliseconds: 0,
            audio_gain: 1.0,
            audio_input: AudioInput::Mix,
            placement: placement.sanitized(),
            parameters: BTreeMap::new(),
            opacity: 1.0,
            reaction: AudioReaction::default(),
            rotation_degrees: 0.0,
            spin_degrees_per_second: 0.0,
            keyframes: Vec::new(),
            sharp_pixels: None,
        };
        self.tracks
            .iter_mut()
            .find(|track| track.id == track_id)?
            .clips
            .push(clip);
        self.grow_to_fit_clips();
        Some(clip_id)
    }

    /// Replaces all keyframes of a clip (they are put in order of time).
    /// Returns `true` only if this changed something.
    pub fn set_clip_keyframes(&mut self, clip_id: ClipId, mut keyframes: Vec<Keyframe>) -> bool {
        for keyframe in &mut keyframes {
            keyframe.transform = keyframe.transform.sanitized();
        }
        keyframes.sort_by_key(|keyframe| keyframe.time_milliseconds);
        match self.clip_mut(clip_id) {
            Some(clip) if clip.keyframes != keyframes => {
                clip.keyframes = keyframes;
                true
            }
            _ => false,
        }
    }

    /// Changes what a text element says or how it looks, and the place it
    /// takes up in the picture (its size follows from the text). Returns
    /// `true` only if this changed something.
    pub fn set_text(
        &mut self,
        media_id: MediaId,
        properties: TextProperties,
        placement: Placement,
    ) -> bool {
        let properties = properties.sanitized();
        let placement = placement.sanitized();
        let Some(source) = self.media.iter_mut().find(|source| source.id == media_id) else {
            return false;
        };
        let MediaContent::Text(current) = &mut source.content else {
            return false;
        };
        let has_text_changed = *current != properties;
        *current = properties;

        let mut has_placement_changed = false;
        if let Some(clip) = self.clip_of_media_mut(media_id)
            && clip.placement != placement
        {
            // Keyframes keep their own size relative to the text, around
            // their own middle.
            let width_ratio = placement.width / clip.placement.width.max(1e-9);
            let height_ratio = placement.height / clip.placement.height.max(1e-9);
            for keyframe in &mut clip.keyframes {
                let old = keyframe.transform.placement;
                let (width, height) = (old.width * width_ratio, old.height * height_ratio);
                keyframe.transform.placement = Placement {
                    x: old.x + (old.width - width) / 2.0,
                    y: old.y + (old.height - height) / 2.0,
                    width,
                    height,
                }
                .sanitized();
            }
            clip.placement = placement;
            has_placement_changed = true;
        }
        has_text_changed || has_placement_changed
    }

    fn clip_of_media_mut(&mut self, media_id: MediaId) -> Option<&mut Clip> {
        self.tracks
            .iter_mut()
            .flat_map(|track| &mut track.clips)
            .find(|clip| clip.media == media_id)
    }

    /// Changes a visual clip's state (place, size, rotation, opacity) at
    /// `project_milliseconds`. Without keyframes this is its resting state;
    /// with keyframes the keyframe at that moment is changed, or made.
    /// Returns `true` only if something changed.
    pub fn edit_clip_transform_at(
        &mut self,
        id: ClipId,
        project_milliseconds: u64,
        change: impl FnOnce(&mut Transform),
    ) -> bool {
        let Some(clip) = self.clip_mut(id) else {
            return false;
        };
        let moment = moment_in_clip(clip, project_milliseconds);
        let mut transform = clip.transform_at(moment as f64);
        change(&mut transform);
        clip.set_transform_at(moment, transform)
    }

    /// Records a clip's current state at `project_milliseconds` as a
    /// keyframe. Returns `false` if there already is one there.
    pub fn add_keyframe(&mut self, id: ClipId, project_milliseconds: u64) -> bool {
        self.clip_mut(id).is_some_and(|clip| {
            let moment = moment_in_clip(clip, project_milliseconds);
            clip.add_keyframe_at(moment)
        })
    }

    /// Removes the keyframe at `project_milliseconds`. Returns `false` if
    /// there is none there.
    pub fn remove_keyframe(&mut self, id: ClipId, project_milliseconds: u64) -> bool {
        self.clip_mut(id).is_some_and(|clip| {
            let moment = moment_in_clip(clip, project_milliseconds);
            clip.remove_keyframe_at(moment)
        })
    }

    /// Removes all keyframes of a clip; it keeps the state it has at
    /// `project_milliseconds`. Returns `false` if it had none.
    pub fn clear_keyframes(&mut self, id: ClipId, project_milliseconds: u64) -> bool {
        let Some(clip) = self.clip_mut(id) else {
            return false;
        };
        if clip.keyframes.is_empty() {
            return false;
        }
        let kept = clip.transform_at(moment_in_clip(clip, project_milliseconds) as f64);
        clip.keyframes.clear();
        clip.set_transform_at(0, kept);
        true
    }

    /// Sets how the values travel to the keyframe at
    /// `project_milliseconds`. Returns `true` only if something changed.
    pub fn set_keyframe_easing(
        &mut self,
        id: ClipId,
        project_milliseconds: u64,
        easing: Easing,
    ) -> bool {
        self.clip_mut(id).is_some_and(|clip| {
            let moment = moment_in_clip(clip, project_milliseconds);
            clip.set_keyframe_easing_at(moment, easing)
        })
    }

    /// Sets how fast a visual clip keeps turning, in degrees per second.
    /// Returns `true` only if this changed something.
    pub fn set_clip_spin(&mut self, id: ClipId, degrees_per_second: f64) -> bool {
        let spin = if degrees_per_second.is_finite() {
            degrees_per_second.clamp(-3600.0, 3600.0)
        } else {
            0.0
        };
        match self.clip_mut(id) {
            Some(clip) if clip.spin_degrees_per_second != spin => {
                clip.spin_degrees_per_second = spin;
                true
            }
            _ => false,
        }
    }

    /// Uses beats placed by hand for an audio file instead of the ones
    /// found automatically: `beats` (seconds in the file) to start from,
    /// for example a copy of the found ones, or none. Returns `true` only
    /// if this changed something.
    pub fn use_manual_beats(&mut self, media_id: MediaId, mut beats: Vec<f64>) -> bool {
        beats.retain(|seconds| seconds.is_finite() && *seconds >= 0.0);
        beats.sort_by(f64::total_cmp);
        beats.dedup_by(|a, b| (*a - *b).abs() < MANUAL_BEAT_MERGE_SECONDS);
        self.set_manual_beats(media_id, Some(beats))
    }

    /// Goes back to the beats found automatically for an audio file. Its
    /// hand-placed beats are forgotten. Returns `true` only if it had any.
    pub fn use_automatic_beats(&mut self, media_id: MediaId) -> bool {
        self.set_manual_beats(media_id, None)
    }

    /// Adds a hand-placed beat at `seconds` of an audio file. Nothing
    /// happens if the file uses automatic beats, or if there already is a
    /// beat right there. Returns `true` only if a beat was added.
    pub fn add_manual_beat(&mut self, media_id: MediaId, seconds: f64) -> bool {
        let Some(beats) = self.manual_beats_mut(media_id) else {
            return false;
        };
        if !seconds.is_finite() || seconds < 0.0 {
            return false;
        }
        let index = beats.partition_point(|beat| *beat < seconds);
        let is_taken = [index.checked_sub(1), Some(index)]
            .into_iter()
            .flatten()
            .filter_map(|neighbour| beats.get(neighbour))
            .any(|beat| (beat - seconds).abs() < MANUAL_BEAT_MERGE_SECONDS);
        if is_taken {
            return false;
        }
        beats.insert(index, seconds);
        true
    }

    /// Removes the hand-placed beat nearest to `seconds` of an audio file,
    /// if it is within `reach_seconds`. Returns `true` only if one was
    /// removed.
    pub fn remove_manual_beat_near(
        &mut self,
        media_id: MediaId,
        seconds: f64,
        reach_seconds: f64,
    ) -> bool {
        let Some(beats) = self.manual_beats_mut(media_id) else {
            return false;
        };
        let nearest = (0..beats.len()).min_by(|a, b| {
            (beats[*a] - seconds)
                .abs()
                .total_cmp(&(beats[*b] - seconds).abs())
        });
        match nearest {
            Some(index) if (beats[index] - seconds).abs() <= reach_seconds => {
                beats.remove(index);
                true
            }
            _ => false,
        }
    }

    /// Sets the tempo of an audio file, in beats per minute (`None`: the
    /// tempo found automatically). Returns `true` only if this changed
    /// something.
    pub fn set_audio_tempo(&mut self, media_id: MediaId, tempo_bpm: Option<f64>) -> bool {
        let tempo = tempo_bpm
            .filter(|tempo| tempo.is_finite())
            .map(|tempo| tempo.clamp(30.0, 300.0));
        let Some(source) = self
            .media
            .iter_mut()
            .find(|source| source.id == media_id && source.audio().is_some())
        else {
            return false;
        };
        let has_changed = source.tempo_bpm != tempo;
        source.tempo_bpm = tempo;
        has_changed
    }

    fn set_manual_beats(&mut self, media_id: MediaId, beats: Option<Vec<f64>>) -> bool {
        let Some(source) = self
            .media
            .iter_mut()
            .find(|source| source.id == media_id && source.audio().is_some())
        else {
            return false;
        };
        let has_changed = source.manual_beats != beats;
        source.manual_beats = beats;
        has_changed
    }

    fn manual_beats_mut(&mut self, media_id: MediaId) -> Option<&mut Vec<f64>> {
        self.media
            .iter_mut()
            .find(|source| source.id == media_id)?
            .manual_beats
            .as_mut()
    }

    /// Sets how strongly a visual clip hears its audio (see
    /// `Clip::audio_gain`). Returns `true` only if this changed something.
    pub fn set_clip_audio_gain(&mut self, clip_id: ClipId, gain: f64) -> bool {
        let (quietest, loudest) = Clip::AUDIO_GAIN_RANGE;
        let gain = if gain.is_finite() {
            gain.clamp(quietest, loudest)
        } else {
            1.0
        };
        match self.clip_mut(clip_id) {
            Some(clip) if clip.audio_gain != gain => {
                clip.audio_gain = gain;
                true
            }
            _ => false,
        }
    }

    /// Sets how opaque a clip is as a whole (0 to 1). Returns `true` only
    /// if this changed something.
    pub fn set_clip_opacity(&mut self, clip_id: ClipId, opacity: f64) -> bool {
        let opacity = if opacity.is_finite() {
            opacity.clamp(0.0, 1.0)
        } else {
            1.0
        };
        match self.clip_mut(clip_id) {
            Some(clip) if clip.opacity != opacity => {
                clip.opacity = opacity;
                true
            }
            _ => false,
        }
    }

    /// Whether a clip's picture keeps sharp pixels when enlarged: its own
    /// choice, or else what suits its media.
    pub fn clip_sharp_pixels(&self, clip: &Clip) -> bool {
        clip.sharp_pixels.unwrap_or_else(|| {
            self.media_source(clip.media)
                .is_some_and(MediaSource::sharp_pixels_by_default)
        })
    }

    /// Sets whether a clip's picture keeps sharp pixels when enlarged.
    /// Returns `true` only if this changed something.
    pub fn set_clip_sharp_pixels(&mut self, clip_id: ClipId, sharp: bool) -> bool {
        match self.clip_mut(clip_id) {
            Some(clip) if clip.sharp_pixels != Some(sharp) => {
                clip.sharp_pixels = Some(sharp);
                true
            }
            _ => false,
        }
    }

    /// Sets how a clip reacts to the sound. Returns `true` only if this
    /// changed something.
    pub fn set_clip_reaction(&mut self, clip_id: ClipId, reaction: AudioReaction) -> bool {
        let reaction = reaction.sanitized();
        match self.clip_mut(clip_id) {
            Some(clip) if clip.reaction != reaction => {
                clip.reaction = reaction;
                true
            }
            _ => false,
        }
    }

    fn clip_mut(&mut self, clip_id: ClipId) -> Option<&mut Clip> {
        self.tracks
            .iter_mut()
            .flat_map(|track| &mut track.clips)
            .find(|clip| clip.id == clip_id)
    }

    /// Sets the values of one plugin parameter on a clip. Returns `true`
    /// only if this changed something.
    pub fn set_clip_parameter(&mut self, clip_id: ClipId, id: &str, values: Vec<f64>) -> bool {
        let clip = self
            .tracks
            .iter_mut()
            .flat_map(|track| &mut track.clips)
            .find(|clip| clip.id == clip_id);
        match clip {
            Some(clip) if clip.parameters.get(id) != Some(&values) => {
                clip.parameters.insert(id.to_owned(), values);
                true
            }
            _ => false,
        }
    }

    /// Forgets all parameter values of a clip, so the plugin's defaults
    /// apply again. Returns `true` only if there was something to forget.
    pub fn reset_clip_parameters(&mut self, clip_id: ClipId) -> bool {
        let clip = self
            .tracks
            .iter_mut()
            .flat_map(|track| &mut track.clips)
            .find(|clip| clip.id == clip_id);
        match clip {
            Some(clip) if !clip.parameters.is_empty() => {
                clip.parameters.clear();
                true
            }
            _ => false,
        }
    }

    /// The MilkDrop preset shown at a point in time.
    pub fn milkdrop_preset_at(&self, milliseconds: u64) -> Option<&MediaSource> {
        self.milkdrop_clip_at(milliseconds)
            .map(|(_, source)| source)
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

/// Two hand-placed beats closer than this are one beat, in seconds.
const MANUAL_BEAT_MERGE_SECONDS: f64 = 0.03;

/// A moment of the project as a moment of the clip, kept within the clip.
fn moment_in_clip(clip: &Clip, project_milliseconds: u64) -> u64 {
    project_milliseconds
        .saturating_sub(clip.start_milliseconds)
        .min(clip.duration_milliseconds)
}

/// Where a layer is moved to by [`Project::move_layer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerMove {
    /// One step towards the front (drawn over one more layer).
    Up,
    /// One step towards the back.
    Down,
    /// In front of all other layers.
    ToFront,
    /// Behind all other layers.
    ToBack,
}

/// The largest placement with the shape of `content` that fits into a
/// frame of size `frame` (both in pixels), centered.
fn fitted_placement(content: (u32, u32), frame: (u32, u32)) -> Placement {
    scaled_placement(content, frame, f64::min)
}

/// The smallest placement with the shape of `content` that covers a frame
/// of size `frame` completely, centered. Part of it lies outside the frame
/// unless both have the same shape.
fn covering_placement(content: (u32, u32), frame: (u32, u32)) -> Placement {
    scaled_placement(content, frame, f64::max)
}

/// A centered placement with the shape of `content`. `choose` picks one of
/// the two scale factors (fit the width, fit the height).
fn scaled_placement(
    content: (u32, u32),
    frame: (u32, u32),
    choose: fn(f64, f64) -> f64,
) -> Placement {
    let (content_width, content_height) = (f64::from(content.0), f64::from(content.1));
    let (frame_width, frame_height) = (f64::from(frame.0), f64::from(frame.1));
    if content_width <= 0.0 || content_height <= 0.0 || frame_width <= 0.0 || frame_height <= 0.0 {
        return Placement::FULL;
    }

    let scale = choose(frame_width / content_width, frame_height / content_height);
    let width = content_width * scale / frame_width;
    let height = content_height * scale / frame_height;
    Placement {
        x: (1.0 - width) / 2.0,
        y: (1.0 - height) / 2.0,
        width,
        height,
    }
    .sanitized()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FrameRate, ReactionBand, ReactionEffect};

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
    fn markers_stay_in_order_and_get_names() {
        let mut project = Project::new();
        assert_eq!(project.add_marker(5000, "Chorus"), 0);
        assert_eq!(project.add_marker(1000, ""), 0);
        // Again at (nearly) the same moment: the same marker.
        assert_eq!(project.add_marker(1004, "Other"), 0);
        assert_eq!(project.markers()[0].name, "Marker 2");

        assert!(project.rename_marker(0, " Intro "));
        assert_eq!(project.move_marker(0, 9000), Some(1));
        let names: Vec<&str> = project
            .markers()
            .iter()
            .map(|marker| marker.name.as_str())
            .collect();
        assert_eq!(names, ["Chorus", "Intro"]);
        assert!(project.remove_marker(0));
        assert!(!project.remove_marker(5));

        let reloaded = Project::from_vzx_str(&project.to_vzx_string().unwrap()).unwrap();
        assert_eq!(reloaded.markers(), project.markers());
    }

    #[test]
    fn a_stem_goes_on_a_muted_track_timed_like_its_song() {
        let mut project = Project::new();
        let (_, song_track) = project.import_audio("song.mp3", audio(60_000));
        let song_clip = project.track(song_track).unwrap().clips[0].id;
        project.move_clip(song_clip, 2000);
        project.trim_clip_start(song_clip, 3000);

        let stem_track = project
            .add_stem("song - Vocals.wav", audio(60_000), "Vocals", song_clip)
            .unwrap();

        let (_, song) = project.clip(song_clip).unwrap();
        let stem = project.track(stem_track).unwrap();
        assert_eq!(stem.name, "Vocals");
        assert!(stem.muted);
        let timing = |clip: &Clip| {
            (
                clip.start_milliseconds,
                clip.duration_milliseconds,
                clip.source_offset_milliseconds,
            )
        };
        assert_eq!(timing(&stem.clips[0]), timing(song));
        // Muting works on audio tracks only.
        assert!(project.set_track_muted(stem_track, false));
        let layer = project.add_default_named_track(TrackKind::Visual);
        assert!(!project.set_track_muted(layer, true));
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
    fn a_visualizer_can_react_to_one_audio_track() {
        let mut project = Project::with_starter_tracks();
        let (_, vocals) = project.import_audio("vocals.wav", audio(10_000));
        let (_, bass) = project.import_audio("bass.wav", audio(10_000));
        let (_, layer) = project.add_milkdrop_preset("a.milk");
        let clip_id = project.track(layer).unwrap().clips[0].id;
        let input_of = |project: &Project| project.track(layer).unwrap().clips[0].audio_input;

        // A new visualizer reacts to everything.
        assert_eq!(input_of(&project), AudioInput::Mix);

        assert!(project.set_clip_audio_input(clip_id, AudioInput::Track(bass)));
        assert!(!project.set_clip_audio_input(clip_id, AudioInput::Track(bass)));
        assert_eq!(input_of(&project), AudioInput::Track(bass));

        // A visual layer is not an audio input.
        assert!(!project.set_clip_audio_input(clip_id, AudioInput::Track(layer)));
        assert_eq!(input_of(&project), AudioInput::Track(bass));

        // Removing another track changes nothing; removing the chosen one
        // falls back to the mix.
        project.remove_track(vocals);
        assert_eq!(input_of(&project), AudioInput::Track(bass));
        project.remove_track(bass);
        assert_eq!(input_of(&project), AudioInput::Mix);
    }

    #[test]
    fn a_clip_can_be_moved_and_resized_within_limits() {
        let mut project = Project::with_starter_tracks();
        project.settings.duration_milliseconds = 10_000;
        let (_, layer) = project.add_milkdrop_preset("a.milk");
        let clip_id = project.track(layer).unwrap().clips[0].id;
        let placement_of = |project: &Project| project.track(layer).unwrap().clips[0].placement;

        assert_eq!(placement_of(&project), Placement::FULL);

        let left_half = Placement {
            x: 0.0,
            y: 0.0,
            width: 0.5,
            height: 1.0,
        };
        assert!(project.set_clip_placement(clip_id, left_half));
        assert!(!project.set_clip_placement(clip_id, left_half));
        assert_eq!(placement_of(&project), left_half);

        // Too small and far outside the frame: corrected, not refused.
        let lost = Placement {
            x: 50.0,
            y: -50.0,
            width: 0.0,
            height: 100.0,
        };
        assert!(project.set_clip_placement(clip_id, lost));
        let corrected = placement_of(&project);
        assert_eq!(corrected.width, Placement::MIN_SIZE);
        assert_eq!(corrected.height, Placement::MAX_SIZE);
        // A sliver stays inside the frame on both axes.
        assert!(corrected.x < 1.0 && corrected.right() > 0.0);
        assert!(corrected.y < 1.0 && corrected.bottom() > 0.0);

        let not_a_number = Placement {
            x: f64::NAN,
            ..left_half
        };
        project.set_clip_placement(clip_id, not_a_number);
        assert_eq!(placement_of(&project), Placement::FULL);
    }

    #[test]
    fn all_layers_covering_a_time_are_part_of_the_picture() {
        let mut project = Project::with_starter_tracks();
        project.settings.duration_milliseconds = 10_000;
        project.add_milkdrop_preset("bottom.milk");
        project.add_milkdrop_preset("top.milk");

        let paths: Vec<&str> = project
            .visual_layers_at(5_000)
            .iter()
            .map(|(_, _, preset)| preset.path.as_str())
            .collect();

        assert_eq!(paths, ["top.milk", "bottom.milk"]);
        assert!(project.visual_layers_at(10_000).is_empty());
    }

    #[test]
    fn plugin_parameters_are_stored_per_clip_and_survive_saving() {
        let mut project = Project::with_starter_tracks();
        project.settings.duration_milliseconds = 10_000;
        let (media_id, layer) = project.add_visualizer_plugin("bundled-visualizers:/bars");
        let clip_id = project.track(layer).unwrap().clips[0].id;

        let source = project.media_source(media_id).unwrap();
        assert!(source.is_visualizer_plugin() && source.is_visual());
        assert!(!source.is_milkdrop_preset());
        // A plugin is part of the picture, but it is not a MilkDrop preset.
        assert_eq!(project.visual_layers_at(0).len(), 1);
        assert_eq!(project.milkdrop_clips().count(), 0);

        assert!(project.set_clip_parameter(clip_id, "bars", vec![48.0]));
        assert!(!project.set_clip_parameter(clip_id, "bars", vec![48.0]));
        assert!(project.set_clip_parameter(clip_id, "color", vec![1.0, 0.5, 0.0]));

        let saved = project.to_vzx_string().unwrap();
        let loaded = Project::from_vzx_str(&saved).unwrap();
        assert_eq!(loaded, project);
        assert_eq!(
            loaded.track(layer).unwrap().clips[0].parameters["color"],
            [1.0, 0.5, 0.0]
        );

        assert!(project.reset_clip_parameters(clip_id));
        assert!(!project.reset_clip_parameters(clip_id));
    }

    #[test]
    fn an_image_is_fitted_into_the_frame_keeping_its_shape() {
        let mut project = Project::with_starter_tracks(); // 1920 x 1080
        project.settings.duration_milliseconds = 10_000;

        // A square image: as tall as the frame, centered horizontally.
        let (media_id, layer) = project.add_image(
            "cover.png",
            ImageProperties {
                width: 1000,
                height: 1000,
            },
        );

        let clip = &project.track(layer).unwrap().clips[0];
        assert_eq!(clip.duration_milliseconds, 10_000);
        assert!((clip.placement.height - 1.0).abs() < 1e-9);
        assert!((clip.placement.width - 0.5625).abs() < 1e-9); // 1080 / 1920
        assert!((clip.placement.x - 0.21875).abs() < 1e-9);
        assert_eq!(clip.placement.y, 0.0);

        let source = project.media_source(media_id).unwrap();
        assert!(source.is_visual() && !source.is_visualizer());
        assert_eq!(source.image().unwrap().width, 1000);
        assert_eq!(project.visual_layers_at(0).len(), 1);

        // A very wide image: as wide as the frame, centered vertically.
        let wide = fitted_placement((4000, 500), (1920, 1080));
        assert!((wide.width - 1.0).abs() < 1e-9 && wide.x == 0.0);
        assert!((wide.y + wide.height / 2.0 - 0.5).abs() < 1e-9);
    }

    #[test]
    fn opacity_and_reaction_are_kept_within_limits() {
        let mut project = Project::with_starter_tracks();
        let (_, layer) = project.add_milkdrop_preset("a.milk");
        let clip_id = project.track(layer).unwrap().clips[0].id;
        let clip = |project: &Project| project.track(layer).unwrap().clips[0].clone();

        // A new clip is fully opaque, hears its audio unchanged and does
        // not react.
        assert_eq!(clip(&project).opacity, 1.0);
        assert_eq!(clip(&project).audio_gain, 1.0);
        assert!(project.set_clip_audio_gain(clip_id, 0.1));
        assert!(!project.set_clip_audio_gain(clip_id, 0.1));
        project.set_clip_audio_gain(clip_id, 0.0);
        assert_eq!(clip(&project).audio_gain, Clip::AUDIO_GAIN_RANGE.0);
        assert!(!clip(&project).reaction.is_active());

        assert!(project.set_clip_opacity(clip_id, 0.4));
        assert!(!project.set_clip_opacity(clip_id, 0.4));
        assert!(project.set_clip_opacity(clip_id, 7.0));
        assert_eq!(clip(&project).opacity, 1.0);

        let wild = AudioReaction {
            band: ReactionBand::Bass,
            scale: 50.0,
            fade: -3.0,
            move_x: -9.0,
            rotate_degrees: 1000.0,
            sensitivity: f64::NAN,
            release_seconds: 60.0,
            ..AudioReaction::default()
        };
        assert!(project.set_clip_reaction(clip_id, wild));
        let stored = clip(&project).reaction;
        assert_eq!(stored.band, ReactionBand::Bass);
        assert_eq!(stored.scale, AudioReaction::MAX_SCALE);
        assert_eq!(stored.fade, 0.0);
        assert_eq!(stored.move_x, -AudioReaction::MAX_MOVE);
        assert_eq!(stored.rotate_degrees, AudioReaction::MAX_ROTATION);
        assert_eq!(stored.sensitivity, 1.0);
        assert_eq!(stored.release_seconds, AudioReaction::MAX_RELEASE_SECONDS);
        assert!(stored.is_active());
    }

    #[test]
    fn a_reaction_animates_with_loudness() {
        let reaction = AudioReaction {
            scale: 0.2,
            fade: 0.5,
            move_y: -0.1,
            rotate_degrees: 30.0,
            ..AudioReaction::default()
        };

        // Silent: untouched except for the fade.
        let silent = reaction.at_level(0.0);
        assert_eq!(
            silent,
            ReactionEffect {
                opacity: 0.5,
                ..ReactionEffect::NONE
            }
        );
        assert_eq!(reaction.at_level(f64::NAN), silent);

        // Full loudness: 20 % larger, fully visible, moved up, turned.
        let loud = reaction.at_level(1.0);
        assert!((loud.scale - 1.2).abs() < 1e-12);
        assert_eq!(loud.opacity, 1.0);
        assert_eq!((loud.offset_x, loud.offset_y), (0.0, -0.1));
        assert_eq!(loud.rotation_degrees, 30.0);

        // Sensitivity makes quieter sound count as louder, up to the top.
        let sensitive = AudioReaction {
            sensitivity: 4.0,
            ..reaction
        };
        assert_eq!(sensitive.at_level(0.25), loud);
        assert_eq!(sensitive.at_level(0.9), loud);

        // No reaction leaves the element alone at any loudness.
        assert_eq!(AudioReaction::default().at_level(0.7), ReactionEffect::NONE);
    }

    #[test]
    fn a_reaction_saved_by_an_older_version_still_loads() {
        // Format version 10 knew only these three fields.
        let old: AudioReaction =
            serde_json::from_str(r#"{ "band": "Bass", "scale": 0.3, "fade": 0.0 }"#).unwrap();

        assert_eq!(
            old,
            AudioReaction {
                band: ReactionBand::Bass,
                scale: 0.3,
                ..AudioReaction::default()
            }
        );
    }

    fn layer_names(project: &Project) -> Vec<&str> {
        project
            .tracks()
            .iter()
            .map(|track| track.name.as_str())
            .collect()
    }

    #[test]
    fn layers_can_be_reordered_but_stay_above_the_audio_tracks() {
        let mut project = Project::with_starter_tracks();
        let bottom = project.tracks()[0].id; // "Layer 1"
        let middle = project.add_default_named_track(TrackKind::Visual);
        let top = project.add_default_named_track(TrackKind::Visual);
        let audio = project.tracks()[3].id;
        assert_eq!(
            layer_names(&project),
            ["Layer 3", "Layer 2", "Layer 1", "Audio"]
        );

        assert!(project.move_layer(bottom, LayerMove::Up));
        assert_eq!(
            layer_names(&project),
            ["Layer 3", "Layer 1", "Layer 2", "Audio"]
        );
        assert!(project.move_layer(top, LayerMove::ToBack));
        assert_eq!(
            layer_names(&project),
            ["Layer 1", "Layer 2", "Layer 3", "Audio"]
        );
        assert!(project.move_layer(middle, LayerMove::ToFront));
        assert_eq!(
            layer_names(&project),
            ["Layer 2", "Layer 1", "Layer 3", "Audio"]
        );
        assert!(project.move_layer(middle, LayerMove::Down));
        assert_eq!(
            layer_names(&project),
            ["Layer 1", "Layer 2", "Layer 3", "Audio"]
        );

        // Already at the edge, or not a layer at all: nothing happens.
        assert!(!project.move_layer(top, LayerMove::Down));
        assert!(!project.move_layer(bottom, LayerMove::ToFront));
        assert!(!project.move_layer(audio, LayerMove::Up));
        assert_eq!(
            layer_names(&project),
            ["Layer 1", "Layer 2", "Layer 3", "Audio"]
        );
    }

    #[test]
    fn a_background_goes_behind_everything_and_covers_the_frame() {
        let mut project = Project::with_starter_tracks(); // 1920 x 1080
        project.settings.duration_milliseconds = 10_000;
        project.add_milkdrop_preset("a.milk");

        // A square picture must be wider than the frame is high to cover it.
        let (_, background) = project.add_background_image(
            "sky.jpg",
            ImageProperties {
                width: 1000,
                height: 1000,
            },
        );

        assert_eq!(layer_names(&project), ["Layer 1", "Background", "Audio"]);
        let clip = &project.track(background).unwrap().clips[0];
        assert_eq!(clip.duration_milliseconds, 10_000);
        assert_eq!((clip.placement.x, clip.placement.width), (0.0, 1.0));
        // 1920 / 1080 of the frame's height, centered: it sticks out above
        // and below by the same amount.
        assert!((clip.placement.height - 1920.0 / 1080.0).abs() < 1e-9);
        assert!((clip.placement.y + clip.placement.height / 2.0 - 0.5).abs() < 1e-9);
        // It is drawn first, so everything else lies over it.
        assert_eq!(project.visual_layers_at(0).last().unwrap().0.id, background);
    }
    #[test]
    fn a_video_can_be_a_layer_or_the_background() {
        let mut project = Project::with_starter_tracks(); // 1920 x 1080
        let size = ImageProperties {
            width: 1280,
            height: 720,
        };

        let (media_id, layer) = project.add_video("clip.mp4", size);
        let source = project.media_source(media_id).unwrap();
        assert_eq!(source.video(), Some(&size));
        assert!(source.is_visual() && !source.is_visualizer() && source.image().is_none());
        // The same shape as the frame: it fills it exactly.
        assert_eq!(
            project.track(layer).unwrap().clips[0].placement,
            Placement::FULL
        );

        let (_, background) = project.add_background_video("loop.mp4", size);
        assert_eq!(layer_names(&project), ["Layer 1", "Background", "Audio"]);
        assert_eq!(project.visual_layers_at(0).last().unwrap().0.id, background);
    }

    #[test]
    fn a_text_is_an_element_of_the_picture_named_after_its_first_line() {
        let mut project = Project::with_starter_tracks();
        project.settings.duration_milliseconds = 10_000;
        let place = Placement {
            x: 0.25,
            y: 0.4,
            width: 0.5,
            height: 0.2,
        };

        let (media_id, layer) = project.add_text(TextProperties::default(), place);

        let source = project.media_source(media_id).unwrap();
        assert!(source.is_visual() && source.is_still() && !source.is_visualizer());
        assert_eq!(source.display_name(), "Text");
        assert_eq!(project.track(layer).unwrap().clips[0].placement, place);
        assert_eq!(project.visual_layers_at(0).len(), 1);

        let edited = TextProperties {
            text: "\n  My Song  \nby me".to_owned(),
            size: 99.0,
            ..TextProperties::default()
        };
        assert!(project.set_text(media_id, edited.clone(), place));
        assert!(!project.set_text(media_id, edited, place));
        let source = project.media_source(media_id).unwrap();
        assert_eq!(source.display_name(), "My Song");
        assert_eq!(source.text().unwrap().size, TextProperties::SIZE_RANGE.1);

        // Texts are saved with the project.
        let saved = project.to_vzx_string().unwrap();
        assert_eq!(Project::from_vzx_str(&saved).unwrap(), project);
        // A file keeps its file name as its name.
        let (audio_id, _) = project.import_audio("C:/Music/song.mp3", audio(1000));
        assert_eq!(
            project.media_source(audio_id).unwrap().display_name(),
            "song.mp3"
        );
    }

    /// Start, length and source offset of a clip.
    fn timing(project: &Project, id: ClipId) -> (u64, u64, u64) {
        let (_, clip) = project.clip(id).unwrap();
        (
            clip.start_milliseconds,
            clip.duration_milliseconds,
            clip.source_offset_milliseconds,
        )
    }

    #[test]
    fn an_audio_clip_is_trimmed_within_its_file_and_keeps_its_content_in_place() {
        let mut project = Project::with_starter_tracks();
        let (_, track) = project.import_audio("song.wav", audio(10_000));
        let id = project.track(track).unwrap().clips[0].id;

        // Cut off the first 2 s: what was at 2 s of the file stays at 2 s
        // of the project.
        assert!(project.trim_clip_start(id, 2_000));
        assert_eq!(timing(&project, id), (2_000, 8_000, 2_000));
        // Bring 1 s of it back.
        assert!(project.trim_clip_start(id, 1_000));
        assert_eq!(timing(&project, id), (1_000, 9_000, 1_000));
        // There is nothing before the start of the file.
        project.trim_clip_start(id, 0);
        project.move_clip(id, 5_000);
        assert!(!project.trim_clip_start(id, 0));
        assert_eq!(timing(&project, id), (5_000, 10_000, 0));

        // The right edge: shorter, and never beyond the end of the file.
        assert!(project.trim_clip_end(id, 9_000));
        assert_eq!(timing(&project, id), (5_000, 4_000, 0));
        project.trim_clip_end(id, 99_000);
        assert_eq!(timing(&project, id), (5_000, 10_000, 0));
        // A clip cannot be trimmed away completely.
        project.trim_clip_end(id, 0);
        assert_eq!(timing(&project, id).1, Clip::MIN_DURATION_MILLISECONDS);
        project.trim_clip_end(id, 15_000);
        project.trim_clip_start(id, 99_000);
        assert_eq!(timing(&project, id).1, Clip::MIN_DURATION_MILLISECONDS);
    }

    #[test]
    fn a_visual_clip_is_trimmed_freely_and_the_project_grows_with_its_clips() {
        let mut project = Project::with_starter_tracks();
        project.settings.duration_milliseconds = 10_000;
        let (_, layer) = project.add_milkdrop_preset("a.milk");
        let id = project.track(layer).unwrap().clips[0].id;

        // A visualizer has no "beginning of the file" to skip.
        assert!(project.trim_clip_start(id, 4_000));
        assert_eq!(timing(&project, id), (4_000, 6_000, 0));
        assert!(project.trim_clip_start(id, 1_000));
        assert_eq!(timing(&project, id), (1_000, 9_000, 0));

        // It can be made longer than the project, which then grows.
        assert!(project.trim_clip_end(id, 12_000));
        assert_eq!(project.settings.duration_milliseconds, 12_000);
        assert!(project.move_clip(id, 5_000));
        assert!(!project.move_clip(id, 5_000));
        assert_eq!(project.settings.duration_milliseconds, 16_000);
        // Moving back does not shorten the project again.
        project.move_clip(id, 0);
        assert_eq!(project.settings.duration_milliseconds, 16_000);
    }

    #[test]
    fn splitting_gives_two_clips_that_together_are_the_original() {
        let mut project = Project::with_starter_tracks();
        let (_, track) = project.import_audio("song.wav", audio(10_000));
        let first = project.track(track).unwrap().clips[0].id;
        project.trim_clip_start(first, 1_000); // offset 1 s

        let second = project.split_clip(first, 4_000).unwrap();

        assert_ne!(first, second);
        assert_eq!(timing(&project, first), (1_000, 3_000, 1_000));
        // The second part continues in the file where the first one stops.
        assert_eq!(timing(&project, second), (4_000, 6_000, 4_000));
        assert_eq!(project.track(track).unwrap().clips.len(), 2);
        // Both use the same file; it is in the library once.
        assert_eq!(project.media().len(), 1);

        // Not outside the clip, and not so close to an edge that a part
        // would be too short.
        assert!(project.split_clip(first, 500).is_none());
        assert!(project.split_clip(first, 4_000).is_none());
        assert!(project.split_clip(first, 1_050).is_none());
    }

    #[test]
    fn copies_of_a_text_can_be_edited_separately_and_unused_media_is_removed() {
        let mut project = Project::with_starter_tracks();
        project.settings.duration_milliseconds = 10_000;
        let (_, layer) = project.add_text(TextProperties::default(), Placement::FULL);
        let first = project.track(layer).unwrap().clips[0].id;
        project.trim_clip_end(first, 2_000);

        // A second line right after the first one.
        let second = project.duplicate_clip(first).unwrap();
        assert_eq!(timing(&project, second), (2_000, 2_000, 0));
        let second_media = project.clip(second).unwrap().1.media;
        assert_ne!(project.clip(first).unwrap().1.media, second_media);
        let new_line = TextProperties {
            text: "Second line".to_owned(),
            ..TextProperties::default()
        };
        project.set_text(second_media, new_line, Placement::FULL);
        let names: Vec<&str> = project
            .media()
            .iter()
            .map(MediaSource::display_name)
            .collect();
        assert_eq!(names, ["Text", "Second line"]);

        // Splitting a text also gives the second part its own text.
        let third = project.split_clip(second, 3_000).unwrap();
        assert_eq!(project.media().len(), 3);

        assert!(project.remove_clip(second));
        assert!(!project.remove_clip(second));
        assert_eq!(project.media().len(), 2);
        assert!(project.clip(first).is_some() && project.clip(third).is_some());
    }

    #[test]
    fn removing_one_of_two_clips_of_a_file_keeps_the_file_in_the_library() {
        let mut project = Project::with_starter_tracks();
        let (_, track) = project.import_audio("song.wav", audio(10_000));
        let first = project.track(track).unwrap().clips[0].id;
        let second = project.split_clip(first, 5_000).unwrap();

        project.remove_clip(first);
        assert_eq!(project.media().len(), 1);
        project.remove_clip(second);
        assert!(project.media().is_empty());
    }

    #[test]
    fn a_clip_can_change_to_another_track_of_the_same_kind_only() {
        let mut project = Project::with_starter_tracks();
        project.settings.duration_milliseconds = 10_000;
        let (_, layer) = project.add_milkdrop_preset("a.milk");
        let clip = project.track(layer).unwrap().clips[0].id;
        project.move_clip(clip, 3_000);
        let other_layer = project.add_default_named_track(TrackKind::Visual);
        let audio_track = project
            .tracks()
            .iter()
            .find(|track| track.kind == TrackKind::Audio)
            .unwrap()
            .id;

        assert!(!project.move_clip_to_track(clip, audio_track));
        assert!(!project.move_clip_to_track(clip, layer)); // already there
        assert!(project.move_clip_to_track(clip, other_layer));

        assert!(project.track(layer).unwrap().clips.is_empty());
        assert_eq!(project.clip(clip).unwrap().0.id, other_layer);
        // Its time is unchanged.
        assert_eq!(timing(&project, clip), (3_000, 10_000, 0));
    }

    fn x_at(project: &Project, id: ClipId, project_milliseconds: u64) -> f64 {
        let (_, clip) = project.clip(id).unwrap();
        let moment = project_milliseconds - clip.start_milliseconds;
        clip.transform_at(moment as f64).placement.x
    }

    #[test]
    fn edits_change_the_resting_state_until_there_are_keyframes() {
        let mut project = Project::with_starter_tracks();
        project.settings.duration_milliseconds = 10_000;
        let (_, layer) = project.add_milkdrop_preset("a.milk");
        let id = project.track(layer).unwrap().clips[0].id;

        // No keyframes: an edit anywhere moves the whole clip.
        assert!(project.edit_clip_transform_at(id, 3_000, |t| t.placement.x = 0.2));
        assert!(!project.edit_clip_transform_at(id, 3_000, |t| t.placement.x = 0.2));
        assert_eq!(x_at(&project, id, 9_000), 0.2);

        // A keyframe at 2 s, then an edit at 6 s: a second keyframe, and
        // the clip glides from one to the other.
        assert!(project.add_keyframe(id, 2_000));
        assert!(!project.add_keyframe(id, 2_010)); // the same moment
        assert!(project.edit_clip_transform_at(id, 6_000, |t| {
            t.placement.x = 0.6;
            t.rotation_degrees = 90.0;
        }));
        let (_, clip) = project.clip(id).unwrap();
        assert_eq!(clip.keyframes.len(), 2);
        assert_eq!(x_at(&project, id, 1_000), 0.2);
        assert!((x_at(&project, id, 4_000) - 0.4).abs() < 1e-9);
        assert_eq!(x_at(&project, id, 8_000), 0.6);

        assert!(project.set_keyframe_easing(id, 6_000, Easing::Hold));
        assert_eq!(x_at(&project, id, 5_900), 0.2);

        // Removing the keyframes leaves the clip as it was where the last
        // one was.
        assert!(project.remove_keyframe(id, 2_000));
        assert!(!project.remove_keyframe(id, 2_000));
        assert!(project.remove_keyframe(id, 6_000));
        let (_, clip) = project.clip(id).unwrap();
        assert!(clip.keyframes.is_empty());
        assert_eq!((clip.placement.x, clip.rotation_degrees), (0.6, 90.0));

        assert!(project.set_clip_spin(id, 45.0));
        assert_eq!(
            project
                .clip(id)
                .unwrap()
                .1
                .motion()
                .animated_at(2.0)
                .rotation_degrees,
            180.0
        );
    }

    #[test]
    fn keyframes_stay_in_place_when_a_clip_is_trimmed_or_split() {
        let mut project = Project::with_starter_tracks();
        project.settings.duration_milliseconds = 10_000;
        let (_, layer) = project.add_milkdrop_preset("a.milk");
        let id = project.track(layer).unwrap().clips[0].id;
        project.edit_clip_transform_at(id, 0, |t| t.placement.x = 0.0);
        project.add_keyframe(id, 2_000);
        project.edit_clip_transform_at(id, 6_000, |t| t.placement.x = 0.4);
        let before: Vec<f64> = [3_000, 4_000, 5_000]
            .iter()
            .map(|moment| x_at(&project, id, *moment))
            .collect();

        // Cut off the first 3 s and the last 5 s: what remains looks the
        // same, and new keyframes hold the edges.
        assert!(project.trim_clip_start(id, 3_000));
        assert!(project.trim_clip_end(id, 5_000));
        let after: Vec<f64> = [3_000, 4_000, 5_000]
            .iter()
            .map(|moment| x_at(&project, id, *moment))
            .collect();
        for (a, b) in before.iter().zip(&after) {
            assert!((a - b).abs() < 1e-9, "{before:?} vs {after:?}");
        }
        // The keyframe at 2 s now lies before the clip; it is kept.
        let times: Vec<i64> = project
            .clip(id)
            .unwrap()
            .1
            .keyframes
            .iter()
            .map(|k| k.time_milliseconds)
            .collect();
        assert_eq!(times, [-1_000, 3_000]);

        // Split at 4 s: both parts continue the motion.
        let second = project.split_clip(id, 4_000).unwrap();
        assert!((x_at(&project, id, 3_000) - before[0]).abs() < 1e-9);
        assert!((x_at(&project, second, 4_000) - before[1]).abs() < 1e-9);
        assert!((x_at(&project, second, 5_000) - before[2]).abs() < 1e-9);
    }

    #[test]
    fn beats_can_be_placed_by_hand_instead_of_the_found_ones() {
        let mut project = Project::with_starter_tracks();
        let (song, _) = project.import_audio("song.wav", audio(10_000));
        let (_, layer) = project.add_milkdrop_preset("a.milk");
        let preset = project.track(layer).unwrap().clips[0].media;
        let manual = |project: &Project| project.media_source(song).unwrap().manual_beats.clone();

        // Automatic at first; nothing can be added by hand yet.
        assert_eq!(manual(&project), None);
        assert!(!project.add_manual_beat(song, 1.0));

        // Starting from a copy of found beats: sorted, without doubles.
        assert!(project.use_manual_beats(song, vec![2.0, 1.0, 1.01, f64::NAN]));
        assert_eq!(manual(&project), Some(vec![1.0, 2.0]));
        assert!(project.add_manual_beat(song, 1.5));
        assert!(!project.add_manual_beat(song, 1.51)); // already one there
        assert_eq!(manual(&project), Some(vec![1.0, 1.5, 2.0]));

        assert!(project.remove_manual_beat_near(song, 1.97, 0.05));
        assert!(!project.remove_manual_beat_near(song, 3.0, 0.05));
        assert_eq!(manual(&project), Some(vec![1.0, 1.5]));

        // Only audio has beats.
        assert!(!project.use_manual_beats(preset, Vec::new()));

        // Saved with the project, and given up again.
        let saved = project.to_vzx_string().unwrap();
        assert_eq!(Project::from_vzx_str(&saved).unwrap(), project);
        assert!(project.set_audio_tempo(song, Some(75.0)));
        assert!(!project.set_audio_tempo(song, Some(75.0)));
        project.set_audio_tempo(song, Some(1_000.0));
        assert_eq!(project.media_source(song).unwrap().tempo_bpm, Some(300.0));
        assert!(!project.set_audio_tempo(preset, Some(90.0)));
        assert!(project.use_automatic_beats(song));
        assert_eq!(manual(&project), None);
    }

    #[test]
    fn a_frame_animation_is_a_visual_element_fitted_like_an_image() {
        let mut project = Project::with_starter_tracks(); // 1920 x 1080
        let sheet = FrameAnimation::sprite_sheet((800, 200), 4, 1, 12.0);

        let (media_id, layer) = project.add_animation("run.png", sheet.clone());

        let source = project.media_source(media_id).unwrap();
        assert!(source.is_visual() && !source.is_still() && source.animation().is_some());
        // Square frames: as tall as the frame, centered.
        let placement = project.track(layer).unwrap().clips[0].placement;
        assert!((placement.height - 1.0).abs() < 1e-9);
        assert!((placement.width - 0.5625).abs() < 1e-9);

        let mut faster = sheet;
        faster.frames_per_second = 24.0;
        assert!(project.set_animation(media_id, faster.clone()));
        assert!(!project.set_animation(media_id, faster));
        let saved = project.to_vzx_string().unwrap();
        assert_eq!(Project::from_vzx_str(&saved).unwrap(), project);
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
