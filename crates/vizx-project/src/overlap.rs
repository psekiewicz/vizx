//! Keeping clips on one track from covering each other.
//!
//! The editor can forbid overlapping clips (the usual way of video editors:
//! a track plays one clip at a time). The functions here find where clips
//! may go: how far a group may be shifted, how far an edge may be trimmed,
//! and whether a clip fits on another track.

use crate::{ClipId, Project, TrackId};

/// Whether the time ranges `[a_start, a_end)` and `[b_start, b_end)` share
/// any moment. Clips that only touch do not overlap.
fn ranges_overlap((a_start, a_end): (i64, i64), (b_start, b_end): (i64, i64)) -> bool {
    a_start < b_end && b_start < a_end
}

impl Project {
    /// The time ranges of the clips on `track_id`, except `ignored` ones.
    fn occupied(&self, track_id: TrackId, ignored: &[ClipId]) -> Vec<(i64, i64)> {
        self.track(track_id)
            .map(|track| {
                track
                    .clips
                    .iter()
                    .filter(|clip| !ignored.contains(&clip.id))
                    .map(|clip| (clip.start_milliseconds as i64, clip.end_milliseconds() as i64))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether shifting the clips `ids` by `shift` milliseconds keeps them
    /// clear of every other clip on their tracks and after the start.
    fn shift_is_free(&self, ids: &[ClipId], shift: i64) -> bool {
        ids.iter().all(|id| {
            let Some((track, clip)) = self.clip(*id) else {
                return true;
            };
            let start = clip.start_milliseconds as i64 + shift;
            let range = (start, start + clip.duration_milliseconds as i64);
            start >= 0
                && !self
                    .occupied(track.id, ids)
                    .into_iter()
                    .any(|other| ranges_overlap(range, other))
        })
    }

    /// Shifts at which one of the clips `ids` would just touch another clip
    /// of its track, or the start of the project.
    fn touching_shifts(&self, ids: &[ClipId]) -> Vec<i64> {
        let mut shifts = Vec::new();
        for id in ids {
            let Some((track, clip)) = self.clip(*id) else {
                continue;
            };
            let (start, end) = (clip.start_milliseconds as i64, clip.end_milliseconds() as i64);
            shifts.push(-start);
            for (other_start, other_end) in self.occupied(track.id, ids) {
                shifts.push(other_end - start); // right after the other
                shifts.push(other_start - end); // right before the other
            }
        }
        shifts
    }

    /// The shift closest to `wanted` (in milliseconds) by which the clips
    /// `ids` can move together without covering another clip: `wanted`
    /// itself if there is room, otherwise the nearest place where they fit,
    /// pressed against a neighbour. 0 if they cannot move at all.
    pub fn free_shift(&self, ids: &[ClipId], wanted: i64) -> i64 {
        if self.shift_is_free(ids, wanted) {
            return wanted;
        }
        self.touching_shifts(ids)
            .into_iter()
            // Only between where they are and where they are wanted, so a
            // dragged clip never jumps over a neighbour.
            .filter(|shift| {
                let (low, high) = (wanted.min(0), wanted.max(0));
                (low..=high).contains(shift)
            })
            .filter(|shift| self.shift_is_free(ids, *shift))
            .min_by_key(|shift| (shift - wanted).abs())
            .unwrap_or(0)
    }

    /// The smallest shift of 0 or more after which the clips `ids` cover no
    /// other clip (for pasted and duplicated clips, which go to the next
    /// free place).
    pub fn first_free_shift_forward(&self, ids: &[ClipId]) -> i64 {
        let mut candidates: Vec<i64> = self
            .touching_shifts(ids)
            .into_iter()
            .filter(|shift| *shift >= 0)
            .collect();
        candidates.push(0);
        candidates.sort_unstable();
        candidates
            .into_iter()
            .find(|shift| self.shift_is_free(ids, *shift))
            .unwrap_or(0)
    }

    /// How far the edges of a clip may be trimmed without covering its
    /// neighbours: the end of the clip before it, and the start of the clip
    /// after it (`None` if there is none).
    pub fn trim_room(&self, id: ClipId) -> (u64, Option<u64>) {
        let Some((track, clip)) = self.clip(id) else {
            return (0, None);
        };
        let (start, end) = (clip.start_milliseconds as i64, clip.end_milliseconds() as i64);
        let others = self.occupied(track.id, &[id]);
        let earliest = others
            .iter()
            .filter(|(_, other_end)| *other_end <= start)
            .map(|(_, other_end)| *other_end)
            .max()
            .unwrap_or(0);
        let latest = others
            .iter()
            .filter(|(other_start, _)| *other_start >= end)
            .map(|(other_start, _)| *other_start)
            .min();
        (earliest as u64, latest.map(|latest| latest as u64))
    }

    /// Whether the clips `moves` (each with the track it would go to) fit
    /// there without covering clips that stay where they are.
    pub fn fits_on_tracks(&self, moves: &[(ClipId, TrackId)]) -> bool {
        let moving: Vec<ClipId> = moves.iter().map(|(id, _)| *id).collect();
        moves.iter().all(|(id, track_id)| {
            let Some((_, clip)) = self.clip(*id) else {
                return true;
            };
            let range = (clip.start_milliseconds as i64, clip.end_milliseconds() as i64);
            // Clips moving onto the same track must not cover each other.
            let fellow_movers = moves.iter().filter(|(other, other_track)| {
                other != id && other_track == track_id
            });
            let covers_mover = fellow_movers.filter_map(|(other, _)| self.clip(*other)).any(
                |(_, other)| {
                    ranges_overlap(
                        range,
                        (other.start_milliseconds as i64, other.end_milliseconds() as i64),
                    )
                },
            );
            !covers_mover
                && !self
                    .occupied(*track_id, &moving)
                    .into_iter()
                    .any(|other| ranges_overlap(range, other))
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{AudioProperties, Project};

    fn song(milliseconds: u64) -> AudioProperties {
        AudioProperties {
            duration_milliseconds: milliseconds,
            sample_rate: 44_100,
            channel_count: 2,
        }
    }

    /// One audio track with clips at 0-2 s and 5-7 s; returns their ids.
    fn two_clips() -> (Project, crate::ClipId, crate::ClipId) {
        let mut project = Project::new();
        let (_, track) = project.import_audio("a.wav", song(2_000));
        let first = project.track(track).unwrap().clips[0].clone();
        let source = project.media_source(first.media).unwrap().clone();
        let second = project.paste_clip(&first, &source, track, 5_000).unwrap();
        (project, first.id, second)
    }

    #[test]
    fn a_dragged_clip_stops_at_its_neighbour() {
        let (project, first, second) = two_clips();
        // Room enough: moves as wanted.
        assert_eq!(project.free_shift(&[first], 1_000), 1_000);
        // Into the next clip: stops right before it (ends at 5 s).
        assert_eq!(project.free_shift(&[first], 4_000), 3_000);
        // Not before the start of the project.
        assert_eq!(project.free_shift(&[second], -6_000), -3_000);
        // Both together move freely.
        assert_eq!(project.free_shift(&[first, second], 500), 500);
    }

    #[test]
    fn trimming_and_pasting_respect_neighbours() {
        let (project, first, second) = two_clips();
        assert_eq!(project.trim_room(first), (0, Some(5_000)));
        assert_eq!(project.trim_room(second), (2_000, None));
        // A copy laid over the first clip goes to the next free place.
        assert_eq!(project.first_free_shift_forward(&[first]), 0);
    }
}
