# VizX architecture

This document records how VizX is structured and why. Update it whenever a
decision here changes.

## Guiding principle

VizX is one editor, not several tools in one window. Every feature works on
the same project data and the same timeline, and every panel is a view of the
same state. A new feature must plug into that shared state instead of keeping
its own copy of anything.

## Crates

The workspace has seven crates. Dependencies point in one direction only:

```text
vizx-app  ──►  vizx-core  ──►  vizx-project
 (GUI)         (logic)    │     (data)
                          ├──►  vizx-audio
                          │     (audio files, playback)
                          ├──►  vizx-milkdrop
                          │     (MilkDrop presets via projectM)
                          ├──►  vizx-visualizer
                          │     (visualizer plugins: shaders via wgpu)
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
| `vizx-core` | Editing state and logic operating on a project. Future home of subsystem coordination. | GUI code |
| `vizx-app` | The window, panels, menus and visual style. | Project rules or logic that other front ends would need |

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
`source_offset_milliseconds`, version **15** its `audio_gain`.

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
  handles stay at its placement. A fixed rotation of a clip and keyframes
  are not implemented yet.

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
  clip on the timeline to make texts that appear at set times (lyrics).

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

Everything that can be added to a project (audio file, image, video, text, MilkDrop preset,
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
| Local ML | Stem and voice separation; no model chosen yet |

Each will add data to `vizx-project`, logic coordinated through `vizx-core`,
and views in `vizx-app`.
