//! Visualizer plugins for VizX.
//!
//! A plugin is a folder with two files:
//!
//! - `visualizer.json`: its name and the parameters the user can adjust,
//! - `shader.wgsl`: a function `visualize(uv)` that returns the color of
//!   one pixel.
//!
//! VizX runs that function for every pixel of every frame on the graphics
//! card and tells it about the sound: the spectrum, the waveform, the recent
//! history of the spectrum, overall levels, the time, and the previous
//! frame. That is enough to build spectrum bars, oscilloscopes, waterfalls,
//! pulsing shapes, feedback effects and so on, without touching VizX itself.
//!
//! This crate loads plugins ([`VisualizerPlugin`]) and renders them to
//! pixels ([`ShaderRenderer`]). It knows nothing about the project, the
//! timeline or the GUI. See `docs/visualizer-plugins.md` for the plugin
//! author's guide.

mod plugin;
mod renderer;

pub use plugin::{
    MANIFEST_FILE_NAME, Manifest, PARAMETER_SLOTS, ParameterKind, ParameterSpec, ParameterValues,
    SHADER_FILE_NAME, VisualizerPlugin,
};
pub use renderer::{FrameInput, HISTORY_ROWS, ShaderRenderer};
