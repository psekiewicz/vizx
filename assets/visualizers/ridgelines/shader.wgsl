// Ridgelines: row after row of the spectrum's past, each row a line whose
// height is the loudness, lows in the middle, flat at the sides. The front
// row is now; rows further back (higher up) are older. Every row is filled
// below its line, so it hides the rows behind it, as on the cover of
// Unknown Pleasures.

// Height of row `row` (0 is the back) at `x`, as a share of the picture.
fn row_height(row: f32, rows: f32, x: f32) -> f32 {
    let age = (1.0 - row / max(rows - 1.0, 1.0)) * p_history();
    // Lows in the middle, highs towards the sides, mirrored.
    let frequency = clamp(abs(x - 0.5) * 2.2, 0.0, 1.0);
    // Flat edges: only the middle of the line moves.
    let envelope = smoothstep(0.08, 0.35, x) * (1.0 - smoothstep(0.65, 0.92, x));
    // A little ripple keeps quiet rows from being perfectly straight.
    let ripple = 0.04 * sin(x * 60.0 + row * 1.7);
    return (spectrum_history(frequency, age) + ripple * envelope) * envelope * p_height();
}

fn visualize(uv: vec2f) -> vec4f {
    let rows = floor(p_lines());
    let pixel = 1.0 / vizx.resolution.y;
    let thickness = pixel * p_thickness();
    // The rows sit between 25 % and 92 % of the height.
    let spacing = 0.67 / max(rows - 1.0, 1.0);

    // From the front row backwards: the first row whose line is above this
    // pixel covers it.
    for (var index = i32(rows) - 1; index >= 0; index--) {
        let row = f32(index);
        let base = 0.25 + row * spacing;
        let top = base - row_height(row, rows, uv.x);
        if uv.y >= top - thickness {
            let depth = row / max(rows - 1.0, 1.0);
            let color = mix(p_color_back(), p_color_front(), depth);
            let line = 1.0 - smoothstep(thickness * 0.5, thickness * 1.5, abs(uv.y - top));
            // Below the line: the fill, which hides the rows behind.
            let fill = p_fill() * step(top, uv.y);
            let alpha = max(line, fill);
            let rgb = mix(p_color_fill(), color, line);
            return vec4f(rgb, alpha);
        }
    }
    return vec4f(0.0);
}
