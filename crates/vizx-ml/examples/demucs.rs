//! Splits an audio file into stems with HTDemucs and writes them as WAV
//! files.
//!
//! ```text
//! cargo run --release -p vizx-ml --example demucs -- <audio file> <output folder>
//! ```

use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let [file, folder] = arguments.as_slice() else {
        eprintln!("usage: demucs <audio file> <output folder>");
        return ExitCode::FAILURE;
    };
    let result = (|| -> Result<(), String> {
        let audio = vizx_audio::decode_audio_file(Path::new(file)).map_err(|e| e.to_string())?;
        let setup = vizx_ml::find_ml_setup()?;
        let started = Instant::now();
        let mut model = vizx_ml::DemucsModel::load(&setup)?;
        println!("model loaded in {:.1} s", started.elapsed().as_secs_f64());
        let started = Instant::now();
        let stems = model.separate(&audio, &AtomicBool::new(false), &|share| {
            println!("{:.0} %", share * 100.0);
        })?;
        println!(
            "separated {:.1} s of audio in {:.1} s",
            audio.duration_seconds(),
            started.elapsed().as_secs_f64()
        );
        let name = Path::new(file)
            .file_stem()
            .map_or("song".into(), |stem| stem.to_string_lossy());
        for (stem, path) in stems
            .write_wav_files(Path::new(folder), &name)
            .map_err(|e| e.to_string())?
        {
            println!("{stem}: {}", path.display());
        }
        Ok(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(problem) => {
            eprintln!("{problem}");
            ExitCode::FAILURE
        }
    }
}
