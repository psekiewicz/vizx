# VizX architecture

This document records how VizX is structured and why. Update it whenever a
decision here changes.

## Guiding principle

VizX is one editor, not several tools in one window. Every feature works on
the same project data and the same timeline, and every panel is a view of the
same state. A new feature must plug into that shared state instead of keeping
its own copy of anything.

## Crates

The workspace has three crates. Dependencies point in one direction only:

```text
vizx-app  ──►  vizx-core  ──►  vizx-project
 (GUI)         (logic)          (data)
```

| Crate | Responsibility | Must not contain |
|---|---|---|
| `vizx-project` | What a project *is*: plain serializable data. | GUI, rendering, audio devices, file dialogs |
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
- All types derive Serde's `Serialize` and `Deserialize`. The on-disk
  encoding of `.vzx` is **not decided yet**; tests use JSON only to prove the
  model survives a round trip.

### `vizx-core`

- `EditorSession` is the state of the one open project: the `Project`, its
  file path (if saved) and whether it has unsaved changes.
- The project can only be modified through `EditorSession::edit_project`.
  Funnelling every change through one method means the unsaved-changes flag
  cannot be forgotten, and gives undo/redo a single place to hook into later.

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

## Decisions

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

Direct dependencies: `eframe`, `egui_dock`, `serde`, and `serde_json` (tests
only). Planned libraries (cpal, RustFFT, FFmpeg bindings) are added in the
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
