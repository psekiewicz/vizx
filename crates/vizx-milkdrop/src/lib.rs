//! MilkDrop preset rendering for VizX.
//!
//! MilkDrop is the classic music visualizer from Winamp. Its presets
//! (`.milk` files) are small programs that describe a visual. VizX does not
//! reimplement them: it uses **projectM**, the open-source library that
//! plays original MilkDrop presets, so they look the way their authors
//! made them.
//!
//! projectM is a C++ library that draws with OpenGL. This crate hides both
//! facts behind [`MilkdropRenderer`]: give it a preset and audio, get back
//! frames as plain RGBA pixels. Nothing outside this crate touches OpenGL
//! or C.
//!
//! projectM is licensed under the LGPL. It is a separate file that VizX
//! loads while running, so VizX builds and starts without it;
//! [`MilkdropRenderer::new`] then returns an error saying what is missing.
//! See `docs/milkdrop.md` for how to set it up.

use std::fmt;

mod ffi;
// The off-screen OpenGL context is the only platform-specific part.
#[cfg_attr(windows, path = "gl_context_windows.rs")]
#[cfg_attr(not(windows), path = "gl_context_unsupported.rs")]
mod gl_context;
mod renderer;

pub use ffi::PROJECTM_DIR_VARIABLE;
pub use renderer::MilkdropRenderer;

/// Why a MilkDrop operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MilkdropError(String);

impl MilkdropError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for MilkdropError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "MilkDrop: {}", self.0)
    }
}

impl std::error::Error for MilkdropError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_sized_pictures_are_rejected_before_anything_is_loaded() {
        let error = MilkdropRenderer::new(0, 360, 30).err().unwrap();

        assert!(error.to_string().contains("size"), "{error}");
    }
}
