// Core: a disc in the middle that grows with the bass, with the spectrum
// wrapped around it as rays.

fn visualize(uv: vec2f) -> vec4f {
    // Coordinates with (0, 0) in the middle and the top and bottom edges at
    // -1 and 1, the same scale in both directions so circles stay round.
    var point = (uv - 0.5) * 2.0;
    point.x *= aspect_ratio();
    let radius = length(point);
    let pixel = 2.0 / vizx.resolution.y;

    let core_radius = p_size() * (1.0 + vizx.bass * p_pulse());

    // The core: brightest in the middle, with a soft edge.
    let core = 1.0 - smoothstep(core_radius - pixel * 2.0, core_radius, radius);
    let core_shade = 1.0 - 0.45 * radius / max(core_radius, 0.001);

    // Angle around the middle: 0 at the top, 1 at the bottom, the same on
    // the left and on the right. The spectrum is laid along it, so the ring
    // is symmetric and closes without a seam.
    let turn = atan2(point.x, -point.y) / PI + vizx.time * p_spin();
    let around = abs(fract(turn * 0.5 + 0.5) * 2.0 - 1.0);

    // Cut the ring into rays and give each one loudness value.
    let count = floor(p_rays());
    let ray = floor(around * count);
    let within = fract(around * count);
    let level = spectrum((ray + 0.5) / count * 0.85);

    let ray_start = core_radius + 0.03;
    let ray_end = ray_start + level * p_reach();
    let along = smoothstep(ray_start - pixel, ray_start + pixel, radius)
        * (1.0 - smoothstep(ray_end - pixel, ray_end + pixel, radius));
    let across = smoothstep(0.12, 0.22, within) * (1.0 - smoothstep(0.78, 0.88, within));
    let rays = along * across * step(0.004, level);

    // A halo around the core that brightens with the overall loudness.
    let halo = exp(-max(radius - core_radius, 0.0) * 5.0) * (0.15 + 0.6 * vizx.volume);

    var color = p_color_core() * core * core_shade;
    color += p_color_rays() * (rays + halo * (1.0 - core));

    // Echo: last frame's picture, slightly enlarged and fainter, which
    // makes the rays fly outwards.
    let previous = previous_frame((uv - 0.5) * 0.985 + 0.5);
    let echo = previous.rgb * previous.a * p_echo();

    // Everything above was drawn as light on black; from_light makes the
    // black transparent.
    return from_light(max(color, echo));
}