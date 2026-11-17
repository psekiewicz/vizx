# VizX

VizX is a native desktop application for creating music-driven audiovisual
compositions: music visualizers, lyric and karaoke videos, audio-reactive
animations and image compositions.

It is one integrated editor. Audio, visuals, text and animation live in a
single project and share one timeline.

> **Status: prototype.** Usable end to end, from importing a song to
> exporting a video, but the file format and interface still change.

## Features

- **Timeline and editing.** Tracks for audio and visuals with clips that
  can be moved, trimmed, split, copied and grouped. Also markers, loop
  playback, thumbnails, undo/redo, autosave and recovery, and keyboard
  shortcuts (F1 lists them).
- **Visuals.** Images (with transparency, animated GIFs), video files, image
  sequences, sprite sheets and TrueType text. Each can be moved, resized and
  rotated in the preview, has keyframes, a blend mode (Normal, Add, Screen,
  Multiply, Lighten) and opacity, and can react to the sound.
- **Visualizers.** MilkDrop presets through projectM, pre-rendered with a
  bundled preset collection, and eleven bundled WGSL visualizer plugins:
  Spectrum Bars, Oscilloscope, Waterfall, Core, Ring, Tunnel, Warp Stars,
  Ridgelines, Synthwave, Kaleidoscope and Beat Burst. You can
  [write your own](docs/visualizer-plugins.md).
- **Sound-driven animation.** Beat detection (or beats drawn by hand) and
  a chosen tempo. Visuals can pulse, shake, spin or step frames on the
  beat, and effects can follow the loudness.
- **Effects.** Blur, color split, glow, glitch, grain, vignette and tint on
  any clip. Effects layers act as a camera that shakes, zooms or tints
  everything below them.
- **Scenes.** Every marker starts a scene with a transition into it (cut,
  flash, fade through black, zoom punch, glitch, blur, spin, shake).
- **Stems.** Splits a song into vocals, drums, bass and other, using
  HTDemucs when it is installed and a quick built-in split otherwise, so
  each visual can listen to the part that suits it.
- **Lyrics and subtitles.** Paste lyrics and time them, or recognize them
  from the vocals with Whisper. "Correct Words from Official Lyrics" then
  puts the published words on the recognized timing. Subtitle layers set
  the look of all their lines, the Subtitle Editor edits lines in a table
  (and saves `.srt`), and Separate Singers puts each singer on a layer of
  their own (duets included).
- **Export.** MP4 (H.264, H.265), WebM VP9, MOV ProRes 422, GIF, and with
  a transparent background as MOV ProRes 4444 or a PNG sequence. Resolution
  goes up to 2160p, and you can export the whole project or the loop range.
  The preview and the export use the same compositor.
- **Projects.** `.vzx` files that find moved media again, and Collect All
  and Save, which copies a project and all its files into one folder.

All machine learning (stem separation, lyrics recognition, telling
singers apart) runs on your own computer. Nothing is uploaded.

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
│   ├── vizx-ml/           Local ML: HTDemucs stems, Whisper lyrics, WeSpeaker singers (no GUI)
│   ├── vizx-core/         Shared application logic (no GUI)
│   └── vizx-app/          Desktop application: window and interface
├── assets/                Bundled visualizer plugins, MilkDrop presets (git submodule), fonts, icons
├── scripts/               Model installer and portable packaging (Windows)
└── docs/
    ├── architecture.md    Architecture and design decisions
    ├── milkdrop.md        Setting up projectM for MilkDrop presets
    └── visualizer-plugins.md  Writing a visualizer plugin
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

Optional, for the local machine learning features. Without them VizX uses
a quick built-in stem separation, and lyrics recognition and Separate
Singers are unavailable:

| What | Used for | Found at (next to `vizx.exe`) or set with |
|---|---|---|
| [ONNX Runtime](https://github.com/microsoft/onnxruntime/releases) | HTDemucs and WeSpeaker | `onnxruntime.dll`, `VIZX_ONNXRUNTIME_DIR` |
| [HTDemucs ONNX model](https://huggingface.co/StemSplitio/htdemucs-onnx) | stem separation | `models/htdemucs.onnx`, `VIZX_DEMUCS_MODEL` |
| [Whisper](https://huggingface.co/ggerganov/whisper.cpp) (`ggml-large-v3-turbo-q5_0.bin`) | lyrics recognition | `models/`, `VIZX_WHISPER_MODEL` |
| [WeSpeaker ResNet34](https://huggingface.co/Wespeaker/wespeaker-voxceleb-resnet34-LM) | Separate Singers | `models/`, `VIZX_SPEAKER_MODEL` |

`VIZX_STEMS_DIR` puts the separated stems in one folder instead of next to
each song.

Building with lyrics recognition compiles whisper.cpp from source. That
needs CMake and libclang (`LIBCLANG_PATH`), and on Windows also
`CL = "/O2 /D_USE_MATH_DEFINES"`. Without that setting whisper.cpp is
about ten times slower. See
[docs/architecture.md](docs/architecture.md#recognizing-lyrics-vocals-first-then-whisper).

The easiest way to get the models: run `scripts\Install models.cmd` (or the
copy next to `vizx.exe` in a packaged build). It asks which models to
install, then downloads HTDemucs, ONNX Runtime, the Whisper model and the
WeSpeaker model from where their authors publish them (Hugging Face,
GitHub). It checks every file against its SHA-256 and puts it next to
`vizx.exe`. It only uses tools that come with Windows 10 (1803 and later)
and 11 (`curl`, `tar`, `certutil`). Interrupted downloads continue where
they stopped, and nothing needs administrator rights.

Pre-rendered MilkDrop frames can take gigabytes; they go to the system's
temporary folder, or to `VIZX_CACHE_DIR` when it is set.

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

### Portable build (Windows)

Double-click `scripts\Package VizX.cmd` (or run
`scripts\package-prototype.ps1` in PowerShell). It builds the release
version and gathers it into `dist\VizX` in the repository, or into a
folder given as the first argument. The folder holds `vizx.exe`, the
bundled visualizers and presets, and `Install models.cmd`, and it runs
without cargo or environment variables.

FFmpeg, projectM, ONNX Runtime and the models are copied in when the
script finds them: from the `VIZX_*` settings (environment variables or
the `[env]` section of cargo's `config.toml`), and FFmpeg also from the
`PATH`. Anything not found is left out with a note at the end. Run
`Install models.cmd` in the folder to download the models and ONNX
Runtime.

A `data` folder next to `vizx.exe` keeps pre-rendered frames and stems
there instead of the system's temporary folder.

## Showcase video

`crates/vizx-core/examples/showcase.rs` builds a project that uses as many
VizX features as it can for one song and exports it with the application's
own export:

```powershell
cargo run --release -p vizx-core --example showcase -- `
    song.mp3 "stems folder" "project with timed lyrics.vzx" D:\vizx-showcase
```

What it shows, part by part of the song (each part also gets a timeline
marker and an on-screen caption naming its features):

- the song with its four stems (vocals, drums, bass, other), each visual
  listening to the one it suits;
- a different bundled MilkDrop preset in every part;
- five of the bundled visualizer plugins: the ring on the drums, the oscilloscope on
  the vocals, spectrum bars, the waterfall on the "other" stem, the core on
  the bass, with their parameters set;
- a background image with a slow keyframed zoom, an emblem that grows,
  turns and shakes with the bass, an image sequence that spins, sprite
  sheets that step one frame per drum beat, and a video that slides in;
- blend modes (Add, Screen, Lighten), opacity, clip effects (glow, color
  split, tint, grain, vignette, blur, glitch);
- effects layers with a look per part: camera shake and effects driven by
  the drums;
- a keyframed title in a TrueType font, and the lyrics as popping
  subtitles, split into one subtitle layer per singer (Separate Singers;
  `--singers N`, 0 lets VizX decide);
- colors that change only where a chorus starts or ends: another MilkDrop
  preset, background, ring color and tint;
- scenes on the markers, with a transition (flash, fade, zoom punch, ...)
  into each one.

`--official lyrics.txt` replaces the recognized words with the published
lyrics (saved by you into a text file, one line per line), on the
recognized timing, the same way as "Correct Words from Official Lyrics" in
the Subtitle Editor.

The pictures, sprite sheet, image sequence and video are generated (the
video by FFmpeg), so the example needs only the song, its stems and the
lyrics. `--samples` exports two seconds of every part at 360p for a quick
look; `--height` sets the video's height (1080 by default).
`--update project.vzx` puts the scene transitions into an existing
showcase project, and `--check project.vzx` exports short clips around
each scene change. The song and the rendered video are not part of the
repository.
