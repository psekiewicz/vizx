# MilkDrop presets in VizX

VizX plays original MilkDrop presets (`.milk` files) through
[projectM](https://github.com/projectM-visualizer/projectm), the open-source
implementation of the Winamp visualizer. VizX does not imitate MilkDrop; the
presets are run by projectM itself, so they look the way their authors made
them.

projectM is **optional**. VizX builds and runs without it; a MilkDrop preset
on the timeline then shows a message in the preview instead of a picture.

## How it fits together

```text
vizx-app      MilkdropPreview: asks for a frame when the playhead moves,
              shows the newest finished frame in the Preview panel
vizx-core     MilkdropWorker: a background thread that owns the renderer
vizx-milkdrop MilkdropRenderer: projectM + an off-screen OpenGL context,
              returns frames as RGBA pixels
projectM      separate DLL, loaded while VizX is running
```

- projectM draws with **OpenGL**; the VizX window is drawn with wgpu
  (DirectX 12 on Windows). They cannot share a surface, so projectM gets its
  own OpenGL context on a window that is never shown, and each finished
  frame is read back as pixels and uploaded as a texture.
- An OpenGL context belongs to one thread, so the renderer lives entirely on
  its own thread. The UI sends requests through a channel and never waits.
- The visualizer reacts to the same mix the sound card receives
  (`vizx_audio::mix_to_stereo`).
- `vizx-milkdrop` is the only crate that contains `unsafe` code, C function
  declarations or OpenGL. Its off-screen context is currently implemented
  for Windows only (WGL); Linux will need an EGL version.

## Licensing

projectM is licensed under the **LGPL**. VizX loads it as a separate library
at run time and does not link it statically, so it stays a replaceable file.
When distributing VizX, `projectM-4.dll` and `glew32.dll` must be shipped as
separate files next to the executable, together with projectM's license.

Preset files have their own authors and are not part of VizX.

## Setting up projectM on Windows

VizX looks for `projectM-4.dll` (with `glew32.dll` beside it):

1. next to the VizX executable, then
2. in `%VIZX_PROJECTM_DIR%\bin` (used during development).

projectM publishes source code only, so it has to be built once.

Requirements: a C++14 compiler (Visual Studio 2017 or newer), CMake 3.21 or
newer, and the GLEW binaries for Windows.

```powershell
# Paths are examples; any folder works.
$tools = "D:\tools"

# 1. Unpack: portable CMake, GLEW's Windows binaries (glew-*-win32.zip) and
#    the libprojectM-4.x source archive into $tools\cmake, $tools\glew and
#    $tools\projectm-src.

# 2. Configure. Use the generator matching your Visual Studio.
& "$tools\cmake\bin\cmake.exe" -S "$tools\projectm-src" -B "$tools\projectm-build" `
    -G "Visual Studio 15 2017" -A x64 `
    "-DCMAKE_INSTALL_PREFIX=$tools\projectm" `
    "-DGLEW_INCLUDE_DIR=$tools\glew\include" `
    "-DGLEW_SHARED_LIBRARY_RELEASE=$tools\glew\lib\Release\x64\glew32.lib" `
    "-DGLEW_SHARED_LIBRARY_DEBUG=$tools\glew\lib\Release\x64\glew32.lib" `
    -DENABLE_PLAYLIST=OFF -DENABLE_SDL_UI=OFF -DBUILD_TESTING=OFF `
    -DBUILD_SHARED_LIBS=ON -DENABLE_SYSTEM_PROJECTM_EVAL=OFF

# 3. Build and install (a few minutes).
& "$tools\cmake\bin\cmake.exe" --build "$tools\projectm-build" --config Release --target install

# 4. Put GLEW's DLL next to projectM's.
Copy-Item "$tools\glew\bin\Release\x64\glew32.dll" "$tools\projectm\bin\"
```

Then tell VizX where it is. For development, add this to your personal Cargo
configuration (`%CARGO_HOME%\config.toml`, not the repository), so that
`cargo run` and `cargo test` pass it on:

```toml
[env]
VIZX_PROJECTM_DIR = "D:/tools/projectm"
```

To check the setup without the VizX window:

```powershell
cargo run -p vizx-milkdrop --example render_frames -- "path\to\preset.milk" "output folder"
```

It prints the projectM and OpenGL versions and writes three frames as BMP
images.

Presets: the original MilkDrop collection is available at
<https://github.com/projectM-visualizer/presets-milkdrop-original>.

## Known limitations

- **Live preview only.** A frame is drawn when the playhead moves. Because a
  MilkDrop picture builds on the previous one, scrubbing or seeking shows an
  approximation; the picture is only "correct" when played from a point
  onward.
- **Real-time animation clock.** projectM 4.1 advances its animation by the
  computer's clock, not by project time. This is fine for live preview but
  not for export: rendering faster or slower than real time would change the
  animation speed. Deterministic, pre-rendered frames need a projectM with a
  settable frame time.
- **Preview resolution** is 640 pixels wide, to keep reading back and
  uploading each frame cheap.
- **Textures.** Presets that use image textures need projectM's texture
  search paths, which VizX does not set yet; those presets render without
  their textures.
- **One preset at a time.** The topmost visual layer with a preset at the
  playhead is shown. Layers are not composited yet.
- **Windows only** for now.
