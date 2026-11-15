// Everything VizX provides to a visualizer shader. This file is appended to
// the plugin's shader.wgsl. See docs/visualizer-plugins.md.

const PI: f32 = 3.14159265358979;
const TAU: f32 = 6.28318530717959;

struct Vizx {
    // Size of the picture in pixels.
    resolution: vec2f,
    // Seconds since the start of the clip.
    time: f32,
    // Number of the frame being drawn, starting at 0.
    frame: f32,
    // Overall loudness, 0 to 1.
    volume: f32,
    // Loudness of the low, middle and high frequencies, 0 to 1.
    bass: f32,
    mid: f32,
    treble: f32,
    // 1 on a beat (times its strength), fading to 0 within a quarter second.
    beat: f32,
    // Seconds since the last beat; very large before the first one.
    since_beat: f32,
    // How many beats there have been so far (count them to alternate
    // colors, sides, ...).
    beat_count: f32,
    // Beats per minute, or 0 if not known.
    tempo: f32,
    // Parameter values; read them with the generated p_<id>() functions.
    params: array<vec4f, 8>,
}

@group(0) @binding(0) var<uniform> vizx: Vizx;
@group(0) @binding(1) var vizx_sampler: sampler;
@group(0) @binding(2) var vizx_audio: texture_2d<f32>;
@group(0) @binding(3) var vizx_history: texture_2d<f32>;
@group(0) @binding(4) var vizx_previous: texture_2d<f32>;

// Loudness (0 to 1) of the frequency at x, where x = 0 is the lowest
// frequency (30 Hz) and x = 1 the highest (16 kHz).
fn spectrum(x: f32) -> f32 {
    return textureSampleLevel(vizx_audio, vizx_sampler, vec2f(x, 0.25), 0.0).r;
}

// The sound wave (-1 to 1) at x, where x = 0 is the oldest sample and
// x = 1 the newest.
fn waveform(x: f32) -> f32 {
    return textureSampleLevel(vizx_audio, vizx_sampler, vec2f(x, 0.75), 0.0).r * 2.0 - 1.0;
}

// The spectrum as it was some frames ago: age = 0 is the current frame,
// age = 1 is the oldest remembered frame (256 frames back).
fn spectrum_history(x: f32, age: f32) -> f32 {
    return textureSampleLevel(vizx_history, vizx_sampler, vec2f(x, age), 0.0).r;
}

// The color and opacity this visualizer returned at uv in the previous
// frame. Fully transparent for the first frame. Use it for trails, echoes
// and other feedback effects.
fn previous_frame(uv: vec2f) -> vec4f {
    // Pictures are stored with the color already multiplied by the opacity;
    // undo that, so this returns what visualize() returned.
    let stored = textureSampleLevel(vizx_previous, vizx_sampler, uv, 0.0);
    if stored.a <= 0.0 {
        return vec4f(0.0);
    }
    return vec4f(stored.rgb / stored.a, stored.a);
}

// Turns "light on a black background" into a color with opacity: black
// becomes fully transparent and bright light fully opaque. Draw glows and
// other additive effects as if the background were black, then return
// from_light(color) to get a visualizer that can lie on top of anything.
fn from_light(light: vec3f) -> vec4f {
    let opacity = clamp(max(light.r, max(light.g, light.b)), 0.0, 1.0);
    if opacity <= 0.0 {
        return vec4f(0.0);
    }
    return vec4f(light / opacity, opacity);
}

// Width of the picture divided by its height.
fn aspect_ratio() -> f32 {
    return vizx.resolution.x / max(vizx.resolution.y, 1.0);
}

struct VizxVertexOutput {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
}

// One triangle that covers the whole picture.
@vertex
fn vizx_vertex(@builtin(vertex_index) index: u32) -> VizxVertexOutput {
    let corner = vec2f(f32((index << 1u) & 2u), f32(index & 2u));
    var output: VizxVertexOutput;
    output.position = vec4f(corner * 2.0 - 1.0, 0.0, 1.0);
    // uv (0, 0) is the top left corner, (1, 1) the bottom right.
    output.uv = vec2f(corner.x, 1.0 - corner.y);
    return output;
}

@fragment
fn vizx_fragment(input: VizxVertexOutput) -> @location(0) vec4f {
    let color = visualize(input.uv);
    // The fourth value is the opacity: 0 lets the layers below show
    // through, 1 hides them. The picture is stored "premultiplied" (color
    // times opacity), which is what compositing needs.
    let opacity = clamp(color.a, 0.0, 1.0);
    return vec4f(clamp(color.rgb, vec3f(0.0), vec3f(1.0)) * opacity, opacity);
}
