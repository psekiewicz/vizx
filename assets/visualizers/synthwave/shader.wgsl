// Synthwave: a flat floor seen from a little above it, with a sun and
// mountains on the horizon. A point of the floor at screen height y lies
// at distance 1 / (y - horizon) from the viewer, so a grid with even
// spacing on the floor crowds together towards the horizon by itself.

fn visualize(uv: vec2f) -> vec4f {
    let horizon = p_horizon();
    let pixel = 1.0 / vizx.resolution.y;
    let aspect = aspect_ratio();
    let x = (uv.x - 0.5) * aspect;

    // --- The floor, below the horizon.
    if uv.y > horizon {
        let below = uv.y - horizon;
        let distance = 0.4 / below;
        let jump = vizx.beat_count - exp(-vizx.since_beat * 6.0);
        let travel = vizx.time * p_speed() + jump * p_kick();
        // Grid coordinates on the floor, and how much they change from one
        // pixel to the next, so every line is about 1.5 pixels wide on
        // screen however far away it is.
        let lines = p_grid() / aspect;
        let u = x * distance * lines * 0.5;
        let v = distance * 2.0 + travel;
        let du = pixel * distance * lines * 0.5;
        let dv = 2.0 * distance / below * pixel;
        let across = abs(fract(u + 0.5) - 0.5) * 2.0;
        let along = abs(fract(v) - 0.5) * 2.0;
        // Far away lines would be thinner than a pixel: they fade instead.
        let grid_across = (1.0 - smoothstep(0.0, 3.0 * du, across)) * min(1.0, 0.25 / du);
        let grid_along = (1.0 - smoothstep(0.0, 3.0 * dv, along)) * min(1.0, 0.25 / dv);
        let grid = max(grid_across, grid_along);
        // Lines fade into a haze near the horizon and glow with the bass.
        let haze = smoothstep(0.0, 0.25, below);
        let glow = (0.55 + 0.8 * vizx.bass) * haze;
        let ground = p_color_ground();
        let rgb = mix(ground, p_color_grid() * glow + ground, grid * haze);
        // A glowing band right at the horizon.
        let band = exp(-below * 60.0) * (0.5 + vizx.volume);
        return vec4f(rgb + p_color_grid() * band, 1.0);
    }

    // --- Mountains along the horizon, shaped by the spectrum (lows in the
    // middle, mirrored), with a bright edge.
    let frequency = clamp(abs(uv.x - 0.5) * 2.0, 0.0, 1.0);
    let ridge = p_mountains() * (spectrum(frequency) * 0.8 + 0.15 * (1.0 - frequency))
        * (0.6 + 0.4 * sin(uv.x * 23.0));
    let mountain_top = horizon - ridge;
    if uv.y > mountain_top {
        let edge = 1.0 - smoothstep(0.0, pixel * 3.0, uv.y - mountain_top);
        let rgb = mix(p_color_ground() * 1.6, p_color_grid(), edge);
        return vec4f(rgb, 1.0);
    }

    // --- The sun, above the horizon, cut by stripes that drift down.
    let size = p_sun() * (1.0 + 0.12 * vizx.bass + 0.08 * vizx.beat);
    let middle = vec2f(0.0, horizon - p_sun() * 0.55);
    let from_middle = vec2f(x, uv.y) - middle;
    let distance = length(from_middle);
    let inside = 1.0 - smoothstep(size - pixel, size + pixel, distance);
    // Height within the sun, 0 at the top and 1 at the bottom.
    let height = clamp((from_middle.y / size + 1.0) * 0.5, 0.0, 1.0);
    let stripes = fract(height * 9.0 - vizx.time * 0.25);
    let gap = smoothstep(0.45, 1.0, height) * 0.55;
    let cut = smoothstep(gap - 0.02, gap + 0.02, stripes);
    let sun = inside * select(1.0, cut, height > 0.45);
    let sun_color = mix(p_color_sun_top(), p_color_sun_bottom(), height);
    // A soft glow around it, as light on black.
    let halo = exp(-max(distance - size, 0.0) * 12.0) * (0.35 + 0.5 * vizx.bass) * (1.0 - inside);
    let light = sun_color * sun + p_color_sun_bottom() * halo;
    return from_light(light);
}
