// Waterfall: a spectrogram. Left to right is frequency, and the other
// direction is time: the newest spectrum enters at one edge and older ones
// move away from it.

// Turns a loudness from 0 to 1 into a color: black, the quiet color, the
// loud color and finally almost white for the loudest sounds.
fn heat(level: f32) -> vec3f {
    var color = p_color_low() * smoothstep(0.0, 0.45, level);
    color = mix(color, p_color_high(), smoothstep(0.35, 0.85, level));
    return mix(color, vec3f(1.0), smoothstep(0.85, 1.0, level) * 0.8);
}

fn visualize(uv: vec2f) -> vec4f {
    // age 0 is "now". By default the newest row is at the top and the
    // picture flows down like a waterfall.
    var age = uv.y;
    if p_rising() > 0.5 {
        age = 1.0 - uv.y;
    }

    let level = clamp(spectrum_history(uv.x, age * p_span()) * p_gain(), 0.0, 1.0);
    return vec4f(heat(level), 1.0);
}