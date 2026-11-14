// Oscilloscope: draws the waveform as a line across the picture.

// Height of the line at x, as a uv coordinate (0.5 is the middle).
fn line_height(x: f32) -> f32 {
    return 0.5 - waveform(x) * p_amplitude() * 0.45;
}

fn visualize(uv: vec2f) -> vec4f {
    let step_x = 1.0 / vizx.resolution.x;
    let here = line_height(uv.x);

    // Vertical distance to the line, in pixels. Where the line is steep
    // that overestimates the real distance, so it is corrected with the
    // line's slope; otherwise steep parts would look thinner.
    let slope = (line_height(uv.x + step_x) - line_height(uv.x - step_x)) * 0.5
        * vizx.resolution.y;
    let distance = abs(uv.y - here) * vizx.resolution.y / sqrt(1.0 + slope * slope);

    let half_thickness = p_thickness() * 0.5;
    let line = 1.0 - smoothstep(half_thickness, half_thickness + 1.5, distance);
    let glow = p_glow() * 0.6 * exp(-distance / (p_thickness() * 5.0 + 1.0));
    let fresh = p_color() * (line + glow);

    // The trail is last frame's picture, a little darker each time.
    let trail = previous_frame(uv).rgb * p_trail();

    return vec4f(max(fresh, trail), 1.0);
}