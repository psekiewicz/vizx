//! Tells the singers of a song apart, line by line, without the window.
//!
//! ```text
//! cargo run --release -p vizx-ml --example singers -- <vocals.wav> <lines.tsv> [singers]
//! ```
//!
//! `lines.tsv` has one line per row: start seconds, end seconds and the
//! words, separated by tabs. `singers` fixes their number; without it the
//! number follows from how alike the voices are.

use std::sync::atomic::AtomicBool;
use std::time::Instant;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        return Err("usage: singers <vocals.wav> <lines.tsv> [singers]".to_owned());
    }
    let vocals = vizx_audio::decode_audio_file(std::path::Path::new(&args[0]))
        .map_err(|error| error.to_string())?;
    let table = std::fs::read_to_string(&args[1]).map_err(|error| error.to_string())?;
    let lines: Vec<(f64, f64, String)> = table
        .lines()
        .filter_map(|row| {
            let mut parts = row.splitn(3, '\t');
            let start = parts.next()?.trim().parse().ok()?;
            let end = parts.next()?.trim().parse().ok()?;
            Some((start, end, parts.next().unwrap_or("").to_owned()))
        })
        .collect();
    let singers = args.get(2).and_then(|count| count.parse().ok());

    let runtime = vizx_ml::find_onnx_runtime()?;
    let mut model = vizx_ml::SpeakerModel::load(&runtime, &vizx_ml::find_speaker_model()?)?;
    let started = Instant::now();
    let times: Vec<(f64, f64)> = lines.iter().map(|(start, end, _)| (*start, *end)).collect();
    let found = vizx_ml::identify_singers(
        &mut model,
        &vocals,
        &times,
        singers,
        &AtomicBool::new(false),
        &|_| {},
    )?;
    for ((start, end, words), singer) in lines.iter().zip(&found) {
        println!("{start:7.2} {end:7.2}  singer {}  {words}", singer + 1);
    }
    println!(
        "{} lines, {} singers, {:.1} s",
        lines.len(),
        found.iter().max().map_or(0, |most| most + 1),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}
