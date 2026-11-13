# VizX

VizX is a native desktop application for creating music-driven audiovisual
compositions: music visualizers, lyric and karaoke videos, audio-reactive
animations and image compositions.

It is one integrated editor. Audio, visuals, text and animation live in a
single project and share one timeline.

> **Status: prototype, milestone 0.1 (workspace foundation).**
> The application window, dockable panel layout and a minimal project model
> exist. There is no audio, rendering, timeline editing or export yet.

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
│   ├── vizx-core/         Shared application logic (no GUI)
│   └── vizx-app/          Desktop application: window and interface
├── assets/                Icons, fonts and shaders (empty for now)
└── docs/
    └── architecture.md    Architecture and design decisions
```

See [docs/architecture.md](docs/architecture.md) for how the crates fit
together and why.

## Requirements

- Rust, installed with [rustup](https://rustup.rs). The toolchain version is
  selected automatically from `rust-toolchain.toml`.
- **Windows:** the MSVC C++ build tools and a Windows SDK (installed with
  Visual Studio or "Build Tools for Visual Studio"). Rust uses their linker.
- **Linux:** a C toolchain plus the usual X11/Wayland development packages.

No dedicated GPU is required.

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
