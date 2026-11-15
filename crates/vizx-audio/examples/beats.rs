//! Prints the beats and the tempo VizX finds in an audio file.
//!
//! ```text
//! cargo run -p vizx-audio --example beats -- <audio file> [seconds to list]
//! ```
//!
//! Handy for checking the beat detection on real music without the VizX
//! window. Only the beats of the first seconds (default 10) are listed.

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let Some(file) = arguments.first() else {
        eprintln!("usage: beats <audio file> [seconds to list]");
        return ExitCode::FAILURE;
    };
    let listed_seconds: f64 = arguments
        .get(1)
        .and_then(|text| text.parse().ok())
        .unwrap_or(10.0);

    let audio = match vizx_audio::decode_audio_file(Path::new(file)) {
        Ok(audio) => audio,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let started = std::time::Instant::now();
    let analysis = vizx_audio::detect_beats(&audio);

    println!(
        "{:.1} s of audio analysed in {:.2} s",
        audio.duration_seconds(),
        started.elapsed().as_secs_f64()
    );
    match analysis.tempo_bpm {
        Some(tempo) => println!("tempo: {tempo:.1} BPM"),
        None => println!("tempo: unknown"),
    }
    println!(
        "{} beats, {:.2} per second",
        analysis.beats.len(),
        analysis.beats.len() as f64 / audio.duration_seconds().max(0.001)
    );
    let mut previous = None;
    for beat in analysis
        .beats
        .iter()
        .filter(|beat| beat.seconds < listed_seconds)
    {
        let gap = previous.map_or(String::new(), |previous: f64| {
            format!("  (+{:.3} s)", beat.seconds - previous)
        });
        println!("{:8.3} s  strength {:.2}{gap}", beat.seconds, beat.strength);
        previous = Some(beat.seconds);
    }
    ExitCode::SUCCESS
}
