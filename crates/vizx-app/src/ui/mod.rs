//! User interface of VizX.
//!
//! Each submodule draws one region of the window. They all read and modify
//! the same application state, so they are views of one editor rather than
//! independent tools.

pub mod about_window;
pub mod menu_bar;
pub mod panels;
pub mod project_settings_window;
pub mod status_bar;
pub mod toolbar;
pub mod unsaved_changes_dialog;
pub mod workspace;

/// Formats a length of time as `m:ss`, e.g. `3:35`.
pub fn format_duration(milliseconds: u64) -> String {
    let total_seconds = (milliseconds + 500) / 1000;
    format!("{}:{:02}", total_seconds / 60, total_seconds % 60)
}

/// Tooltip for controls whose feature belongs to a later milestone.
pub const NOT_AVAILABLE_YET: &str = "Not available yet in this prototype";
