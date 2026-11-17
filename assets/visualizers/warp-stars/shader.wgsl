// Warp Stars: space around the middle is cut into cells by angle and by the
// logarithm of the distance (so cells grow as they move outwards, like
// things coming closer). Some cells hold a star; moving the cells outwards
// moves every star along its own ray. Three layers of different size give
// depth. Each pixel looks only at its own cell, so there are no loops over
// stars.

fn star_hash(cell: vec2f) -> f32 {
    return fract(sin(dot(cell, vec2f(127.1, 311.7))) * 43758.5453);
}

// Light of one layer of stars at this pixel.
fn star_layer(angle: f32, log_radius: f32, columns: f32, travel: f32, seed: f32) -> f32 {
    // Square cells: one cell is as wide (in angle) as it is long (in
    // log-distance).
    let across = (angle / TAU + 0.5) * columns;
    let outwards = log_radius * columns / TAU - travel;
    let cell = floor(vec2f(across, outwards));
    // Stars appear only in some cells.
    if star_hash(cell + seed) > p_density() {
        return 0.0;
    }
    let place = vec2f(star_hash(cell + seed + 3.1), star_hash(cell + seed + 7.7)) * 0.6 + 0.2;
    var offset = fract(vec2f(across, outwards)) - place;
    // Loud music and beats stretch the star along its ray.
    let stretch = 1.0 + p_streak() * (2.0 + 6.0 * vizx.volume + 10.0 * vizx.beat);
    offset.y /= stretch;
    let size = 0.07 * p_size();
    let brightness = 0.4 + 0.6 * star_hash(cell + seed + 1.9);
    return brightness * (1.0 - smoothstep(size * 0.3, size, length(offset)));
}

fn visualize(uv: vec2f) -> vec4f {
    var point = (uv - 0.5) * 2.0;
    point.x *= aspect_ratio();
    let radius = max(length(point), 0.0005);
    let angle = atan2(point.y, point.x);
    let log_radius = log(radius);

    let jump = vizx.beat_count - exp(-vizx.since_beat * 5.0);
    let travel = vizx.time * p_speed() + jump * p_kick();

    var light = 0.0;
    light += star_layer(angle, log_radius, 48.0, travel * 1.0, 0.0);
    light += star_layer(angle, log_radius, 80.0, travel * 0.7, 17.0) * 0.7;
    light += star_layer(angle, log_radius, 130.0, travel * 0.45, 41.0) * 0.45;

    // Stars are born faint in the middle and brighten as they come closer.
    light *= smoothstep(0.03, 0.5, radius);
    let tint = mix(p_color(), p_color_beat(), clamp(vizx.beat, 0.0, 1.0));
    return from_light(tint * light);
}
