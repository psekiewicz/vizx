// Spectrum Bars: the picture is cut into vertical bands, and each band is
// filled from the bottom up to the loudness of its frequency.

fn visualize(uv: vec2f) -> vec4f {
    let count = floor(p_bars());

    // With "Mirror" the low notes are in the middle and the picture is
    // symmetric; otherwise they are on the left.
    var x = uv.x;
    if p_mirror() > 0.5 {
        x = abs(uv.x * 2.0 - 1.0);
    }

    // Which bar this pixel belongs to, and where inside that bar it is.
    let position = x * count;
    let bar = floor(position);
    let within = fract(position);

    // One loudness value per bar, read at the middle of its band.
    let level = clamp(spectrum((bar + 0.5) / count) * p_gain(), 0.0, 1.0);

    // Soft edges about one pixel wide, so bars do not look jagged.
    let pixel_x = count / vizx.resolution.x;
    let pixel_y = 1.0 / vizx.resolution.y;
    let half_gap = p_gap() * 0.5;
    let inside_bar = smoothstep(half_gap - pixel_x, half_gap + pixel_x, within)
        * (1.0 - smoothstep(1.0 - half_gap - pixel_x, 1.0 - half_gap + pixel_x, within));

    let height = 1.0 - uv.y; // 0 at the bottom, 1 at the top
    let below_top = 1.0 - smoothstep(level - pixel_y, level + pixel_y, height);

    let color = mix(p_color_bottom(), p_color_top(), height);
    // The fourth value is the opacity: 1 inside a bar, 0 (transparent)
    // everywhere else, so whatever is on the layers below stays visible.
    return vec4f(color, inside_bar * below_top);
}