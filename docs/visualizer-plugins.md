# Writing a visualizer plugin

A VizX visualizer is a small program that decides the color of every pixel
from the sound. It is written in **WGSL**, the shading language of WebGPU,
and runs on the graphics card. The four visualizers that ship with VizX
(Spectrum Bars, Oscilloscope, Waterfall, Core) are ordinary plugins; they
live in [`assets/visualizers`](../assets/visualizers) and are good starting
points to copy.

Nothing has to be compiled or installed. A plugin is a folder with two text
files, and VizX reloads it whenever you save one of them.

## A plugin in five minutes

1. Make a folder anywhere, for example `my-first-visualizer`.

2. In it, create `visualizer.json`:

   ```json
   {
       "name": "My First Visualizer",
       "author": "Your name",
       "description": "A disc that grows with the bass.",
       "parameters": [
           { "id": "size", "name": "Size", "kind": "number", "min": 0.1, "max": 1.0, "default": 0.4 },
           { "id": "color", "name": "Color", "kind": "color", "default": [1.0, 0.4, 0.1] }
       ]
   }
   ```

3. And `shader.wgsl`:

   ```wgsl
   fn visualize(uv: vec2f) -> vec4f {
       // Distance from the middle of the picture, with round circles.
       var point = (uv - 0.5) * 2.0;
       point.x *= aspect_ratio();
       let distance = length(point);

       let radius = p_size() * (0.5 + vizx.bass);
       // 1 inside the disc, 0 outside: the disc is opaque and everything
       // around it is transparent.
       let inside = 1.0 - smoothstep(radius - 0.01, radius, distance);
       return vec4f(p_color(), inside);
   }
   ```

4. In VizX, import a song, right-click the timeline and choose
   **Add > Visualizer...**, then **Load from Folder...** and pick the folder.

5. Keep VizX open, change the shader in your editor and save. The preview
   renders the clip again. A mistake in the shader is shown in the preview
   with the line number of `shader.wgsl`.

## How a visualizer works

VizX calls `visualize(uv)` once for every pixel of every frame. The result
is a color (red, green, blue from 0 to 1) and, as the fourth value, its
**opacity**: 1 covers whatever is on the layers below, 0 is fully
transparent and lets it show through. Leave the background transparent, so
the visualizer can lie on top of an image or another visualizer.

`uv` says which pixel is being asked about: `(0, 0)` is the top left corner
and `(1, 1)` the bottom right, whatever the size of the picture. A
visualizer has no loops over bars or lines; instead each pixel works out for
itself whether it is inside a bar, near a line, and so on. Look at
`spectrum-bars/shader.wgsl` for the idea in thirty lines.

Frames are rendered ahead of time, in order, and stored (see
[milkdrop.md](milkdrop.md), "Pre-rendering"). A visualizer therefore gives
the same picture for the same moment of the song on every computer.

## What the shader knows

### The sound

| Function | Returns |
|---|---|
| `spectrum(x)` | Loudness, 0 to 1, of the frequency at `x`. `x = 0` is 30 Hz, `x = 1` is 16 kHz, spaced like the keys of a piano. Values rise at once and fall smoothly. |
| `waveform(x)` | The sound wave, -1 to 1. `x = 0` is the oldest and `x = 1` the newest of the last 512 samples. |
| `spectrum_history(x, age)` | The spectrum of earlier frames. `age = 0` is the current frame, `age = 1` is 256 frames ago. This is what a waterfall is made of. |

| Value | Meaning |
|---|---|
| `vizx.volume` | Overall loudness, 0 to 1. |
| `vizx.bass` | Loudness below 250 Hz, 0 to 1. |
| `vizx.mid` | Loudness from 250 Hz to 4 kHz, 0 to 1. |
| `vizx.treble` | Loudness above 4 kHz, 0 to 1. |

The sound is whatever the clip's **Reacts to** setting selects: the whole
mix or a single audio track.

### Time and the picture

| Value / function | Meaning |
|---|---|
| `vizx.time` | Seconds since the start of the clip. |
| `vizx.frame` | Number of the frame, starting at 0. |
| `vizx.resolution` | Size of the picture in pixels (`vec2f`). |
| `aspect_ratio()` | Width divided by height. |
| `previous_frame(uv)` | The color and opacity this visualizer returned at `uv` one frame ago; transparent in the first frame. Use it for trails and echoes (see Oscilloscope and Core). |
| `from_light(rgb)` | Turns "light on a black background" into a color with opacity: black becomes transparent, bright becomes opaque. Handy for glows: add up light as if on black, then `return from_light(color);`. |
| `PI`, `TAU` | 3.14159..., 6.28318... |

A clip that covers only part of the frame is rendered at that size and
shape, so use `vizx.resolution` or `aspect_ratio()` rather than assuming
16:9.

### Parameters

Every entry in `"parameters"` becomes a control in the Properties panel and
a function in the shader named `p_` plus its `id`:

| `"kind"` | Manifest fields | Control | In the shader |
|---|---|---|---|
| `"number"` | `min`, `max`, `default`, optional `step` | Slider | `p_id()` returns `f32` |
| `"color"` | `default`: `[red, green, blue]`, each 0 to 1 | Color button | `p_id()` returns `vec3f` |
| `"toggle"` | `default`: `true` or `false` | Checkbox | `p_id()` returns `f32`, 1 or 0 |

An `id` consists of lower-case letters, digits and `_` and starts with a
letter. All parameters together may use 32 numbers; a color counts as three.

Parameter values are saved in the project, per clip. Changing one renders
the clip again.

## Names to avoid

VizX adds its own code after yours. Do not define anything called `vizx`,
`PI`, `TAU`, or starting with `vizx_` or `p_`, and do not define the
functions listed above.

## Trying a plugin without the VizX window

```sh
cargo run -p vizx-visualizer --example render_plugin -- "path/to/plugin" out.jpg 90
```

This renders 90 frames for a made-up sound (a beat and a wandering note) and
saves the last one. Shader errors are printed.

## Sharing and bundling

A plugin folder can be copied to another computer as it is. A project
remembers a plugin of your own by the folder's path, so on another computer
the folder has to be in the same place (re-linking moved files is planned).

Plugins that ship with VizX are in `assets/visualizers` in the source tree
and in a `visualizers` folder next to the executable in an installed VizX.
Projects refer to them as `bundled-visualizers:/<folder name>`, which works
on every computer. To add a plugin to VizX itself, put its folder there.

## Current limits

- **One way of blending.** A visualizer is laid over the layers below
  according to its opacity. Other blend modes (add, multiply, ...) do not
  exist yet.
- **No textures or images** of your own in a plugin yet.
- **Preview size.** The preview renders a full-frame clip 640 pixels wide.
  Use `vizx.resolution` for anything measured in pixels, so the visualizer
  looks the same when export renders it larger.
- **The history is counted in frames**, so `spectrum_history` reaches back
  256 frames: about 8.5 seconds at 30 fps and half of that at 60 fps.
