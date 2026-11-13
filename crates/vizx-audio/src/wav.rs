use std::fs;
use std::io;
use std::path::Path;

/// Writes samples as an uncompressed 16-bit PCM WAV file.
///
/// `samples` are interleaved: for stereo that is left, right, left, right,
/// and so on. VizX uses this to create known audio for its tests; it is the
/// simplest audio format there is, a short header followed by raw samples.
pub fn write_wav_file(
    path: &Path,
    sample_rate: u32,
    channel_count: u16,
    samples: &[i16],
) -> io::Result<()> {
    let bytes_per_frame = u32::from(channel_count) * 2;
    let data_size = u32::try_from(samples.len() * 2)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "too many samples for WAV"))?;

    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_size).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes()); // size of the fmt chunk
    bytes.extend_from_slice(&1u16.to_le_bytes()); // 1 = uncompressed PCM
    bytes.extend_from_slice(&channel_count.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * bytes_per_frame).to_le_bytes());
    bytes.extend_from_slice(&(bytes_per_frame as u16).to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_size.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }

    fs::write(path, bytes)
}
