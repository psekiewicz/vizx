# VizX

VizX is a native desktop application for creating music-driven audiovisual
compositions: music visualizers, lyric and karaoke videos, audio-reactive
animations and image compositions.

It is one integrated editor. Audio, visuals, text and animation live in a
single project and share one timeline.

> **Status: early prototype.**
> The application window, dockable panel layout, project files (`.vzx`), an
> adaptive timeline with tracks, audio import, waveform display, audio
> playback, pre-rendered MilkDrop presets with a bundled preset collection, and
> visualizer plugins (WGSL shaders) and images, all with transparency, that can
> be moved and resized in the preview and react to the sound, video files as
> layers or background, text, clip editing on the timeline (move, trim,
> split), image sequences and sprite sheets, keyframe animation, beat detection, undo/redo and export to MP4 exist. There is no export yet.

## Technology

| Area | Choice |
|---|---|
| Language | Rust (stable) |
| GUI | [egui](https://github.com/emilk/egui) via eframe |
| Dockable panels | [egui_dock](https://github.com/Adanos020/egui_dock) |
| GPU | wgpu (DirectX 12, Vulkan or OpenGL, chosen at startup) |
| Serialization | Serde |
| Project file extension | `.vzx` |

Supported platforms: Windows 10/11 and Linux.

## Repository layout

```text
vizx/
├── Cargo.toml             Workspace manifest: members, shared versions
├── rust-toolchain.toml    Rust toolchain used by this project
├── crates/
│   ├── vizx-project/      Project data model (no GUI)
│   ├── vizx-audio/        Audio files, mixing and playback (no GUI)
│   ├── vizx-milkdrop/     MilkDrop presets through projectM (no GUI)
│   ├── vizx-visualizer/   Visualizer plugins: WGSL shaders via wgpu (no GUI)
│   ├── vizx-render/       Off-screen wgpu device and layer compositor (no GUI)
│   ├── vizx-ml/           Local stem separation: HTDemucs via ONNX Runtime (no GUI)
│   ├── vizx-core/         Shared application logic (no GUI)
│   └── vizx-app/          Desktop application: window and interface
├── assets/                Bundled visualizer plugins and MilkDrop presets (git submodule)
└── docs/
    └── architecture.md    Architecture and design decisions
```

See [docs/architecture.md](docs/architecture.md) for how the crates fit
together and why, and [docs/visualizer-plugins.md](docs/visualizer-plugins.md)
for writing a visualizer of your own.

## Requirements

- Rust, installed with [rustup](https://rustup.rs). The toolchain version is
  selected automatically from `rust-toolchain.toml`.
- **Windows:** the MSVC C++ build tools and a Windows SDK (installed with
  Visual Studio or "Build Tools for Visual Studio"). Rust uses their linker.
- **Linux:** a C toolchain plus the usual X11/Wayland development packages.

No dedicated GPU is required.

Optional: the [projectM](docs/milkdrop.md) library, for MilkDrop presets.
VizX builds and runs without it.

Optional: [FFmpeg](https://ffmpeg.org), for video files and for exporting.
VizX starts the `ffmpeg` program; it finds it next to its own executable,
in `%VIZX_FFMPEG_DIR%/bin` or on the `PATH`. For development, set
`VIZX_FFMPEG_DIR` in your personal Cargo configuration, next to
`VIZX_PROJECTM_DIR` (see [docs/milkdrop.md](docs/milkdrop.md)).

Optional: [ONNX Runtime](https://github.com/microsoft/onnxruntime/releases)
and the [HTDemucs ONNX model](https://huggingface.co/StemSplitio/htdemucs-onnx),
for clean stem separation (vocals, drums, bass, other). Without them VizX
uses a quick built-in separation. VizX looks for `onnxruntime.dll` and
`models/htdemucs.onnx` next to its executable, or uses
`VIZX_ONNXRUNTIME_DIR` (the unpacked archive) and `VIZX_DEMUCS_MODEL` (the
`.onnx` file). `VIZX_STEMS_DIR` puts the separated stems in one folder
instead of next to each song. Everything runs locally.

The bundled MilkDrop presets are a git submodule. Clone with
`git clone --recurse-submodules`, or run `git submodule update --init` in an
existing clone.

## Build, run, test

Run these from the repository root:

```sh
cargo run -p vizx-app        # build and start VizX (debug build)
cargo test --workspace       # run all tests
cargo clippy --workspace --all-targets -- -D warnings   # lints
cargo fmt --all              # format the code
```

The first build compiles all dependencies and takes several minutes. Later
builds only recompile what changed.

### Choosing a graphics backend

VizX lets wgpu pick the graphics backend. To force one, for example to work
around a driver problem, set `WGPU_BACKEND` before starting the app:

```powershell
$env:WGPU_BACKEND = "gl"      # or: dx12, vulkan
cargo run -p vizx-app
```

The backend and adapter in use are shown on the right side of the status bar.
