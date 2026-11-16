//! Recognizes the lyrics of a vocals file without the VizX window, and says
//! how long it took. Handy for checking speed and quality.
//!
//! ```text
//! cargo run --release -p vizx-ml --example lyrics -- <vocals file> [fast|balanced|accurate] [language]
//! ```

use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use vizx_ml::{RecognitionQuality, find_whisper_model, recognize_lyrics};

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let Some(file) = arguments.first() else {
        eprintln!("usage: lyrics <vocals file> [fast|balanced|accurate] [language]");
        return ExitCode::FAILURE;
    };
    let quality = match arguments.get(1).map(String::as_str) {
        Some("fast") => RecognitionQuality::Fast,
        Some("accurate") => RecognitionQuality::Accurate,
        _ => RecognitionQuality::Balanced,
    };
    let language = arguments.get(2).map(String::as_str);

    let outcome = (|| {
        let model = find_whisper_model()?;
        let audio = vizx_audio::decode_audio_file(Path::new(file)).map_err(|e| e.to_string())?;
        let seconds = audio.frame_count() as f64 / f64::from(audio.sample_rate);
        let started = Instant::now();
        let lines = recognize_lyrics(
            &model,
            &audio,
            language,
            quality,
            &AtomicBool::new(false),
            &|_| {},
        )?;
        let took = started.elapsed().as_secs_f64();
        for line in &lines {
            println!("[{:7.2}] {}", line.start_seconds, line.text());
        }
        println!(
            "{} lines, {:.0} s of audio, {} quality: {:.0} s ({:.2} x the length)",
            lines.len(),
            seconds,
            quality.label(),
            took,
            took / seconds
        );
        Ok::<(), String>(())
    })();
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(problem) => {
            eprintln!("{problem}");
            ExitCode::FAILURE
        }
    }
}
