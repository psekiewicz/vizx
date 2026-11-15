//! Reading and writing video files with FFmpeg.
//!
//! VizX does not contain a video decoder or encoder. It starts the separate
//! `ffmpeg` program and exchanges raw pictures with it through pipes: a
//! reader gets one decoded frame after another from FFmpeg's output, and a
//! writer feeds finished frames into FFmpeg's input, which compresses them
//! into a video file. FFmpeg is optional: without it, everything except
//! video files and export works.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// Name of the environment variable with FFmpeg's folder (the one that
/// contains `bin/`). Used during development; an installed VizX finds
/// `ffmpeg` next to its executable or on the system's `PATH` instead.
pub const FFMPEG_DIR_VARIABLE: &str = "VIZX_FFMPEG_DIR";

/// File extensions of the moving-picture formats offered when importing,
/// in lower case: videos and animated GIFs. FFmpeg reads many more; these
/// are the common ones.
pub const SUPPORTED_VIDEO_EXTENSIONS: [&str; 6] = ["mp4", "mov", "mkv", "webm", "avi", "gif"];

#[cfg(windows)]
const FFMPEG_FILE_NAME: &str = "ffmpeg.exe";
#[cfg(not(windows))]
const FFMPEG_FILE_NAME: &str = "ffmpeg";

/// Finds the `ffmpeg` program: next to the VizX executable, then in
/// `%VIZX_FFMPEG_DIR%/bin`, then wherever the system finds `ffmpeg`.
pub fn find_ffmpeg() -> Option<PathBuf> {
    let next_to_executable = std::env::current_exe()
        .ok()
        .and_then(|path| Some(path.parent()?.join(FFMPEG_FILE_NAME)));
    let in_configured_folder = std::env::var_os(FFMPEG_DIR_VARIABLE)
        .map(|folder| PathBuf::from(folder).join("bin").join(FFMPEG_FILE_NAME));

    let found = next_to_executable
        .into_iter()
        .chain(in_configured_folder)
        .find(|path| path.is_file());
    if found.is_some() {
        return found;
    }

    // Not a known file: ask the system to run "ffmpeg" and see if it can.
    let on_path = PathBuf::from(FFMPEG_FILE_NAME);
    let runs = quiet_command(&on_path)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    runs.then_some(on_path)
}

/// Whether a path has one of the [`SUPPORTED_VIDEO_EXTENSIONS`].
pub fn has_supported_video_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            SUPPORTED_VIDEO_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

/// A command that does not open a console window of its own on Windows.
fn quiet_command(program: &Path) -> Command {
    #[allow(unused_mut)] // only changed on Windows
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Reads the size in pixels of a video file's picture.
pub fn probe_video(ffmpeg: &Path, file: &Path) -> Result<(u32, u32), String> {
    // With only an input and nothing to do, FFmpeg describes the file on
    // its error output and stops.
    let output = quiet_command(ffmpeg)
        .arg("-hide_banner")
        .arg("-i")
        .arg(file)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("could not start FFmpeg: {error}"))?;
    let description = String::from_utf8_lossy(&output.stderr);

    parse_video_size(&description).ok_or_else(|| {
        let last_line = description.lines().last().unwrap_or("no video found");
        format!("not a readable video: {last_line}")
    })
}

/// Finds the picture size in FFmpeg's description of a file, in a line such
/// as `Stream #0:0: Video: h264 (High), yuv420p(tv), 1920x1080 [SAR 1:1], 30 fps`.
fn parse_video_size(description: &str) -> Option<(u32, u32)> {
    let video_line = description.lines().find(|line| line.contains("Video:"))?;
    video_line
        .split([',', ' '])
        .filter_map(|word| {
            let (width, height) = word.split_once('x')?;
            Some((width.parse::<u32>().ok()?, height.parse::<u32>().ok()?))
        })
        .find(|(width, height)| *width > 0 && *height > 0)
}

/// Decodes a video file frame by frame.
pub struct VideoReader {
    child: Child,
    output: ChildStdout,
    frame_bytes: usize,
}

impl VideoReader {
    /// Starts decoding `file`, converted to the given size and frame rate,
    /// beginning `start_seconds` into the video. The video starts over when
    /// it ends, so it never runs out of frames.
    pub fn open(
        ffmpeg: &Path,
        file: &Path,
        (width, height): (u32, u32),
        frames_per_second: u32,
        start_seconds: f64,
    ) -> Result<Self, String> {
        let mut child = quiet_command(ffmpeg)
            .args(["-v", "error", "-stream_loop", "-1"])
            .args(["-ss", &format!("{:.3}", start_seconds.max(0.0))])
            .arg("-i")
            .arg(file)
            .args(["-an", "-vf"])
            .arg(format!("scale={width}:{height},fps={frames_per_second}"))
            .args(["-pix_fmt", "rgba", "-f", "rawvideo", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("could not start FFmpeg: {error}"))?;
        let output = child
            .stdout
            .take()
            .ok_or("FFmpeg's output could not be opened")?;

        Ok(Self {
            child,
            output,
            frame_bytes: width as usize * height as usize * 4,
        })
    }

    /// Reads the next frame into `pixels` as opaque RGBA bytes, top row
    /// first.
    pub fn read_frame(&mut self, pixels: &mut Vec<u8>) -> Result<(), String> {
        pixels.resize(self.frame_bytes, 0);
        self.output
            .read_exact(pixels)
            .map_err(|_| "the video could not be decoded".to_owned())
    }
}

impl Drop for VideoReader {
    /// The video loops forever, so FFmpeg has to be told to stop.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Compresses frames (and optionally sound) into an MP4 video file.
pub struct VideoWriter {
    child: Child,
    input: Option<ChildStdin>,
    frame_bytes: usize,
}

impl VideoWriter {
    /// Starts writing `output`.
    ///
    /// `frame_rate` is a fraction (frames, seconds), e.g. (30000, 1001).
    /// `audio` is a sound file that becomes the video's sound track.
    pub fn create(
        ffmpeg: &Path,
        output: &Path,
        (width, height): (u32, u32),
        frame_rate: (u32, u32),
        audio: Option<&Path>,
    ) -> Result<Self, String> {
        let mut command = quiet_command(ffmpeg);
        command
            .args(["-y", "-v", "error"])
            // Input 1: raw pictures arriving through the pipe.
            .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
            .args(["-s", &format!("{width}x{height}")])
            .args([
                "-framerate",
                &format!("{}/{}", frame_rate.0, frame_rate.1.max(1)),
            ])
            .args(["-i", "-"]);
        if let Some(audio) = audio {
            // Input 2: the sound.
            command.arg("-i").arg(audio);
        }
        command
            // H.264 picture in the color layout every player understands,
            // which needs an even width and height.
            .args(["-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2"])
            .args(["-c:v", "libx264", "-preset", "medium", "-crf", "18"])
            .args(["-pix_fmt", "yuv420p"]);
        if audio.is_some() {
            command.args(["-c:a", "aac", "-b:a", "192k", "-shortest"]);
        }
        let mut child = command
            // Lets a player start before the whole file is downloaded.
            .args(["-movflags", "+faststart"])
            .arg(output)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("could not start FFmpeg: {error}"))?;
        let input = child.stdin.take();

        Ok(Self {
            child,
            input,
            frame_bytes: width as usize * height as usize * 4,
        })
    }

    /// Adds one frame: RGBA bytes, top row first.
    pub fn write_frame(&mut self, pixels: &[u8]) -> Result<(), String> {
        if pixels.len() != self.frame_bytes {
            return Err(format!(
                "a frame of {} bytes was given, expected {}",
                pixels.len(),
                self.frame_bytes
            ));
        }
        let written = self
            .input
            .as_mut()
            .ok_or("the video is already finished")?
            .write_all(pixels);
        if written.is_err() {
            // FFmpeg stopped reading; its own message says why.
            return Err(self.failure_message());
        }
        Ok(())
    }

    /// Completes the file. Must be called; otherwise the file is unusable.
    pub fn finish(mut self) -> Result<(), String> {
        // Closing the pipe tells FFmpeg that no more frames will come.
        self.input = None;
        let status = self
            .child
            .wait()
            .map_err(|error| format!("waiting for FFmpeg failed: {error}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(self.failure_message())
        }
    }

    /// What FFmpeg reported on its error output.
    fn failure_message(&mut self) -> String {
        self.input = None;
        let _ = self.child.wait();
        let mut message = String::new();
        if let Some(errors) = self.child.stderr.as_mut() {
            let _ = errors.read_to_string(&mut message);
        }
        let message = message.trim();
        if message.is_empty() {
            "FFmpeg could not write the video".to_owned()
        } else {
            format!("FFmpeg could not write the video: {message}")
        }
    }
}

impl Drop for VideoWriter {
    /// An abandoned export must not leave FFmpeg running.
    fn drop(&mut self) {
        self.input = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picture_size_is_found_in_ffmpegs_description() {
        let description = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'clip.mp4':\n  \
            Duration: 00:00:10.00, start: 0.000000, bitrate: 2000 kb/s\n  \
            Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(tv, bt709), \
            1920x1080 [SAR 1:1 DAR 16:9], 1990 kb/s, 30 fps, 30 tbr (default)\n  \
            Stream #0:1[0x2](und): Audio: aac (LC), 48000 Hz, stereo, fltp";

        assert_eq!(parse_video_size(description), Some((1920, 1080)));
        // The codec tag 0x31637661 contains an "x" but is not a size.
        assert_eq!(parse_video_size("Stream #0:0: Audio: mp3, 44100 Hz"), None);
        assert_eq!(
            parse_video_size("clip.mp4: No such file or directory"),
            None
        );
    }

    #[test]
    fn video_extensions_are_recognized() {
        assert!(has_supported_video_extension(Path::new("Loop.MP4")));
        assert!(has_supported_video_extension(Path::new("a/b/clouds.webm")));
        assert!(!has_supported_video_extension(Path::new("cover.png")));
    }

    /// Writes a small video, then reads its size and its frames back.
    #[test]
    fn a_written_video_can_be_probed_and_read() {
        let Some(ffmpeg) = find_ffmpeg() else {
            eprintln!("FFmpeg not found: skipping");
            return;
        };
        let file =
            std::env::temp_dir().join(format!("vizx-ffmpeg-test-{}.mp4", std::process::id()));

        // 12 frames of 64 x 48: the first six red, the rest blue.
        let mut writer = VideoWriter::create(&ffmpeg, &file, (64, 48), (12, 1), None).unwrap();
        for frame in 0..12 {
            let color = if frame < 6 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            };
            writer.write_frame(&color.repeat(64 * 48)).unwrap();
        }
        assert!(writer.write_frame(&[0; 16]).is_err()); // wrong size
        writer.finish().unwrap();

        let size = probe_video(&ffmpeg, &file);
        // Read at half the size and the same frame rate.
        let mut reader = VideoReader::open(&ffmpeg, &file, (32, 24), 12, 0.0).unwrap();
        let mut frames = Vec::new();
        let mut pixels = Vec::new();
        // 14 frames from a 12 frame video: it starts over.
        for _ in 0..14 {
            reader.read_frame(&mut pixels).unwrap();
            frames.push([pixels[0], pixels[1], pixels[2], pixels[3]]);
        }
        drop(reader);

        assert_eq!(size, Ok((64, 48)));
        assert_eq!(pixels.len(), 32 * 24 * 4);
        // Video compression changes colors a little.
        let is_red = |color: [u8; 4]| color[0] > 180 && color[2] < 80 && color[3] == 255;
        let is_blue = |color: [u8; 4]| color[2] > 180 && color[0] < 80;
        assert!(is_red(frames[1]), "{:?}", frames[1]);
        assert!(is_blue(frames[10]), "{:?}", frames[10]);
        assert!(is_red(frames[13]), "after looping: {:?}", frames[13]);

        // Starting 0.75 s in (frame 9 of 12) skips the red half.
        let mut late = VideoReader::open(&ffmpeg, &file, (32, 24), 12, 0.75).unwrap();
        late.read_frame(&mut pixels).unwrap();
        assert!(is_blue([pixels[0], pixels[1], pixels[2], pixels[3]]));
        drop(late);
        let _ = std::fs::remove_file(&file);
    }

    /// An animated GIF with transparent parts reads like a video, keeps
    /// its transparency and starts over when it ends.
    #[test]
    fn an_animated_gif_keeps_its_transparency_and_loops() {
        let Some(ffmpeg) = find_ffmpeg() else {
            eprintln!("FFmpeg not found: skipping");
            return;
        };
        let file =
            std::env::temp_dir().join(format!("vizx-ffmpeg-test-{}.gif", std::process::id()));
        // 4 frames at 4 per second: an opaque red square on the left half,
        // the right half transparent, the square turning blue at frame 2.
        // A GIF stores transparency as one reserved palette color, which
        // FFmpeg only sets aside when asked to.
        let made = quiet_command(&ffmpeg)
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=16x16:r=4:d=1",
            ])
            .arg("-filter_complex")
            .arg(
                "[0]format=rgba,\
                 geq=r='if(lt(N,2),255,0)':g=0:b='if(lt(N,2),0,255)':a='if(lt(X,8),255,0)',\
                 split[a][b];[a]palettegen=reserve_transparent=1:stats_mode=single[p];\
                 [b][p]paletteuse=alpha_threshold=128:new=1",
            )
            .args(["-loop", "0"])
            .arg(&file)
            .status()
            .is_ok_and(|status| status.success());
        assert!(made, "FFmpeg could not make the test GIF");

        let size = probe_video(&ffmpeg, &file);
        let mut reader = VideoReader::open(&ffmpeg, &file, (16, 16), 4, 0.0).unwrap();
        let mut frames = Vec::new();
        for _ in 0..6 {
            let mut pixels = Vec::new();
            reader.read_frame(&mut pixels).unwrap();
            frames.push(pixels);
        }
        drop(reader);
        let _ = std::fs::remove_file(&file);

        assert_eq!(size, Ok((16, 16)));
        let pixel = |frame: &[u8], x: usize| {
            [
                frame[(8 * 16 + x) * 4],
                frame[(8 * 16 + x) * 4 + 2],
                frame[(8 * 16 + x) * 4 + 3],
            ]
        };
        // A GIF has a palette of 256 colors, so values are close to, not
        // exactly, what was asked for.
        let is_red = |[red, blue, opacity]: [u8; 3]| red > 230 && blue < 25 && opacity == 255;
        let is_blue = |[red, blue, opacity]: [u8; 3]| blue > 230 && red < 25 && opacity == 255;
        assert!(is_red(pixel(&frames[0], 2)), "{:?}", pixel(&frames[0], 2));
        assert_eq!(pixel(&frames[0], 13)[2], 0, "the right half is transparent");
        assert!(is_blue(pixel(&frames[3], 2)), "{:?}", pixel(&frames[3], 2));
        // Frame 4 is the first one again.
        assert!(is_red(pixel(&frames[4], 2)), "{:?}", pixel(&frames[4], 2));
    }
    #[test]
    fn a_file_that_is_not_a_video_is_refused() {
        let Some(ffmpeg) = find_ffmpeg() else {
            return;
        };
        let missing = std::env::temp_dir().join("vizx-ffmpeg-test-missing.mp4");

        assert!(probe_video(&ffmpeg, &missing).is_err());
    }
}
