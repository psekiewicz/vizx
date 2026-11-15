//! Splits an audio file into stems (vocals, drums, bass, other) with the
//! built-in separation and writes them as WAV files.
//!
//! ```text
//! cargo run --release -p vizx-audio --example separate -- <audio file> <output folder>
//! ```

use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let [file, folder] = arguments.as_slice() else {
        eprintln!("usage: separate <audio file> <output folder>");
        return ExitCode::FAILURE;
    };
    let audio = match vizx_audio::decode_audio_file(Path::new(file)) {
        Ok(audio) => audio,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let started = Instant::now();
    let stems = match vizx_audio::separate_stems(&audio, &AtomicBool::new(false), &|_| {}) {
        Ok(stems) => stems,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "separated {:.1} s of audio in {:.1} s",
        audio.duration_seconds(),
        started.elapsed().as_secs_f64()
    );
    let name = Path::new(file)
        .file_stem()
        .map_or("song".into(), |stem| stem.to_string_lossy());
    match stems.write_wav_files(Path::new(folder), &name) {
        Ok(written) => {
            for (stem, path) in written {
                println!("{stem}: {}", path.display());
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("could not write the stems: {error}");
            ExitCode::FAILURE
        }
    }
}
