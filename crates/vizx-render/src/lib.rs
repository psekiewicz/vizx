//! The VizX rendering engine.
//!
//! This crate draws with the graphics card, without a window:
//!
//! - [`open_device`] connects to a graphics card,
//! - [`Compositor`] lays pictures (layers) over each other into one frame.
//!
//! The preview and the video export both build their frames with the same
//! [`Compositor`], so what is exported is what the preview shows. The crate
//! knows nothing about projects, timelines or the GUI: it is given pixels
//! and where to put them.

mod compositor;
mod gpu;

pub use compositor::{Compositor, LayerDraw};
pub use gpu::{open_device, read_back_rgba};
