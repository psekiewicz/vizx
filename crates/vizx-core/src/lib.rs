//! VizX shared application logic.
//!
//! This crate sits between the project data (`vizx-project`) and the user
//! interface (`vizx-app`). It owns the editing state that every panel looks
//! at, and it is where future subsystems (timeline, audio, composition,
//! rendering) will be coordinated. It must never depend on a GUI library.

mod animation;
mod app_data;
mod audio_store;
mod composition;
mod export;
mod ffmpeg;
mod history;
mod image_store;
mod lyrics;
mod prerender;
mod preset_library;
mod selection;
mod session;
mod stems;
mod text;
mod thumbnail;
mod visualizer_catalog;

pub use animation::{AnimationReader, find_image_sequence, find_row_lengths, guess_sprite_sheet};
pub use app_data::{
    AUTOSAVE_INTERVAL, AppSettings, Autosaver, Recovery, app_data_dir, find_recoveries,
};
pub use audio_store::{AudioState, AudioStore, LoadedAudio};
pub use composition::{
    LayerGeometry, ReactionMeter, audio_sources, beat_count, beat_pulse, beat_times, frame_source,
    last_beat, layer_effects, layer_geometry, tempo_of,
};
pub use export::{
    Export, ExportContent, ExportJob, ExportLayer, ExportQuality, ExportSettings, VideoFormat,
    layer_blend,
};
pub use ffmpeg::{
    FFMPEG_DIR_VARIABLE, SUPPORTED_VIDEO_EXTENSIONS, VideoReader, VideoWriter, extract_video_sound,
    find_ffmpeg, has_supported_video_extension, probe_video,
};
pub use image_store::{
    DecodedImage, ImageState, ImageStore, SUPPORTED_IMAGE_EXTENSIONS, has_supported_image_extension,
};
pub use lyrics::{
    LyricLine, LyricsStyle, lines_on_beats, lyric_keyframes, parse_timed_lyrics, split_lyrics,
    spread_lines,
};
pub use prerender::{FrameSource, Prerender, PrerenderJob, PrerenderedFrame};
pub use preset_library::{
    BUNDLED_PRESET_PREFIX, PresetEntry, PresetLibrary, find_bundled_presets, resolve_preset_path,
};
pub use selection::Selection;
pub use session::{CopiedClip, EditorSession, UNTITLED_PROJECT_NAME};
pub use stems::{STEM_NAMES, STEMS_DIR_VARIABLE, StemEngine, StemFiles, StemJob, stem_folder};
pub use text::render_text;
pub use thumbnail::{THUMBNAIL_EDGE, make_thumbnail};
pub use visualizer_catalog::{
    BUNDLED_VISUALIZER_PREFIX, BundledVisualizer, PluginState, VisualizerCatalog,
};
// The plugin description types, so the user interface can show a plugin's
// parameters without depending on the rendering crate itself.
pub use vizx_visualizer::{Manifest, ParameterKind, ParameterSpec, ParameterValues};
// The compositor, so the preview can use it without depending on the
// rendering crate itself.
pub use vizx_render::{Compositor, LayerBlend, LayerDraw, LayerEffects};

/// Name of the application as shown to the user.
pub const APP_NAME: &str = "VizX (Prototype)";

/// Version of the application, taken from the workspace `Cargo.toml`.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
