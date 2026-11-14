//! Placeholder for platforms where the off-screen OpenGL context has not
//! been written yet (Linux will use EGL). It has the same interface as the
//! real one, so the rest of the crate compiles, but creating it fails.

pub struct OffscreenGlContext {
    // No value of this type can exist: `new` always fails.
    never: std::convert::Infallible,
}

impl OffscreenGlContext {
    pub fn new(_width: u32, _height: u32) -> Result<Self, String> {
        Err("MilkDrop rendering is not supported on this operating system yet".to_owned())
    }

    pub fn description(&self) -> String {
        match self.never {}
    }

    pub fn read_pixels(&self, _pixels: &mut Vec<u8>) {
        match self.never {}
    }
}
