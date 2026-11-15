//! Renders a visualizer plugin to a JPEG picture without the VizX window.
//!
//! Handy while writing a plugin: it reports shader errors and shows what
//! the plugin draws for a made-up sound (a beat with a moving melody).
//!
//! ```text
//! cargo run -p vizx-visualizer --example render_plugin -- <plugin folder> <output.jpg> [frames]
//! ```
//!
//! `frames` (default 90) is how many frames are rendered before the picture
//! is saved; plugins with trails or history look different after a while.

use std::path::Path;
use std::process::ExitCode;

use vizx_visualizer::{FrameInput, ParameterValues, ShaderRenderer, VisualizerPlugin};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 360;
const FRAMES_PER_SECOND: f32 = 30.0;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let (Some(folder), Some(output)) = (arguments.first(), arguments.get(1)) else {
        eprintln!("usage: render_plugin <plugin folder> <output.jpg> [frames]");
        return ExitCode::FAILURE;
    };
    let frame_count = arguments
        .get(2)
        .and_then(|text| text.parse().ok())
        .unwrap_or(90);

    match render(Path::new(folder), Path::new(output), frame_count) {
        Ok(name) => {
            println!("rendered {frame_count} frames of \"{name}\" to {output}");
            ExitCode::SUCCESS
        }
        Err(problem) => {
            eprintln!("{problem}");
            ExitCode::FAILURE
        }
    }
}

fn render(folder: &Path, output: &Path, frame_count: u32) -> Result<String, String> {
    let plugin = VisualizerPlugin::load(folder)?;
    let mut renderer = ShaderRenderer::new(&plugin, WIDTH, HEIGHT)?;
    let parameters = plugin.pack_parameters(&ParameterValues::new());

    let mut pixels = Vec::new();
    for frame in 0..frame_count.max(1) {
        let seconds = frame as f32 / FRAMES_PER_SECOND;
        let sound = made_up_sound(seconds);
        renderer.render(
            &FrameInput {
                time_seconds: seconds,
                spectrum: &sound.spectrum,
                waveform: &sound.waveform,
                volume: sound.beat * 0.8,
                bass: sound.beat,
                mid: 0.4,
                treble: 0.25,
                beat: sound.beat,
                seconds_since_beat: (seconds * 2.0).fract() / 2.0,
                beat_count: (seconds * 2.0).floor(),
                tempo_bpm: 120.0,
                parameters,
            },
            &mut pixels,
        )?;
    }

    let mut jpeg = Vec::new();
    jpeg_encoder::Encoder::new(&mut jpeg, 92)
        .encode(
            &pixels,
            WIDTH as u16,
            HEIGHT as u16,
            jpeg_encoder::ColorType::Rgba,
        )
        .map_err(|error| format!("could not encode the picture: {error}"))?;
    std::fs::write(output, jpeg)
        .map_err(|error| format!("could not write {}: {error}", output.display()))?;
    Ok(plugin.manifest.name)
}

struct Sound {
    spectrum: Vec<f32>,
    waveform: Vec<f32>,
    /// 1 right on a beat, falling to 0 before the next one.
    beat: f32,
}

/// Something that looks like music to a visualizer: a kick drum twice a
/// second in the bass, a melody note that wanders through the middle, and
/// a little noise everywhere.
fn made_up_sound(seconds: f32) -> Sound {
    let beat = (1.0 - (seconds * 2.0).fract()).powi(3);
    let melody_position = 0.45 + 0.2 * (seconds * 0.9).sin();

    let spectrum = (0..512)
        .map(|bin| {
            let position = bin as f32 / 512.0;
            let bump = |center: f32, width: f32| (-((position - center) / width).powi(2)).exp();
            let ripple = 0.5 + 0.5 * (position * 90.0 + seconds * 3.0).sin();
            let level = beat * bump(0.08, 0.08)
                + 0.75 * bump(melody_position, 0.03)
                + 0.35 * bump(melody_position * 1.5, 0.02)
                + 0.18 * ripple * (1.0 - position);
            level.clamp(0.0, 1.0)
        })
        .collect();
    let waveform = (0..512)
        .map(|point| {
            let phase = point as f32 / 512.0;
            0.6 * beat * (phase * 12.0).sin() + 0.3 * (phase * 70.0 + seconds * 20.0).sin()
        })
        .collect();

    Sound {
        spectrum,
        waveform,
        beat,
    }
}
