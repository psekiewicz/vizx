// Draws one layer: a textured rectangle that can be placed anywhere in the
// frame, scaled, turned and faded, with optional effects (blur, color
// split, glow, tint, vignette, grain, glitch).

struct Layer {
    // Middle of the layer, in pixels from the frame's top left corner.
    center: vec2f,
    // Half of the layer's width and height, in pixels.
    half_size: vec2f,
    // Size of the frame in pixels.
    frame: vec2f,
    // Turn in radians, clockwise.
    rotation: f32,
    opacity: f32,
    // Blur radius and color split, in frame pixels; glow strength; grain
    // strength.
    blur_split_glow_grain: vec4f,
    // Glitch strength, vignette strength, time in seconds, glow radius in
    // frame pixels.
    glitch_vignette_time_halo: vec4f,
    // Tint color and how much of it.
    tint: vec4f,
    // Fill color (the picture turns into it) and how much of it.
    fill: vec4f,
}

@group(0) @binding(0) var<uniform> layer: Layer;
@group(0) @binding(1) var layer_sampler: sampler;
@group(0) @binding(2) var layer_texture: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
}

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> VertexOutput {
    // The four corners of the rectangle, from -1 to 1.
    let corner = vec2f(f32(index & 1u), f32((index >> 1u) & 1u)) * 2.0 - 1.0;

    // Turn the corner around the layer's middle. Pixel coordinates grow
    // downwards, where this formula turns clockwise.
    let local = corner * layer.half_size;
    let turned = vec2f(
        local.x * cos(layer.rotation) - local.y * sin(layer.rotation),
        local.x * sin(layer.rotation) + local.y * cos(layer.rotation),
    );
    let pixel = layer.center + turned;

    var output: VertexOutput;
    // From pixels to the graphics card's coordinates (-1 to 1, y upwards).
    output.position = vec4f(
        pixel.x / layer.frame.x * 2.0 - 1.0,
        1.0 - pixel.y / layer.frame.y * 2.0,
        0.0,
        1.0,
    );
    output.uv = corner * 0.5 + 0.5;
    return output;
}

// A well-mixed whole number from another (the PCG hash).
fn mix_bits(value: u32) -> u32 {
    let state = value * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

// A random-looking number from 0 to 1 that is always the same for the same
// two inputs.
fn noise(a: f32, b: f32) -> f32 {
    let bits = mix_bits(u32(i32(a) + 100000) ^ mix_bits(u32(i32(b) + 100000)));
    return f32(bits) / 4294967295.0;
}

// The layer's picture at `uv`; transparent outside of it, so moved and
// split parts do not smear its edge.
fn tap(uv: vec2f) -> vec4f {
    let color = textureSampleLevel(layer_texture, layer_sampler, uv, 0.0);
    let inside = all(uv >= vec2f(0.0)) && all(uv <= vec2f(1.0));
    return select(vec4f(0.0), color, inside);
}

const BLUR_TAPS: i32 = 24;

// The picture averaged over a disc of `radius` (in uv units per axis).
fn blurred(uv: vec2f, radius: vec2f) -> vec4f {
    if radius.x <= 0.0 && radius.y <= 0.0 {
        return tap(uv);
    }
    var sum = tap(uv);
    // Points spread evenly over the disc along a spiral.
    for (var index = 1; index < BLUR_TAPS; index++) {
        let distance = sqrt(f32(index) / f32(BLUR_TAPS - 1));
        let angle = f32(index) * 2.39996;
        sum += tap(uv + vec2f(cos(angle), sin(angle)) * distance * radius);
    }
    return sum / f32(BLUR_TAPS);
}

@fragment
fn fragment(input: VertexOutput) -> @location(0) vec4f {
    let blur = layer.blur_split_glow_grain.x;
    let split = layer.blur_split_glow_grain.y;
    let glow = layer.blur_split_glow_grain.z;
    let grain = layer.blur_split_glow_grain.w;
    let glitch = layer.glitch_vignette_time_halo.x;
    let vignette = layer.glitch_vignette_time_halo.y;
    let time = layer.glitch_vignette_time_halo.z;
    let halo = layer.glitch_vignette_time_halo.w;
    // From frame pixels to the layer's uv units.
    let per_pixel = 1.0 / (2.0 * max(layer.half_size, vec2f(1.0)));

    var uv = input.uv;
    var split_shift = split;
    if glitch > 0.0 {
        // Horizontal bands jump sideways, a dozen times a second.
        let moment = floor(time * 12.0);
        let band = floor(uv.y * 18.0 + noise(moment, 7.0) * 9.0);
        if noise(band, moment) < glitch * 0.6 {
            uv.x += (noise(band + 31.0, moment) - 0.5) * glitch * 0.3;
            // Hit bands also split their colors.
            split_shift += glitch * 12.0;
        }
    }

    let blur_radius = blur * per_pixel;
    var color: vec4f;
    if split_shift > 0.0 {
        let shift = vec2f(split_shift * per_pixel.x, 0.0);
        let red = blurred(uv + shift, blur_radius);
        let green = blurred(uv, blur_radius);
        let blue = blurred(uv - shift, blur_radius);
        color = vec4f(red.r, green.g, blue.b, max(max(red.a, green.a), blue.a));
    } else {
        color = blurred(uv, blur_radius);
    }

    if glow > 0.0 {
        // Light spread around the bright parts, added on top.
        let light = blurred(uv, halo * per_pixel);
        color = vec4f(color.rgb + light.rgb * glow, min(color.a + light.a * glow, 1.0));
    }

    if layer.tint.a > 0.0 {
        // The tint color at the brightness of the picture.
        let brightness = dot(color.rgb, vec3f(0.299, 0.587, 0.114));
        color = vec4f(mix(color.rgb, layer.tint.rgb * brightness, layer.tint.a), color.a);
    }

    if vignette > 0.0 {
        let from_middle = length(input.uv - 0.5) * 1.4142;
        color = vec4f(color.rgb * (1.0 - vignette * smoothstep(0.35, 1.0, from_middle)), color.a);
    }

    if grain > 0.0 {
        let pixel = floor(input.uv * 2.0 * layer.half_size);
        let speck = noise(pixel.x + pixel.y * 4099.0, floor(time * 24.0)) - 0.5;
        color = vec4f(max(color.rgb + speck * grain * 0.4 * color.a, vec3f(0.0)), color.a);
    }

    if layer.fill.a > 0.0 {
        // Towards a solid color, covering what is transparent too.
        let amount = layer.fill.a;
        color = vec4f(mix(color.rgb, layer.fill.rgb, amount), mix(color.a, 1.0, amount));
    }

    // Colors are premultiplied by their opacity, so fading the layer is one
    // multiplication of all four values.
    return color * layer.opacity;
}
