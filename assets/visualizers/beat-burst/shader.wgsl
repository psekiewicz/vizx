// Beat Burst: only the sparks of the latest beat are drawn; older ones live
// on as trails in last frame's picture (`previous_frame`), which is laid
// back a little enlarged and fainter. Each spark's direction and speed come
// from the beat's number, so every beat looks different but the same each
// time the song is played.

fn burst_hash(value: f32) -> f32 {
    return fract(sin(value * 91.345 + 47.853) * 43758.5453);
}

fn visualize(uv: vec2f) -> vec4f {
    var point = (uv - 0.5) * 2.0;
    point.x *= aspect_ratio();
    let pixel = 2.0 / vizx.resolution.y;

    // Time since the beat, and how far a spark has flown by now: fast at
    // first, slowing down (1 - e^-t).
    let age = vizx.since_beat;
    let flown = (1.0 - exp(-age * 3.0)) * p_reach() * (0.6 + 0.6 * vizx.volume);
    let fade = exp(-age * 2.2);

    var light = vec3f(0.0);
    if fade > 0.01 {
        let count = i32(p_sparks());
        for (var index = 0; index < count; index++) {
            let seed = vizx.beat_count * 13.17 + f32(index) * 1.618;
            let direction = burst_hash(seed) * TAU;
            let speed = 0.4 + 0.6 * burst_hash(seed + 5.3);
            let position = vec2f(cos(direction), sin(direction)) * flown * speed;
            let size = pixel * (2.0 + 4.0 * burst_hash(seed + 9.1)) * p_size();
            let distance = length(point - position);
            light += p_color() * (1.0 - smoothstep(size * 0.3, size, distance)) * fade;
        }
        // The shockwave: a ring that expands and thins out.
        let ring_radius = (1.0 - exp(-age * 4.0)) * p_reach() * 1.2;
        let ring_width = pixel * 2.0 + 0.04 * fade;
        let ring = (1.0 - smoothstep(0.0, ring_width, abs(length(point) - ring_radius)))
            * fade * p_ring();
        light += p_color_ring() * ring;
    }

    // Trails: last frame, slightly enlarged around the middle and fainter.
    let previous = previous_frame((uv - 0.5) * 0.99 + 0.5);
    let trail = previous.rgb * previous.a * p_trail();
    return from_light(max(light, trail));
}
