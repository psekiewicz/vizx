//! Builds a showcase project that uses as many VizX features as possible
//! for one song, saves it, and exports it to MP4 with the same export the
//! application uses.
//!
//! ```text
//! cargo run --release -p vizx-core --example showcase -- \
//!     <song> <stems folder> <project with timed lyrics> <output folder> \
//!     [--height 1080] [--samples]
//! ```
//!
//! * `<stems folder>` holds `<song name> - Vocals.wav` and the other three
//!   stems, as VizX writes them.
//! * The lyrics are the text clips of the topmost layer of an existing VizX
//!   project (for example one made with Recognize from Vocals).
//! * The pictures, the sprite sheet, the image sequence and the video it
//!   uses are generated into `<output folder>/assets`, so nothing else is
//!   needed.
//! * `--samples` exports two seconds of every part of the song, small, to
//!   check them quickly instead of the whole video.
//!
//! The song's structure (where the verses and choruses are) is written
//! down for ANTIVILLAIN; for another song the parts come out wrong but
//! everything still works.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use image::{Rgba, RgbaImage};
use vizx_core::{
    AudioStore, EditorSession, Export, ExportJob, ExportQuality, ExportSettings, LyricLine,
    LyricsStyle, PresetLibrary, STEM_NAMES, VideoFormat, VisualizerCatalog, find_bundled_presets,
    find_ffmpeg, lyric_keyframes, render_text,
};
use vizx_project::{
    AudioInput, AudioReaction, BlendMode, ClipEffects, ClipId, Easing, FrameAnimation, Keyframe,
    MediaId, Placement, Project, ReactionBand, TextAlignment, TextProperties, TrackId, TrackKind,
    Transform,
};

/// One part of the song: when it starts, its name, and what the caption
/// says is shown in it.
struct Part {
    start: f64,
    name: &'static str,
    caption: &'static str,
    kind: PartKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PartKind {
    Intro,
    Verse,
    PreChorus,
    Chorus,
    Bridge,
    Countdown,
    Rap,
    Outro,
}

/// ANTIVILLAIN, from its lyrics' timing.
const PARTS: [Part; 13] = [
    Part {
        start: 0.0,
        name: "Intro",
        caption: "INTRO  -  background image with keyframes, image sequence, title text",
        kind: PartKind::Intro,
    },
    Part {
        start: 15.0,
        name: "Verse 1",
        caption: "VERSE 1  -  MilkDrop preset, ring on the drum stem, oscilloscope on the vocal stem",
        kind: PartKind::Verse,
    },
    Part {
        start: 40.2,
        name: "Pre-chorus 1",
        caption: "PRE-CHORUS  -  effects layer: shake and tint on the beat",
        kind: PartKind::PreChorus,
    },
    Part {
        start: 52.8,
        name: "Chorus 1",
        caption: "CHORUS  -  spectrum bars, sprite sheets stepping on beats, glitch on the drums",
        kind: PartKind::Chorus,
    },
    Part {
        start: 65.8,
        name: "Verse 2",
        caption: "VERSE 2  -  another preset, the emblem grows with the bass",
        kind: PartKind::Verse,
    },
    Part {
        start: 91.5,
        name: "Pre-chorus 2",
        caption: "PRE-CHORUS  -  blend modes: Add, Screen, Lighten",
        kind: PartKind::PreChorus,
    },
    Part {
        start: 104.1,
        name: "Chorus 2",
        caption: "CHORUS  -  color split and glow follow the drums",
        kind: PartKind::Chorus,
    },
    Part {
        start: 116.7,
        name: "Bridge",
        caption: "BRIDGE  -  waterfall on the 'other' stem, spinning image sequence",
        kind: PartKind::Bridge,
    },
    Part {
        start: 148.6,
        name: "Countdown",
        caption: "COUNTDOWN  -  video layer sliding in, core visualizer on the bass",
        kind: PartKind::Countdown,
    },
    Part {
        start: 178.0,
        name: "Pre-chorus 3",
        caption: "PRE-CHORUS  -  grain and vignette",
        kind: PartKind::PreChorus,
    },
    Part {
        start: 191.1,
        name: "Chorus 3",
        caption: "CHORUS  -  everything at once",
        kind: PartKind::Chorus,
    },
    Part {
        start: 203.3,
        name: "Rap",
        caption: "RAP  -  core visualizer, hard glitch and shake on every beat",
        kind: PartKind::Rap,
    },
    Part {
        start: 241.9,
        name: "Outro",
        caption: "OUTRO  -  made with VizX",
        kind: PartKind::Outro,
    },
];

/// MilkDrop collection folders, one per part, in turn.
const PRESET_CATEGORIES: [&str; 6] = [
    "Supernova",
    "Fractal",
    "Hypnotic",
    "Particles",
    "Reaction",
    "Geometric",
];

const TITLE_FONT: &str = r"C:\Windows\Fonts\ariblk.ttf";

fn main() {
    if let Err(problem) = run() {
        eprintln!("showcase: {problem}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 4 {
        return Err(
            "usage: showcase <song> <stems folder> <lyrics project> <output folder> \
             [--height N] [--samples]"
                .to_owned(),
        );
    }
    let song = PathBuf::from(&args[0]);
    let stems_folder = PathBuf::from(&args[1]);
    let lyrics_project = PathBuf::from(&args[2]);
    let output = PathBuf::from(&args[3]);
    let mut height = 1080;
    let mut samples = false;
    let mut rest = args[4..].iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--height" => {
                height = rest
                    .next()
                    .and_then(|value| value.parse().ok())
                    .ok_or("--height needs a number")?;
            }
            "--samples" => samples = true,
            other => return Err(format!("unknown option {other}")),
        }
    }

    let assets = output.join("assets");
    std::fs::create_dir_all(&assets).map_err(|error| error.to_string())?;
    let ffmpeg = find_ffmpeg().ok_or("FFmpeg was not found")?;
    println!("Making the pictures and the video...");
    let made = make_assets(&assets, &ffmpeg)?;

    println!("Building the project...");
    let session = build_project(&song, &stems_folder, &lyrics_project, &made, &ffmpeg)?;
    let mut session = session;
    let project_path = output.join("ANTIVILLAIN showcase.vzx");
    session
        .save_as(&project_path)
        .map_err(|error| error.to_string())?;
    println!("Saved {}", project_path.display());

    export(&session, &ffmpeg, &output, height, samples)
}

// ---------------------------------------------------------------------------
// The project
// ---------------------------------------------------------------------------

fn build_project(
    song: &Path,
    stems_folder: &Path,
    lyrics_project: &Path,
    made: &Assets,
    ffmpeg: &Path,
) -> Result<EditorSession, String> {
    let mut session = EditorSession::new_untitled();
    session.edit_project(|project| {
        project.settings.width = 1920;
        project.settings.height = 1080;
        true
    });

    // The song and its stems (muted: visuals listen to them, the mix is
    // heard).
    let song_track = session
        .import_audio(song)
        .map_err(|error| format!("{}: {error}", song.display()))?;
    let song_clip = only_clip(&session, song_track);
    let song_name = song
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stems: Vec<(String, PathBuf)> = STEM_NAMES
        .iter()
        .map(|name| {
            (
                (*name).to_owned(),
                stems_folder.join(format!("{song_name} - {name}.wav")),
            )
        })
        .collect();
    let stem_tracks = session.add_stems(song_clip, &stems)?;
    let [vocals, drums, bass, other] =
        [0, 1, 2, 3].map(|index| AudioInput::Track(stem_tracks[index]));

    let end = session.project().settings.duration_seconds();
    let parts: Vec<(f64, f64, &Part)> = PARTS
        .iter()
        .enumerate()
        .map(|(index, part)| {
            let next = PARTS.get(index + 1).map_or(end, |next| next.start);
            (part.start, next.min(end), part)
        })
        .filter(|(start, stop, _)| stop > start)
        .collect();
    let spans = |kinds: &[PartKind]| -> Vec<(f64, f64)> {
        parts
            .iter()
            .filter(|(_, _, part)| kinds.contains(&part.kind))
            .map(|(start, stop, _)| (*start, *stop))
            .collect()
    };

    session.edit_project(|project| {
        for (start, _, part) in &parts {
            project.add_marker(milliseconds(*start), part.name);
        }
        true
    });

    // 1. Background image, slowly zooming in over the whole song.
    let background = session.import_background_image(&made.background)?;
    let background_clip = only_clip(&session, background);
    edit(&mut session, |project| {
        let (_, clip) = project.clip(background_clip).expect("just added");
        let resting = clip.placement;
        let duration = clip.duration_milliseconds as i64;
        project.set_clip_keyframes(
            background_clip,
            vec![
                key(0, resting, 0.0, 1.0, Easing::Linear),
                key(duration, scaled(resting, 1.25), 4.0, 1.0, Easing::Linear),
            ],
        );
    });

    // 2. MilkDrop presets behind everything, a different one in every part
    //    after the intro.
    let library = find_bundled_presets()
        .and_then(|root| PresetLibrary::scan(&root).ok())
        .ok_or("the bundled MilkDrop presets were not found")?;
    let mut backdrop: Option<TrackId> = None;
    for (index, (start, stop, part)) in parts.iter().enumerate() {
        if part.kind == PartKind::Intro {
            continue;
        }
        let category = PRESET_CATEGORIES[index % PRESET_CATEGORIES.len()];
        let in_category: Vec<_> = library
            .entries()
            .iter()
            .filter(|entry| entry.category == category)
            .collect();
        if in_category.is_empty() {
            continue;
        }
        let entry = in_category[(index * 7) % in_category.len()];
        println!("  {}: MilkDrop {} / {}", part.name, category, entry.name);
        let added = session.add_bundled_milkdrop_preset(entry);
        let (clip, track) = segment(&mut session, added, backdrop, *start, *stop);
        backdrop = Some(track);
        edit(&mut session, |project| {
            project.set_clip_opacity(clip, 0.85);
            project.set_clip_effects(
                clip,
                ClipEffects {
                    vignette: 0.35,
                    ..ClipEffects::default()
                },
            );
        });
    }
    if let Some(track) = backdrop {
        edit(&mut session, |project| {
            project.rename_track(track, "MilkDrop");
        });
    }

    // 3. Image sequence: a turning hexagon, in the intro and the bridge.
    let mut sequence_track = None;
    for (start, stop) in spans(&[PartKind::Intro, PartKind::Bridge]) {
        let added = session.import_image_sequence(&made.sequence_first)?;
        let (clip, track) = segment(&mut session, added, sequence_track, start, stop);
        sequence_track = Some(track);
        edit(&mut session, |project| {
            project.set_clip_placement(clip, square(0.5, 0.5, 0.62));
            project.set_clip_spin(clip, 24.0);
            project.set_clip_blend_mode(clip, BlendMode::Add);
            project.set_clip_opacity(clip, 0.8);
            project.set_clip_audio_input(clip, drums);
            project.set_clip_reaction(
                clip,
                AudioReaction {
                    band: ReactionBand::Beat,
                    scale: 0.12,
                    release_seconds: 0.3,
                    ..AudioReaction::default()
                },
            );
        });
    }
    name_track(&mut session, sequence_track, "Image sequence");

    // 4. Video (made by FFmpeg: the Game of Life), sliding in during the
    //    countdown.
    for (start, stop) in spans(&[PartKind::Countdown]) {
        let added = session.import_video(&made.video, ffmpeg, false)?;
        let (clip, track) = segment(&mut session, added, None, start, stop);
        name_track(&mut session, Some(track), "Video");
        let placed = Placement {
            x: 0.6,
            y: 0.06,
            width: 0.36,
            height: 0.36,
        };
        let outside = Placement { x: 1.05, ..placed };
        let duration = milliseconds(stop - start) as i64;
        edit(&mut session, |project| {
            project.set_clip_placement(clip, placed);
            project.set_clip_sharp_pixels(clip, true);
            project.set_clip_effects(
                clip,
                ClipEffects {
                    glow: 0.5,
                    tint: 0.4,
                    tint_color: [1.0, 0.15, 0.45],
                    ..ClipEffects::default()
                },
            );
            project.set_clip_keyframes(
                clip,
                vec![
                    key(0, outside, -8.0, 0.0, Easing::Smooth),
                    key(1500, placed, 0.0, 1.0, Easing::Smooth),
                    key(duration - 1500, placed, 0.0, 1.0, Easing::Smooth),
                    key(duration, outside, 8.0, 0.0, Easing::Smooth),
                ],
            );
        });
    }

    // 5. Waterfall on the "other" stem, see-through, in the bridge.
    let waterfall = plugin(
        &mut session,
        "waterfall",
        spans(&[PartKind::Bridge]),
        |project, clip| {
            project.set_clip_audio_input(clip, other);
            project.set_clip_blend_mode(clip, BlendMode::Lighten);
            project.set_clip_opacity(clip, 0.7);
            project.set_clip_parameter(clip, "see_through", vec![1.0]);
            project.set_clip_parameter(clip, "color_low", vec![0.25, 0.0, 0.6]);
            project.set_clip_parameter(clip, "color_high", vec![1.0, 0.3, 0.75]);
        },
    );
    name_track(&mut session, waterfall, "Waterfall");

    // 6. The ring, from the first verse to the end, on the drums.
    let ring_spans = vec![(PARTS[1].start, end)];
    let ring = plugin(&mut session, "ring", ring_spans, |project, clip| {
        project.set_clip_placement(clip, square(0.5, 0.5, 0.95));
        project.set_clip_audio_input(clip, drums);
        project.set_clip_blend_mode(clip, BlendMode::Add);
        project.set_clip_parameter(clip, "color", vec![1.0, 0.25, 0.55]);
        project.set_clip_parameter(clip, "spin", vec![0.08]);
        project.set_clip_reaction(
            clip,
            AudioReaction {
                band: ReactionBand::Beat,
                scale: 0.06,
                release_seconds: 0.25,
                ..AudioReaction::default()
            },
        );
        project.set_clip_effects(
            clip,
            ClipEffects {
                glow: 0.45,
                ..ClipEffects::default()
            },
        );
    });
    name_track(&mut session, ring, "Ring");

    // 7. Core on the bass: countdown and rap.
    let core = plugin(
        &mut session,
        "core",
        spans(&[PartKind::Countdown, PartKind::Rap]),
        |project, clip| {
            project.set_clip_placement(clip, square(0.5, 0.5, 1.0));
            project.set_clip_audio_input(clip, bass);
            project.set_clip_blend_mode(clip, BlendMode::Screen);
            project.set_clip_parameter(clip, "color_core", vec![1.0, 0.85, 0.3]);
            project.set_clip_parameter(clip, "color_rays", vec![1.0, 0.2, 0.4]);
        },
    );
    name_track(&mut session, core, "Core");

    // 8. Oscilloscope on the vocals, in the verses.
    let scope = plugin(
        &mut session,
        "oscilloscope",
        spans(&[PartKind::Verse, PartKind::Rap]),
        |project, clip| {
            project.set_clip_placement(
                clip,
                Placement {
                    x: 0.0,
                    y: 0.36,
                    width: 1.0,
                    height: 0.28,
                },
            );
            project.set_clip_audio_input(clip, vocals);
            project.set_clip_audio_gain(clip, 1.6);
            project.set_clip_blend_mode(clip, BlendMode::Add);
            project.set_clip_parameter(clip, "color", vec![0.4, 0.95, 1.0]);
            project.set_clip_opacity(clip, 0.75);
        },
    );
    name_track(&mut session, scope, "Oscilloscope (vocals)");

    // 9. Spectrum bars along the bottom in the choruses and the outro.
    let bars = plugin(
        &mut session,
        "spectrum-bars",
        spans(&[PartKind::Chorus, PartKind::PreChorus, PartKind::Outro]),
        |project, clip| {
            project.set_clip_placement(
                clip,
                Placement {
                    x: 0.0,
                    y: 0.7,
                    width: 1.0,
                    height: 0.3,
                },
            );
            project.set_clip_blend_mode(clip, BlendMode::Screen);
            project.set_clip_parameter(clip, "mirror", vec![1.0]);
            project.set_clip_parameter(clip, "bars", vec![72.0]);
            project.set_clip_parameter(clip, "color_bottom", vec![0.5, 0.0, 0.9]);
            project.set_clip_parameter(clip, "color_top", vec![1.0, 0.35, 0.6]);
            project.set_clip_opacity(clip, 0.9);
        },
    );
    name_track(&mut session, bars, "Spectrum bars");

    // 10. The emblem in the ring: grows, turns and shakes with the bass.
    let emblem = session.import_image(&made.emblem)?;
    let emblem_clip = only_clip(&session, emblem);
    edit(&mut session, |project| {
        project.trim_clip_start(emblem_clip, milliseconds(PARTS[1].start));
        project.set_clip_placement(emblem_clip, square(0.5, 0.5, 0.36));
        project.set_clip_audio_input(emblem_clip, bass);
        project.set_clip_reaction(
            emblem_clip,
            AudioReaction {
                band: ReactionBand::Bass,
                scale: 0.22,
                rotate_degrees: 6.0,
                shake: 0.008,
                sensitivity: 1.4,
                release_seconds: 0.18,
                ..AudioReaction::default()
            },
        );
        project.set_clip_effects(
            emblem_clip,
            ClipEffects {
                glow: 0.6,
                ..ClipEffects::default()
            },
        );
    });
    name_track(&mut session, Some(emblem), "Emblem");

    // 11. Sprite sheets in the top corners, one frame per drum beat.
    for (side, x) in [("left", 0.1), ("right", 0.9)] {
        let mut track = None;
        for (start, stop) in spans(&[PartKind::Chorus]) {
            let added = session.import_sprite_sheet(&made.sprite_sheet)?;
            let (clip, placed_track) = segment(&mut session, added, track, start, stop);
            track = Some(placed_track);
            let media = clip_media(&session, clip);
            edit(&mut session, |project| {
                // The grid is known: 4 x 2 frames of 256 pixels.
                let mut animation = FrameAnimation::sprite_sheet((1024, 512), 4, 2, 12.0);
                animation.on_beat = true;
                project.set_animation(media, animation);
                project.set_clip_placement(clip, square(x, 0.16, 0.22));
                project.set_clip_audio_input(clip, drums);
                project.set_clip_blend_mode(clip, BlendMode::Add);
                project.set_clip_spin(clip, if side == "left" { 40.0 } else { -40.0 });
            });
        }
        name_track(&mut session, track, &format!("Sprite sheet ({side})"));
    }

    // 12. Effects layers: a different look in every part, shake and
    //     effects driven by the drums.
    let mut effects_track = None;
    for (start, stop, part) in &parts {
        let (effects, reaction) = part_effects(part.kind);
        if effects.is_none() && reaction.shake == 0.0 {
            continue;
        }
        let added = session.add_effects_layer();
        let (clip, track) = segment(&mut session, added, effects_track, *start, *stop);
        effects_track = Some(track);
        edit(&mut session, |project| {
            project.set_clip_effects(clip, effects);
            project.set_clip_audio_input(clip, drums);
            project.set_clip_reaction(clip, reaction);
        });
    }

    // 13. Title in the intro: pops in, then flies off.
    let title_track = session.add_track(TrackKind::Visual);
    name_track(&mut session, Some(title_track), "Title");
    let title = TextProperties {
        text: "ANTIVILLAIN".to_owned(),
        font_path: TITLE_FONT.to_owned(),
        size: 0.17,
        color: [1.0, 0.92, 0.96],
        alignment: TextAlignment::Center,
    };
    let subtitle = TextProperties {
        text: "a VizX showcase".to_owned(),
        size: 0.05,
        color: [1.0, 0.45, 0.7],
        ..TextProperties::default()
    };
    for (properties, middle_y, start, stop) in
        [(title, 0.45, 1.5, 14.0), (subtitle, 0.6, 3.0, 14.0)]
    {
        let placement = text_placement(&properties, (0.5, middle_y))?;
        let duration = milliseconds(stop - start) as i64;
        let clip = add_text_clip(
            &mut session,
            title_track,
            properties,
            placement,
            start,
            stop,
        )?;
        edit(&mut session, |project| {
            project.set_clip_keyframes(
                clip,
                vec![
                    key(0, scaled(placement, 0.4), -12.0, 0.0, Easing::Smooth),
                    key(900, placement, 0.0, 1.0, Easing::Smooth),
                    key(
                        duration - 1200,
                        scaled(placement, 1.05),
                        0.0,
                        1.0,
                        Easing::Linear,
                    ),
                    key(duration, scaled(placement, 2.2), 0.0, 0.0, Easing::Smooth),
                ],
            );
            project.set_clip_effects(
                clip,
                ClipEffects {
                    glow: 0.5,
                    color_split: 0.25,
                    ..ClipEffects::default()
                },
            );
        });
    }

    // 14. A caption in every part saying what is shown.
    let captions = session.add_track(TrackKind::Visual);
    name_track(&mut session, Some(captions), "Captions");
    let caption_style = LyricsStyle {
        text: TextProperties {
            size: 0.026,
            color: [0.85, 0.85, 0.9],
            alignment: TextAlignment::Left,
            ..TextProperties::default()
        },
        fade_seconds: 0.4,
        ..LyricsStyle::default()
    };
    for (start, stop, part) in &parts {
        let shown_until = (start + 7.0).min(*stop);
        let properties = TextProperties {
            text: part.caption.to_owned(),
            ..caption_style.text.clone()
        };
        let size = text_placement(&properties, (0.5, 0.5))?;
        let placement = Placement {
            x: 0.025,
            y: 0.03,
            ..size
        };
        let clip = add_text_clip(
            &mut session,
            captions,
            properties,
            placement,
            *start,
            shown_until,
        )?;
        edit(&mut session, |project| {
            let duration = milliseconds(shown_until - start);
            project.set_clip_keyframes(clip, lyric_keyframes(placement, duration, &caption_style));
        });
    }

    // 15. The lyrics on top, popping in, with a glow.
    let lines = lyric_lines(lyrics_project)?;
    let lyrics_style = LyricsStyle {
        text: TextProperties {
            size: 0.058,
            color: [1.0, 1.0, 1.0],
            font_path: r"C:\Windows\Fonts\arialbd.ttf".to_owned(),
            ..TextProperties::default()
        },
        height: 0.86,
        fade_seconds: 0.18,
        pop: true,
    };
    let lyrics = session.add_lyrics(&lines, &lyrics_style, None)?;
    let lyric_clips: Vec<ClipId> = session
        .project()
        .track(lyrics)
        .map(|track| track.clips.iter().map(|clip| clip.id).collect())
        .unwrap_or_default();
    edit(&mut session, |project| {
        for clip in lyric_clips {
            project.set_clip_effects(
                clip,
                ClipEffects {
                    glow: 0.35,
                    ..ClipEffects::default()
                },
            );
        }
    });

    // Layers that ended up empty (the starter layer) go.
    let empty: Vec<TrackId> = session
        .project()
        .tracks()
        .iter()
        .filter(|track| track.clips.is_empty())
        .map(|track| track.id)
        .collect();
    for track in empty {
        session.remove_track(track);
    }
    Ok(session)
}

/// The look of the effects layer in one kind of part.
fn part_effects(kind: PartKind) -> (ClipEffects, AudioReaction) {
    let beat = |shake: f64, scale: f64, drives_effects: bool| AudioReaction {
        band: ReactionBand::Beat,
        shake,
        scale,
        release_seconds: 0.22,
        drives_effects,
        ..AudioReaction::default()
    };
    let none = ClipEffects::default();
    match kind {
        PartKind::Intro => (
            ClipEffects {
                vignette: 0.5,
                grain: 0.2,
                ..none
            },
            AudioReaction::default(),
        ),
        PartKind::Verse => (
            ClipEffects {
                vignette: 0.45,
                grain: 0.15,
                ..none
            },
            beat(0.006, 0.012, false),
        ),
        PartKind::PreChorus => (
            ClipEffects {
                tint: 0.55,
                tint_color: [0.55, 0.2, 1.0],
                vignette: 0.4,
                ..none
            },
            beat(0.012, 0.02, true),
        ),
        PartKind::Chorus => (
            ClipEffects {
                color_split: 0.55,
                glitch: 0.3,
                glow: 0.35,
                vignette: 0.35,
                ..none
            },
            beat(0.025, 0.04, true),
        ),
        PartKind::Bridge => (
            ClipEffects {
                blur: 0.12,
                glow: 0.5,
                tint: 0.35,
                tint_color: [0.3, 0.6, 1.0],
                ..none
            },
            beat(0.004, 0.01, false),
        ),
        PartKind::Countdown => (
            ClipEffects {
                grain: 0.5,
                tint: 0.6,
                tint_color: [1.0, 0.15, 0.2],
                glitch: 0.2,
                vignette: 0.6,
                ..none
            },
            beat(0.01, 0.015, true),
        ),
        PartKind::Rap => (
            ClipEffects {
                color_split: 0.6,
                glitch: 0.55,
                grain: 0.25,
                vignette: 0.4,
                ..none
            },
            beat(0.04, 0.05, true),
        ),
        PartKind::Outro => (
            ClipEffects {
                vignette: 0.7,
                blur: 0.2,
                grain: 0.3,
                ..none
            },
            AudioReaction::default(),
        ),
    }
}

/// Adds a bundled visualizer plugin in each of `spans`, on one layer, and
/// sets up every clip with `set_up`.
fn plugin(
    session: &mut EditorSession,
    name: &str,
    spans: Vec<(f64, f64)>,
    set_up: impl Fn(&mut Project, ClipId),
) -> Option<TrackId> {
    let reference = format!("{}{name}", vizx_core::BUNDLED_VISUALIZER_PREFIX);
    let mut track = None;
    for (start, stop) in spans {
        let added = session.add_visualizer_plugin(&reference);
        let (clip, placed) = segment(session, added, track, start, stop);
        track = Some(placed);
        edit(session, |project| set_up(project, clip));
    }
    track
}

/// Trims the clip just added on `added` to `start`..`stop` (seconds) and
/// moves it onto `onto` if given. Returns the clip and its layer.
fn segment(
    session: &mut EditorSession,
    added: TrackId,
    onto: Option<TrackId>,
    start: f64,
    stop: f64,
) -> (ClipId, TrackId) {
    let clip = only_clip(session, added);
    edit(session, |project| {
        project.trim_clip_end(clip, milliseconds(stop));
        project.trim_clip_start(clip, milliseconds(start));
    });
    match onto {
        Some(target) if target != added => {
            edit(session, |project| {
                project.move_clip_to_track(clip, target);
            });
            session.remove_track(added);
            (clip, target)
        }
        _ => (clip, added),
    }
}

fn add_text_clip(
    session: &mut EditorSession,
    track: TrackId,
    properties: TextProperties,
    placement: Placement,
    start: f64,
    stop: f64,
) -> Result<ClipId, String> {
    let mut added = None;
    edit(session, |project| {
        added = project.add_text_clip(
            track,
            properties,
            placement,
            milliseconds(start),
            milliseconds(stop - start),
        );
    });
    added.ok_or_else(|| "could not add a text".to_owned())
}

/// Where a text sits with its middle at `middle`, exactly as large as it
/// is drawn in a 1920 x 1080 frame.
fn text_placement(properties: &TextProperties, middle: (f64, f64)) -> Result<Placement, String> {
    let image = render_text(properties, 1080)?;
    let width = f64::from(image.width) / 1920.0;
    let height = f64::from(image.height) / 1080.0;
    Ok(Placement {
        x: middle.0 - width / 2.0,
        y: middle.1 - height / 2.0,
        width,
        height,
    })
}

/// The timed lines of the topmost layer of text clips in `path`.
fn lyric_lines(path: &Path) -> Result<Vec<LyricLine>, String> {
    let project = Project::load_from_file(path).map_err(|error| error.to_string())?;
    let track = project
        .tracks()
        .iter()
        .find(|track| {
            track.clips.iter().any(|clip| {
                project
                    .media_source(clip.media)
                    .is_some_and(|source| source.text().is_some())
            })
        })
        .ok_or("the lyrics project has no text clips")?;
    let mut lines: Vec<LyricLine> = track
        .clips
        .iter()
        .filter_map(|clip| {
            let text = project.media_source(clip.media)?.text()?.text.clone();
            Some(LyricLine {
                text,
                start_seconds: clip.start_milliseconds as f64 / 1000.0,
                end_seconds: clip.end_milliseconds() as f64 / 1000.0,
            })
        })
        .collect();
    lines.sort_by(|a, b| a.start_seconds.total_cmp(&b.start_seconds));
    Ok(lines)
}

fn only_clip(session: &EditorSession, track: TrackId) -> ClipId {
    session
        .project()
        .track(track)
        .and_then(|track| track.clips.last())
        .map(|clip| clip.id)
        .expect("the layer has a clip")
}

fn clip_media(session: &EditorSession, clip: ClipId) -> MediaId {
    session
        .project()
        .clip(clip)
        .map(|(_, clip)| clip.media)
        .expect("the clip exists")
}

fn name_track(session: &mut EditorSession, track: Option<TrackId>, name: &str) {
    if let Some(track) = track {
        edit(session, |project| {
            project.rename_track(track, name);
        });
    }
}

fn edit(session: &mut EditorSession, change: impl FnOnce(&mut Project)) {
    session.edit_project(|project| {
        change(project);
        true
    });
}

fn milliseconds(seconds: f64) -> u64 {
    (seconds.max(0.0) * 1000.0).round() as u64
}

/// A square (on screen) of `size` times the frame's height around a middle.
fn square(middle_x: f64, middle_y: f64, size: f64) -> Placement {
    let width = size * 1080.0 / 1920.0;
    Placement {
        x: middle_x - width / 2.0,
        y: middle_y - size / 2.0,
        width,
        height: size,
    }
}

fn scaled(placement: Placement, factor: f64) -> Placement {
    let (width, height) = (placement.width * factor, placement.height * factor);
    Placement {
        x: placement.x + (placement.width - width) / 2.0,
        y: placement.y + (placement.height - height) / 2.0,
        width,
        height,
    }
}

fn key(
    time_milliseconds: i64,
    placement: Placement,
    rotation_degrees: f64,
    opacity: f64,
    easing: Easing,
) -> Keyframe {
    Keyframe {
        time_milliseconds,
        transform: Transform {
            placement,
            rotation_degrees,
            opacity,
        },
        easing,
    }
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

fn export(
    session: &EditorSession,
    ffmpeg: &Path,
    output: &Path,
    height: u32,
    samples: bool,
) -> Result<(), String> {
    let project = session.project();
    let mut audio_store = AudioStore::default();
    let paths: Vec<String> = project
        .media()
        .iter()
        .filter(|media| media.audio().is_some())
        .map(|media| media.path.clone())
        .collect();
    println!("Loading {} audio files...", paths.len());
    for path in &paths {
        audio_store.request(path, || {});
    }
    loop {
        audio_store.collect_finished();
        if paths.iter().all(|path| audio_store.loaded(path).is_some()) {
            break;
        }
        if let Some(path) = paths.iter().find(|path| {
            matches!(
                audio_store.state(path),
                Some(vizx_core::AudioState::Failed(_))
            )
        }) {
            return Err(format!("could not load {path}"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let mut catalog = VisualizerCatalog::new();
    let bundled_presets = find_bundled_presets();
    let end = project.settings.duration_seconds();
    let jobs: Vec<(PathBuf, ExportSettings)> = if samples {
        PARTS
            .iter()
            .filter(|part| part.start < end)
            .map(|part| {
                let at = (part.start + 8.0).min(end - 2.0);
                (
                    output.join(format!("sample {:05.1} {}.mp4", part.start, part.name)),
                    ExportSettings {
                        format: VideoFormat::Mp4H264,
                        quality: ExportQuality::Medium,
                        height: Some(360),
                        range: Some((at, at + 2.0)),
                    },
                )
            })
            .collect()
    } else {
        vec![(
            output.join("ANTIVILLAIN - VizX showcase.mp4"),
            ExportSettings {
                format: VideoFormat::Mp4H264,
                quality: ExportQuality::High,
                height: Some(height),
                range: None,
            },
        )]
    };

    for (file, settings) in jobs {
        let job = ExportJob::from_project(
            project,
            &audio_store,
            &mut catalog,
            bundled_presets.as_deref(),
            ffmpeg,
            &file,
            &settings,
        )?;
        let frames = job.frame_count();
        println!("Exporting {} ({frames} frames)...", file.display());
        let started = Instant::now();
        let running = Export::start(job, || {});
        let mut last_report = Instant::now();
        let outcome = loop {
            if let Some(outcome) = running.result() {
                break outcome;
            }
            if last_report.elapsed() > Duration::from_secs(30) {
                last_report = Instant::now();
                let done = running.finished_frame_count();
                let rate = f64::from(done) / started.elapsed().as_secs_f64().max(0.001);
                println!(
                    "  {done} / {frames} frames, {rate:.1} frames/s, about {:.0} min left",
                    f64::from(frames - done.min(frames)) / rate.max(0.001) / 60.0
                );
            }
            std::thread::sleep(Duration::from_millis(200));
        };
        outcome?;
        println!(
            "  done in {:.1} min",
            started.elapsed().as_secs_f64() / 60.0
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Generated pictures and video
// ---------------------------------------------------------------------------

struct Assets {
    background: PathBuf,
    emblem: PathBuf,
    sprite_sheet: PathBuf,
    sequence_first: PathBuf,
    video: PathBuf,
}

fn make_assets(folder: &Path, ffmpeg: &Path) -> Result<Assets, String> {
    let save = |image: &RgbaImage, path: &Path| {
        image
            .save(path)
            .map_err(|error| format!("{}: {error}", path.display()))
    };

    // Background: a dark purple glow with faint stars.
    let background = folder.join("background.png");
    let mut picture = RgbaImage::new(1920, 1080);
    for (x, y, pixel) in picture.enumerate_pixels_mut() {
        let (u, v) = (f64::from(x) / 1920.0 - 0.5, f64::from(y) / 1080.0 - 0.55);
        let glow = (-(u * u * 2.5 + v * v * 6.0) * 3.0).exp();
        let star = if hash(x, y).is_multiple_of(1400) { 0.8 } else { 0.0 };
        let red = 0.04 + 0.45 * glow + star;
        let green = 0.01 + 0.08 * glow + star;
        let blue = 0.08 + 0.55 * glow + star;
        *pixel = Rgba([to_byte(red), to_byte(green), to_byte(blue), 255]);
    }
    save(&picture, &background)?;

    // Emblem: a ring with a "V" inside, on transparency.
    let emblem = folder.join("emblem.png");
    let mut picture = RgbaImage::new(512, 512);
    for (x, y, pixel) in picture.enumerate_pixels_mut() {
        let (u, v) = (f64::from(x) / 256.0 - 1.0, f64::from(y) / 256.0 - 1.0);
        let radius = (u * u + v * v).sqrt();
        let ring = 1.0 - ((radius - 0.86).abs() / 0.05).min(1.0);
        // Two strokes meeting at the bottom.
        let stroke = |slope: f64| {
            let distance = (u - slope * (v - 0.45)).abs() / (1.0 + slope * slope).sqrt();
            if (-0.55..=0.45).contains(&v) {
                1.0 - (distance / 0.07).min(1.0)
            } else {
                0.0
            }
        };
        let letter = stroke(0.62).max(stroke(-0.62));
        let alpha = ring.max(letter).clamp(0.0, 1.0);
        let red = 1.0;
        let green = 0.3 + 0.6 * letter;
        let blue = 0.6 + 0.3 * letter;
        *pixel = Rgba([
            to_byte(red * alpha),
            to_byte(green * alpha),
            to_byte(blue * alpha),
            to_byte(alpha),
        ]);
    }
    save(&picture, &emblem)?;

    // Sprite sheet: a four-pointed star pulsing over 8 frames, 4 x 2.
    let sprite_sheet = folder.join("star sheet.png");
    let cell = 256;
    let mut picture = RgbaImage::new(cell * 4, cell * 2);
    for frame in 0..8 {
        let (column, row) = (frame % 4, frame / 4);
        let pulse = 0.55 + 0.45 * (f64::from(frame) / 8.0 * std::f64::consts::TAU).cos();
        for y in 0..cell {
            for x in 0..cell {
                let (u, v) = (
                    f64::from(x) / f64::from(cell / 2) - 1.0,
                    f64::from(y) / f64::from(cell / 2) - 1.0,
                );
                let star = (1.0 - (u.abs().sqrt() + v.abs().sqrt()) / (1.2 * pulse)).max(0.0);
                let core = (-(u * u + v * v) * 30.0).exp();
                let alpha = (star * 2.5 + core).min(1.0);
                picture.put_pixel(
                    column * cell + x,
                    row * cell + y,
                    Rgba([
                        to_byte(alpha),
                        to_byte(alpha * (0.6 + 0.4 * core)),
                        to_byte(alpha * 0.8),
                        to_byte(alpha),
                    ]),
                );
            }
        }
    }
    save(&picture, &sprite_sheet)?;

    // Image sequence: a hexagon outline turning over 24 frames.
    let sequence_folder = folder.join("hexagon");
    std::fs::create_dir_all(&sequence_folder).map_err(|error| error.to_string())?;
    let frames = 24;
    for frame in 0..frames {
        let mut picture = RgbaImage::new(384, 384);
        let turn = f64::from(frame) / f64::from(frames) * std::f64::consts::TAU / 6.0;
        for (x, y, pixel) in picture.enumerate_pixels_mut() {
            let (u, v) = (f64::from(x) / 192.0 - 1.0, f64::from(y) / 192.0 - 1.0);
            let angle = v.atan2(u) - turn;
            let sector = std::f64::consts::TAU / 6.0;
            let local = (angle.rem_euclid(sector)) - sector / 2.0;
            // Distance to the hexagon's edge along this direction.
            let edge = 0.8 * (sector / 2.0).cos() / local.cos();
            let radius = (u * u + v * v).sqrt();
            let line = 1.0 - ((radius - edge).abs() / 0.025).min(1.0);
            let inner = 1.0 - ((radius - edge * 0.62).abs() / 0.012).min(1.0);
            let dashes = if (angle * 18.0).sin() > 0.0 {
                inner
            } else {
                0.0
            };
            let alpha = line.max(dashes * 0.8);
            *pixel = Rgba([
                to_byte(alpha * 0.45),
                to_byte(alpha * 0.85),
                to_byte(alpha),
                to_byte(alpha),
            ]);
        }
        save(
            &picture,
            &sequence_folder.join(format!("hexagon_{:04}.png", frame + 1)),
        )?;
    }

    // Video: the Game of Life from FFmpeg's test sources, 20 seconds.
    let video = folder.join("life.mp4");
    if !video.is_file() {
        let status = Command::new(ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
            ])
            .arg(
                "life=s=192x192:mold=10:r=15:ratio=0.12:death_color=#12002a:life_color=#ff3d8b,\
                 scale=576:576:flags=neighbor,fps=30",
            )
            .args([
                "-t", "20", "-pix_fmt", "yuv420p", "-c:v", "libx264", "-crf", "18",
            ])
            .arg(&video)
            .status()
            .map_err(|error| format!("could not start FFmpeg: {error}"))?;
        if !status.success() {
            return Err("FFmpeg could not make the video".to_owned());
        }
    }

    Ok(Assets {
        background,
        emblem,
        sprite_sheet,
        sequence_first: sequence_folder.join("hexagon_0001.png"),
        video,
    })
}

fn hash(x: u32, y: u32) -> u32 {
    let mut value = x.wrapping_mul(374_761_393) ^ y.wrapping_mul(668_265_263);
    value = (value ^ (value >> 13)).wrapping_mul(1_274_126_177);
    value ^ (value >> 16)
}

fn to_byte(value: f64) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}
