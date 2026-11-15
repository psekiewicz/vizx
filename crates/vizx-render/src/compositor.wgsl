// Draws one layer: a textured rectangle that can be placed anywhere in the
// frame, scaled, turned and faded.

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

@fragment
fn fragment(input: VertexOutput) -> @location(0) vec4f {
    // Colors are premultiplied by their opacity, so fading the layer is one
    // multiplication of all four values.
    return textureSample(layer_texture, layer_sampler, input.uv) * layer.opacity;
}