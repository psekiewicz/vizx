//! VizX shared application logic.
//!
//! This crate sits between the project data (`vizx-project`) and the user
//! interface (`vizx-app`). It owns the editing state that every panel looks
//! at, and it is where future subsystems (timeline, audio, composition,
//! rendering) will be coordinated. It must never depend on a GUI library.

mod audio_store;
mod milkdrop_worker;
mod selection;
mod session;

pub use audio_store::{AudioState, AudioStore, LoadedAudio};
pub use milkdrop_worker::{MilkdropFrame, MilkdropFrameRequest, MilkdropWorker};
pub use selection::Selection;
pub use session::{EditorSession, UNTITLED_PROJECT_NAME};

/// Name of the application as shown to the user.
pub const APP_NAME: &str = "VizX (Prototype)";

/// Version of the application, taken from the workspace `Cargo.toml`.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
