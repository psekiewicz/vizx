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
vizx-app      MilkdropPreview: starts a pre-render for every MilkDrop clip
              and shows the stored frame that belongs to the playhead
vizx-core     MilkdropPrerender: renders all frames of a clip in the
              background and stores them in a temporary file
vizx-milkdrop MilkdropRenderer: projectM + an off-screen OpenGL context,
              returns frames as RGBA pixels
projectM      separate DLL, loaded while VizX is running
```

- projectM draws with **OpenGL**; the VizX window is drawn with wgpu
  (DirectX 12 on Windows). They cannot share a surface, so projectM gets its
  own OpenGL context on a window that is never shown, and each finished
  frame is read back as pixels and uploaded as a texture.
- An OpenGL context belongs to one thread, so the renderer lives entirely on
  its own background thread. The UI never waits for it.
- Each visualizer clip has an audio input (**Reacts to** in the Properties
  panel): the whole mix, which is the default and is what the sound card
  receives, or a single audio track. With stems on separate audio tracks
  (vocals, melody, bass), each visualizer can follow one of them. Playback
  always plays the whole mix.
- `vizx-milkdrop` is the only crate that contains `unsafe` code, C function
  declarations or OpenGL. Its off-screen context is currently implemented
  for Windows only (WGL); Linux will need an EGL version.

## Pre-rendering

A MilkDrop picture builds on the previous one: each frame is the last frame,
warped and faded, with something new drawn on top. A frame is therefore only
correct if every frame before it was rendered too. Drawing "live" at the
playhead cannot show the right picture after a jump, and it would make the
result depend on how fast the computer is.

So VizX renders each MilkDrop clip once, from its first frame to its last,
and keeps the frames:

- **Time comes from the frame number.** Before each frame VizX tells projectM
  what time it is (`frame / frames per second`). projectM 4.1 has no function
  for that; it is added by the small patch in
  [`patches/`](../patches/projectm-4.1.8-frame-time.patch), under the same
  name projectM's development version uses, so the patch can be dropped once
  a release contains it. Without the patch pre-rendering reports an error.
- **Audio comes from the timeline.** For each frame projectM receives exactly
  the audio between the previous frame and this one, taken from the clip's
  audio input (the mix or one audio track).
- **Two threads.** One draws frames with projectM (graphics card), the other
  compresses them as JPEG and appends them to a file (processor). A small
  buffer between them keeps memory use flat.
- **Frames live on disk.** The frames of one song are far too large for
  memory, so they go into one temporary file per clip, in the system's
  temporary folder under `vizx-prerender`. Only each frame's position in the
  file is kept in memory. The file is deleted when it is no longer needed;
  leftovers from a crash are removed after a day.
- **Re-rendering.** A pre-render belongs to a "key" made of everything that
  affects the frames: preset, size, frame rate, clip position and length, and
  the audio. If any of these change, the frames are thrown away and rendered
  again. Clips are rendered one at a time.

While a clip is being pre-rendered, the preview shows the frames that are
done and a progress message for the rest; the status bar shows overall
progress and the disk space used.

Measured on the development laptop (Intel HD Graphics 4600, 640x360,
30 fps): a detailed preset renders at roughly 95 frames per second, so a
four-minute song takes about 80 seconds and about 600 MB of temporary disk
space. Simple presets are faster and smaller.

Two renders of the same clip are visually the same but not identical to the
last bit: projectM seeds some noise textures randomly.

## Bundled presets

VizX ships with projectM's "Cream of the Crop" collection: 9795 presets,
curated by ISOSCELES and sorted into folders by style. Right-click the
timeline and choose **Add > MilkDrop Preset...** (or press Ctrl+M) to open a
browser for it, with a search field and a category list. **Add > MilkDrop
Preset File...** in the same menu adds a `.milk` file of your own instead.

- **In the repository** the collection is a git submodule in
  `assets/presets/cream-of-the-crop`. It is a link to projectM's repository
  at one fixed commit, not a copy, so VizX's history stays small and the
  collection can be updated with one command. After cloning VizX, download
  it with `git submodule update --init`.
- **Where VizX looks:** in a `presets` folder next to the executable (an
  installed VizX), then in `assets/presets` of the source tree (development).
  Every folder inside is one preset pack; a pack's top-level folders are the
  categories. Without the folder VizX still runs; the browser then says the
  presets are missing.
- **In a project** a bundled preset is stored as
  `bundled-presets:/cream-of-the-crop/<category>/.../<name>.milk`. The
  location on the current computer is only worked out when the preset is
  rendered, so a project made on Windows opens on Linux and the other way
  round. A reference that tries to leave the presets folder (`..`) is
  refused.
- **Indexing** all files takes a moment on a hard disk, so it happens on a
  background thread the first time the browser is opened. The list draws
  only its visible rows.

## Licensing
projectM is licensed under the **LGPL**. VizX loads it as a separate library
at run time and does not link it statically, so it stays a replaceable file.
When distributing VizX, `projectM-4.dll` and `glew32.dll` must be shipped as
separate files next to the executable, together with projectM's license.

The presets were written by many authors and almost none of them carries a
license. The collection's `LICENSE.md` explains that they were released
freely and have been redistributed for two decades, so projectM treats them
as public domain, and offers to remove a preset if its author asks. That is
an assumption, not a formal grant. VizX follows it: the presets are shipped
unmodified together with that `LICENSE.md`, and because they are a
submodule, a preset removed upstream disappears from VizX with the next
update.

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

# 2. Apply the VizX patch that pre-rendering needs (see "Pre-rendering").
Push-Location "$tools\projectm-src"
git apply "<vizx repository>\patches\projectm-4.1.8-frame-time.patch"
Pop-Location

# 3. Configure. Use the generator matching your Visual Studio.
& "$tools\cmake\bin\cmake.exe" -S "$tools\projectm-src" -B "$tools\projectm-build" `
    -G "Visual Studio 15 2017" -A x64 `
    "-DCMAKE_INSTALL_PREFIX=$tools\projectm" `
    "-DGLEW_INCLUDE_DIR=$tools\glew\include" `
    "-DGLEW_SHARED_LIBRARY_RELEASE=$tools\glew\lib\Release\x64\glew32.lib" `
    "-DGLEW_SHARED_LIBRARY_DEBUG=$tools\glew\lib\Release\x64\glew32.lib" `
    -DENABLE_PLAYLIST=OFF -DENABLE_SDL_UI=OFF -DBUILD_TESTING=OFF `
    -DBUILD_SHARED_LIBS=ON -DENABLE_SYSTEM_PROJECTM_EVAL=OFF

# 4. Build and install (a few minutes).
& "$tools\cmake\bin\cmake.exe" --build "$tools\projectm-build" --config Release --target install

# 5. Put GLEW's DLL next to projectM's.
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

It prints the projectM and OpenGL versions, renders 300 frames as fast as it
can and writes three of them as BMP images.

Presets: "Cream of the Crop" ships with VizX (see "Bundled presets"). The
original MilkDrop presets are a separate collection of about 550
(<https://github.com/projectM-visualizer/presets-milkdrop-original>).

## Known limitations

- **Preview resolution** is 640 pixels wide, to keep rendering and storing
  every frame affordable. Export will need its own full-size render.
- **Disk space.** A pre-rendered clip of song length takes hundreds of
  megabytes in the temporary folder while the project is open.
- **Nothing is kept between sessions.** Opening a project renders its
  MilkDrop clips again.
- **Textures.** Presets that use image textures need projectM's texture
  search paths, which VizX does not set yet; those presets render without
  their textures.
- **One preset at a time.** The topmost visual layer with a clip at the
  playhead is shown. Layers are not composited yet, so several visualizers
  that follow different stems cannot be seen together yet.
- **Integrated graphics on hybrid laptops.** The off-screen OpenGL context
  is created on whichever GPU Windows picks by default, usually the
  integrated one.
- **Windows only** for now.