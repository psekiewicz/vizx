//! The VizX installer: one setup program that carries the whole application
//! (VizX, its tools, presets, visualizers and models) and installs it for
//! the current user, with shortcuts and an entry in "Apps & features".
//!
//! - [`payload`]: how the files are stored at the end of the setup program.
//! - [`install`]: putting them into a folder, and removing them again.
//!
//! The installer is built by `scripts/build-installer.ps1`.

pub mod install;
pub mod payload;
