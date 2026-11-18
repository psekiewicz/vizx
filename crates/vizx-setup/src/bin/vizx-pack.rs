//! Makes the one-file VizX installer: appends every file of a prepared
//! VizX folder to the setup program.
//!
//! Usage: vizx-pack <setup program> <VizX folder> <installer to write>

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [stub, folder, output] = arguments.as_slice() else {
        eprintln!("usage: vizx-pack <setup program> <VizX folder> <installer to write>");
        return ExitCode::from(2);
    };
    let mut count = 0u64;
    let result = vizx_setup::payload::pack(stub, folder, output, |name, size| {
        count += 1;
        if size > 20 * 1024 * 1024 || count.is_multiple_of(1000) {
            println!("  {name} ({:.0} MB)", size as f64 / 1_048_576.0);
        }
    });
    match result {
        Ok(trailer) => {
            let written = std::fs::metadata(output).map_or(0, |metadata| metadata.len());
            println!(
                "{}: {} files, {:.0} MB installed, {:.0} MB installer",
                output.display(),
                trailer.entry_count,
                trailer.total_size as f64 / 1_048_576.0,
                written as f64 / 1_048_576.0
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("vizx-pack: {error}");
            ExitCode::FAILURE
        }
    }
}
