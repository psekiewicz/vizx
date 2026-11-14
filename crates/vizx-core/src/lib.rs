//! VizX shared application logic.
//!
//! This crate sits between the project data (`vizx-project`) and the user
//! interface (`vizx-app`). It owns the editing state that every panel looks
//! at, and it is where future subsystems (timeline, audio, composition,
//! rendering) will be coordinated. It must never depend on a GUI library.

mod audio_store;
mod prerender;
mod preset_library;
mod selection;
mod session;
mod visualizer_catalog;

pub use audio_store::{AudioState, AudioStore, LoadedAudio};
pub use prerender::{FrameSource, Prerender, PrerenderJob, PrerenderedFrame};
pub use preset_library::{
    BUNDLED_PRESET_PREFIX, PresetEntry, PresetLibrary, find_bundled_presets, resolve_preset_path,
};
pub use selection::Selection;
pub use session::{EditorSession, UNTITLED_PROJECT_NAME};
pub use visualizer_catalog::{
    BUNDLED_VISUALIZER_PREFIX, BundledVisualizer, PluginState, VisualizerCatalog,
};
// The plugin description types, so the user interface can show a plugin's
// parameters without depending on the rendering crate itself.
pub use vizx_visualizer::{Manifest, ParameterKind, ParameterSpec, ParameterValues};

/// Name of the application as shown to the user.
pub const APP_NAME: &str = "VizX (Prototype)";

/// Version of the application, taken from the workspace `Cargo.toml`.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
