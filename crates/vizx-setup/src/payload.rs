//! The installer's payload: VizX's files appended to the end of the setup
//! program, so that the installer is one file.
//!
//! Layout of the installer file:
//!
//! ```text
//! [setup program]                 the stub: an ordinary .exe
//! [entry] [entry] ...             one per file
//! [trailer, 32 bytes]             where the payload starts, how many
//!                                 entries, their total size, "VIZXPAK1"
//! ```
//!
//! An entry is: path length (u32), path (UTF-8, `/` between folders),
//! whether it is compressed (u8), its size (u64), its stored size (u64),
//! then the stored bytes (deflate, or the file as it is). Numbers are
//! little-endian. Files that do not get smaller (models, pictures) are
//! stored as they are.
//!
//! Windows ignores what follows the program in an .exe, so the stub runs
//! as usual and finds its payload by reading its own file from the end.

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;

const MAGIC: &[u8; 8] = b"VIZXPAK1";
const TRAILER_LENGTH: u64 = 32;
/// Progress is reported at least this often while a large file is copied.
const PROGRESS_STEP: u64 = 4 * 1024 * 1024;

/// What the end of an installer says about its payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trailer {
    /// Where the first entry starts: the length of the setup program.
    pub payload_offset: u64,
    pub entry_count: u64,
    /// The size of all files once installed, in bytes.
    pub total_size: u64,
}

/// Files that are stored as they are: compressing them gains nothing.
fn is_stored_as_is(path: &Path) -> bool {
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(
        extension.as_str(),
        "onnx" | "bin" | "jpg" | "jpeg" | "png" | "zip" | "gz" | "mp3" | "mp4"
    )
}

/// Every file inside `folder`, as paths relative to it, in a fixed order.
fn files_in(folder: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut folders = vec![folder.to_path_buf()];
    while let Some(current) = folders.pop() {
        for entry in fs::read_dir(&current)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                folders.push(path);
            } else if let Ok(relative) = path.strip_prefix(folder) {
                files.push(relative.to_path_buf());
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Writes the installer `output`: the setup program `stub` followed by
/// every file of `folder`. `report` hears of each file as it is added
/// (its path and size).
pub fn pack(
    stub: &Path,
    folder: &Path,
    output: &Path,
    mut report: impl FnMut(&str, u64),
) -> io::Result<Trailer> {
    let files = files_in(folder)?;
    let mut out = BufWriter::new(File::create(output)?);
    let payload_offset = io::copy(&mut File::open(stub)?, &mut out)?;
    let mut total_size = 0;
    for relative in &files {
        let source = folder.join(relative);
        let name = relative.to_string_lossy().replace('\\', "/");
        let size = fs::metadata(&source)?.len();
        let is_compressed = !is_stored_as_is(&source);
        report(&name, size);

        out.write_all(&(name.len() as u32).to_le_bytes())?;
        out.write_all(name.as_bytes())?;
        out.write_all(&[u8::from(is_compressed)])?;
        out.write_all(&size.to_le_bytes())?;
        // The stored size is known only afterwards; written then.
        let stored_size_at = out.stream_position()?;
        out.write_all(&0u64.to_le_bytes())?;
        let data_start = out.stream_position()?;
        let mut input = BufReader::new(File::open(&source)?);
        if is_compressed {
            let mut encoder = DeflateEncoder::new(&mut out, Compression::default());
            io::copy(&mut input, &mut encoder)?;
            encoder.finish()?;
        } else {
            io::copy(&mut input, &mut out)?;
        }
        let data_end = out.stream_position()?;
        out.seek(SeekFrom::Start(stored_size_at))?;
        out.write_all(&(data_end - data_start).to_le_bytes())?;
        out.seek(SeekFrom::Start(data_end))?;
        total_size += size;
    }
    let trailer = Trailer {
        payload_offset,
        entry_count: files.len() as u64,
        total_size,
    };
    out.write_all(&trailer.payload_offset.to_le_bytes())?;
    out.write_all(&trailer.entry_count.to_le_bytes())?;
    out.write_all(&trailer.total_size.to_le_bytes())?;
    out.write_all(MAGIC)?;
    out.flush()?;
    Ok(trailer)
}

/// The trailer of the installer `file`, or `None` if it has no payload
/// (the bare setup program, which is the uninstaller).
pub fn read_trailer(file: &mut File) -> io::Result<Option<Trailer>> {
    let length = file.metadata()?.len();
    if length < TRAILER_LENGTH {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(length - TRAILER_LENGTH))?;
    let mut bytes = [0u8; TRAILER_LENGTH as usize];
    file.read_exact(&mut bytes)?;
    if &bytes[24..32] != MAGIC {
        return Ok(None);
    }
    let number = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    let trailer = Trailer {
        payload_offset: number(0),
        entry_count: number(8),
        total_size: number(16),
    };
    if trailer.payload_offset > length - TRAILER_LENGTH {
        return Ok(None);
    }
    Ok(Some(trailer))
}

/// A path from the payload as a path inside the install folder. Only plain
/// names are allowed, so nothing can be written outside of that folder.
fn safe_relative_path(name: &str) -> io::Result<PathBuf> {
    let path = PathBuf::from(name);
    let is_plain = !name.is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    if is_plain {
        Ok(path)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsafe path in the installer: {name}"),
        ))
    }
}

/// Counts the bytes written through it, and reports them now and then.
struct Counting<'a, W> {
    inner: W,
    written: &'a mut u64,
    reported: u64,
    report: &'a mut dyn FnMut(u64),
}

impl<W: Write> Write for Counting<'_, W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let count = self.inner.write(buffer)?;
        *self.written += count as u64;
        if *self.written - self.reported >= PROGRESS_STEP {
            self.reported = *self.written;
            (self.report)(*self.written);
        }
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Unpacks the payload of the installer `installer` into `target`.
/// `progress` hears the file being unpacked and how many bytes of all are
/// done. Returns the unpacked files, relative to `target`.
pub fn unpack(
    installer: &Path,
    target: &Path,
    mut progress: impl FnMut(&str, u64),
) -> io::Result<Vec<PathBuf>> {
    let mut file = File::open(installer)?;
    let trailer = read_trailer(&mut file)?.ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "this setup program holds no VizX")
    })?;
    file.seek(SeekFrom::Start(trailer.payload_offset))?;
    let mut reader = BufReader::with_capacity(1 << 20, file);
    let mut done = 0u64;
    let mut unpacked = Vec::new();
    for _ in 0..trailer.entry_count {
        let mut length = [0u8; 4];
        reader.read_exact(&mut length)?;
        let mut name = vec![0u8; u32::from_le_bytes(length) as usize];
        reader.read_exact(&mut name)?;
        let name = String::from_utf8(name)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "a file name is not UTF-8"))?;
        let mut flag = [0u8; 1];
        reader.read_exact(&mut flag)?;
        let mut number = [0u8; 8];
        reader.read_exact(&mut number)?;
        let size = u64::from_le_bytes(number);
        reader.read_exact(&mut number)?;
        let stored_size = u64::from_le_bytes(number);

        let relative = safe_relative_path(&name)?;
        let destination = target.join(&relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        progress(&name, done);
        let start = done;
        let mut report = |written: u64| progress(&name, start + written);
        let mut written = 0;
        let mut output = Counting {
            inner: BufWriter::new(File::create(&destination).map_err(|error| {
                io::Error::new(error.kind(), format!("{}: {error}", destination.display()))
            })?),
            written: &mut written,
            reported: 0,
            report: &mut report,
        };
        let mut stored = (&mut reader).take(stored_size);
        if flag[0] == 1 {
            io::copy(&mut DeflateDecoder::new(&mut stored), &mut output)?;
        } else {
            io::copy(&mut stored, &mut output)?;
        }
        output.flush()?;
        drop(output);
        // Whatever the decoder did not need belongs to this entry still.
        io::copy(&mut stored, &mut io::sink())?;
        if written != size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{name} is damaged in the installer"),
            ));
        }
        done += size;
        unpacked.push(relative);
    }
    progress("", done);
    Ok(unpacked)
}

/// Writes the setup program without its payload to `output` (it then
/// serves as the uninstaller).
pub fn write_stub(installer: &Path, output: &Path) -> io::Result<()> {
    let mut file = File::open(installer)?;
    let length = match read_trailer(&mut file)? {
        Some(trailer) => trailer.payload_offset,
        None => file.metadata()?.len(),
    };
    file.seek(SeekFrom::Start(0))?;
    let mut out = BufWriter::new(File::create(output)?);
    io::copy(&mut file.take(length), &mut out)?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("vizx-setup-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn files_come_out_as_they_went_in() {
        let root = test_folder("pack");
        let source = root.join("source");
        fs::create_dir_all(source.join("presets/Deep")).unwrap();
        fs::write(source.join("vizx.exe"), b"MZ program").unwrap();
        let text = "a preset line\n".repeat(5000);
        fs::write(source.join("presets/Deep/one.milk"), &text).unwrap();
        let model: Vec<u8> = (0..300_000u32).map(|n| (n * 7919 % 251) as u8).collect();
        fs::write(source.join("model.onnx"), &model).unwrap();
        fs::write(root.join("stub.exe"), b"STUB").unwrap();

        let installer = root.join("setup.exe");
        let trailer = pack(&root.join("stub.exe"), &source, &installer, |_, _| {}).unwrap();
        assert_eq!(trailer.payload_offset, 4);
        assert_eq!(trailer.entry_count, 3);
        // The text was compressed.
        assert!(fs::metadata(&installer).unwrap().len() < (text.len() + model.len()) as u64);

        let target = root.join("installed");
        let mut last = 0;
        let files = unpack(&installer, &target, |_, done| last = done).unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(last, trailer.total_size);
        assert_eq!(
            fs::read_to_string(target.join("presets/Deep/one.milk")).unwrap(),
            text
        );
        assert_eq!(fs::read(target.join("model.onnx")).unwrap(), model);

        // Without its payload it is the bare program again.
        write_stub(&installer, &root.join("uninstall.exe")).unwrap();
        assert_eq!(fs::read(root.join("uninstall.exe")).unwrap(), b"STUB");
        let mut bare = File::open(root.join("uninstall.exe")).unwrap();
        assert_eq!(read_trailer(&mut bare).unwrap(), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn paths_cannot_leave_the_install_folder() {
        assert!(safe_relative_path("presets/a.milk").is_ok());
        assert!(safe_relative_path("../evil.exe").is_err());
        assert!(safe_relative_path("C:/Windows/evil.exe").is_err());
        assert!(safe_relative_path("/evil.exe").is_err());
        assert!(safe_relative_path("").is_err());
    }
}
