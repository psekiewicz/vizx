//! Measures how long the compositor takes per frame: plain layers, layers
//! with effects, and an effects layer, at the preview's size and at 1080p.
//!
//! ```text
//! cargo run --release -p vizx-render --example bench_compositor
//! ```

use std::time::Instant;

use vizx_render::{Compositor, LayerBlend, LayerDraw, LayerEffects};

const FRAMES: u32 = 60;

fn main() {
    let picture: Vec<u8> = (0..1280 * 720)
        .flat_map(|index: u32| {
            let x = (index % 1280) as u8;
            let y = (index / 1280) as u8;
            [x, y, x ^ y, 255]
        })
        .collect();
    let layer = |key: u64, effects: LayerEffects| LayerDraw {
        key,
        version: 0,
        width: 1280,
        height: 720,
        pixels: &picture,
        center: [0.5, 0.5],
        size: [1.0, 1.0],
        rotation_degrees: 0.0,
        opacity: 1.0,
        sharp_pixels: false,
        blend: LayerBlend::Normal,
        effects,
        shows_below: false,
    };
    let heavy = LayerEffects {
        blur: 0.5,
        color_split: 0.5,
        glow: 0.5,
        grain: 0.3,
        glitch: 0.3,
        vignette: 0.5,
        ..LayerEffects::NONE
    };
    let camera = LayerDraw {
        shows_below: true,
        effects: heavy,
        ..layer(9, LayerEffects::NONE)
    };

    let cases: [(&str, Vec<LayerDraw<'_>>); 4] = [
        (
            "3 plain layers",
            (0..3).map(|key| layer(key, LayerEffects::NONE)).collect(),
        ),
        (
            "3 layers, light effects (tint, vignette, grain)",
            (0..3)
                .map(|key| {
                    layer(
                        key,
                        LayerEffects {
                            tint: 0.5,
                            vignette: 0.5,
                            grain: 0.3,
                            ..LayerEffects::NONE
                        },
                    )
                })
                .collect(),
        ),
        (
            "3 layers, heavy effects (blur + split + glow)",
            (0..3).map(|key| layer(key, heavy)).collect(),
        ),
        (
            "3 plain layers + effects layer (heavy)",
            (0..3)
                .map(|key| layer(key, LayerEffects::NONE))
                .chain([camera])
                .collect(),
        ),
    ];

    for (width, height) in [(960, 540), (1920, 1080)] {
        let mut compositor = Compositor::new(width, height).expect("a graphics adapter");
        println!("{width} x {height}:");
        for (name, layers) in &cases {
            let mut pixels = Vec::new();
            // The first frame uploads the pictures; it is not counted.
            compositor.render(layers, &mut pixels).unwrap();
            let started = Instant::now();
            for frame in 0..FRAMES {
                let moving: Vec<LayerDraw<'_>> = layers
                    .iter()
                    .map(|layer| LayerDraw {
                        effects: LayerEffects {
                            seconds: frame as f32 / 30.0,
                            ..layer.effects
                        },
                        ..*layer
                    })
                    .collect();
                compositor.render(&moving, &mut pixels).unwrap();
            }
            let per_frame = started.elapsed().as_secs_f64() * 1000.0 / f64::from(FRAMES);
            println!(
                "  {name:<48} {per_frame:6.1} ms per frame ({:.0} fps)",
                1000.0 / per_frame
            );
        }
    }
}
