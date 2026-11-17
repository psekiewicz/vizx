// Kaleidoscope: the plane around the middle is folded into a number of
// mirrored wedges, so whatever is drawn in one wedge repeats all around.
// In the wedge: petals whose length is the spectrum, rings that ripple out
// with the bass, and a slow pattern for the space between.

fn hue(value: f32) -> vec3f {
    let shifted = fract(value) * 6.0;
    return clamp(
        vec3f(abs(shifted - 3.0) - 1.0, 2.0 - abs(shifted - 2.0), 2.0 - abs(shifted - 4.0)),
        vec3f(0.0),
        vec3f(1.0),
    );
}

fn visualize(uv: vec2f) -> vec4f {
    var point = (uv - 0.5) * 2.0;
    point.x *= aspect_ratio();
    point /= p_zoom();
    let radius = length(point);
    let pixel = 2.0 / vizx.resolution.y / p_zoom();

    // Fold the angle into one mirrored wedge.
    let wedges = floor(p_mirrors()) * 2.0;
    let wedge = TAU / wedges;
    var angle = atan2(point.y, point.x) + vizx.time * p_spin();
    angle = abs(fract(angle / wedge + 0.5) - 0.5) * wedge; // 0 .. wedge / 2
    let across = angle / (wedge * 0.5); // 0 on the mirror line, 1 at the edge

    // Petals: along the mirror line, as long as the loudness of a note.
    let level = spectrum(clamp(across * 0.7 + 0.05, 0.0, 1.0));
    let petal_length = 0.3 + p_petals() * level * 1.4;
    let petal_width = 0.35 * (1.0 - radius / max(petal_length, 0.01));
    let petal = (1.0 - smoothstep(petal_width - 0.05, petal_width + 0.05, across))
        * (1.0 - smoothstep(petal_length - pixel * 2.0, petal_length, radius));

    // Rings rippling out from the middle, faster and brighter with the bass.
    let wave = sin(radius * 28.0 - vizx.time * 4.0 - (vizx.beat_count - exp(-vizx.since_beat * 6.0)) * 3.0);
    let ripple = smoothstep(0.75, 1.0, wave) * p_ripples() * (0.3 + vizx.bass) * exp(-radius * 1.2);

    // A slow mesh of lines in between, bending with the mid frequencies.
    let mesh_point = vec2f(radius * cos(angle), radius * sin(angle)) * 6.0;
    let mesh = sin(mesh_point.x + vizx.time * 0.7) * sin(mesh_point.y * 2.0 - vizx.time * 0.5 + vizx.mid * 3.0);
    let lines = (1.0 - smoothstep(0.0, 0.12, abs(mesh))) * 0.6 * (0.4 + vizx.volume) * exp(-radius * 0.6);

    // Colors run around the wheel with the distance and with time.
    let tone = radius * 0.6 + vizx.time * 0.05 * p_colors() + across * 0.15;
    let color = mix(vec3f(1.0), hue(tone), p_saturation());
    let flash = 1.0 + vizx.beat * 0.6;
    let light = color * (petal * (0.9 + level) + ripple * 1.5 + lines) * flash * 1.3;
    return from_light(light);
}
