// Tunnel: the picture is the inside of a tube seen from its axis. A pixel's
// distance from the middle says how far down the tube it looks (near the
// edge is close, the middle is far away), its angle says where around the
// wall. Rings are drawn at steady distances along the tube and stripes
// along it; moving the distances makes the rings rush towards the viewer.

fn visualize(uv: vec2f) -> vec4f {
    var point = (uv - 0.5) * 2.0;
    point.x *= aspect_ratio();
    let radius = max(length(point), 0.0005);
    let pixel = 2.0 / vizx.resolution.y;

    // How far along the tube this pixel looks.
    let depth = 0.35 / radius;
    // Distance travelled: steady speed, plus a lurch forward on every beat
    // that eases out (the beat count jumps, the exponential smooths it).
    let lurch = vizx.beat_count - exp(-vizx.since_beat * 7.0);
    let travel = vizx.time * p_speed() + lurch * p_kick();
    let along = depth + travel;

    // Rings: thin bright lines at whole steps along the tube, about two
    // pixels wide on screen. Far away they crowd together; there they fade
    // out instead of flickering.
    let band = fract(along * p_rings());
    let from_ring = min(band, 1.0 - band);
    let step_per_pixel = p_rings() * 0.35 / (radius * radius) * pixel;
    let ring_width = max(step_per_pixel * 2.0, 0.015);
    let ring = (1.0 - smoothstep(0.0, ring_width, from_ring)) * clamp(0.25 / ring_width, 0.0, 1.0);

    // Around the wall: 0 to 1, symmetric left and right, twisting with
    // depth. The spectrum lights the stripes: lows at the top.
    let turn = atan2(point.x, -point.y) / TAU + 0.5 + depth * p_twist() * 0.15;
    let around = abs(fract(turn) * 2.0 - 1.0);
    let count = floor(p_stripes());
    let within = fract(turn * count);
    let stripe = 1.0 - smoothstep(0.0, 0.12 + pixel * depth * 2.0, abs(within - 0.5) * 2.0 - 0.6);
    let level = spectrum(around * 0.9);

    // Far away fades into the dark; near the edge is brightest.
    let near = smoothstep(0.0, 0.7, radius);
    let color = mix(p_color_far(), p_color_near(), near);
    var light = color * (ring * (0.6 + 0.9 * vizx.bass) + stripe * level * 1.2) * near;

    // The far end of the tunnel glows with the bass and flashes on beats.
    let glow = exp(-radius * 9.0) * (0.25 + vizx.bass + 0.6 * vizx.beat);
    light += p_color_far() * glow;
    return from_light(light);
}
