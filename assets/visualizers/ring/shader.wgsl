// Ring: a glowing ring bent by the spectrum, as in NCS videos. Inside it,
// layers of dots wave against each other like a mesh seen in 3D. The ring
// grows with the bass; the middle and the background stay transparent, so
// a background image and a logo show through.

// The loudness that bends the ring at `angle` (radians), smoothed over a
// few neighbouring frequencies. The spectrum runs from the top of the ring
// (low notes) down both sides to the bottom (high notes), so the ring is
// symmetric and has no seam.
fn bend_at(angle: f32) -> f32 {
    let around = abs(fract(angle / TAU + 0.5) * 2.0 - 1.0);
    let x = around * 0.7;
    return (spectrum(x - 0.012) + 2.0 * spectrum(x) + spectrum(x + 0.012)) * 0.25;
}

// Distance from the middle of the ring's band at `angle`.
fn ring_radius(angle: f32) -> f32 {
    let grow = 1.0 + p_bass() * vizx.bass * 0.14;
    return p_radius() * grow + bend_at(angle) * p_bend() * 0.12;
}

// Distance from the middle of dot layer `layer` (0 is next to the ring,
// the last is deepest inside) at `angle`. Each layer is a wave of its own,
// moving at its own speed, so the layers cross like a folded mesh; the
// sound makes them fold further in.
fn layer_radius(angle: f32, layer: f32, count: f32) -> f32 {
    let depth = (layer + 1.0) / count;
    // Two slow waves going round in opposite directions; where they meet,
    // the mesh folds deep into the ring, elsewhere it hugs the band.
    let one = 0.5 + 0.5 * sin(angle * 2.0 + vizx.time * 0.35 + layer * 0.45);
    let two = 0.5 + 0.5 * sin(angle * 3.0 - vizx.time * 0.5 + layer * 0.3 + 1.7);
    let sheet = pow(one * two, 2.0);
    let push = 0.3 + bend_at(angle) + 0.6 * vizx.bass;
    let fold = depth * (0.04 + 0.55 * sheet * push);
    return ring_radius(angle) - p_thickness() * 0.5 - fold * p_radius();
}

fn visualize(uv: vec2f) -> vec4f {
    // (0, 0) in the middle, the top and bottom edges at -1 and 1, the same
    // scale in both directions so the ring stays round.
    var point = (uv - 0.5) * 2.0;
    point.x *= aspect_ratio();
    let distance = length(point);
    let pixel = 2.0 / vizx.resolution.y;
    let angle = atan2(point.x, -point.y) + vizx.time * p_spin();

    // The band: bright and solid in the middle, softly edged.
    let from_band = abs(distance - ring_radius(angle));
    let half = p_thickness() * 0.5;
    let band = 1.0 - smoothstep(half - pixel, half + pixel * 1.5, from_band);
    // Light around it, stronger with the volume.
    let reach = 0.015 + 0.05 * p_glow();
    let glow = exp(-max(from_band - half, 0.0) / reach) * p_glow() * (0.55 + 0.45 * vizx.volume);

    // The dots: for every layer, the nearest dot on its wave. Dots sit at
    // fixed angles, and every layer is a little fainter than the one
    // before it, as if further away.
    let count = floor(p_layers());
    let dots_around = 150.0;
    let step = TAU / dots_around;
    let dot_size = max(pixel * 1.8, 0.006);
    var dots = 0.0;
    for (var index = 0; index < i32(count); index++) {
        let layer = f32(index);
        // Every other layer is shifted by half a dot, which makes a mesh.
        let shift = step * 0.5 * (layer % 2.0);
        let nearest = round((angle - shift) / step);
        var closest = 1e6;
        for (var neighbour = -1; neighbour <= 1; neighbour++) {
            let dot_angle = (nearest + f32(neighbour)) * step + shift;
            let dot_distance = layer_radius(dot_angle, layer, count);
            // Back from the turned angle to the picture's angle.
            let picture_angle = dot_angle - vizx.time * p_spin();
            let center = vec2f(sin(picture_angle), -cos(picture_angle)) * dot_distance;
            closest = min(closest, length(point - center));
        }
        let fade = 1.0 - 0.75 * layer / max(count, 1.0);
        dots = max(dots, (1.0 - smoothstep(dot_size * 0.5, dot_size, closest)) * fade);
    }

    let color = p_color();
    // The middle of the band is nearly white, like an overexposed light.
    var light = color * (band + glow) + vec3f(0.35) * band * band;
    light += color * dots * (0.55 + 0.45 * vizx.mid);
    // Everything was drawn as light on black; from_light makes black
    // transparent, so the background and the middle stay empty.
    return from_light(light);
}
