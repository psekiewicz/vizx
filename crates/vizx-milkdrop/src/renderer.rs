use std::ffi::{CStr, CString, c_char, c_void};
use std::path::Path;

use crate::MilkdropError;
use crate::ffi::{self, Api};
use crate::gl_context::OffscreenGlContext;

/// Renders MilkDrop presets to pixels with the projectM library.
///
/// One renderer owns one projectM instance and one off-screen OpenGL
/// context. OpenGL ties a context to a thread, so a renderer must be
/// created, used and dropped on the same thread; the compiler enforces this
/// (the type cannot be sent to another thread).
pub struct MilkdropRenderer {
    // Field order matters: fields are dropped top to bottom. The projectM
    // instance must be destroyed while its OpenGL context still exists, and
    // the library must stay loaded until the instance is gone.
    instance: *mut ffi::ProjectM,
    gl_context: OffscreenGlContext,
    api: Api,
    /// Message of the last failed preset load, written by projectM through
    /// a callback. Boxed so its address stays the same when the renderer is
    /// moved.
    load_error: Box<Option<String>>,
    max_samples_per_call: usize,
    version: String,
    width: u32,
    height: u32,
}

impl MilkdropRenderer {
    /// Creates a renderer producing pictures of `width` x `height` pixels at
    /// `frames_per_second`.
    pub fn new(width: u32, height: u32, frames_per_second: u32) -> Result<Self, MilkdropError> {
        if width == 0 || height == 0 {
            return Err(MilkdropError::new("picture size must not be zero"));
        }
        let api = Api::load()?;
        let gl_context = OffscreenGlContext::new(width, height).map_err(MilkdropError::new)?;
        api.initialize_opengl()?;

        // SAFETY: an OpenGL context is current on this thread, which is
        // what `projectm_create` requires. It returns null on failure.
        let instance = unsafe { (api.create)() };
        if instance.is_null() {
            return Err(MilkdropError::new(
                "projectM could not start (it needs OpenGL 3.3 or newer)",
            ));
        }

        let mut load_error: Box<Option<String>> = Box::new(None);
        // SAFETY: `instance` is valid. The callback receives a pointer to
        // the boxed `load_error`, which lives as long as the renderer does.
        let max_samples_per_call = unsafe {
            (api.set_window_size)(instance, width as usize, height as usize);
            (api.set_fps)(instance, frames_per_second as i32);
            // VizX decides which preset is shown; projectM must not switch
            // presets by itself, neither on a timer nor on loud beats.
            (api.set_preset_locked)(instance, true);
            (api.set_hard_cut_enabled)(instance, false);
            (api.set_preset_switch_failed_event_callback)(
                instance,
                Some(preset_switch_failed),
                (&raw mut *load_error).cast(),
            );
            (api.pcm_get_max_samples)() as usize
        };
        let version = read_version(&api);

        Ok(Self {
            instance,
            gl_context,
            api,
            load_error,
            max_samples_per_call: max_samples_per_call.max(1),
            version,
            width,
            height,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Version of the projectM library in use, e.g. `4.1.8`.
    pub fn projectm_version(&self) -> &str {
        &self.version
    }

    /// The OpenGL version and graphics card projectM is drawing with.
    pub fn opengl_description(&self) -> String {
        self.gl_context.description()
    }

    /// Loads a `.milk` preset file and switches to it immediately.
    pub fn load_preset(&mut self, path: &Path) -> Result<(), MilkdropError> {
        if !path.is_file() {
            return Err(MilkdropError::new(format!(
                "preset file not found: {}",
                path.display()
            )));
        }
        let path_text = CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| MilkdropError::new("preset path contains a zero byte"))?;

        *self.load_error = None;
        // SAFETY: the instance is valid and `path_text` is a
        // zero-terminated string that outlives the call. `false` = switch
        // at once, without a cross-fade.
        unsafe { (self.api.load_preset_file)(self.instance, path_text.as_ptr(), false) };

        match self.load_error.take() {
            Some(message) => Err(MilkdropError::new(format!(
                "could not load preset {}: {message}",
                path.display()
            ))),
            None => Ok(()),
        }
    }

    /// Sets the time of the next frame, in seconds since the first frame.
    ///
    /// Without this, projectM animates by the computer's clock, so a frame
    /// would look different depending on how long rendering happens to
    /// take. With it, a frame depends only on its position in time, which
    /// is what rendering ahead of time (and export) needs.
    ///
    /// Fails if the loaded projectM library does not offer this function
    /// (it comes from the patch described in `docs/milkdrop.md`).
    pub fn set_frame_time(&mut self, seconds: f64) -> Result<(), MilkdropError> {
        let Some(set_frame_time) = self.api.set_frame_time else {
            return Err(MilkdropError::new(
                "this projectM library cannot set the frame time; \
                 it must be built with the patch from the VizX repository (see docs/milkdrop.md)",
            ));
        };
        // SAFETY: the instance is valid; the function only stores the value.
        unsafe { set_frame_time(self.instance, seconds.max(0.0)) };
        Ok(())
    }

    /// Gives projectM the audio to react to: stereo samples in interleaved
    /// order (left, right, left, right, ...).
    ///
    /// projectM keeps only the most recent fraction of a second, so before
    /// each frame pass the audio that leads up to that frame's time.
    pub fn add_stereo_audio(&mut self, interleaved_samples: &[f32]) {
        // projectM accepts a limited number of frames per call.
        for chunk in interleaved_samples.chunks(self.max_samples_per_call * 2) {
            // SAFETY: the pointer and frame count describe `chunk` exactly.
            unsafe {
                (self.api.pcm_add_float)(
                    self.instance,
                    chunk.as_ptr(),
                    (chunk.len() / 2) as u32,
                    ffi::PROJECTM_STEREO,
                );
            }
        }
    }

    /// Draws the next frame and copies it into `pixels` as RGBA bytes, top
    /// row first (`width * height * 4` bytes).
    pub fn render_frame(&mut self, pixels: &mut Vec<u8>) {
        // SAFETY: the instance is valid and its OpenGL context is current
        // on this thread (guaranteed by the type not being sendable).
        unsafe { (self.api.opengl_render_frame)(self.instance) };
        self.gl_context.read_pixels(pixels);
    }
}

impl Drop for MilkdropRenderer {
    fn drop(&mut self) {
        // SAFETY: the instance came from `projectm_create`, its OpenGL
        // context is still current (the `gl_context` field is dropped after
        // this runs), and it is not used again.
        unsafe { (self.api.destroy)(self.instance) };
    }
}

fn read_version(api: &Api) -> String {
    // SAFETY: the function returns a zero-terminated string that must be
    // released with `projectm_free_string`, which is done right after
    // copying it.
    unsafe {
        let text = (api.get_version_string)();
        if text.is_null() {
            return "unknown".to_owned();
        }
        let version = CStr::from_ptr(text).to_string_lossy().into_owned();
        (api.free_string)(text);
        version
    }
}

/// Called by projectM when a preset cannot be loaded. Stores the message
/// where `load_preset` picks it up.
unsafe extern "C" fn preset_switch_failed(
    _preset_filename: *const c_char,
    message: *const c_char,
    user_data: *mut c_void,
) {
    if user_data.is_null() {
        return;
    }
    let text = if message.is_null() {
        "unknown error".to_owned()
    } else {
        // SAFETY: projectM passes a valid zero-terminated string.
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    };
    // SAFETY: `user_data` is the pointer to the renderer's boxed
    // `load_error`, registered in `new` and valid while the renderer lives.
    unsafe { *user_data.cast::<Option<String>>() = Some(text) };
}
