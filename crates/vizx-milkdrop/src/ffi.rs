//! The projectM C functions VizX calls, loaded from the projectM DLL while
//! VizX is running.
//!
//! "FFI" (foreign function interface) is how Rust calls code written in
//! another language. projectM is loaded at run time rather than linked when
//! VizX is built, for two reasons: VizX then builds and starts on a machine
//! without projectM (MilkDrop is simply reported as unavailable), and
//! projectM stays a separate, replaceable file, which is what its LGPL
//! license asks for.
//!
//! The compiler cannot check these signatures against the real library, so
//! each one is copied by hand from projectM's public headers
//! (`projectM-4/*.h`, version 4.1) and must match them exactly. Everything
//! here is `unsafe` to call; the safe wrapper is in `renderer.rs`.

use std::env;
use std::ffi::{c_char, c_double, c_float, c_int, c_uint, c_void};
use std::path::{Path, PathBuf};

use libloading::Library;

use crate::MilkdropError;

/// An opaque projectM instance (`projectm_handle` in C).
#[repr(C)]
pub struct ProjectM {
    _private: [u8; 0],
}

/// `PROJECTM_STEREO` from `projectm_channels`.
pub const PROJECTM_STEREO: c_int = 2;

/// Called by projectM when a preset could not be loaded.
pub type PresetSwitchFailedCallback = unsafe extern "C" fn(
    preset_filename: *const c_char,
    message: *const c_char,
    user_data: *mut c_void,
);

/// Environment variable naming projectM's install folder (the one that
/// contains `bin/` or `lib/`). Used during development; an installed VizX
/// finds the library next to its executable instead.
pub const PROJECTM_DIR_VARIABLE: &str = "VIZX_PROJECTM_DIR";

#[cfg(windows)]
const LIBRARY_FILE_NAME: &str = "projectM-4.dll";
#[cfg(not(windows))]
const LIBRARY_FILE_NAME: &str = "libprojectM-4.so";

/// The loaded projectM library and the functions VizX uses from it.
///
/// The function pointers stay valid for as long as `_library` is loaded,
/// which is as long as this struct exists.
pub struct Api {
    pub create: unsafe extern "C" fn() -> *mut ProjectM,
    pub destroy: unsafe extern "C" fn(*mut ProjectM),
    pub get_version_string: unsafe extern "C" fn() -> *mut c_char,
    pub free_string: unsafe extern "C" fn(*const c_char),
    pub load_preset_file: unsafe extern "C" fn(*mut ProjectM, *const c_char, bool),
    pub set_preset_switch_failed_event_callback:
        unsafe extern "C" fn(*mut ProjectM, Option<PresetSwitchFailedCallback>, *mut c_void),
    pub set_window_size: unsafe extern "C" fn(*mut ProjectM, usize, usize),
    pub set_fps: unsafe extern "C" fn(*mut ProjectM, i32),
    pub set_preset_locked: unsafe extern "C" fn(*mut ProjectM, bool),
    pub set_hard_cut_enabled: unsafe extern "C" fn(*mut ProjectM, bool),
    pub pcm_get_max_samples: unsafe extern "C" fn() -> c_uint,
    pub pcm_add_float: unsafe extern "C" fn(*mut ProjectM, *const c_float, c_uint, c_int),
    pub opengl_render_frame: unsafe extern "C" fn(*mut ProjectM),
    /// `projectm_set_frame_time`: sets the time of the next frame instead of
    /// letting projectM read the system clock. Not part of projectM 4.1; it
    /// is added by the patch in `patches/`. `None` if the loaded library
    /// does not have it.
    pub set_frame_time: Option<unsafe extern "C" fn(*mut ProjectM, c_double)>,
    /// GLEW, the helper library projectM uses on Windows to reach OpenGL.
    /// `None` on systems where projectM does not need it.
    glew: Option<Glew>,
    _library: Library,
}

/// The GLEW library and its initialization function.
struct Glew {
    /// `glewInit`: looks up every OpenGL function of the current context.
    /// Returns 0 on success.
    init: unsafe extern "system" fn() -> c_uint,
    _library: Library,
}

impl Api {
    /// Finds and loads the projectM library.
    pub fn load() -> Result<Self, MilkdropError> {
        let candidates = candidate_paths();
        let Some(path) = candidates.iter().find(|path| path.is_file()) else {
            let searched: Vec<String> = candidates
                .iter()
                .map(|path| path.display().to_string())
                .collect();
            return Err(MilkdropError::new(format!(
                "the projectM library was not found (looked for {}); see docs/milkdrop.md",
                searched.join(", ")
            )));
        };

        let library = open_library(path).map_err(|error| {
            MilkdropError::new(format!("could not load {}: {error}", path.display()))
        })?;

        // Looks up one function by its C name and copies out the pointer.
        macro_rules! function {
            ($name:literal) => {
                // SAFETY: the type the pointer is stored as (the struct
                // field) matches the function's declaration in projectM's
                // headers.
                *unsafe { library.get(concat!($name, "\0").as_bytes()) }.map_err(|error| {
                    MilkdropError::new(format!(
                        "{} has no function {}: {error}",
                        path.display(),
                        $name
                    ))
                })?
            };
        }

        Ok(Self {
            create: function!("projectm_create"),
            destroy: function!("projectm_destroy"),
            get_version_string: function!("projectm_get_version_string"),
            free_string: function!("projectm_free_string"),
            load_preset_file: function!("projectm_load_preset_file"),
            set_preset_switch_failed_event_callback: function!(
                "projectm_set_preset_switch_failed_event_callback"
            ),
            set_window_size: function!("projectm_set_window_size"),
            set_fps: function!("projectm_set_fps"),
            set_preset_locked: function!("projectm_set_preset_locked"),
            set_hard_cut_enabled: function!("projectm_set_hard_cut_enabled"),
            pcm_get_max_samples: function!("projectm_pcm_get_max_samples"),
            pcm_add_float: function!("projectm_pcm_add_float"),
            opengl_render_frame: function!("projectm_opengl_render_frame"),
            // SAFETY: the signature matches the declaration added by the patch.
            set_frame_time: unsafe { library.get(b"projectm_set_frame_time\0") }
                .ok()
                .map(|symbol| *symbol),
            glew: load_glew(path)?,
            _library: library,
        })
    }

    /// Prepares OpenGL for projectM. Must be called once, on the thread
    /// whose OpenGL context projectM will use, before `projectm_create`.
    ///
    /// On Windows, projectM reaches OpenGL through GLEW's table of function
    /// pointers, and it is the host program's job to fill that table.
    /// Skipping this leaves the table empty and projectM crashes on its
    /// first OpenGL call.
    pub fn initialize_opengl(&self) -> Result<(), MilkdropError> {
        let Some(glew) = &self.glew else {
            return Ok(());
        };
        // SAFETY: an OpenGL context is current on this thread, which is all
        // `glewInit` requires.
        match unsafe { (glew.init)() } {
            0 => Ok(()),
            code => Err(MilkdropError::new(format!(
                "GLEW could not initialize OpenGL (error {code})"
            ))),
        }
    }
}

/// Loads the copy of GLEW that sits next to projectM, which is the one
/// projectM itself uses.
#[cfg(windows)]
fn load_glew(projectm_path: &Path) -> Result<Option<Glew>, MilkdropError> {
    let path = projectm_path.with_file_name("glew32.dll");
    let library = open_library(&path).map_err(|error| {
        MilkdropError::new(format!("could not load {}: {error}", path.display()))
    })?;
    // SAFETY: `glewInit` takes no arguments and returns a GLenum (u32).
    let init = *unsafe { library.get(b"glewInit\0") }.map_err(|error| {
        MilkdropError::new(format!(
            "{} has no function glewInit: {error}",
            path.display()
        ))
    })?;
    Ok(Some(Glew {
        init,
        _library: library,
    }))
}

/// Elsewhere projectM calls the system's OpenGL directly.
#[cfg(not(windows))]
fn load_glew(_projectm_path: &Path) -> Result<Option<Glew>, MilkdropError> {
    Ok(None)
}

/// Places the library may be, in the order they are tried.
fn candidate_paths() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    // 1. Next to the VizX executable: how an installed VizX finds it.
    if let Some(executable_folder) = env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_owned))
    {
        candidates.push(executable_folder.join(LIBRARY_FILE_NAME));
    }

    // 2. The folder projectM was installed to when it was built: used
    //    while developing VizX.
    if let Some(projectm_dir) = env::var_os(PROJECTM_DIR_VARIABLE).map(PathBuf::from) {
        candidates.push(projectm_dir.join("bin").join(LIBRARY_FILE_NAME));
        candidates.push(projectm_dir.join("lib").join(LIBRARY_FILE_NAME));
    }

    candidates
}

#[cfg(windows)]
fn open_library(path: &Path) -> Result<Library, libloading::Error> {
    use libloading::os::windows::{LOAD_WITH_ALTERED_SEARCH_PATH, Library as WindowsLibrary};

    // With this flag Windows looks for the DLLs that projectM itself needs
    // (GLEW) in projectM's own folder instead of VizX's.
    // SAFETY: loading a library runs its initialization code; projectM is a
    // library VizX was set up to use.
    unsafe { WindowsLibrary::load_with_flags(path, LOAD_WITH_ALTERED_SEARCH_PATH) }
        .map(Library::from)
}

#[cfg(not(windows))]
fn open_library(path: &Path) -> Result<Library, libloading::Error> {
    // SAFETY: as above.
    unsafe { Library::new(path) }
}
