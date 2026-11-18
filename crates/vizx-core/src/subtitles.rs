//! Where the lines of subtitle layers are drawn.
//!
//! A subtitle line is drawn like a text element, but its look and place
//! come from its layer (`vizx_project::SubtitleStyle`) and are worked out
//! here, for the preview and the export alike:
//!
//! 1. **Fit.** A line wider than the frame's safe area (inside the margin)
//!    is broken into rows between words, and made smaller only if that is
//!    not enough, so no line runs past the edge.
//! 2. **Place.** Around the layer's fixed point, or at a random spot of its
//!    area; random spots come from the layer's seed and the clip, so they
//!    are the same every time.
//! 3. **Keep apart.** Lines shown at the same time (from one layer or from
//!    several, say two singers) are moved until they no longer cover each
//!    other: a fixed line steps up or down by its own height, a random one
//!    tries other spots.
//! 4. **Appear.** Fading in and out (and growing, with "pop") as the layer
//!    says.

use std::collections::HashMap;

use vizx_project::{
    ClipId, Motion, Placement, Project, SubtitleFit, SubtitlePosition, SubtitleStyle,
    TextProperties, TrackId, TrackKind, Transform,
};

use crate::text::{measure_text, wrap_text};
use crate::{LyricsStyle, lyric_keyframes};

/// One subtitle line, ready to be drawn like a text element.
#[derive(Debug, Clone, PartialEq)]
pub struct SubtitleLine {
    pub track_id: TrackId,
    /// The words, already broken into rows, in the layer's look.
    pub text: TextProperties,
    /// Where it is and how it appears over the clip's time.
    pub motion: Motion,
}

/// Every subtitle line of `project`, by clip, laid out for a frame
/// `frame_height` pixels high (the width follows the project's shape).
pub fn subtitle_layout(project: &Project, frame_height: u32) -> HashMap<ClipId, SubtitleLine> {
    let settings = &project.settings;
    let frame_height = frame_height.max(2);
    let aspect = f64::from(settings.width.max(1)) / f64::from(settings.height.max(1));
    let frame_width = (f64::from(frame_height) * aspect).round().max(2.0);

    // Every line, earliest first; at the same moment the higher layer first.
    let mut lines: Vec<(TrackId, &SubtitleStyle, &vizx_project::Clip, &str)> = project
        .tracks()
        .iter()
        .filter(|track| track.kind == TrackKind::Subtitles)
        .flat_map(|track| {
            let style = track.subtitle_style.as_ref();
            track.clips.iter().filter_map(move |clip| {
                let text = project.media_source(clip.media)?.text()?;
                Some((track.id, style?, clip, text.text.as_str()))
            })
        })
        .collect();
    lines.sort_by_key(|(_, _, clip, _)| clip.start_milliseconds);

    let mut placed: Vec<(u64, u64, Placement)> = Vec::new();
    let mut layout = HashMap::new();
    for (track_id, style, clip, words) in lines {
        let style = style.clone().sanitized();
        let margin_x = style.margin / aspect;
        let margin_y = style.margin;
        let (text, size) =
            fitted_text(&style, words, frame_height, frame_width, margin_x, margin_y);
        let (start, end) = (clip.start_milliseconds, clip.end_milliseconds());
        let candidates = candidate_places(&style, clip.id, size, (margin_x, margin_y));
        let taken: Vec<Placement> = placed
            .iter()
            .filter(|(other_start, other_end, _)| *other_start < end && start < *other_end)
            .map(|(_, _, place)| *place)
            .collect();
        let placement = candidates
            .iter()
            .copied()
            .min_by(|a, b| {
                let cover = |place: &Placement| -> f64 {
                    taken.iter().map(|other| overlap_area(place, other)).sum()
                };
                // The first candidate that covers nothing wins; otherwise
                // the one that covers least.
                cover(a).total_cmp(&cover(b))
            })
            .unwrap_or(Placement {
                x: 0.5 - size.0 / 2.0,
                y: 0.5 - size.1 / 2.0,
                width: size.0,
                height: size.1,
            });
        placed.push((start, end, placement));

        let keyframes = lyric_keyframes(
            placement,
            clip.duration_milliseconds,
            &LyricsStyle {
                text: text.clone(),
                height: 0.0,
                fade_seconds: style.fade_seconds,
                pop: style.pop,
            },
        );
        layout.insert(
            clip.id,
            SubtitleLine {
                track_id,
                text,
                motion: Motion {
                    resting: Transform {
                        placement,
                        rotation_degrees: 0.0,
                        opacity: 1.0,
                    },
                    keyframes,
                    spin_degrees_per_second: 0.0,
                    entrance_exit: clip.entrance_exit,
                    clip_seconds: clip.duration_milliseconds as f64 / 1000.0,
                },
            },
        );
    }
    layout
}

/// The line in the layer's look, made to fit the safe area as the layer
/// says, and the size of its picture as fractions of the frame.
fn fitted_text(
    style: &SubtitleStyle,
    words: &str,
    frame_height: u32,
    frame_width: f64,
    margin_x: f64,
    margin_y: f64,
) -> (TextProperties, (f64, f64)) {
    let mut text = style.text(words);
    let room_width = (frame_width * (1.0 - 2.0 * margin_x)).max(8.0);
    let room_height = (f64::from(frame_height) * (1.0 - 2.0 * margin_y)).max(8.0);
    let measure = |text: &TextProperties| {
        measure_text(text, frame_height).map_or((0.0, 0.0), |(width, height)| {
            (f64::from(width), f64::from(height))
        })
    };
    if style.fit == SubtitleFit::Wrap
        && let Ok(wrapped) = wrap_text(&text, frame_height, room_width as u32)
    {
        text.text = wrapped;
    }
    let (mut width, mut height) = measure(&text);
    if style.fit != SubtitleFit::Overflow && (width > room_width || height > room_height) {
        // A little smaller than needed: the picture's own margin does not
        // shrink exactly with the letters.
        let factor = (room_width / width).min(room_height / height) * 0.97;
        text.size = (text.size * factor).max(SubtitleStyle::SIZE_RANGE.0 / 4.0);
        (width, height) = measure(&text);
    }
    (
        text,
        (width / frame_width, height / f64::from(frame_height)),
    )
}

/// Places to try for a line of `size` (fractions of the frame), best first,
/// each kept inside the margins.
fn candidate_places(
    style: &SubtitleStyle,
    clip_id: ClipId,
    size: (f64, f64),
    margins: (f64, f64),
) -> Vec<Placement> {
    let (width, height) = size;
    let keep_inside = style.fit != SubtitleFit::Overflow;
    let around = |middle_x: f64, middle_y: f64| {
        let mut place = Placement {
            x: middle_x - width / 2.0,
            y: middle_y - height / 2.0,
            width,
            height,
        };
        if keep_inside {
            place.x = clamp_span(place.x, width, margins.0);
            place.y = clamp_span(place.y, height, margins.1);
        }
        place
    };
    match style.position {
        SubtitlePosition::Fixed { x, y } => {
            // Step away from the point by whole line heights, first towards
            // the middle of the frame (up from the bottom, down from the
            // top), then the other way.
            let towards_middle = if y > 0.5 { -1.0 } else { 1.0 };
            let step = height * 1.08;
            let mut places = vec![around(x, y)];
            for distance in 1..=6 {
                let distance = f64::from(distance);
                places.push(around(x, y + towards_middle * step * distance));
                places.push(around(x, y - towards_middle * step * distance));
            }
            places
        }
        SubtitlePosition::Random { area } => {
            let mut random = Random::new(style.seed, clip_id);
            (0..16)
                .map(|_| {
                    let spot = |start: f64, span: f64, own: f64, unit: f64| {
                        let free = (span - own).max(0.0);
                        start + own / 2.0 + free * unit
                    };
                    around(
                        spot(area.x, area.width, width, random.unit()),
                        spot(area.y, area.height, height, random.unit()),
                    )
                })
                .collect()
        }
    }
}

/// `start` moved so that a span of `length` lies within `margin` of both
/// edges of 0..1 (centered when it is longer than the room there is).
fn clamp_span(start: f64, length: f64, margin: f64) -> f64 {
    let (lowest, highest) = (margin, 1.0 - margin - length);
    if highest < lowest {
        (1.0 - length) / 2.0
    } else {
        start.clamp(lowest, highest)
    }
}

fn overlap_area(a: &Placement, b: &Placement) -> f64 {
    // A little space between lines counts as covering.
    let gap = 0.01;
    let width = (a.x + a.width + gap).min(b.x + b.width + gap) - a.x.max(b.x);
    let height = (a.y + a.height + gap).min(b.y + b.height + gap) - a.y.max(b.y);
    width.max(0.0) * height.max(0.0)
}

/// Numbers that look random but follow from the layer's seed and the clip
/// (SplitMix64).
struct Random(u64);

impl Random {
    fn new(seed: u64, clip_id: ClipId) -> Self {
        let clip = format!("{clip_id:?}");
        let mixed = clip
            .bytes()
            .fold(seed ^ 0x9E37_79B9_7F4A_7C15, |state, byte| {
                (state ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3)
            });
        Self(mixed)
    }

    /// A number from 0 to 1.
    fn unit(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^= value >> 31;
        (value >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_with_lines(lines: &[(&str, u64, u64)]) -> (Project, TrackId, Vec<ClipId>) {
        let mut project = Project::new();
        project.settings.width = 1920;
        project.settings.height = 1080;
        let track = project.add_subtitle_track("Singer");
        let clips = lines
            .iter()
            .map(|(text, start, duration)| {
                project
                    .add_subtitle_line(track, text, *start, *duration)
                    .unwrap()
            })
            .collect();
        (project, track, clips)
    }

    fn inside_frame(place: &Placement, margin_x: f64, margin_y: f64) -> bool {
        let slack = 1e-6;
        place.x >= margin_x - slack
            && place.y >= margin_y - slack
            && place.x + place.width <= 1.0 - margin_x + slack
            && place.y + place.height <= 1.0 - margin_y + slack
    }

    #[test]
    fn a_long_line_is_wrapped_to_stay_inside_the_frame() {
        let words = "Take a long look at all the people all around you, do they look \
                     happy with the choices that they have made";
        let (project, _, clips) = project_with_lines(&[(words, 0, 3000)]);
        let line = &subtitle_layout(&project, 1080)[&clips[0]];
        assert!(line.text.text.lines().count() >= 2, "{}", line.text.text);
        let margin = SubtitleStyle::default().margin;
        assert!(inside_frame(
            &line.motion.resting.placement,
            margin * 1080.0 / 1920.0,
            margin
        ));
        // It fades in and out.
        assert_eq!(
            line.motion.keyframes.first().unwrap().transform.opacity,
            0.0
        );
    }

    #[test]
    fn a_single_huge_word_is_shrunk_instead() {
        let (mut project, track, clips) = project_with_lines(&[("Supercalifragilistic", 0, 1000)]);
        project.set_subtitle_style(
            track,
            SubtitleStyle {
                size: 0.3,
                ..SubtitleStyle::default()
            },
        );
        let line = &subtitle_layout(&project, 1080)[&clips[0]];
        assert!(line.text.size < 0.3);
        assert!(line.motion.resting.placement.width <= 1.0 - 2.0 * 0.05 * 1080.0 / 1920.0 + 1e-6);
    }

    #[test]
    fn lines_shown_together_do_not_cover_each_other() {
        // Two singers at the same moment, on two layers at the same place.
        let (mut project, _, first) = project_with_lines(&[("First singer", 0, 2000)]);
        let second_track = project.add_subtitle_track("Second");
        let second = project
            .add_subtitle_line(second_track, "Second singer", 500, 2000)
            .unwrap();
        let later = project
            .add_subtitle_line(second_track, "Later line", 5000, 1000)
            .unwrap();
        let layout = subtitle_layout(&project, 1080);
        let (a, b) = (
            layout[&first[0]].motion.resting.placement,
            layout[&second].motion.resting.placement,
        );
        assert_eq!(overlap_area(&a, &b), 0.0, "{a:?} {b:?}");
        // The second moved up from the bottom, towards the middle.
        assert!(b.y < a.y);
        // A line on its own stays at the layer's place.
        let alone = layout[&later].motion.resting.placement;
        assert!((alone.y + alone.height / 2.0 - 0.86).abs() < 0.01);
    }

    #[test]
    fn random_places_stay_in_the_area_and_repeat_and_change_with_the_seed() {
        let lines: Vec<(String, u64, u64)> = (0..12)
            .map(|index| (format!("Line {index}"), index * 1000, 1000))
            .collect();
        let borrowed: Vec<(&str, u64, u64)> = lines
            .iter()
            .map(|(text, start, duration)| (text.as_str(), *start, *duration))
            .collect();
        let (mut project, track, clips) = project_with_lines(&borrowed);
        let area = Placement {
            x: 0.5,
            y: 0.0,
            width: 0.5,
            height: 0.5,
        };
        let style = SubtitleStyle {
            position: SubtitlePosition::Random { area },
            margin: 0.0,
            ..SubtitleStyle::default()
        };
        project.set_subtitle_style(track, style.clone());
        let first = subtitle_layout(&project, 1080);
        let middles: Vec<(f64, f64)> = clips
            .iter()
            .map(|clip| {
                let place = first[clip].motion.resting.placement;
                (place.x + place.width / 2.0, place.y + place.height / 2.0)
            })
            .collect();
        for (x, y) in &middles {
            assert!(
                (0.5..=1.0).contains(x) && (0.0..=0.5).contains(y),
                "{x}, {y}"
            );
        }
        // Not all in one spot.
        let spread = middles
            .iter()
            .map(|(x, _)| *x)
            .fold(f64::MAX, f64::min)
            .max(0.0);
        assert!(middles.iter().any(|(x, _)| x - spread > 0.05));
        // The same every time; another seed, other places.
        assert_eq!(subtitle_layout(&project, 1080), first);
        project.set_subtitle_style(track, SubtitleStyle { seed: 2, ..style });
        assert_ne!(subtitle_layout(&project, 1080), first);
    }
}
