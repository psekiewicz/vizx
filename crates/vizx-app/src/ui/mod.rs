//! User interface of VizX.
//!
//! Each submodule draws one region of the window. They all read and modify
//! the same application state, so they are views of one editor rather than
//! independent tools.

pub mod about_window;
pub mod menu_bar;
pub mod panels;
pub mod status_bar;
pub mod toolbar;
pub mod workspace;

/// Tooltip for controls whose feature belongs to a later milestone.
pub const NOT_AVAILABLE_YET: &str = "Not available yet in this prototype";
