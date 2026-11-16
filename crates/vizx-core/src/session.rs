use std::path::{Path, PathBuf};

use vizx_audio::AudioFileError;
use vizx_project::{
    AudioProperties, ClipId, ImageProperties, MediaId, Placement, Project, ProjectFileError,
    TextProperties, Track, TrackId, TrackKind,
};

use crate::history::History;
use crate::{PresetEntry, PresetLibrary, Selection};

/// Frames per second of a newly imported frame animation.
const DEFAULT_ANIMATION_SPEED: f64 = 12.0;

/// Name shown for a project that has not been saved to a file yet.
pub const UNTITLED_PROJECT_NAME: &str = "Untitled";

/// A clip on the clipboard, with the media it shows or plays, so it can be
/// pasted even into another project.
#[derive(Debug, Clone, PartialEq)]
pub struct CopiedClip {
    pub clip: vizx_project::Clip,
    pub source: vizx_project::MediaSource,
    /// How much later than the earliest copied clip it starts.
    pub offset_milliseconds: u64,
    /// The track it was on, used again when pasting into the same project.
    pub track_id: TrackId,
}

/// The state of one open project while it is being edited.
///
/// There is exactly one session in the application and every UI panel is a
/// view of it. Panels never keep their own copy of project data, which is
/// what keeps the editor unified instead of a set of separate tools.
#[derive(Debug, Clone)]
pub struct EditorSession {
    project: Project,
    /// Where the project is stored on disk. `None` until it is first saved.
    file_path: Option<PathBuf>,
    has_unsaved_changes: bool,
    selection: Selection,
    /// Current position on the project timeline, in seconds.
    playhead_seconds: f64,
    /// Earlier states of the project, for Undo and Redo.
    history: History,
    /// The stretch of time played over and over, in seconds, if one was
    /// marked; and whether playback loops it right now.
    loop_region: Option<(f64, f64)>,
    is_looping: bool,
    /// More clips selected together with the one in `selection` (with
    /// Ctrl+click or a rectangle). Only counts while a clip is selected.
    extra_clips: Vec<ClipId>,
}

impl EditorSession {
    /// Starts a session with a new, never-saved project.
    pub fn new_untitled() -> Self {
        Self::with_project(Project::with_starter_tracks(), None)
    }

    fn with_project(project: Project, file_path: Option<PathBuf>) -> Self {
        Self {
            history: History::new(&project),
            project,
            file_path,
            has_unsaved_changes: false,
            selection: Selection::Nothing,
            playhead_seconds: 0.0,
            loop_region: None,
            is_looping: false,
            extra_clips: Vec::new(),
        }
    }

    /// Lets the undo history take note of the project. Call once per frame.
    ///
    /// `is_user_busy` is true while a mouse button is held down, so that a
    /// whole drag becomes one undo step instead of hundreds.
    pub fn settle_history(&mut self, is_user_busy: bool) {
        self.history
            .observe(&self.project, is_user_busy, std::time::Instant::now());
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo(&self.project)
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo(&self.project)
    }

    /// Takes the project back to how it was before the last change.
    /// Returns `false` if there is nothing to undo.
    pub fn undo(&mut self) -> bool {
        let was_undone = self.history.undo(&mut self.project);
        self.after_history_step(was_undone)
    }

    /// Brings back the change that was undone last. Returns `false` if
    /// there is nothing to redo.
    pub fn redo(&mut self) -> bool {
        let was_redone = self.history.redo(&mut self.project);
        self.after_history_step(was_redone)
    }

    fn after_history_step(&mut self, has_changed: bool) -> bool {
        if has_changed {
            // The project differs from the file again (simply assuming so
            // is safe: at worst the user is asked to save once too often).
            self.has_unsaved_changes = true;
            // The playhead may now lie beyond the end of the project.
            self.set_playhead_seconds(self.playhead_seconds);
        }
        has_changed
    }

    /// Starts a session by loading a project from a `.vzx` file.
    pub fn open(path: &Path) -> Result<Self, ProjectFileError> {
        Ok(Self::with_project(
            Project::load_from_file(path)?,
            Some(path.to_owned()),
        ))
    }

    /// Starts a session from an autosaved copy of a project (after VizX did
    /// not close properly). It belongs to `original_path` again, the file it
    /// was a copy of (`None` for a project that was never saved), and counts
    /// as unsaved until the user saves it.
    pub fn recover(
        autosave: &Path,
        original_path: Option<PathBuf>,
    ) -> Result<Self, ProjectFileError> {
        let mut session = Self::with_project(Project::load_from_file(autosave)?, original_path);
        session.has_unsaved_changes = true;
        Ok(session)
    }

    /// Where the playhead is on the project timeline, in seconds.
    ///
    /// Every panel that shows "now" (the timeline, the time display, later
    /// the preview) reads this one value. Like the selection, it is editing
    /// state: moving it is not a change to the project and it is not saved.
    pub fn playhead_seconds(&self) -> f64 {
        self.playhead_seconds
    }

    /// Moves the playhead, keeping it within the project.
    /// Moves the playhead by whole frames of the project (negative: back),
    /// landing exactly on a frame.
    pub fn step_playhead_frames(&mut self, frames: i32) {
        let rate = self.project.settings.frame_rate.as_f64().max(1.0);
        let current = (self.playhead_seconds * rate).round();
        self.set_playhead_seconds((current + f64::from(frames)) / rate);
    }

    /// Moves the playhead to the nearest place after it (`forward`) or
    /// before it where a clip starts or ends or a marker is, or to the
    /// project's start or end. Returns `false` if there is none in that direction.
    pub fn jump_to_clip_edge(&mut self, forward: bool) -> bool {
        // A moment closer than this counts as "here".
        const HERE: f64 = 0.001;
        let now = self.playhead_seconds;
        let edges = self
            .project
            .tracks()
            .iter()
            .flat_map(|track| &track.clips)
            .flat_map(|clip| [clip.start_milliseconds, clip.end_milliseconds()])
            .chain(
                self.project
                    .markers()
                    .iter()
                    .map(|marker| marker.milliseconds),
            )
            .map(|milliseconds| milliseconds as f64 / 1000.0)
            .chain([0.0, self.project.settings.duration_seconds()]);
        let target = if forward {
            edges.filter(|edge| *edge > now + HERE).reduce(f64::min)
        } else {
            edges.filter(|edge| *edge < now - HERE).reduce(f64::max)
        };
        match target {
            Some(edge) => {
                self.set_playhead_seconds(edge);
                true
            }
            None => false,
        }
    }

    /// Moves the playhead to the next (`forward`) or previous marker.
    /// Returns `false` if there is none in that direction.
    pub fn jump_to_marker(&mut self, forward: bool) -> bool {
        const HERE: f64 = 0.001;
        let now = self.playhead_seconds;
        let mut times = self
            .project
            .markers()
            .iter()
            .map(|marker| marker.milliseconds as f64 / 1000.0);
        let target = if forward {
            times.find(|time| *time > now + HERE)
        } else {
            times.rev().find(|time| *time < now - HERE)
        };
        target.is_some_and(|time| {
            self.set_playhead_seconds(time);
            true
        })
    }

    /// Adds a marker at the playhead and returns its place among the
    /// markers.
    pub fn add_marker_at_playhead(&mut self) -> usize {
        let milliseconds = (self.playhead_seconds * 1000.0).round() as u64;
        let index = self.project.add_marker(milliseconds, "");
        self.has_unsaved_changes = true;
        index
    }

    /// The stretch of time played over and over, if one was marked (also
    /// while looping is off).
    pub fn loop_region(&self) -> Option<(f64, f64)> {
        self.loop_region
    }

    /// Whether playback loops the loop region right now.
    pub fn is_looping(&self) -> bool {
        self.is_looping && self.loop_region.is_some()
    }

    /// Marks the stretch from `start` to `end` seconds (either order) to be
    /// played over and over, and turns looping on.
    pub fn set_loop_region(&mut self, start: f64, end: f64) {
        const SHORTEST: f64 = 0.1;
        let project_end = self.project.settings.duration_seconds();
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let start = start.clamp(0.0, project_end);
        let end = end.clamp(0.0, project_end);
        if end - start < SHORTEST {
            return;
        }
        self.loop_region = Some((start, end));
        self.is_looping = true;
    }

    /// The loop starts at the playhead (it ends where it did, or at the end
    /// of the project).
    pub fn set_loop_start_at_playhead(&mut self) {
        let end = self
            .loop_region
            .map(|(_, end)| end)
            .filter(|end| *end > self.playhead_seconds)
            .unwrap_or_else(|| self.project.settings.duration_seconds());
        self.set_loop_region(self.playhead_seconds, end);
    }

    /// The loop ends at the playhead (it starts where it did, or at the
    /// start of the project).
    pub fn set_loop_end_at_playhead(&mut self) {
        let start = self
            .loop_region
            .map(|(start, _)| start)
            .filter(|start| *start < self.playhead_seconds)
            .unwrap_or(0.0);
        self.set_loop_region(start, self.playhead_seconds);
    }

    /// Turns looping on or off. Without a loop region yet, the selected
    /// clip (or else the whole project) becomes the region. Returns whether
    /// it loops now.
    pub fn toggle_looping(&mut self) -> bool {
        if self.loop_region.is_none() {
            let (start, end) = match self.selected_clip_id().and_then(|id| self.project.clip(id)) {
                Some((_, clip)) => (
                    clip.start_milliseconds as f64 / 1000.0,
                    clip.end_milliseconds() as f64 / 1000.0,
                ),
                None => (0.0, self.project.settings.duration_seconds()),
            };
            self.set_loop_region(start, end);
            return self.is_looping();
        }
        self.is_looping = !self.is_looping;
        self.is_looping
    }

    /// Forgets the loop region.
    pub fn clear_loop(&mut self) {
        self.loop_region = None;
        self.is_looping = false;
    }

    pub fn set_playhead_seconds(&mut self, seconds: f64) {
        let project_end = self.project.settings.duration_seconds();
        self.playhead_seconds = if seconds.is_finite() {
            seconds.clamp(0.0, project_end)
        } else {
            0.0
        };
    }

    pub fn selection(&self) -> Selection {
        self.selection
    }

    /// Changes the selection. Selecting is not an edit, so it does not mark
    /// the project as changed.
    pub fn select(&mut self, selection: Selection) {
        self.set_selection(selection);
    }

    fn set_selection(&mut self, selection: Selection) {
        self.selection = selection;
        self.extra_clips.clear();
    }

    /// Every selected clip that still exists, the main one first.
    pub fn selected_clip_ids(&self) -> Vec<ClipId> {
        let Some(main) = self.selected_clip_id() else {
            return Vec::new();
        };
        let mut ids = vec![main];
        ids.extend(
            self.extra_clips
                .iter()
                .copied()
                .filter(|id| *id != main && self.project.clip(*id).is_some()),
        );
        ids
    }

    /// Adds a clip to the selection, or takes it out if it is in it
    /// (Ctrl+click).
    pub fn toggle_clip_selection(&mut self, clip_id: ClipId) {
        let mut ids = self.selected_clip_ids();
        match ids.iter().position(|id| *id == clip_id) {
            Some(index) => {
                ids.remove(index);
            }
            None => ids.push(clip_id),
        }
        self.select_clips(&ids);
    }

    /// Selects exactly these clips (the first is the main one, shown in the
    /// Properties panel). None selects nothing.
    pub fn select_clips(&mut self, ids: &[ClipId]) {
        match ids.split_first() {
            Some((main, rest)) => {
                self.selection = Selection::Clip(*main);
                self.extra_clips = rest.to_vec();
            }
            None => self.set_selection(Selection::Nothing),
        }
    }

    /// Selects every clip of the project.
    pub fn select_all_clips(&mut self) {
        let ids: Vec<ClipId> = self
            .project
            .tracks()
            .iter()
            .flat_map(|track| &track.clips)
            .map(|clip| clip.id)
            .collect();
        self.select_clips(&ids);
    }

    /// Moves all selected clips together, so that `anchor` (one of them,
    /// the one being dragged) starts at `start_milliseconds`. None of them
    /// goes before the start of the project. Returns `true` if anything
    /// moved.
    pub fn move_selected_clips(&mut self, anchor: ClipId, start_milliseconds: u64) -> bool {
        let ids = self.selected_clip_ids();
        let Some((_, anchor_clip)) = self.project.clip(anchor) else {
            return false;
        };
        let mut delta = start_milliseconds as i64 - anchor_clip.start_milliseconds as i64;
        // The earliest clip stops at 0, and the others keep their distance.
        let earliest = ids
            .iter()
            .filter_map(|id| self.project.clip(*id))
            .map(|(_, clip)| clip.start_milliseconds as i64)
            .min()
            .unwrap_or(0);
        delta = delta.max(-earliest);
        if delta == 0 {
            return false;
        }
        let starts: Vec<(ClipId, u64)> = ids
            .iter()
            .filter_map(|id| self.project.clip(*id))
            .map(|(_, clip)| (clip.id, (clip.start_milliseconds as i64 + delta) as u64))
            .collect();
        for (id, start) in starts {
            self.project.move_clip(id, start);
        }
        self.has_unsaved_changes = true;
        true
    }

    /// Moves the selected clips up or down together, so that `anchor` (the
    /// one being dragged) lands on `target_track`. Every selected clip on a
    /// track of the same kind as the anchor's moves by the same number of
    /// tracks of that kind; clips on tracks of another kind stay where they
    /// are. Nothing moves if any clip would go past the first or last track
    /// of its kind. Returns `true` if anything moved.
    pub fn move_selected_clips_to_track(&mut self, anchor: ClipId, target_track: TrackId) -> bool {
        let tracks = self.project.tracks();
        let Some((anchor_track, _)) = self.project.clip(anchor) else {
            return false;
        };
        let kind = anchor_track.kind;
        let same_kind: Vec<TrackId> = tracks
            .iter()
            .filter(|track| track.kind == kind)
            .map(|track| track.id)
            .collect();
        let position = |id: TrackId| same_kind.iter().position(|track| *track == id);
        let (Some(from), Some(to)) = (position(anchor_track.id), position(target_track)) else {
            return false;
        };
        let shift = to as i64 - from as i64;
        if shift == 0 {
            return false;
        }
        let mut moves = Vec::new();
        for id in self.selected_clip_ids() {
            let Some((track, _)) = self.project.clip(id) else {
                continue;
            };
            if track.kind != kind {
                continue;
            }
            let Some(index) = position(track.id) else {
                continue;
            };
            let new_index = index as i64 + shift;
            if new_index < 0 || new_index >= same_kind.len() as i64 {
                return false;
            }
            moves.push((id, same_kind[new_index as usize]));
        }
        for (id, track) in moves {
            self.project.move_clip_to_track(id, track);
        }
        self.has_unsaved_changes = true;
        true
    }

    /// The selected track, or the track of the selected clip, if it still
    /// exists.
    pub fn selected_track(&self) -> Option<&Track> {
        match self.selection {
            Selection::Track(id) => self.project.track(id),
            Selection::Clip(id) => self.project.clip(id).map(|(track, _)| track),
            Selection::Nothing => None,
        }
    }

    /// The selected clip, if a clip is selected and still exists.
    pub fn selected_clip_id(&self) -> Option<ClipId> {
        match self.selection {
            Selection::Clip(id) if self.project.clip(id).is_some() => Some(id),
            _ => None,
        }
    }

    /// Whether a clip should be shown as selected: it is selected itself,
    /// or its whole track is.
    pub fn is_clip_selected(&self, track_id: TrackId, clip_id: ClipId) -> bool {
        match self.selection {
            Selection::Clip(id) => id == clip_id || self.extra_clips.contains(&clip_id),
            Selection::Track(id) => id == track_id,
            Selection::Nothing => false,
        }
    }

    /// Removes what is selected: the clip, or the whole track. Returns a
    /// short description of what was removed, or `None` if nothing was.
    pub fn delete_selection(&mut self) -> Option<&'static str> {
        let clips = self.selected_clip_ids();
        if clips.len() > 1 {
            let track_id = self.project.clip(clips[0]).map(|(track, _)| track.id)?;
            for id in clips {
                self.project.remove_clip(id);
            }
            self.has_unsaved_changes = true;
            self.set_selection(Selection::Track(track_id));
            return Some("clips");
        }
        match self.selection {
            Selection::Clip(id) => {
                // Afterwards the clip's track is selected, so the user
                // stays where they were working.
                let track_id = self.project.clip(id).map(|(track, _)| track.id)?;
                self.project.remove_clip(id);
                self.has_unsaved_changes = true;
                self.set_selection(Selection::Track(track_id));
                Some("clip")
            }
            Selection::Track(id) => {
                self.project.track(id)?;
                self.remove_track(id);
                Some("track")
            }
            Selection::Nothing => None,
        }
    }

    /// Adds a hand-placed beat at the playhead to the selected audio clip's
    /// file ("tapping along"). Returns why not if that is not possible.
    pub fn tap_beat(&mut self) -> Result<(), &'static str> {
        self.tap_beat_at(self.playhead_seconds)
    }

    /// Like [`Self::tap_beat`], at `seconds` of the project instead of the
    /// playhead (the moment actually heard, which lags the playhead a
    /// little during playback).
    pub fn tap_beat_at(&mut self, seconds: f64) -> Result<(), &'static str> {
        let (_, clip) = self
            .selected_clip_id()
            .and_then(|id| self.project.clip(id))
            .ok_or("select an audio clip")?;
        let source = self
            .project
            .media_source(clip.media)
            .filter(|source| source.audio().is_some())
            .ok_or("select an audio clip")?;
        if source.manual_beats.is_none() {
            return Err("turn on manual beats first (Properties > Beats > Edit Beats)");
        }
        let playhead = (seconds.max(0.0) * 1000.0).round() as u64;
        if playhead < clip.start_milliseconds || playhead > clip.end_milliseconds() {
            return Err("the playhead is outside the selected clip");
        }
        let seconds_in_file =
            (playhead - clip.start_milliseconds + clip.source_offset_milliseconds) as f64 / 1000.0;
        let media = source.id;
        if self.project.add_manual_beat(media, seconds_in_file) {
            self.has_unsaved_changes = true;
        }
        Ok(())
    }

    /// Cuts the selected clip in two at the playhead. The first part stays
    /// selected. Returns `false` if no clip is selected or the playhead is
    /// not inside it.
    pub fn split_selected_clip(&mut self) -> bool {
        let Some(id) = self.selected_clip_id() else {
            return false;
        };
        let at_milliseconds = (self.playhead_seconds * 1000.0).round() as u64;
        let was_split = self.project.split_clip(id, at_milliseconds).is_some();
        self.has_unsaved_changes |= was_split;
        was_split
    }

    /// Puts a copy of the selected clip right after it and selects the
    /// copy. Returns `false` if no clip is selected.
    pub fn duplicate_selected_clip(&mut self) -> bool {
        let Some(copy) = self
            .selected_clip_id()
            .and_then(|id| self.project.duplicate_clip(id))
        else {
            return false;
        };
        self.has_unsaved_changes = true;
        self.set_selection(Selection::Clip(copy));
        true
    }

    /// Copies of the selected clips with their media, for pasting later
    /// (also into another project). Empty if no clip is selected.
    pub fn copy_selected_clips(&self) -> Vec<CopiedClip> {
        let clips: Vec<(&Track, &vizx_project::Clip)> = self
            .selected_clip_ids()
            .into_iter()
            .filter_map(|id| self.project.clip(id))
            .collect();
        let earliest = clips
            .iter()
            .map(|(_, clip)| clip.start_milliseconds)
            .min()
            .unwrap_or(0);
        clips
            .into_iter()
            .filter_map(|(track, clip)| {
                Some(CopiedClip {
                    clip: clip.clone(),
                    source: self.project.media_source(clip.media)?.clone(),
                    offset_milliseconds: clip.start_milliseconds - earliest,
                    track_id: track.id,
                })
            })
            .collect()
    }

    /// Pastes copied clips at the playhead, keeping their distances. Each
    /// goes back onto its own track if this project has it (and it is of
    /// the right kind); otherwise onto the selected track if that fits, the
    /// first track that does, or a new one. The pasted clips become
    /// selected.
    pub fn paste_clips(&mut self, copied: &[CopiedClip]) -> Vec<ClipId> {
        let at = (self.playhead_seconds * 1000.0).round() as u64;
        let selected_track = self.selected_track().map(|track| (track.id, track.kind));
        let mut pasted = Vec::new();
        for copy in copied {
            let kind = if copy.source.audio().is_some() {
                TrackKind::Audio
            } else {
                TrackKind::Visual
            };
            let own_track = self
                .project
                .track(copy.track_id)
                .filter(|track| track.kind == kind && copied.len() > 1)
                .map(|track| track.id);
            let track_id = own_track
                .or_else(|| selected_track.filter(|(_, k)| *k == kind).map(|(id, _)| id))
                .or_else(|| {
                    self.project
                        .tracks()
                        .iter()
                        .find(|track| track.kind == kind)
                        .map(|track| track.id)
                })
                .unwrap_or_else(|| self.project.add_default_named_track(kind));
            if let Some(id) = self.project.paste_clip(
                &copy.clip,
                &copy.source,
                track_id,
                at + copy.offset_milliseconds,
            ) {
                pasted.push(id);
            }
        }
        if !pasted.is_empty() {
            self.has_unsaved_changes = true;
            self.select_clips(&pasted);
        }
        pasted
    }
    /// Adds a track to the timeline and selects it.
    pub fn add_track(&mut self, kind: TrackKind) -> TrackId {
        let id = self.project.add_default_named_track(kind);
        self.has_unsaved_changes = true;
        self.set_selection(Selection::Track(id));
        id
    }

    /// Imports an audio file: reads its properties, adds it to the project's
    /// media and places it on an audio track, which becomes selected.
    ///
    /// The project is left untouched if the file cannot be read.
    pub fn import_audio(&mut self, path: &Path) -> Result<TrackId, AudioFileError> {
        let info = vizx_audio::probe_audio_file(path)?;
        let properties = AudioProperties {
            duration_milliseconds: info.duration_milliseconds,
            sample_rate: info.sample_rate,
            channel_count: info.channel_count,
        };

        let (_media_id, track_id) = self
            .project
            .import_audio(path.to_string_lossy(), properties);
        self.has_unsaved_changes = true;
        self.set_selection(Selection::Track(track_id));
        Ok(track_id)
    }

    /// Adds the stems separated from the song of `clip_id`, each on a muted
    /// audio track of its own named after it, timed like that clip.
    /// Returns the new tracks.
    pub fn add_stems(
        &mut self,
        clip_id: ClipId,
        stems: &[(String, PathBuf)],
    ) -> Result<Vec<TrackId>, String> {
        if self.project.clip(clip_id).is_none() {
            return Err("the song's clip is no longer there".to_owned());
        }
        let mut tracks = Vec::new();
        for (name, path) in stems {
            let info = vizx_audio::probe_audio_file(path)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            let properties = AudioProperties {
                duration_milliseconds: info.duration_milliseconds,
                sample_rate: info.sample_rate,
                channel_count: info.channel_count,
            };
            let track_id = self
                .project
                .add_stem(path.to_string_lossy(), properties, name, clip_id)
                .ok_or("the song's clip is no longer there")?;
            tracks.push(track_id);
        }
        self.has_unsaved_changes = true;
        Ok(tracks)
    }

    /// Adds a MilkDrop preset (`.milk` file) to the project and places it
    /// on a visual layer, which becomes selected.
    ///
    /// Only the file's existence is checked here. Whether projectM can
    /// actually run the preset is found out when it is first drawn.
    pub fn add_milkdrop_preset(&mut self, path: &Path) -> Result<TrackId, String> {
        let is_milk_file = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("milk"));
        if !is_milk_file {
            return Err("not a MilkDrop preset (.milk) file".to_owned());
        }
        if !path.is_file() {
            return Err("the file does not exist".to_owned());
        }

        let (_media_id, track_id) = self.project.add_milkdrop_preset(path.to_string_lossy());
        self.has_unsaved_changes = true;
        self.set_selection(Selection::Track(track_id));
        Ok(track_id)
    }

    /// Adds a preset that ships with VizX and places it on a visual layer,
    /// which becomes selected.
    ///
    /// The project stores where the preset is inside the bundled collection
    /// instead of a path on this computer, so the project also opens on
    /// other machines (see [`BUNDLED_PRESET_PREFIX`](crate::BUNDLED_PRESET_PREFIX)).
    pub fn add_bundled_milkdrop_preset(&mut self, entry: &PresetEntry) -> TrackId {
        let reference = PresetLibrary::project_reference(entry);
        let (_media_id, track_id) = self.project.add_milkdrop_preset(reference);
        self.has_unsaved_changes = true;
        self.set_selection(Selection::Track(track_id));
        track_id
    }

    /// Imports an image: reads its size, adds it to the project's media and
    /// places it on a visual layer, which becomes selected.
    ///
    /// The project is left untouched if the file cannot be read.
    pub fn import_image(&mut self, path: &Path) -> Result<TrackId, String> {
        if !crate::has_supported_image_extension(path) {
            return Err("not a PNG or JPEG image".to_owned());
        }
        let (width, height) = crate::image_store::image_file_size(path)?;

        let (_media_id, track_id) = self
            .project
            .add_image(path.to_string_lossy(), ImageProperties { width, height });
        self.has_unsaved_changes = true;
        self.set_selection(Selection::Track(track_id));
        Ok(track_id)
    }

    /// Imports an image as the background: on a new layer behind all
    /// others, covering the whole frame. The new layer becomes selected.
    pub fn import_background_image(&mut self, path: &Path) -> Result<TrackId, String> {
        if !crate::has_supported_image_extension(path) {
            return Err("not a PNG or JPEG image".to_owned());
        }
        let (width, height) = crate::image_store::image_file_size(path)?;

        let (_media_id, track_id) = self
            .project
            .add_background_image(path.to_string_lossy(), ImageProperties { width, height });
        self.has_unsaved_changes = true;
        self.set_selection(Selection::Track(track_id));
        Ok(track_id)
    }

    /// Imports a video file for its picture: on a visual layer of its own,
    /// or, with `as_background`, behind all layers and covering the frame.
    /// The new layer becomes selected. `ffmpeg` is the FFmpeg program,
    /// which reads the size of the video's picture.
    pub fn import_video(
        &mut self,
        path: &Path,
        ffmpeg: &Path,
        as_background: bool,
    ) -> Result<TrackId, String> {
        let (width, height) = crate::probe_video(ffmpeg, path)?;
        let properties = ImageProperties { width, height };

        let (_media_id, track_id) = if as_background {
            self.project
                .add_background_video(path.to_string_lossy(), properties)
        } else {
            self.project.add_video(path.to_string_lossy(), properties)
        };
        self.has_unsaved_changes = true;
        self.set_selection(Selection::Track(track_id));
        Ok(track_id)
    }

    /// Adds a text element in the middle of the picture, on a visual layer,
    /// which becomes selected.
    pub fn add_text(&mut self) -> Result<TrackId, String> {
        let properties = TextProperties::default();
        let placement = self.text_placement(&properties, (0.5, 0.5))?;

        let (_media_id, track_id) = self.project.add_text(properties, placement);
        self.has_unsaved_changes = true;
        self.set_selection(Selection::Track(track_id));
        Ok(track_id)
    }

    /// Changes what a text element says or how it looks.
    ///
    /// The element's place in the picture is made to fit the new text,
    /// around the same middle as before. Nothing changes if the text cannot
    /// be drawn (for example because its font file is missing).
    pub fn set_text(
        &mut self,
        media_id: MediaId,
        properties: TextProperties,
    ) -> Result<(), String> {
        let current = self
            .project
            .visual_clips()
            .find(|(_, media)| media.id == media_id)
            .map(|(clip, _)| clip.placement)
            .ok_or("the text no longer exists")?;
        let middle = (
            current.x + current.width / 2.0,
            current.y + current.height / 2.0,
        );
        let placement = self.text_placement(&properties, middle)?;

        if self.project.set_text(media_id, properties, placement) {
            self.has_unsaved_changes = true;
        }
        Ok(())
    }

    /// Adds a visual layer named "Lyrics" for lines of lyrics, and selects
    /// it.
    pub fn add_lyrics_track(&mut self) -> TrackId {
        let track_id = self.project.add_track("Lyrics", TrackKind::Visual);
        self.has_unsaved_changes = true;
        self.set_selection(Selection::Track(track_id));
        track_id
    }

    /// Adds timed lines of lyrics as text clips, one after another on the
    /// layer `track_id` (a new "Lyrics" layer if `None`), in `style`.
    /// Returns the layer.
    pub fn add_lyrics(
        &mut self,
        lines: &[crate::LyricLine],
        style: &crate::LyricsStyle,
        track_id: Option<TrackId>,
    ) -> Result<TrackId, String> {
        if lines.is_empty() {
            return Err("there are no lines".to_owned());
        }
        // Measured before anything changes, so a bad font changes nothing.
        let mut placements = Vec::new();
        for line in lines {
            placements.push(self.lyric_placement(&line.text, style)?);
        }
        let track_id = track_id.unwrap_or_else(|| self.add_lyrics_track());
        for (line, placement) in lines.iter().zip(placements) {
            let start = (line.start_seconds.max(0.0) * 1000.0).round() as u64;
            let end = (line.end_seconds.max(0.0) * 1000.0).round() as u64;
            let duration = end.saturating_sub(start);
            let properties = TextProperties {
                text: line.text.clone(),
                ..style.text.clone()
            };
            let clip_id = self
                .project
                .add_text_clip(track_id, properties, placement, start, duration)
                .ok_or("the lyrics layer is gone")?;
            let duration = self.clip_duration(clip_id);
            self.project
                .set_clip_keyframes(clip_id, crate::lyric_keyframes(placement, duration, style));
        }
        self.has_unsaved_changes = true;
        self.set_selection(Selection::Track(track_id));
        Ok(track_id)
    }

    /// Starts a line of lyrics at `at_seconds` while it is being tapped
    /// along with the music. It lasts until it is ended with
    /// [`Self::end_lyric_line`] (or a few seconds, if it never is).
    pub fn start_lyric_line(
        &mut self,
        track_id: TrackId,
        text: &str,
        at_seconds: f64,
        style: &crate::LyricsStyle,
    ) -> Result<ClipId, String> {
        const UNTIL_ENDED_MILLISECONDS: u64 = 5000;
        let placement = self.lyric_placement(text, style)?;
        let properties = TextProperties {
            text: text.to_owned(),
            ..style.text.clone()
        };
        let start = (at_seconds.max(0.0) * 1000.0).round() as u64;
        let clip_id = self
            .project
            .add_text_clip(
                track_id,
                properties,
                placement,
                start,
                UNTIL_ENDED_MILLISECONDS,
            )
            .ok_or("the lyrics layer is gone")?;
        self.has_unsaved_changes = true;
        Ok(clip_id)
    }

    /// Ends a line of lyrics started by [`Self::start_lyric_line`] at
    /// `at_seconds`, and lets it fade as `style` says.
    pub fn end_lyric_line(&mut self, clip_id: ClipId, at_seconds: f64, style: &crate::LyricsStyle) {
        let end = (at_seconds.max(0.0) * 1000.0).round() as u64;
        self.project.trim_clip_end(clip_id, end);
        if let Some((_, clip)) = self.project.clip(clip_id) {
            let keyframes =
                crate::lyric_keyframes(clip.placement, clip.duration_milliseconds, style);
            self.project.set_clip_keyframes(clip_id, keyframes);
        }
        self.has_unsaved_changes = true;
    }

    /// Gives every text on the layer `track_id` the look of `style` (each
    /// keeps its words and timing). Returns how many texts changed.
    pub fn restyle_lyrics(
        &mut self,
        track_id: TrackId,
        style: &crate::LyricsStyle,
    ) -> Result<usize, String> {
        let texts: Vec<(ClipId, MediaId, String)> = self
            .project
            .track(track_id)
            .ok_or("the layer is gone")?
            .clips
            .iter()
            .filter_map(|clip| {
                let source = self.project.media_source(clip.media)?;
                Some((clip.id, source.id, source.text()?.text.clone()))
            })
            .collect();
        for (clip_id, media_id, text) in &texts {
            let placement = self.lyric_placement(text, style)?;
            let properties = TextProperties {
                text: text.clone(),
                ..style.text.clone()
            };
            self.project.set_text(*media_id, properties, placement);
            let duration = self.clip_duration(*clip_id);
            self.project
                .set_clip_keyframes(*clip_id, crate::lyric_keyframes(placement, duration, style));
        }
        if !texts.is_empty() {
            self.has_unsaved_changes = true;
        }
        Ok(texts.len())
    }

    /// Where a line of lyrics goes: centered across, at the style's height.
    fn lyric_placement(&self, text: &str, style: &crate::LyricsStyle) -> Result<Placement, String> {
        let properties = TextProperties {
            text: text.to_owned(),
            ..style.text.clone()
        };
        self.text_placement(&properties, (0.5, style.height.clamp(0.0, 1.0)))
    }

    fn clip_duration(&self, clip_id: ClipId) -> u64 {
        self.project
            .clip(clip_id)
            .map_or(0, |(_, clip)| clip.duration_milliseconds)
    }

    /// The part of the picture a text takes up when its middle is at
    /// `middle` (fractions of the frame): exactly as large as the text is
    /// drawn, so it is neither stretched nor squeezed.
    fn text_placement(
        &self,
        properties: &TextProperties,
        middle: (f64, f64),
    ) -> Result<Placement, String> {
        let settings = &self.project.settings;
        let image = crate::render_text(properties, settings.height)?;
        let width = f64::from(image.width) / f64::from(settings.width.max(1));
        let height = f64::from(image.height) / f64::from(settings.height.max(1));
        Ok(Placement {
            x: middle.0 - width / 2.0,
            y: middle.1 - height / 2.0,
            width,
            height,
        })
    }

    /// Imports the image sequence that `file` belongs to (the numbered files
    /// next to it) as a frame animation on a visual layer, which becomes
    /// selected.
    pub fn import_image_sequence(&mut self, file: &Path) -> Result<TrackId, String> {
        let (first_file, animation) = crate::find_image_sequence(file, DEFAULT_ANIMATION_SPEED)?;
        let (_media_id, track_id) = self.project.add_animation(first_file, animation);
        self.has_unsaved_changes = true;
        self.set_selection(Selection::Track(track_id));
        Ok(track_id)
    }

    /// Imports an image as a sprite sheet on a visual layer, which becomes
    /// selected. Its grid is guessed; it can be set in the Properties panel.
    pub fn import_sprite_sheet(&mut self, file: &Path) -> Result<TrackId, String> {
        if !crate::has_supported_image_extension(file) {
            return Err("not a PNG or JPEG image".to_owned());
        }
        let animation = crate::guess_sprite_sheet(file, DEFAULT_ANIMATION_SPEED)?;
        let (_media_id, track_id) = self
            .project
            .add_animation(file.to_string_lossy(), animation);
        self.has_unsaved_changes = true;
        self.set_selection(Selection::Track(track_id));
        Ok(track_id)
    }

    /// Adds a visualizer plugin and places it on a visual layer, which
    /// becomes selected. `stored_path` is the plugin's folder, or the
    /// reference of a bundled plugin (see `VisualizerCatalog`).
    pub fn add_visualizer_plugin(&mut self, stored_path: &str) -> TrackId {
        let (_media_id, track_id) = self.project.add_visualizer_plugin(stored_path);
        self.has_unsaved_changes = true;
        self.set_selection(Selection::Track(track_id));
        track_id
    }

    /// Removes a track, clearing the selection if it was selected.
    pub fn remove_track(&mut self, id: TrackId) {
        if self.project.remove_track(id) {
            self.has_unsaved_changes = true;
            if self.selection == Selection::Track(id) {
                self.set_selection(Selection::Nothing);
            }
        }
    }

    /// Read-only access to the project.
    pub fn project(&self) -> &Project {
        &self.project
    }

    /// Applies an edit to the project and records that it changed.
    ///
    /// All modifications go through this method so the "unsaved changes"
    /// flag cannot be forgotten. It is also the natural place to hook in
    /// undo/redo later. The closure returns `true` if it changed anything.
    pub fn edit_project(&mut self, edit: impl FnOnce(&mut Project) -> bool) {
        if edit(&mut self.project) {
            self.has_unsaved_changes = true;
        }
    }

    pub fn file_path(&self) -> Option<&Path> {
        self.file_path.as_deref()
    }

    /// The project's name: the name of its file without the extension, or
    /// `Untitled` while it has never been saved.
    ///
    /// This is the only name a project has, so the window title, dialogs
    /// and the file on disk can never disagree.
    pub fn project_name(&self) -> String {
        self.file_path
            .as_deref()
            .and_then(Path::file_stem)
            .map_or_else(
                || UNTITLED_PROJECT_NAME.to_owned(),
                |stem| stem.to_string_lossy().into_owned(),
            )
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.has_unsaved_changes
    }

    /// Saves the project to `path` and makes that its file from now on.
    ///
    /// If writing fails, the session is left exactly as it was: it keeps its
    /// previous path and still counts as having unsaved changes.
    pub fn save_as(&mut self, path: &Path) -> Result<(), ProjectFileError> {
        self.project.save_to_file(path)?;
        self.file_path = Some(path.to_owned());
        self.has_unsaved_changes = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique path in the system temp directory for one test.
    fn temporary_project_path(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "vizx-session-{test_name}-{}.vzx",
            std::process::id()
        ))
    }

    fn rename_project(session: &mut EditorSession, name: &str) {
        session.edit_project(|project| {
            project.metadata.author = name.to_owned();
            true
        });
    }

    #[test]
    fn bundled_preset_is_stored_by_its_place_in_the_collection() {
        let root =
            std::env::temp_dir().join(format!("vizx-session-bundled-{}", std::process::id()));
        let preset_file = root.join("pack/Fractal/Swirl.milk");
        std::fs::create_dir_all(preset_file.parent().unwrap()).unwrap();
        std::fs::write(&preset_file, "[preset00]").unwrap();
        let library = PresetLibrary::scan(&root).unwrap();
        let _ = std::fs::remove_dir_all(&root);

        let mut session = EditorSession::new_untitled();
        session.add_bundled_milkdrop_preset(&library.entries()[0]);

        assert!(session.has_unsaved_changes());
        let (_clip, preset) = session.project().milkdrop_clips().next().unwrap();
        // No folder of this computer ends up in the project file.
        assert_eq!(preset.path, "bundled-presets:/pack/Fractal/Swirl.milk");
        assert_eq!(
            crate::resolve_preset_path(&preset.path, Some(&root)),
            Some(root.join("pack/Fractal/Swirl.milk"))
        );
    }

    #[test]
    fn lyrics_become_timed_texts_on_one_layer_and_share_a_style() {
        let mut session = EditorSession::new_untitled();
        let lines = crate::spread_lines(&crate::split_lyrics("one\ntwo|rows"), 1.0, 5.0);
        let mut style = crate::LyricsStyle::default();

        let track_id = session.add_lyrics(&lines, &style, None).unwrap();
        // Tapped along: a third line, started and ended by hand.
        let tapped = session
            .start_lyric_line(track_id, "three", 6.0, &style)
            .unwrap();
        session.end_lyric_line(tapped, 7.5, &style);

        let track = session.project().track(track_id).unwrap();
        assert_eq!(track.name, "Lyrics");
        let timing: Vec<(u64, u64)> = track
            .clips
            .iter()
            .map(|clip| (clip.start_milliseconds, clip.end_milliseconds()))
            .collect();
        assert_eq!(timing[0].0, 1000);
        assert_eq!(timing[0].1, timing[1].0);
        assert_eq!(timing[1].1, 5000);
        assert_eq!(timing[2], (6000, 7500));
        // Faded in and out: invisible at both ends, shown in the middle.
        let tapped_clip = &track.clips[2];
        assert_eq!(tapped_clip.transform_at(0.0).opacity, 0.0);
        assert_eq!(tapped_clip.transform_at(750.0).opacity, 1.0);
        assert_eq!(tapped_clip.transform_at(1500.0).opacity, 0.0);
        let first_size = track.clips[0].placement.height;

        style.text.size *= 2.0;
        assert_eq!(session.restyle_lyrics(track_id, &style), Ok(3));

        let track = session.project().track(track_id).unwrap();
        let texts: Vec<_> = track
            .clips
            .iter()
            .map(|clip| {
                session
                    .project()
                    .media_source(clip.media)
                    .unwrap()
                    .text()
                    .unwrap()
            })
            .collect();
        assert_eq!(texts[1].text, "two\nrows");
        assert!(texts.iter().all(|text| text.size == style.text.size));
        assert!(track.clips[0].placement.height > 1.5 * first_size);
    }

    #[test]
    fn a_copied_clip_is_pasted_at_the_playhead_even_into_another_project() {
        let mut session = EditorSession::new_untitled();
        session.add_text().unwrap();
        let text_clip = session.project().visual_clips().next().unwrap().0.id;
        session.select(Selection::Clip(text_clip));
        let copied = session.copy_selected_clips();

        session.set_playhead_seconds(0.0);
        let pasted = session.paste_clips(&copied)[0];
        let mut other = EditorSession::new_untitled();
        other.set_playhead_seconds(1.5);
        let pasted_elsewhere = other.paste_clips(&copied)[0];

        let (_, clip) = session.project().clip(pasted).unwrap();
        // A pasted text is a text of its own, so it can be edited alone.
        assert_ne!(clip.media, copied[0].clip.media);
        assert_eq!(session.selected_clip_id(), Some(pasted));
        let (track, clip) = other.project().clip(pasted_elsewhere).unwrap();
        assert_eq!(track.kind, TrackKind::Visual);
        assert_eq!(clip.start_milliseconds, 1500);
        let source = other.project().media_source(clip.media).unwrap();
        assert_eq!(source.text(), copied[0].source.text());
    }

    #[test]
    fn the_playhead_steps_by_frames_and_jumps_between_clip_edges() {
        let mut session = EditorSession::new_untitled();
        session.add_text().unwrap();
        let clip = session.project().visual_clips().next().unwrap().0.id;
        session.edit_project(|project| {
            project.move_clip(clip, 2000);
            project.trim_clip_end(clip, 3000)
        });

        session.step_playhead_frames(3);
        assert!((session.playhead_seconds() - 0.1).abs() < 1e-9); // 30 fps
        session.step_playhead_frames(-10);
        assert_eq!(session.playhead_seconds(), 0.0);

        assert!(session.jump_to_clip_edge(true));
        assert_eq!(session.playhead_seconds(), 2.0);
        assert!(session.jump_to_clip_edge(true));
        assert_eq!(session.playhead_seconds(), 3.0);
        assert!(session.jump_to_clip_edge(false));
        assert_eq!(session.playhead_seconds(), 2.0);
        session.set_playhead_seconds(0.0);
        assert!(!session.jump_to_clip_edge(false));
    }

    #[test]
    fn a_loop_region_is_marked_toggled_and_markers_are_jumped_to() {
        let mut session = EditorSession::new_untitled();
        session.add_text().unwrap();
        session.set_playhead_seconds(2.0);
        session.add_marker_at_playhead();
        session.set_playhead_seconds(5.0);
        session.add_marker_at_playhead();

        session.set_playhead_seconds(0.0);
        assert!(session.jump_to_marker(true));
        assert_eq!(session.playhead_seconds(), 2.0);
        assert!(session.jump_to_marker(true));
        assert!(!session.jump_to_marker(true));
        assert!(session.jump_to_marker(false));
        assert_eq!(session.playhead_seconds(), 2.0);

        // In and out at the playhead.
        session.set_loop_start_at_playhead();
        session.set_playhead_seconds(4.0);
        session.set_loop_end_at_playhead();
        assert_eq!(session.loop_region(), Some((2.0, 4.0)));
        assert!(session.is_looping());
        assert!(!session.toggle_looping());
        assert_eq!(session.loop_region(), Some((2.0, 4.0)));
        // Too short a loop is ignored.
        session.set_loop_region(3.0, 3.01);
        assert_eq!(session.loop_region(), Some((2.0, 4.0)));
        session.clear_loop();
        // Without a region, the whole project loops.
        assert!(session.toggle_looping());
        assert_eq!(session.loop_region().unwrap().0, 0.0);
    }

    #[test]
    fn selected_clips_move_to_other_tracks_together() {
        let mut session = EditorSession::new_untitled();
        session.add_text().unwrap();
        session.add_text().unwrap();
        session.add_track(TrackKind::Visual);
        let visual: Vec<TrackId> = session
            .project()
            .tracks()
            .iter()
            .filter(|track| track.kind == TrackKind::Visual)
            .map(|track| track.id)
            .collect();
        assert_eq!(visual.len(), 3);
        let clip_on = |session: &EditorSession, track: TrackId| {
            session
                .project()
                .track(track)
                .unwrap()
                .clips
                .first()
                .map(|clip| clip.id)
        };
        let (a, b) = (
            clip_on(&session, visual[1]).unwrap(),
            clip_on(&session, visual[2]).unwrap(),
        );
        let track_of = |session: &EditorSession, id| session.project().clip(id).unwrap().0.id;

        session.select_clips(&[a, b]);
        // Dragging the lower clip up one track takes the other one along.
        assert!(session.move_selected_clips_to_track(b, visual[1]));
        assert_eq!(
            (track_of(&session, a), track_of(&session, b)),
            (visual[0], visual[1])
        );
        // The top clip cannot go higher, so nothing moves.
        assert!(!session.move_selected_clips_to_track(b, visual[0]));
        assert_eq!(track_of(&session, a), visual[0]);
    }

    #[test]
    fn several_clips_are_selected_moved_copied_and_deleted_together() {
        let mut session = EditorSession::new_untitled();
        session.add_text().unwrap();
        session.add_text().unwrap();
        let ids: Vec<ClipId> = session
            .project()
            .visual_clips()
            .map(|(clip, _)| clip.id)
            .collect();
        session.edit_project(|project| {
            project.trim_clip_end(ids[0], 2000);
            project.trim_clip_end(ids[1], 2000);
            project.move_clip(ids[1], 1000)
        });

        session.select(Selection::Clip(ids[0]));
        session.toggle_clip_selection(ids[1]);
        assert_eq!(session.selected_clip_ids(), ids);
        let track_of_first = session.project().clip(ids[0]).unwrap().0.id;
        assert!(session.is_clip_selected(track_of_first, ids[0]));

        // Dragging one moves both; none goes before 0.
        assert!(session.move_selected_clips(ids[1], 3000));
        let start =
            |session: &EditorSession, id| session.project().clip(id).unwrap().1.start_milliseconds;
        assert_eq!(
            (start(&session, ids[0]), start(&session, ids[1])),
            (2000, 3000)
        );
        session.move_selected_clips(ids[1], 0);
        assert_eq!(
            (start(&session, ids[0]), start(&session, ids[1])),
            (0, 1000)
        );

        // Copied and pasted, they keep their distance.
        let copied = session.copy_selected_clips();
        session.set_playhead_seconds(5.0);
        let pasted = session.paste_clips(&copied);
        assert_eq!(pasted.len(), 2);
        assert_eq!(
            (start(&session, pasted[0]), start(&session, pasted[1])),
            (5000, 6000)
        );
        assert_eq!(session.selected_clip_ids(), pasted);

        assert_eq!(session.delete_selection(), Some("clips"));
        assert_eq!(session.project().visual_clips().count(), 2);
        // A plain selection forgets the others.
        session.select(Selection::Clip(ids[0]));
        assert_eq!(session.selected_clip_ids(), [ids[0]]);
        session.select_all_clips();
        assert_eq!(session.selected_clip_ids().len(), 2);
    }

    #[test]
    fn a_selected_clip_can_be_split_duplicated_and_deleted() {
        let mut session = EditorSession::new_untitled();
        let layer = session.add_text().unwrap();
        let clip = session.project().track(layer).unwrap().clips[0].id;
        session.edit_project(|project| project.trim_clip_end(clip, 4_000));

        // Nothing happens while only the track is selected.
        assert!(!session.split_selected_clip());
        assert!(!session.duplicate_selected_clip());

        session.select(Selection::Clip(clip));
        assert_eq!(session.selected_track().unwrap().id, layer);
        assert!(session.is_clip_selected(layer, clip));
        // The playhead is at 0, which is the clip's edge, not inside it.
        assert!(!session.split_selected_clip());
        session.set_playhead_seconds(1.0);
        assert!(session.split_selected_clip());
        assert_eq!(session.project().track(layer).unwrap().clips.len(), 2);
        assert_eq!(session.selected_clip_id(), Some(clip));

        assert!(session.duplicate_selected_clip());
        let copy = session.selected_clip_id().unwrap();
        assert_ne!(copy, clip);
        assert_eq!(session.project().track(layer).unwrap().clips.len(), 3);

        // Deleting the clip leaves its track selected; deleting again
        // removes the track.
        assert_eq!(session.delete_selection(), Some("clip"));
        assert_eq!(session.selection(), Selection::Track(layer));
        assert_eq!(session.project().track(layer).unwrap().clips.len(), 2);
        assert_eq!(session.delete_selection(), Some("track"));
        assert!(session.project().track(layer).is_none());
        assert_eq!(session.delete_selection(), None);
    }

    #[test]
    fn any_edit_can_be_undone_and_redone() {
        let mut session = EditorSession::new_untitled();
        let track_count = session.project().tracks().len();
        assert!(!session.can_undo());

        // Edits made through different doors: a session method and
        // `edit_project`. Neither has to know about the history.
        let layer = session.add_track(TrackKind::Visual);
        session.edit_project(|project| project.rename_track(layer, "Title"));
        assert!(session.can_undo());

        assert!(session.undo());
        assert_eq!(session.project().tracks().len(), track_count);
        assert!(session.has_unsaved_changes());
        assert!(!session.undo());

        assert!(session.redo());
        assert_eq!(session.project().track(layer).unwrap().name, "Title");
        assert!(!session.redo());
    }

    #[test]
    fn beats_are_tapped_in_at_the_playhead_once_drawn_by_hand() {
        let mut session = EditorSession::new_untitled();
        let (song, track) = session.project.import_audio(
            "song.wav",
            AudioProperties {
                duration_milliseconds: 10_000,
                sample_rate: 44_100,
                channel_count: 2,
            },
        );
        let clip = session.project().track(track).unwrap().clips[0].id;
        // The clip plays the file from 2 s on, starting at 1 s.
        session.edit_project(|project| project.trim_clip_start(clip, 2_000));
        session.edit_project(|project| project.move_clip(clip, 1_000));
        session.set_playhead_seconds(3.5);

        assert!(session.tap_beat().is_err()); // nothing selected
        session.select(Selection::Clip(clip));
        assert!(session.tap_beat().unwrap_err().contains("manual beats"));

        session.edit_project(|project| project.use_manual_beats(song, Vec::new()));
        session.tap_beat().unwrap();
        // 3.5 s of the project is 2.5 s into the clip, 4.5 s into the file.
        assert_eq!(
            session.project().media_source(song).unwrap().manual_beats,
            Some(vec![4.5])
        );
        session.set_playhead_seconds(0.5);
        assert!(session.tap_beat().unwrap_err().contains("outside"));
    }

    #[test]
    fn new_session_is_unsaved_on_disk_but_has_no_pending_changes() {
        let session = EditorSession::new_untitled();

        assert!(session.file_path().is_none());
        assert!(!session.has_unsaved_changes());
    }

    #[test]
    fn only_real_edits_mark_the_session_as_changed() {
        let mut session = EditorSession::new_untitled();

        session.edit_project(|_project| false);
        assert!(!session.has_unsaved_changes());

        rename_project(&mut session, "Renamed");
        assert!(session.has_unsaved_changes());
        assert_eq!(session.project().metadata.author, "Renamed");
    }

    #[test]
    fn adding_a_track_selects_it_and_marks_the_project_changed() {
        let mut session = EditorSession::new_untitled();
        let track_count_before = session.project().tracks().len();

        let id = session.add_track(TrackKind::Visual);

        assert_eq!(session.project().tracks().len(), track_count_before + 1);
        assert_eq!(session.selection(), Selection::Track(id));
        assert_eq!(session.selected_track().unwrap().kind, TrackKind::Visual);
        assert!(session.has_unsaved_changes());
    }

    #[test]
    fn removing_the_selected_track_clears_the_selection() {
        let mut session = EditorSession::new_untitled();
        let id = session.add_track(TrackKind::Audio);

        session.remove_track(id);

        assert_eq!(session.selection(), Selection::Nothing);
        assert!(session.selected_track().is_none());
    }

    #[test]
    fn importing_an_unreadable_file_changes_nothing() {
        let mut session = EditorSession::new_untitled();
        let missing = std::env::temp_dir().join("vizx-session-missing-audio.wav");

        assert!(session.import_audio(&missing).is_err());
        assert!(session.project().media().is_empty());
        assert!(!session.has_unsaved_changes());
        assert_eq!(session.selection(), Selection::Nothing);
    }

    #[test]
    fn playhead_stays_within_the_project_and_is_not_an_edit() {
        let mut session = EditorSession::new_untitled();
        let project_end = session.project().settings.duration_seconds();

        session.set_playhead_seconds(12.5);
        assert_eq!(session.playhead_seconds(), 12.5);

        session.set_playhead_seconds(-3.0);
        assert_eq!(session.playhead_seconds(), 0.0);

        session.set_playhead_seconds(project_end + 100.0);
        assert_eq!(session.playhead_seconds(), project_end);

        session.set_playhead_seconds(f64::NAN);
        assert_eq!(session.playhead_seconds(), 0.0);

        assert!(!session.has_unsaved_changes());
    }

    #[test]
    fn only_existing_milk_files_can_be_added_as_presets() {
        let mut session = EditorSession::new_untitled();
        let folder = std::env::temp_dir();
        let preset = folder.join(format!("vizx-session-preset-{}.milk", std::process::id()));
        std::fs::write(&preset, "[preset00]\n").unwrap();

        let wrong_extension = session.add_milkdrop_preset(&folder.join("song.mp3"));
        let missing = session.add_milkdrop_preset(&folder.join("vizx-no-such-preset.milk"));
        assert!(!session.has_unsaved_changes());

        let added = session.add_milkdrop_preset(&preset);
        let _ = std::fs::remove_file(&preset);

        assert!(wrong_extension.is_err());
        assert!(missing.is_err());
        let track_id = added.unwrap();
        assert_eq!(session.selection(), Selection::Track(track_id));
        assert!(session.has_unsaved_changes());
        assert!(session.project().milkdrop_preset_at(0).is_some());
    }

    #[test]
    fn selecting_does_not_count_as_a_change() {
        let mut session = EditorSession::new_untitled();
        let id = session.project().tracks()[0].id;

        session.select(Selection::Track(id));

        assert!(!session.has_unsaved_changes());
    }

    #[test]
    fn saving_remembers_the_path_and_clears_unsaved_changes() {
        let path = temporary_project_path("save");
        let mut session = EditorSession::new_untitled();
        rename_project(&mut session, "Saved");

        let result = session.save_as(&path);
        let _ = std::fs::remove_file(&path);

        result.unwrap();
        assert_eq!(session.file_path(), Some(path.as_path()));
        assert!(!session.has_unsaved_changes());
    }

    #[test]
    fn saved_project_can_be_opened_again() {
        let path = temporary_project_path("reopen");
        let mut session = EditorSession::new_untitled();
        rename_project(&mut session, "Reopened");
        session.save_as(&path).unwrap();

        let reopened = EditorSession::open(&path);
        let _ = std::fs::remove_file(&path);

        let reopened = reopened.unwrap();
        assert_eq!(reopened.project(), session.project());
        assert_eq!(reopened.file_path(), Some(path.as_path()));
        assert!(!reopened.has_unsaved_changes());
    }

    #[test]
    fn project_is_named_after_its_file() {
        let path = temporary_project_path("named");
        let mut session = EditorSession::new_untitled();
        assert_eq!(session.project_name(), "Untitled");

        let result = session.save_as(&path);
        let name_after_save = session.project_name();
        let reopened_name = EditorSession::open(&path).map(|session| session.project_name());
        let _ = std::fs::remove_file(&path);

        result.unwrap();
        let expected = path.file_stem().unwrap().to_string_lossy().into_owned();
        assert_eq!(name_after_save, expected);
        assert_eq!(reopened_name.unwrap(), expected);
    }

    #[test]
    fn saving_under_another_name_renames_the_project() {
        let first = temporary_project_path("first-name");
        let second = temporary_project_path("second-name");
        let mut session = EditorSession::new_untitled();

        let first_result = session.save_as(&first);
        let second_result = session.save_as(&second);
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);

        first_result.unwrap();
        second_result.unwrap();
        assert_eq!(
            session.project_name(),
            second.file_stem().unwrap().to_string_lossy()
        );
    }

    #[test]
    fn failed_save_leaves_the_session_unchanged() {
        // A directory that does not exist, so the write must fail.
        let path = std::env::temp_dir()
            .join("vizx-missing-directory")
            .join("project.vzx");
        let mut session = EditorSession::new_untitled();
        rename_project(&mut session, "Not saved");

        assert!(session.save_as(&path).is_err());
        assert!(session.file_path().is_none());
        assert!(session.has_unsaved_changes());
    }
}
