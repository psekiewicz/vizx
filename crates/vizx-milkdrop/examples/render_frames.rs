//! Renders a MilkDrop preset to image files, without the VizX window.
//!
//! A quick way to check that projectM, OpenGL and the pixel readback work on
//! a machine:
//!
//! ```text
//! cargo run -p vizx-milkdrop --example render_frames -- <preset.milk> <output folder>
//! ```
//!
//! It feeds projectM a test tone and writes a few frames as BMP images.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use vizx_milkdrop::MilkdropRenderer;

const WIDTH: u32 = 640;
const HEIGHT: u32 = 360;
const FRAMES_PER_SECOND: u32 = 30;
const SAMPLE_RATE: u32 = 44_100;
const FRAME_COUNT: u32 = 90;
/// Frames written to disk; the others are only rendered.
const SAVED_FRAMES: [u32; 3] = [15, 45, 89];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let (Some(preset), Some(output_folder)) = (arguments.next(), arguments.next()) else {
        return Err("usage: render_frames <preset.milk> <output folder>".into());
    };
    let output_folder = PathBuf::from(output_folder);
    std::fs::create_dir_all(&output_folder)?;

    let mut renderer = MilkdropRenderer::new(WIDTH, HEIGHT, FRAMES_PER_SECOND)?;
    println!("projectM {}", renderer.projectm_version());
    println!("OpenGL {}", renderer.opengl_description());
    renderer.load_preset(Path::new(&preset))?;

    let samples_per_frame = (SAMPLE_RATE / FRAMES_PER_SECOND) as usize;
    let frame_duration = Duration::from_secs_f64(1.0 / f64::from(FRAMES_PER_SECOND));
    let mut pixels = Vec::new();
    let mut sample_index = 0u64;
    let started = Instant::now();

    for frame in 0..FRAME_COUNT {
        renderer.add_stereo_audio(&test_tone(sample_index, samples_per_frame));
        sample_index += samples_per_frame as u64;
        renderer.render_frame(&mut pixels);

        if SAVED_FRAMES.contains(&frame) {
            let path = output_folder.join(format!("frame-{frame:03}.bmp"));
            write_bmp(&path, WIDTH, HEIGHT, &pixels)?;
            println!(
                "{}: {:.1}% of pixels are not black",
                path.display(),
                non_black_percentage(&pixels)
            );
        }

        // projectM 4.1 animates by the real clock, so frames are produced
        // at real-time speed.
        let next_frame_at = frame_duration * (frame + 1);
        if let Some(wait) = next_frame_at.checked_sub(started.elapsed()) {
            std::thread::sleep(wait);
        }
    }

    println!(
        "rendered {FRAME_COUNT} frames in {:.1} s",
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// A stereo test signal: a bass tone that pulses twice per second (so
/// "beat" reactions have something to react to) plus a quieter high tone.
fn test_tone(first_sample: u64, frame_count: usize) -> Vec<f32> {
    let mut samples = Vec::with_capacity(frame_count * 2);
    for offset in 0..frame_count as u64 {
        let seconds = (first_sample + offset) as f32 / SAMPLE_RATE as f32;
        let pulse = 1.0 - (seconds * 2.0).fract();
        let bass = (seconds * 80.0 * std::f32::consts::TAU).sin() * pulse * 0.8;
        let treble = (seconds * 2400.0 * std::f32::consts::TAU).sin() * 0.15;
        samples.push(bass + treble);
        samples.push(bass - treble);
    }
    samples
}

fn non_black_percentage(rgba: &[u8]) -> f64 {
    let pixel_count = rgba.len() / 4;
    let lit = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel[0] > 8 || pixel[1] > 8 || pixel[2] > 8)
        .count();
    lit as f64 * 100.0 / pixel_count.max(1) as f64
}

/// Writes RGBA pixels (top row first) as an uncompressed 24-bit BMP file.
fn write_bmp(path: &Path, width: u32, height: u32, rgba: &[u8]) -> std::io::Result<()> {
    let row_size = (width * 3).div_ceil(4) * 4; // rows are padded to 4 bytes
    let data_size = row_size * height;

    let mut bytes = Vec::with_capacity(54 + data_size as usize);
    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&(54 + data_size).to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&54u32.to_le_bytes()); // where the pixels start
    bytes.extend_from_slice(&40u32.to_le_bytes()); // size of the info header
    bytes.extend_from_slice(&(width as i32).to_le_bytes());
    bytes.extend_from_slice(&(height as i32).to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // color planes
    bytes.extend_from_slice(&24u16.to_le_bytes()); // bits per pixel
    bytes.extend_from_slice(&[0; 24]); // no compression, default settings

    // BMP stores the bottom row first and each pixel as blue, green, red.
    for row in (0..height as usize).rev() {
        let start = row * width as usize * 4;
        for pixel in rgba[start..start + width as usize * 4].as_chunks::<4>().0 {
            bytes.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
        }
        bytes.resize(bytes.len() + (row_size - width * 3) as usize, 0);
    }
    std::fs::write(path, bytes)
}
