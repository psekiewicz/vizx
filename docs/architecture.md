# VizX architecture

This document records how VizX is structured and why. Update it whenever a
decision here changes.

## Guiding principle

VizX is one editor, not several tools in one window. Every feature works on
the same project data and the same timeline, and every panel is a view of the
same state. A new feature must plug into that shared state instead of keeping
its own copy of anything.

## Crates

The workspace has nine crates. Dependencies point in one direction only
(the installer, `vizx-setup`, stands apart: it uses none of the others):

```text
vizx-app  ──►  vizx-core  ──►  vizx-project
 (GUI)         (logic)    │     (data)
                          ├──►  vizx-audio
                          │     (audio files, playback)
                          ├──►  vizx-milkdrop
                          │     (MilkDrop presets via projectM)
                          ├──►  vizx-visualizer
                          │     (visualizer plugins: shaders via wgpu)
                          ├──►  vizx-ml  ──►  vizx-audio
                          │     (HTDemucs stem separation via ONNX Runtime)
                          └──►  vizx-render
                                (off-screen wgpu device, layer compositor)
```

`vizx-project` and `vizx-audio` do not know about each other. `vizx-core`
connects them: it asks `vizx-audio` what is in a file and records the answer
in the project.

| Crate | Responsibility | Must not contain |
|---|---|---|
| `vizx-project` | What a project *is*: plain serializable data. | GUI, rendering, audio devices, file dialogs |
| `vizx-audio` | Audio files and the sound card: reading file properties, decoding, waveform peaks, mixing, playback and analysis (spectrum, waveform, levels). Later: beats. | GUI, project data |
| `vizx-milkdrop` | Rendering MilkDrop presets to pixels with the projectM library. The only crate with `unsafe` code, C declarations and OpenGL. | GUI, project data |
| `vizx-visualizer` | Visualizer plugins: reading a plugin folder and rendering its WGSL shader off-screen with wgpu. | GUI, project data, the timeline |
| `vizx-render` | The rendering engine: an off-screen wgpu device and the compositor that lays pictures over each other into one frame. | GUI, project data, the timeline |
| `vizx-ml` | Local machine learning: the HTDemucs stem separation model, run by ONNX Runtime loaded at run time. Nothing leaves the computer. | GUI, project data, network access |
| `vizx-core` | Editing state and logic operating on a project. Future home of subsystem coordination. The only network access: looking up lyrics on LRCLIB when the user asks. | GUI code |
| `vizx-app` | The window, panels, menus and visual style. | Project rules or logic that other front ends would need |
| `vizx-setup` | The one-file installer and the uninstaller (see "Installer" below). | Anything VizX itself needs at run time |

Because the lower two crates have no GUI dependency, they can be tested
without opening a window and reused by a future command-line exporter.

New crates are added only when a subsystem is actually being implemented
(for example an audio engine), never in advance.

### `vizx-project`

- `Project` is the root of everything saved in a `.vzx` file. It currently
  holds `format_version`, `ProjectMetadata` and `ProjectSettings`.
- `format_version` is written into every project so later versions can detect
  and migrate older files.
- `FrameRate` is stored as a fraction (`numerator / denominator`). Rates like
  29.97 fps are exactly 30000/1001; a fraction keeps frame timing exact,
  which deterministic export depends on.
- All types derive Serde's `Serialize` and `Deserialize`.
- `file.rs` reads and writes `.vzx` files and defines `ProjectFileError`
  (I/O failure, invalid content, or a format version newer than this build
  supports). See "The `.vzx` file format" below.

### `vizx-core`

- `EditorSession` is the state of the one open project: the `Project`, its
  file path (if saved) and whether it has unsaved changes.
- The project can only be modified through `EditorSession::edit_project`.
  Funnelling every change through one method means the unsaved-changes flag
  cannot be forgotten, and gives undo/redo a single place to hook into later.
- `EditorSession::open` and `EditorSession::save_as` load and store the
  project. A failed save leaves the session untouched: same path, changes
  still marked as unsaved.

### `vizx-app`

```text
src/
├── main.rs        Entry point: window options, starts eframe
├── app.rs         VizxApp: top-level state, per-frame layout of the window
├── graphics.rs    wgpu backend selection (only platform-specific code)
├── theme.rs       Colors, spacing, dock style
└── ui/
    ├── menu_bar.rs, toolbar.rs, status_bar.rs, about_window.rs
    ├── workspace.rs       Dockable panel layout (egui_dock)
    └── panels/            One module per panel
```

- egui is an *immediate mode* GUI. There are no long-lived widget objects;
  each frame `VizxApp::ui` describes the whole interface from current state.
  This removes the usual problem of keeping widgets in sync with data.
- The docking system stores only a `PanelKind` value per tab. Panel content
  is drawn from the shared `EditorSession`, so panels own no project data.
- Panels can be dragged, re-docked, split, floated into windows inside the
  main window, resized and closed. The View menu reopens closed panels and
  can reset the layout.
- Features that belong to later milestones appear as disabled controls.
  Nothing pretends to work.
- Menus, toolbar buttons and keyboard shortcuts do not act directly. They
  call `VizxApp::request` with an `AppCommand` (New, Open, Save, Save As,
  Exit), and the command runs once at the end of the frame. Every way of
  triggering a command therefore behaves identically.
- Commands that would discard the open project (New, Open, Exit, and the
  window's close button) are held back while there are unsaved changes, and
  a modal dialog asks whether to save, discard or cancel.

## Decisions

### The `.vzx` file format

A `.vzx` file is the `Project` struct as pretty-printed UTF-8 JSON.

- JSON is readable in any text editor, identical on every operating system,
  and gives meaningful diffs in Git. Project files hold structure and
  references, not media, so file size and parsing speed are not a concern.
- Every file starts with `format_version`. When loading, the version is read
  first; a file from a newer VizX is rejected with a clear "unsupported
  version" error instead of a confusing parse failure.
- Media files (audio, images, video) will be referenced by path, never
  embedded.

Native open/save dialogs come from the `rfd` crate, used only in `vizx-app`.

Format versions: **1** had metadata and settings only. **2** added timeline
tracks and the project duration. Version 1 files still load: the missing
fields get default values (`#[serde(default)]`) and the project is saved in
the current layout from then on.

Version **3** reduced the track kinds to two; the former `Visualizer`,
`Animation` and `Text` kinds load as `Visual`. Version **4** added media
sources and clips. Version **6** added MilkDrop presets as a kind of media. Version **7** added
the audio a clip reacts to (`audio_input`); older files load with the mix.
Version **8** added a clip's `placement`; older files load filling the frame.
Version **9** added visualizer plugins as a kind of media and a clip's
parameter values for them. Version **10** added images as a kind of media
and a clip's `opacity` and `reaction`. Version **11** extended reactions
(move, rotate, sensitivity, release). Version **12** added video files as a
kind of media, version **13** text elements. Version **14** added a clip's
`source_offset_milliseconds`, version **15** its `audio_gain`,
version **16** its `rotation_degrees`, `spin_degrees_per_second` and `keyframes`.

### Collect All and Save

Because media is referenced, not embedded, a project spread over several
folders and drives is hard to move or back up. **File > Collect All and
Save...** asks for a parent folder and makes a new folder there named after
the project (`Name (2)` if it exists): `Name.vzx` plus a `Media` folder with
a copy of every file the project uses.

- Single files (songs, stems, images, videos, sprite sheets, MilkDrop
  presets, fonts) go straight into `Media`; equal names from different
  folders become `song (2).mp3`. A file used several times is copied once.
- An image sequence gets a folder of its own with its frame names unchanged,
  so the numbering still works; a visualizer plugin keeps its whole folder.
- Bundled presets and plugins ship with VizX and are not copied. Missing
  files are reported and keep their old path.
- `vizx-project` only plans the copy (`Project::plan_collection`, no disk
  writes, easy to test). `vizx-core::CollectJob` copies in 4 MB pieces on a
  background thread with progress, so large videos neither freeze the window
  nor stop it from being cancelled; a stopped or failed collection removes
  the folder it made.
- Afterwards the open project *is* the collected one (like Save As): its
  paths point at the copies and it is saved there. Edits made while copying
  are kept and count as unsaved. Opening another project stops the copy.

### Audio drives the visuals; VizX is not an audio workstation

Audio is the input that visuals react to, not material to be mixed. This
shapes the timeline:

- There are only two kinds of track: **audio tracks** and **visual layers**.
- A new project has one audio track (the song) and one visual layer. A
  single, unsplit audio file is the normal case.
- Additional audio tracks exist only for separate stems (vocals, drums, ...)
  that the user imports or, later, extracts. Their purpose is to be a
  reaction source for visual elements, not a mixer channel. There will be no
  faders, panning or audio effects; at most volume and mute.
- Visual layers stack as in a video editor: a layer higher in the list is
  drawn on top. New layers are added at the top, audio tracks at the bottom.
- What a visual element *is* (visualizer, image, animation, text) will be a
  property of the element placed on a layer, not of the layer. Each element
  will be able to choose which audio it reacts to.

### Media sources and clips

- A `MediaSource` is a file the project uses. The project stores a reference
  to the file plus the facts needed to lay it out (for audio: duration,
  sample rate, channel count). File content is never copied into the
  project.
- A `Clip` places a media source on a track at a point in project time.
- Importing audio (`Project::import_audio`) puts the file on the first empty
  audio track and creates a new audio track only if all are in use, so an
  import never overlaps or replaces existing audio.
- The project length follows the audio: the first imported file sets it, a
  later, longer file extends it, and importing never shortens it.
- Media paths are currently stored as imported (absolute). Storing them
  relative to the project file and re-linking moved files is planned; until
  then a project is not portable together with its media.

Audio files are read by `symphonia`, a pure-Rust library with no system
dependencies. Only the formats VizX supports are compiled in: WAV, MP3,
FLAC, Ogg Vorbis and AAC/M4A.

### MilkDrop visualizer

VizX shows original MilkDrop presets by running them in projectM, a C++
library that draws with OpenGL. This is the one deliberate exception to "Rust
and wgpu only": an imitation written from scratch would not look like
MilkDrop. The exception is contained in the `vizx-milkdrop` crate, projectM
is optional and loaded at run time, and everything else in VizX only sees
"preset and audio in, RGBA pixels out". See [milkdrop.md](milkdrop.md) for
the design, setup, licensing (LGPL) and limitations.

In the project, a preset is a kind of media (`MediaContent::MilkdropPreset`)
placed as a clip on a visual layer. The clip records which audio it reacts
to (`AudioInput`: the mix or one audio track), which is what separate stems
are for. Each such clip is pre-rendered in the
background to a temporary frame file, and the preview shows the stored frame
for the playhead.

A clip also records its **placement**: the rectangle of the composition it
is drawn into, as fractions of the composition's width and height, so it
keeps its meaning at any output size. The preview draws every visualizer at
the playhead into its placement, lower layers first. The selected one can be
moved and resized directly in the preview (drag inside, or drag one of the
eight handles; edges snap to the frame edges and the middle) or with numbers
in the Properties panel. A drag is kept in the UI until the mouse button is
released and only then written to the project, so the project changes once
per drag.

VizX ships with a preset collection (`assets/presets`, a git submodule). A
project refers to one of those presets by its place in the collection
(`bundled-presets:/pack/category/name.milk`), not by a path on one computer,
so the project opens on any machine with VizX. `vizx-core::PresetLibrary`
indexes the collection for the preset browser window.

### Images, opacity and reacting to sound

This is the first part of the animation and behavior engine.

- **Images** (PNG, JPEG) are a kind of media (`MediaContent::Image`) placed
  as clips on visual layers, like visualizers. `vizx_core::ImageStore`
  decodes them in the background (scaled down to at most 1920 pixels for the
  preview). They are not pre-rendered; the preview draws the decoded picture
  into the clip's placement.
- **Transparency.** Every picture in VizX (image, plugin frame, MilkDrop
  frame) is RGBA with premultiplied alpha, and the preview lays the layers
  over each other from the bottom up. Plugins return an opacity per pixel;
  MilkDrop frames are as opaque as they are bright.
- **Clip opacity** (`Clip::opacity`) fades a whole clip.
- **Background and foreground.** What is in front is decided by the order
  of the visual layers: a layer higher in the list is drawn over the ones
  below. "Add > Background Image" puts an image on a new layer behind all
  others, enlarged until it covers the frame. A layer's context menu moves
  it up, down, to the front or to the back (`Project::move_layer`).
- **Animation to sound** (`Clip::reaction`, an `AudioReaction`) makes a
  clip grow, move, turn and/or appear the louder a chosen part of the sound
  is (volume, bass, mid or treble of the clip's audio input). A sensitivity
  scales the measured level, and a release time lets the animation fall
  back gradually after a loud moment instead of following every flicker.
  The level is worked out from the audio leading up to the playhead and
  nothing else (for the release, from a few earlier moments as well), so a
  moment of the song always looks the same, whether played or jumped to.
  The animation is applied when the picture is drawn; a clip's frame and
  handles stay at its placement.

### One compositor for preview and export

Every VizX picture is put together in one place, `vizx_render::Compositor`.
It is given layers (premultiplied RGBA pixels plus where to draw them: the
middle, size, rotation and opacity) and draws them over a black frame on
the graphics card, the first layer at the bottom, then reads the frame back.

- The **preview** (`vizx-app/src/visual_preview.rs`) composes at the size
  the panel shows and hands the result to egui as one texture. The panel
  only lays the editing frame and handles over it.
- The **export** (`vizx_core::Export`) composes at the project's size.

What goes into a frame is decided by shared code in
`vizx-core/src/composition.rs`: which audio a clip listens to
(`audio_sources`), what draws it (`frame_source`), how strongly it reacts
right now (`ReactionMeter`) and where exactly it is drawn (`layer_geometry`).
The frames of visualizers and videos come from one `FrameRenderer`, used by
the preview's pre-render at a small size and by the export at full size.
Preview and export therefore cannot drift apart.

### Video files and export (FFmpeg)

VizX contains no video codec. It starts the separate `ffmpeg` program and
exchanges raw pictures with it through pipes (`vizx-core/src/ffmpeg.rs`).
FFmpeg is optional: without it, video files cannot be added and nothing can
be exported; everything else works. It is looked for next to the VizX
executable, in `%VIZX_FFMPEG_DIR%/bin`, and on the system `PATH`. Running it
as a separate program also keeps its licence (LGPL/GPL, depending on the
build) apart from VizX.

- **Video as a layer or background.** A video is a kind of media
  (`MediaContent::Video`) used for its picture only; its sound is ignored.
  Animated GIFs are handled the same way and keep their transparency:
  FFmpeg hands over the opacity of every pixel, and VizX premultiplies the
  colors by it like every other picture.
  It plays from its start when its clip starts and begins again when it
  ends. For the preview it is pre-rendered like a visualizer: FFmpeg decodes
  it at preview size into the frame store.
- **Export** (File > Export Video, Ctrl+E) writes an MP4 with H.264 picture
  and AAC sound. It runs on its own thread from a snapshot of the project
  (`ExportJob`), so editing can continue. For each frame, time is
  `frame number / frame rate`, never the clock, so the result does not
  depend on the computer's speed. The sound is the same mix playback plays,
  written to a temporary WAV file for FFmpeg. A cancelled or failed export
  removes its unfinished file.

### Undo and redo

`vizx_core::History` keeps earlier states of the project. It does not record
actions; it compares. Once per frame the session shows it the project
(`EditorSession::settle_history`), and when the project has changed and then
stayed the same for 0.4 s with no mouse button held, the state before the
change becomes an undo step. A whole drag or a burst of typing is therefore
one step.

- Nothing that edits the project has to know about undo, so nothing can
  forget to support it.
- A project is small (it refers to media files, it does not contain them),
  so a step is simply a copy of the whole project. The newest 100 steps are
  kept.
- Undo (Ctrl+Z) and Redo (Ctrl+Shift+Z or Ctrl+Y) are not handled while a
  text field has the keyboard: the field has its own undo for typing.
- The selection and the playhead are not part of the project and are not
  undone.

### Sensitivity

Every visual clip has an `audio_gain` ("Sensitivity" next to "Reacts to"):
the sound the clip listens to is multiplied by it before the clip's
visualizer (MilkDrop or plugin) or its animation measures it. Playback is
not affected. Lowering it is how a visualizer that is too wild is calmed
down; it ranges from 1 % to 400 % and is part of the pre-render key, so a
change renders the clip again.
### Keyframes, rotation and spin

How a visual clip is shown at a moment is a `Transform`: placement,
rotation and opacity (`vizx-project/src/motion.rs`).

- **Resting state.** Without keyframes the clip's own placement, rotation
  and opacity hold for the whole clip.
- **Keyframes** (`Clip::keyframes`) are transforms at moments of the clip.
  Between two of them every value glides from one to the other; the easing
  of the later one decides how (Smooth, Linear or Hold). Before the first
  and after the last, that keyframe holds.
- **Editing at the playhead.** The Properties panel shows the values at the
  playhead, and every change (also dragging in the preview) goes through
  `Project::edit_clip_transform_at`: without keyframes it changes the
  resting state, with keyframes it changes the keyframe at the playhead or
  makes a new one there ("auto-key"). In the preview the selected element can
  also be turned with the round knob above it (Shift: steps of 15°); its
  frame and handles turn with it, and a turned element is resized along its
  own edges. Add/Remove Keyframe, Previous/Next
  and Clear are next to the values; keyframes show as diamonds on the clip
  in the timeline, and clicking one moves the playhead there.
- **Trimming never changes the motion.** Keyframe times are relative to the
  clip's start and may lie outside it after trimming (`i64`); such
  keyframes are not shown but still shape the motion. Splitting gives both
  parts all keyframes.
- **Spin** (`spin_degrees_per_second`) turns the clip steadily on top of
  everything ("continuous animation"). The animation to sound comes on top
  of that again.
- **Rendering.** A visualizer or video is rendered at the largest size its
  keyframes reach (`Motion::largest_size`), so it stays sharp; the
  compositor scales and turns it per frame. Preview and export take the
  transform from the same `Motion::animated_at`.
### Beats

`vizx_audio::detect_beats` finds the beats of an audio file once, when it
is loaded (`LoadedAudio::beats`, about half a second for a four-minute song
on the development laptop).

1. **Change envelope.** For every 10 ms, how much new energy appears
   (logarithmic spectral flux), separately for the bass (below 200 Hz) and
   the frequencies above it. From each, the steady background is taken
   away, each is scaled to its own strongest hit, and the two are mixed
   65 : 35 in favour of the bass, so the kick drum leads.
2. **Tempo** from the autocorrelation of that envelope, 60 to 200 BPM, with
   a mild preference for tempos near 120 (to choose between a tempo and its
   double).
3. **Beat tracking** (dynamic programming after D. Ellis, 2007): the chain
   of strong moments that follow each other at the tempo. Hi-hats and other
   hits between the beats are left out, and beats in quiet passages are
   dropped. Without a tempo (a short file), strong moments are used as
   they are.

`cargo run -p vizx-audio --example beats -- <file>` prints what is found.

**Choosing the tempo.** Music is often felt at half (or double) the tempo
that repeats most strongly in the sound: a song with a snare between the
kicks repeats every eighth note, but is felt in quarter notes. That choice
cannot be made reliably from the sound alone, so the Properties panel of an
audio clip shows the tempo with "½" and "×2" buttons and lets it be typed
(`MediaSource::tempo_bpm`, saved with the project). The beats are then
tracked again at that tempo (`BeatAnalysis::beats_at_tempo`, a few
milliseconds, from the kept change envelope); "Auto" goes back.
**Beats drawn by hand.** An audio file can use beats placed by hand
instead (`MediaSource::manual_beats`, seconds in the file, saved with the
project). In the Properties panel of an audio clip, "Edit Beats" starts
from a copy of the found ones and "Start Empty" from none; "Use Detected"
forgets them. With the clip selected, a strip along its bottom on the
timeline adds a beat where it is clicked, removes one that is clicked on
and moves one that is dragged (it stays between its neighbours,
`move_manual_beat`), and B places a beat at the moment heard, which is the
playhead minus the sound card's delay (`Transport::heard_seconds`; tapping along while the
song plays). Hand-placed beats have full strength, their tempo is worked
out from the typical gap between them (`vizx_core::tempo_of`), and they
replace the found ones everywhere: animations, visualizer plugins and the
marks on the timeline (drawn in another color). Like every edit they can be
undone.

Beats are used in three places, always in project time
(`vizx_core::beat_times`, which follows trimmed clips and merges the beats
of several stems): "Beat" as what an animation follows, `vizx.beat`,
`vizx.since_beat`, `vizx.beat_count` and `vizx.tempo` in visualizer plugins,
and small marks along the bottom of audio clips on the timeline.
### Editing clips on the timeline

A clip is a piece of media at a place in time: `start`, `duration` and, for
audio and video, `source_offset` (where in the file the clip begins).

- **On the timeline:** click a clip to select it, drag it to move it, drag
  its left or right edge to trim it. A dragged edge snaps to the start of
  the project, the playhead and the edges of other clips. The context menu
  and the Edit menu offer Split at Playhead (Ctrl+K), Duplicate (Ctrl+D)
  and Delete (Del). The Properties panel shows Start and Length as numbers.
- **In the model** (`Project::move_clip`, `trim_clip_start`, `trim_clip_end`,
  `split_clip`, `duplicate_clip`, `remove_clip`): trimming the left edge of
  audio or video raises `source_offset` by the same amount, so what stays
  of the clip does not move in time. Audio cannot be trimmed beyond its
  file; a visualizer, image or text has no file time and can be made any
  length. The project grows when a clip reaches beyond its end and never
  shrinks by itself. Media that no clip uses any more leaves the library.
- **Texts are copied, files are shared.** Splitting or duplicating a text
  clip gives the new clip a text of its own, so a line of lyrics can be
  duplicated and then changed. Clips of a file share the file.
- **Selection** can be a track or a clip (`Selection::Clip`); a selected
  clip's track counts as selected wherever only tracks matter.
- **Everything downstream follows the clip:** playback and export mix only
  the trimmed part of an audio clip (`PlaybackSource::offset_seconds` and
  `duration_seconds`), the waveform shows that part, and a trimmed video
  starts inside its file. A visualizer always starts fresh at the start of
  its clip, so splitting one restarts it at the cut.
- Dragging a clip up or down moves it to another track of the same kind
  (`Project::move_clip_to_track`). While a clip is dragged, its start and
  end times are shown on it.
- Clips on one track may overlap; nothing prevents it yet.
### Frame animations: image sequences and sprite sheets

A frame animation (`MediaContent::Animation`, `vizx-project/src/animation.rs`)
is a picture that changes by itself, made of separate frames:

- an **image sequence**: numbered files such as `walk_0001.png`,
  `walk_0002.png`... (Add > Image Sequence: pick any of them; VizX finds the
  run of numbers without gaps, starting at the lowest), or
- a **sprite sheet**: one image with the frames in a grid, read left to
  right, then top to bottom (Add > Sprite Sheet). A long strip of square
  frames is recognized; otherwise columns, rows and the number of frames
  are set in the Properties panel. Rows need not be full: "Frames per row"
  (`row_lengths`, for example 5, 4, 6) says how many frames each row holds,
  typed or found with "Find", which ends each row after its last cell that
  is not empty (fully transparent, or the flat color of the sheet's corner).

It loops at a set speed (frames per second), or with "On beats" steps one
frame on every beat of the clip's audio. Like an image it can be placed,
turned, keyed and animated to the sound, and keeps its transparency.

`vizx_core::AnimationReader` decodes frames when they are first needed
(premultiplied, scaled down to an edge limit) and keeps them within a
memory budget, so a short loop is decoded once. The preview reads at most
1024 pixels per edge; the export at full size. Both pick the frame with
`FrameAnimation::frame_at`, so they show the same frame at the same moment.

**Sharp pixels.** Pixel art enlarged with smoothing turns blurry. A clip
can keep sharp pixels instead (`Clip::sharp_pixels`, a checkbox in the
Properties panel); when it has not chosen, PNG images and animations are
sharp and everything else smooth. The compositor then samples the layer
with nearest-neighbour filtering when enlarging (shrinking stays smooth).

### Text

A text element (Add > Text, Ctrl+T) is a kind of media that is not a file
(`MediaContent::Text` with `TextProperties`: the text, font, size, color and
alignment). `vizx_core::render_text` draws it once into a transparent
picture with the `ab_glyph` library; from then on it is an ordinary layer
that can be placed, faded and animated to the sound like an image. The
preview and the export call the same function, so a title looks the same in
both.

- The size is a fraction of the frame's height, so the text keeps its
  proportion at any output size.
- The default font is the one built into egui (Ubuntu Light); any TrueType
  or OpenType file on the computer can be chosen instead.
- When the text changes, its placement is resized around the same middle to
  fit the new text exactly (`EditorSession::set_text`).
- A new text lasts for the whole project; trim, split and duplicate its
  clip on the timeline to make texts that appear at set times.

### Lyrics and subtitles ("text video")

The Lyrics window (Add > Lyrics / Subtitles, Ctrl+L) makes many texts at
once: the words are pasted in, one subtitle per line (`|` splits a line in
two rows), and each line becomes a text clip on one layer (a new "Lyrics"
layer, or a chosen one). There is no special lyrics media: every line is an
ordinary text clip that can be moved, trimmed and edited afterwards.

The lines are timed in one of four ways (`vizx-core/src/lyrics.rs`):

- **Tap along**: playback starts and Enter starts each line, ending the one
  before; Backspace ends a line early (a pause), Esc stops
  (`EditorSession::start_lyric_line` / `end_lyric_line`).
- **Spread** over a stretch of time, longer lines getting more of it.
- **On beats**: one line every N beats of the mix, from a chosen moment.
- **Import** an `.lrc` (with repeated times, pauses, enhanced word times)
  or `.srt` file that already carries the times.

All lines share a `LyricsStyle`: font, size, color, alignment, the height
of the line's middle (subtitle-like near the bottom by default), and a
fade in/out, optionally popping in, made of four keyframes per clip.
"Restyle Layer" gives every text on a layer the current look again.

### Subtitle layers

Lyrics and spoken lines have their own kind of layer, `TrackKind::Subtitles`
(format version 23). Every line is a clip with a text; how the lines look
and where they go belongs to the layer (`SubtitleStyle`: font, size, color,
outline, alignment, margin, fit, fade, pop, and a fixed or random position),
so one change restyles all of them. Several subtitle layers can show lines
at the same moment, one per singer or speaker.

`vizx_core::subtitle_layout` works out every line for a frame height, the
same for the preview and the export:

- **Fit:** a line wider than the safe area (inside the margin) is wrapped
  between words (`wrap_text`, which also evens out a short last row), and
  shrunk only if a single word is still too wide; "Shrink" keeps one row,
  "Let it overflow" does neither.
- **Place:** around the layer's fixed point, or at one of 16 spots of its
  random area, drawn from the layer's seed and the clip (SplitMix64), so
  they repeat; "New Random Places" changes the seed.
- **Keep apart:** lines whose times overlap, on any subtitle layer, must not
  cover each other: a fixed line steps by its own height towards the middle
  of the frame (then away), a random one takes the spot that covers least.
- **Appear:** fade and pop keyframes as for lyrics (`lyric_keyframes`).

Text pictures got an outline (`TextProperties::outline`, a round brush grown
from the letters' coverage, drawn behind them). The preview keeps the layout
until anything it depends on changes (`subtitle_key`). The Lyrics window now
adds its lines to a subtitle layer; an old layer of lyric texts becomes one
with "Convert to Subtitle Layer" (right-click its name). The Subtitle Editor
lists every line: start, end, layer and words are edited in the table; lines
are added at the playhead, split (the words divided by time), merged,
deleted, shifted together, and saved as `.srt`. Collect All and Save also
copies the font of a subtitle layer.

### Scenes and transitions

Every timeline marker starts a scene (until the next marker); a marker's
`transition` says how the picture changes there: Cut, Flash, Fade through
black, Zoom punch, Glitch, Blur, Spin or Shake, with a duration centered on
the change (format version 24). They are set in the Scenes window
(View > Scenes) or in a marker's right-click menu.

Transitions need no renderer of their own. `Project::rebuild_scene_transitions`
keeps a layer, "Scene transitions" (`Track::scene_transitions`), placed above
the picture and below the subtitles, filled with short effects-layer clips
around every change: each of 4 to 20 steps has the effect strength of its
moment (rising to 1 at the change and back), and a zoom, turn or shake
keyframed from its start to its end, so motion is smooth. Preview, export
and timeline treat them like any effects layer. The layer is laid out again
whenever a marker moves, goes or gets another transition, and removed when
only cuts are left. Flash and fade need a new effect, `fill` (the picture
turns into a solid color), also available on every clip in Properties.

Lines of different subtitle layers may overlap in time; the layout places
them one above the other. "Duet" in the Subtitle Editor puts a line on
every other singer's layer too, for lines sung together.

### Telling singers apart

**Separate Singers** in the Subtitle Editor gives every singer of a subtitle
layer a layer of their own (`Project::split_subtitle_layer`: "Singer 1"
keeps the lines and the look, "Singer 2", ... get another color and sit
right above it). `vizx_core::SingerSeparation` runs in the background:

1. The song's vocals come from its stems (separated first if needed, as
   for lyrics recognition).
2. Every line's stretch of the vocals (at least 1.5 s around its middle,
   at most 8 s) becomes a voice print: 256 numbers from the WeSpeaker
   ResNet34 model (`voxceleb_resnet34_LM.onnx`, 26 MB, CC BY 4.0, from
   huggingface.co/Wespeaker, run by ONNX Runtime). Its input are Kaldi-style
   log mel filter banks (80 bands, 25 ms Hamming frames every 10 ms, 16 kHz,
   pre-emphasis, mean-normalized), computed in `vizx_ml::speakers`. Lines
   too quiet for a print take the singer of their neighbor.
3. Prints are grouped (agglomerative, cosine similarity of the groups'
   average prints, joined while at least 0.45 alike). Groups of only a line
   or two are noise, not singers; with a number of singers chosen, the most
   alike large groups are joined until that many are left. Every line then
   goes to the singer whose average print is nearest, and singers are
   numbered in the order they are first heard.

Only one voice per line: two people singing at once cannot be split into
two tracks (no local model does that well); such a line goes to the louder
voice. The model learned from speech, so it separates voices that differ
(a man and a woman) far better than two alike ones. On ANTIVILLAIN (71
lines) it found two singers, the second taking over at 2:58 ("Wipe out what
*I've* been told") and the two alternating in the rap, the same with "decide"
and with "2 singers"; it took 6.5 s once the stems existed.
`cargo run --release -p vizx-ml --example singers -- <vocals.wav> <lines.tsv> [singers]`
tries it without the window. The model path is `models/` next to the
executable or `%VIZX_SPEAKER_MODEL%`; `Install models` downloads it.

### Stems: one visualizer for the vocals, one for the drums

An audio clip's context menu offers **Separate Stems**. The song is split
into vocals, drums, bass and other (`vizx_audio::separate_stems`), in a
background thread (`vizx_core::StemJob`, progress in the status bar). The
stems are written as WAV files into `<song> stems/` next to the song and
reused when they are there already. Each stem goes on an audio track of its
own, timed like the song's clip and **muted** (`Track::muted`): a muted
track is neither played nor part of the mix (`audio_sources` and
`beat_times` leave it out), but a visualizer or reaction can choose it as
its input ("Reacts to: Vocals").

Two engines can do the separation (`vizx_core::StemEngine`):

- **HTDemucs** (`vizx-ml`), Meta's Hybrid Transformer Demucs, as an ONNX
  model (the 4-stem single-file export with the STFT inside the graph,
  fp16-stored weights, 166 MB; MIT licence like Demucs). It takes 7.8 s
  pieces of stereo 44.1 kHz audio; the song is converted to that rate, cut
  into pieces overlapping by a quarter and cross-faded back together. On a
  CPU it takes roughly 2.3 times the song's length (about 10 minutes for 4
  minutes of music) and about 2.6 GB of memory. It is used when it is
  installed.
- **Quick** (`vizx_audio::separate_stems`), built in, no model: on the
  spectrogram, harmonic-percussive separation by median filtering (drums),
  the centered harmonic part between ~150 Hz and ~7 kHz (vocals), the
  harmonic part below ~250 Hz (bass) and the rest; the four masks add up
  to 1. Seconds instead of minutes, rough, but enough to drive visuals.

Their stems go to different folders (`<song> stems`, `<song> quick stems`).
The folder is next to the song, or under `VIZX_STEMS_DIR` when it is set
(stems are about 40 MB each for four minutes of music).

**Installing the model.** Nothing is downloaded by the build or the app.
ONNX Runtime is loaded at run time (`ort` with `load-dynamic`) from
`onnxruntime.dll` next to the VizX executable or `%VIZX_ONNXRUNTIME_DIR%/lib`
(the unpacked release archive from github.com/microsoft/onnxruntime); the
model from `models/htdemucs.onnx` (or `htdemucs_fp16weights.onnx`) next to
the executable or `%VIZX_DEMUCS_MODEL%` (huggingface.co/StemSplitio/
htdemucs-onnx). On the development machine both live in `D:\tools` and the
variables are set in cargo's `config.toml`. `cargo run --release -p vizx-ml
--example demucs -- <song> <folder>` separates a file without the window.

### Recognizing lyrics: vocals first, then Whisper

The Lyrics window's **Recognize from Vocals** writes down the sung words
with their times (`vizx_core::LyricsRecognition`, in a background thread):

1. **Vocals.** The song's stems are reused when they exist; otherwise the
   song is separated now (the best `StemEngine`, HTDemucs when installed)
   and the stems are written, so the next time is quick. Whisper is trained
   on speech; music under the voice makes it miss words, so it only ever
   hears the vocals.
2. **Words.** `vizx_ml::recognize_lyrics` runs Whisper (whisper.cpp through
   `whisper-rs`) on the vocals at 16 kHz mono, with token timestamps; tokens
   are joined into words (a token starting with a space starts a word).
   - Only the sung stretches go in (`sung_stretches`, `pack`): louder than
     8 % of the typical vocal loudness, widened by 0.5 s, gaps under 1.5 s
     kept, joined with 0.4 s pauses; times are moved back afterwards
     (`original_time`). Whisper makes up words on silence, and silence
     costs as much to listen to as singing.
   - The packed vocals are cut into pieces of at most 28 s at their
     quietest moment (`window_pieces`), and each piece is decoded on its
     own. Whisper's encoder costs the same for every 30-second window;
     left to itself, Whisper starts a new window after each line it finds,
     which on ANTIVILLAIN meant about 13 windows instead of 9.
   - Every piece is decoded without the text of the one before
     (`no_context`): with it, Whisper merged and dropped lines on songs.
   - A line's times are those of its first and last word (segment times
     are coarse), and lines of more than 11 words are split at punctuation
     or the longest pause (`split_long_line`).
   - **Quality** (`RecognitionQuality`): Fast decodes greedily with no
     retries, Balanced retries unclear pieces at a higher temperature,
     Accurate uses a beam search of 5. The encoder dominates, so they cost
     about the same.
   Lines whose vocals are nearly silent are dropped as well.
3. **Timeline.** The lines are moved from the audio's time to the
   timeline's (the clip's start and trimmed beginning), and shown in the
   Lyrics window as LRC text (`recognized_lyrics_to_lrc`), timed "As Timed".
   The user corrects the words and adds them as subtitles; each word's time
   is kept in `RecognizedLyric::words` for word-by-word text later.

**Model and build.** The model is a whisper.cpp `ggml-*.bin`
(`ggml-large-v3-turbo-q5_0.bin`, 574 MB, from huggingface.co/ggerganov/
whisper.cpp), in `models/` next to the executable or at
`%VIZX_WHISPER_MODEL%`; nothing is downloaded by the build or the app.
`whisper-rs` builds whisper.cpp from source with CMake (`CMAKE`) and
generates its bindings with bindgen, which needs libclang
(`LIBCLANG_PATH`; the `libclang` wheel from PyPI has the DLL). With MSVC,
`CL = "/O2 /D_USE_MATH_DEFINES"` must be set: MSVC 2017 needs the define for
`M_PI`, and the `cmake` crate leaves `/O2` out of the Release flags it hands
to CMake on MSVC (flags in `CFLAGS` lose it too), which made whisper.cpp
about ten times slower. `cl.exe` reads `CL` itself. On the development
machine all of this is set in cargo's `config.toml`.

**Measured speed** (i7-4720HQ, 4 cores, 8 threads; large-v3-turbo q5_0):
one 30-second encoder window takes about 83 s (100 s with 4 threads; flash
attention made it slower). ANTIVILLAIN (251 s, vocals already separated):
Fast 841 s (3.4 x its length), Accurate 873 s (3.5 x). The words came out
nearly all right in both; Accurate split lines more finely.
`cargo run --release -p vizx-ml --example lyrics -- <vocals> [fast|balanced|accurate] [language]`
measures a file without the window.

On stylized singing recognition is much worse. Party Favor (ivycomb feat.
OodleZz, 195 s), against its published lyrics: 53 % of the words wrong on
Balanced, 55 % on Accurate (32 % in the first minute, 97 % in the last,
where a nursery-rhyme line and "na na na" were missed or made up). The line
times were right to within 0.4 s. Hence the published words below.

### Lyrics found online (LRCLIB)

**Find Lyrics Online** in the Lyrics window (`vizx_core::LyricsSearch`, in a
background thread) asks LRCLIB (lrclib.net, a free open lyrics database, no
key) for the song. It is the only place VizX goes on the internet, only on
that button, and it sends only the title and artist (both editable; empty
means guessed from the file name, "Artist - Title"). The HTTP client is
`ureq` with rustls and its own root certificates (no system library).

- **Which song.** A title alone finds mostly the most famous song of that
  name: for "Party Favor" LRCLIB returned twenty Billie Eilish versions, one
  of them 196 s long, and not ivycomb's (195 s). So with an artist given only
  that artist's songs are kept (`same_artist`, ignoring case, punctuation
  and guests); without one the result says whose song it is and asks for
  the artist (`by_title_only`). Of the songs left, one within 4 s of the
  file's length is preferred, timed if possible.
- **Timed lyrics** (LRC) of a song of the file's length are used as they
  are (`timed_lyrics_on_timeline`: moved to the clip's place; a line lasts
  until the next one, at most 6 s, so it does not stay through an
  instrumental break).
- **Words only** (no timed version, or one of another length) go into the
  window untimed. **Recognize from Vocals** then keeps words that are in the
  window without times (pasted or found) and only times them:
  `align_official_lyrics` pairs them with the words heard, the same
  alignment the Subtitle Editor uses for official lyrics.

`cargo test -p vizx-core lyrics_online -- --ignored` asks the real LRCLIB.

### Speed measurements

On the development laptop (i7-4720HQ, Intel HD Graphics 4600), release
build; `cargo run --release -p vizx-render --example bench_compositor`
and the plugin/MilkDrop examples reproduce them.

| What | Speed |
|---|---|
| Compositor, 3 layers, 960x540 / 1080p | 1.9 / 5.2 ms per frame |
| ... with light effects (tint, vignette, grain) | 1.9 / 5.7 ms |
| ... with heavy effects on each (blur + split + glow) | 7.3 / 23.6 ms |
| ... plain layers plus a heavy effects layer | 3.8 / 14.1 ms |
| Visualizer plugins, 640x360 | 170 to 380 frames per second |
| MilkDrop presets, 640x360 | 54 to 190 frames per second |
| HTDemucs stem separation | about 2.3 x the song's length |
| Whisper lyrics recognition | about 3.4 x the song's length |

Everything drawn is faster than playback; separation and recognition are
the slow steps and run once per song (their results are kept).

### Portable prototype

`scripts\package-prototype.ps1` builds the release version and gathers it
into one folder (default `D:\VizX-prototype`) that runs without cargo or
environment variables: `vizx.exe`, `ffmpeg.exe`, projectM and GLEW, ONNX
Runtime, `models\` (hard-linked when on the same drive), the bundled
`visualizers\` and `presets\`, and an empty `data\`. A `data` folder next
to the executable switches on portable mode (`portable_data_folder`):
pre-rendered frames and stems go there instead of the system's temporary
folder and next to the songs.

### Installer

`scripts\build-installer.ps1` turns that folder into one file,
`VizX-Setup-<version>.exe` (default in `D:\vizx-installer`). It adds the
Visual C++ runtime DLLs (VizX, projectM and ONNX Runtime need them; a fresh
Windows does not have them) and the licences of the bundled tools, then
`vizx-pack` appends every file to `vizx-setup.exe`.

- **One file** (`vizx-setup/src/payload.rs`): the setup program is followed
  by its payload, one entry per file (deflate, or stored as is for models
  and pictures, which do not get smaller), and a 32-byte trailer that says
  where the payload starts. Windows ignores what follows a program, so the
  setup program finds its payload by reading its own file from the end.
  Paths in the payload may only name files inside the install folder.
- **Only for me** (default): into `%LOCALAPPDATA%\Programs\VizX`, the
  user's Start menu and desktop, and "Apps & features" under
  `HKEY_CURRENT_USER`. No administrator rights, except as a last resort:
  when the chosen folder cannot be written without them, the setup
  program starts itself again with `runas` (Windows asks) and `--elevated`
  plus the chosen options, and that copy starts at once. An empty `data\`
  switches on portable mode.
- **For everyone on this computer**: into `%ProgramFiles%\VizX`, everyone's
  Start menu and the public desktop, and `HKEY_LOCAL_MACHINE`; always with
  administrator rights. No `data\` folder (users cannot write there), so
  VizX keeps its large files in the temporary folder. "Start VizX" at the
  end goes through Explorer, so VizX does not run with those rights.
- Any folder can be chosen; VizX always gets a folder of its own, and a
  folder holding other files is refused. Installing over an earlier VizX
  updates it. `--silent [--all-users] [--folder <dir>] [--no-shortcuts]`
  installs without a window (and without asking for rights).
- **Uninstalling**: `uninstall.exe` in the install folder is the setup
  program without its payload. It removes exactly the files listed in
  `vizx-install-manifest.txt` (written while installing), the `data\`
  folder and the folders left empty, never anything else, then removes
  itself once it has closed. An installation for everyone (its entry is
  under `HKEY_LOCAL_MACHINE`) is removed with administrator rights, asked
  for the same way.

### Long sessions: autosave, recent projects, clipboard, shortcuts

VizX is used for hours at a time, so losing work or reaching for the mouse
for every step is expensive.

- **Autosave and recovery** (`vizx_core::Autosaver`, `app_data.rs`): once a
  minute, while the project has unsaved changes, a copy is written to
  `%APPDATA%\VizX\autosave\autosave-<process id>.vzx` with a note saying
  which file it belongs to. Saving, closing on purpose, or replacing the
  project deletes it. At startup, copies whose VizX is no longer running
  (checked by process id) are offered in a "Recover Work" window; a
  recovered project belongs to its original file again and counts as
  unsaved. The copies are small (the project only, never media).
- **Open Recent** (File menu): the last ten projects opened or saved, in
  `%APPDATA%\VizX\settings.json` (`AppSettings`).
- **Clipboard**: Ctrl+C / Ctrl+X / Ctrl+V copy, cut and paste the selected
  clip at the playhead (`EditorSession::copy_selected_clip`, `paste_clip`).
  The copy carries its media, so it can be pasted into another project; a
  text becomes a text of its own. egui delivers these keys as clipboard
  events, not key presses, so they are read from the input events.
- **Moving around**: Left/Right step one frame, Shift+Left/Right one
  second, Up/Down jump to the previous/next clip start or end.
- **F1** shows all shortcuts in one window (`shortcuts_window.rs`); keep
  its list in step with `handle_keyboard_shortcuts`.
- **Welcome window** (`welcome_window.rs`), in the spirit of Blender's
  splash screen: an animated banner, New Project templates (Empty, Music
  Visualizer, Lyrics Video, Cover Art Video: each asks for the files it
  needs and sets up the layers), recent projects and Open. It opens at
  startup unless turned off (Help > Welcome Screen brings it back), and not
  over the offer to recover work.
- **Panel layout** is stored in `settings.json` when VizX closes
  (`DockState` through egui_dock's serde support) and restored at startup.
- **Loop**: a loop region (Shift+drag on the ruler, or I / O at the
  playhead; L or the toolbar button turns looping on and off) is editing
  state in the session, not saved. While looping, the transport seeks back
  to its start when playback passes its end.
- **Markers** (format version 19, `Project::markers`): named moments added
  with M, drawn as flags on the ruler; click to go there, drag to move,
  right-click to rename or delete. Ctrl+Left/Right jump between them.
- **Several clips** can be selected: Shift+click, a rectangle dragged over
  empty lanes, or Ctrl+A. The session keeps them next to the main
  selection (`selected_clip_ids`); dragging one moves all, in time and up
  or down (`move_selected_clips_to_track`: clips of the dragged clip's kind
  shift by the same number of tracks, or none moves if one would fall off
  the first or last track), Delete removes all, and copy/paste keeps their
  distances.
- **Thumbnails** on visual clips (`vizx_core::make_thumbnail`,
  `thumbnails.rs`): the image, the first frame of an animation or a video
  (through FFmpeg) or the text, made once in the background and repeated
  along the clip like a film strip.
- **No overlapping clips** (`vizx-project/src/overlap.rs`): moving a clip
  or a group stops at the neighbours on its track (`free_shift`; a place
  past a neighbour is taken once it is free), trimming stops at the next
  clip (`trim_room`), a clip moves to another track only if it fits there
  (`fits_on_tracks`), and pasted or duplicated clips go to the next free
  place (`first_free_shift_forward`). The session applies these
  (`EditorSession::move_clip`, `trim_clip_start`, ...) unless Edit > Allow
  Clip Overlap is on.
- **Panels reopen where they were** (`Workspace::reopen`): back into the
  tab group they shared, or on their side of the default layout.
- **Blend modes** (`Clip::blend_mode`, `vizx_render::LayerBlend`): Normal,
  Add, Screen, Multiply and Lighten are fixed-function blend states on
  premultiplied colors, one render pipeline each, shared by the preview and
  the export.
- **Effects** (`Clip::effects`, a `ClipEffects`; drawn as
  `vizx_render::LayerEffects`): blur, color split, glow, glitch, grain,
  vignette and tint, each 0 to 1. They are drawn in the compositor's layer
  shader in one pass (blur and glow average 24 points on a spiral; glitch
  shifts horizontal bands twelve times a second; grain and glitch take the
  project time, so a moment always looks the same). With the reaction's
  `drives_effects` they are as strong as the sound is loud
  (`layer_effects`).
- **Shake** (`AudioReaction::shake`): a random jump 24 times a second, as
  far as the reaction's level allows; worked out from the moment only
  (`shake_offset`), so the preview and the export agree.
- **Effects layers** (`MediaContent::EffectsLayer`, "Add > Effects Layer"):
  a clip without a picture of its own that acts like a camera on all the
  layers below it. Its placement, rotation, keyframes, reaction (pulse,
  shake...) and effects apply to everything below, for as long as the clip
  lasts. The compositor draws the layers below it, copies the frame
  (`LayerDraw::shows_below`), and draws the copy back moved and changed;
  the clip's opacity mixes the changed picture with the unchanged one.
  `effects_layer_geometry` enlarges it by twice its shake, so shaking never
  shows the frame's edges. It is added on a new layer on top, and takes
  mouse clicks in the preview only once selected, so the layers below stay
  clickable.
- **Export settings** (`ExportSettings`, remembered in `AppSettings`):
  format (MP4 H.264 / H.265, WebM VP9, MOV ProRes 422, GIF with a palette
  made from its own colors, and two transparent ones: MOV ProRes 4444 with
  alpha and a PNG sequence `name_00001.png`...), quality (CRF or ProRes
  profile per format),
  height (width in the project's shape) and range (whole project or the
  loop). A clip that starts before the range is drawn from its start and
  the frames before the range are dropped, so it looks as in a full export.
  For the transparent formats the compositor starts each frame transparent
  instead of black, and the export turns its premultiplied colors into the
  plain RGBA that FFmpeg expects. They are meant for layering VizX's
  visualizers and lyrics in other editors.
- **Sound of videos** (`extract_video_sound`): importing a video also
  writes its sound as `<video> sound.wav` (next to it, or in
  `VIZX_STEMS_DIR`) and adds it on an audio track; a background video's
  sound is added muted.
### Visualizer plugins

Visualizers other than MilkDrop presets are **plugins**: a folder with a
`visualizer.json` (name and adjustable parameters) and a `shader.wgsl` (a
function that returns the color of a pixel). The visualizers that ship with
VizX are plugins too, in `assets/visualizers`. See
[visualizer-plugins.md](visualizer-plugins.md) for the author's guide.

- **Why shaders.** wgpu is already what VizX draws with, a shader runs the
  same on Windows and Linux without compiling anything per platform, and a
  broken plugin produces an error message instead of crashing the editor,
  which a native library (DLL) could not guarantee.
- **What a shader is given:** the spectrum, the waveform, the spectrum of
  the last 256 frames, overall levels, the time, the parameter values and
  its own previous frame. The last two inputs are what make waterfalls and
  feedback effects possible.
- **Audio analysis** (`vizx_audio::AudioAnalyzer`) turns the clip's audio
  input into those numbers with an FFT, once per frame.
- **Rendering** (`vizx_visualizer::ShaderRenderer`) happens off-screen on a
  wgpu device of its own, on the pre-render thread, and returns RGBA pixels:
  the same contract as the MilkDrop renderer. `vizx_core::Prerender`
  therefore handles both through one `FrameSource` enum, and everything
  after it (frame storage, preview, placement, audio input) is shared.
- **In the project** a plugin is a kind of media
  (`MediaContent::VisualizerPlugin`) whose path is the plugin folder, or
  `bundled-visualizers:/<name>` for a bundled one. The clip stores the
  values of the plugin's parameters by parameter id, without interpreting
  them.
- **Live reload.** `vizx_core::VisualizerCatalog` watches the files of the
  plugins in use (their modification times, checked twice a second). A saved
  file changes the plugin's version, which is part of the pre-render key, so
  the clip is rendered again.

### One "Add" menu

Everything that can be added to a project (audio file, image, video, image sequence,
sprite sheet, text, MilkDrop preset,
visualizer plugin, visual layer, audio track) is listed in one function, `ui::add_menu_items`.
It is shown in exactly one place, the context menu of the timeline
(right-click a track, a lane or the empty area), plus keyboard shortcuts
(Ctrl+I, Ctrl+Shift+I, Ctrl+M, Ctrl+Shift+M). The toolbar holds only the transport, and New, Open and
Save are only in the File menu. A new kind of content becomes available by
adding one line, and no panel grows its own row of "add this" buttons.

### Playback and the project clock

- `EditorSession::playhead_seconds` is the one "now" of the project. The
  timeline, the time display and, later, the preview all read it. Like the
  selection it is editing state: not saved, and moving it is not an edit.
- `vizx-audio`'s `AudioPlayer` sends audio to the sound card through `cpal`.
  While playing, **the audio is the clock**: the playhead is set from the
  number of frames actually handed to the sound card, so picture and sound
  cannot drift apart. The preview will be driven by the same value.
- The sound card requests audio on its own thread, which must never wait.
  The UI and audio threads share only atomics (playing flag, position, end
  position) and one mutex that the audio thread only *tries* to lock; if it
  is busy for an instant, that one short buffer is silent.
- Mixing is a pure function (`mix_sources`): for every output frame it adds
  up the sources at that project time, resampling with linear interpolation
  when a file's sample rate differs from the sound card's. Being pure, it is
  tested without any audio device.
- `Transport` (in `vizx-app`) connects the two: it copies the player's
  position into the playhead at the start of each frame, and if the UI moved
  the playhead during the frame (ruler click, Home/End) it tells the player
  to seek. The player is created on first use, and its stream is paused
  whenever nothing plays, so VizX does not keep the sound card busy.
- Audio files are decoded fully into memory (`DecodedAudio`, about 20 MB per
  minute of stereo). `AudioStore` in `vizx-core` loads each file once in the
  background and keeps both the samples (for playback) and the waveform
  peaks (for drawing). Streaming from disk can replace this later.

There are deliberately no per-track volume, pan or effect controls: see
"Audio drives the visuals".

### Waveforms

- `vizx-audio` decodes a file and reduces it to `WaveformPeaks`: the lowest
  and highest sample value of every 5 ms of audio. A song has millions of
  samples but a screen only a couple of thousand pixel columns, so the peaks
  hold everything a waveform drawing can show in a few hundred kilobytes.
- Decoding a whole song takes seconds, so it never happens on the UI thread.
  `AudioStore` (in `vizx-core`) decodes each file once on a background
  thread and receives the result through a channel. The UI asks for the
  current state every frame and draws what is available; a clip shows
  "reading waveform..." until its peaks arrive.
- The timeline draws one vertical line per pixel column, asking the peaks
  for exactly the time that column covers. The same code therefore works at
  every zoom level, and only visible columns are drawn.
- Waveforms are derived data. They are not part of the project, are not
  saved, and are recomputed when a project is opened.
- Audio tracks are taller than visual layers to give the waveform room, so
  lane positions are computed by `TrackRows` rather than by multiplying an
  index.

Dependencies are compiled with optimizations even in debug builds
(`[profile.dev.package."*"]` in the root `Cargo.toml`). Audio decoding is
many times faster that way, and VizX's own crates still build quickly.

### A project is named by its file

A project has exactly one name: the name of its `.vzx` file without the
extension, or "Untitled" before it is first saved
(`EditorSession::project_name`). The window title, dialogs and the Save As
suggestion all use it.

Earlier versions also stored an editable name inside the project. Saving
such a project as `test.vzx` left the window showing a different name than
the file, with no way to tell which one was "the" name. Format version **5**
removed the stored name; a `name` in older files is ignored on load.

### Tracks and selection

- A `Track` has a `TrackId`, a name and a `TrackKind` (`Audio` or
  `Visual`). Everything refers to a track by id, never by its position in
  the list, and ids are never reused within a project.
- The track list is private to `Project`; tracks are added, removed and
  renamed through methods so ids stay consistent.
- `Selection` lives in `vizx-core`'s `EditorSession`. It is shared editing
  state (the timeline sets it, the Properties panel shows it) but it is not
  project data and is not saved.

### The timeline adapts to the project and to the panel

Nothing in the timeline is a fixed drawing. Tracks and the length of the
time axis come from the project. `TimelineView` (UI state in `vizx-app`,
not saved) holds the zoom and scroll position:

- In the default "fit" mode the whole project duration fills the panel
  width, whatever size the panel has.
- Ctrl + mouse wheel zooms around the cursor; zooming all the way out
  returns to "fit" mode. The "Fit" button does the same.
- Lanes scroll in both directions; the ruler and track names stay in place
  and follow the scroll position.
- The distance between ruler labels is chosen from a list of round
  intervals so labels never overlap at any zoom level.

### Properties panel vs. Project Settings window

The Properties panel shows only the current selection. Settings of the
project as a whole (name, author, composition size, frame rate, duration)
are in the Project Settings window, opened on demand from File > Project
Settings or the Preview panel's context menu. Rarely changed settings should
not occupy permanent space. Such windows are egui windows inside the main
window, never separate operating-system windows.

Panels cannot run application commands themselves; they write an
`AppCommand` into a slot passed down from `VizxApp`, the same path the menus
use.

### Rendering the UI through wgpu

eframe is built with its `wgpu` renderer (not the OpenGL-only `glow` one), so
the UI already runs on the same GPU abstraction the future composition
renderer will use.

### Graphics backend selection

VizX never names a GPU vendor. `graphics.rs` only limits which backends wgpu
may consider per operating system:

- **Windows:** DirectX 12, then OpenGL.
- **Linux and others:** wgpu's default set (Vulkan, then OpenGL).

Vulkan is excluded by default on Windows because it depends entirely on the
GPU driver. On the primary development laptop (GeForce GTX 850M, driver from
2018) a debug build crashed inside Vulkan initialization, while DirectX 12
and OpenGL worked. `WGPU_BACKEND` overrides the default at any time.

### Window starts maximized

The default window size is 1440x900, but the window maximizes itself on the
first frame. Requesting size and maximized state together at creation left
the window at 1440x900 on Windows, which does not fit a 1920x1080 display at
125% scaling.

### No workspace-level `tests/` directory

In a Cargo workspace without a root package, a top-level `tests/` folder is
not compiled by Cargo. Tests live next to the code they test, inside each
crate. Cross-crate integration tests will go into the relevant crate's own
`tests/` folder when they are needed.

### Minimal dependencies

Direct dependencies: `eframe`, `egui_dock`, `rfd`, `serde`, `serde_json`,
`symphonia`, `cpal`, `libloading`, `image` and `jpeg-encoder` (reading and
writing pre-rendered frames) and, on Windows only, `windows-sys`.
Planned libraries (cpal, RustFFT, FFmpeg bindings) are added in the
milestone that first uses them.

`egui_dock` 0.21 targets egui 0.36, so eframe/egui stay on 0.36 until
egui_dock supports a newer release.

## Planned subsystems

Not implemented. Listed to show where they will attach:

| Subsystem | Role |
|---|---|
| Timeline engine | One timebase and track model shared by audio, images, animation, text, visualizers, effects |
| Audio engine | Multi-track playback and mixing, waveforms, FFT, beat detection, audio-reactive inputs |
| Animation & behavior engine | Sprite sheets, image sequences, keyframes, beat-triggered and audio-reactive motion |
| Composition engine | Combines all project elements into one frame description |
| Rendering engine | Draws that description with wgpu; the same code path for preview and export |
| Export | Deterministic project clock driving the renderer, encoded through FFmpeg |
| Local ML | More models later (for example voice-only, or lyrics timing from the vocals); stem separation with HTDemucs exists |

Each will add data to `vizx-project`, logic coordinated through `vizx-core`,
and views in `vizx-app`.
