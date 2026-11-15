use std::path::{Path, PathBuf};

use eframe::egui;
use vizx_core::{AudioStore, EditorSession, Export, ExportJob, ImageStore, VisualizerCatalog};
use vizx_project::{PROJECT_FILE_EXTENSION, TrackKind, with_project_extension};

use crate::theme;
use crate::transport::Transport;
use crate::ui::lyrics_window::{self, LyricsWindow};
use crate::ui::preset_browser::PresetBrowser;
use crate::ui::visualizer_browser::VisualizerBrowser;
use crate::ui::welcome_window::{self, Template};
use crate::ui::workspace::Workspace;
use crate::ui::{
    about_window, export_window, menu_bar, preset_browser, project_settings_window,
    recovery_window, shortcuts_window, status_bar, toolbar, unsaved_changes_dialog,
    visualizer_browser,
};
use crate::visual_preview::VisualPreview;

/// An action the user asked for through a menu, button or shortcut.
///
/// The UI code never performs these itself. It only calls
/// [`VizxApp::request`], and the command is executed once per frame in
/// [`VizxApp::run_requested_command`]. This keeps "what the user asked for"
/// separate from "how it is done", and gives every way of triggering a
/// command (menu, toolbar, keyboard) exactly the same behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppCommand {
    NewProject,
    OpenProject,
    /// Start a new project from a ready-made template.
    StartFromTemplate(Template),
    /// Show the welcome window again.
    ShowWelcome,
    /// Open one of the recently used projects (its place in the list).
    OpenRecent(usize),
    SaveProject,
    SaveProjectAs,
    ImportAudio,
    ImportImage,
    /// Pick an image and put it behind everything else, covering the frame.
    ImportBackgroundImage,
    /// Pick a video file and put its picture on a layer of its own.
    ImportVideo,
    /// Pick one file of a numbered image sequence.
    ImportImageSequence,
    /// Pick an image holding the frames of an animation in a grid.
    ImportSpriteSheet,
    /// Pick a video file and put it behind everything else.
    ImportBackgroundVideo,
    /// Write the project as a video file.
    ExportVideo,
    /// Add a text element in the middle of the picture.
    AddText,
    /// Open the Lyrics window, to turn the words of a song into subtitles.
    OpenLyrics,
    /// Pick an .lrc or .srt file and add its timed lines as subtitles.
    ImportLyricsFile,
    /// Split the selected song clip into stems (vocals, drums, bass, other).
    SeparateStems,
    /// Pick a font file for the text on the selected layer.
    ChooseTextFont,
    /// Cut the selected clip in two at the playhead.
    SplitClip,
    /// Put a copy of the selected clip right after it.
    DuplicateClip,
    /// Remove the selected clip, or the selected track.
    DeleteSelection,
    Undo,
    Redo,
    /// Put the selected clip on VizX's clipboard.
    CopyClip,
    /// Copy the selected clip, then delete it.
    CutClip,
    /// Paste the clip on the clipboard at the playhead.
    PasteClip,
    /// Move the playhead by this many frames (negative: back).
    MovePlayheadFrames(i32),
    /// Move the playhead by this many seconds (negative: back).
    MovePlayheadSeconds(i32),
    /// Move the playhead to the previous or next clip start or end.
    PreviousClipEdge,
    NextClipEdge,
    /// Show the list of keyboard shortcuts.
    ShowShortcuts,
    /// Place a beat at the playhead on the selected audio clip.
    TapBeat,
    /// Open the browser for the presets that ship with VizX.
    BrowseMilkdropPresets,
    /// Pick a preset file of the user's own.
    AddMilkdropPreset,
    /// Open the browser for visualizer plugins.
    BrowseVisualizers,
    /// Pick the folder of a visualizer plugin of the user's own.
    AddVisualizerFolder,
    AddVisualLayer,
    AddAudioTrack,
    OpenProjectSettings,
    /// Start playback, or pause it if it is running.
    TogglePlayback,
    /// Pause and return to the start of the project.
    StopPlayback,
    GoToStart,
    GoToEnd,
    Exit,
}

impl AppCommand {
    /// Whether the command throws away the currently open project, in which
    /// case unsaved changes must be confirmed first.
    fn discards_current_project(self) -> bool {
        matches!(
            self,
            Self::NewProject
                | Self::OpenProject
                | Self::OpenRecent(_)
                | Self::StartFromTemplate(_)
                | Self::Exit
        )
    }
}

/// Top-level application state.
///
/// `session` is the shared editing state from `vizx-core`; everything else
/// here is purely about how the UI is currently arranged.
pub struct VizxApp {
    pub session: EditorSession,
    pub workspace: Workspace,
    /// Decoded audio of the project's files, loaded in the background.
    audio_store: AudioStore,
    /// Decoded images of the project's files, loaded in the background.
    image_store: ImageStore,
    transport: Transport,
    visual_preview: VisualPreview,
    /// Folder of the presets that ship with VizX, if there is one.
    bundled_presets: Option<PathBuf>,
    /// The FFmpeg program, if it was found: needed for video files and for
    /// export.
    ffmpeg: Option<PathBuf>,
    /// The video export that is running or has just ended.
    export: Option<Export>,
    pub preset_browser: PresetBrowser,
    pub visualizer_browser: VisualizerBrowser,
    /// The visualizer plugins that ship with VizX and the ones in use.
    pub visualizer_catalog: VisualizerCatalog,
    pub is_about_window_open: bool,
    pub is_project_settings_open: bool,
    pub lyrics: LyricsWindow,
    /// VizX's own folder for settings and autosaves, if the system has one.
    app_data: Option<PathBuf>,
    /// Settings of VizX itself, such as the recent projects.
    pub settings: vizx_core::AppSettings,
    /// Keeps an autosaved copy of the project while it has unsaved changes.
    autosaver: Option<vizx_core::Autosaver>,
    /// Autosaved projects of a VizX that did not close properly, offered
    /// for recovery at startup.
    pub recoveries: Vec<vizx_core::Recovery>,
    /// The clip copied last.
    clipboard: Option<vizx_core::CopiedClip>,
    pub is_shortcuts_open: bool,
    /// The welcome window shown at startup.
    pub is_welcome_open: bool,
    /// The stem separation running in the background, if any.
    stem_job: Option<vizx_core::StemJob>,
    /// Description of the graphics adapter wgpu selected, for diagnostics.
    pub graphics_summary: String,
    /// Result of the last command, shown in the status bar.
    pub status_message: String,
    /// Command waiting to be executed at the end of the current frame.
    requested_command: Option<AppCommand>,
    /// Command that is on hold while the user decides what to do with
    /// unsaved changes. `Some` means the confirmation dialog is open.
    command_awaiting_confirmation: Option<AppCommand>,
    /// Set once the user has agreed to exit, so the close is not questioned
    /// a second time.
    is_exit_confirmed: bool,
    /// Last title sent to the OS window, so it is only updated on change.
    window_title: String,
    has_maximized_on_startup: bool,
}

impl VizxApp {
    pub fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&creation_context.egui_ctx);
        // Looked up once: both the preset browser and the preview need it.
        let bundled_presets = vizx_core::find_bundled_presets();
        let ffmpeg = vizx_core::find_ffmpeg();
        let app_data = vizx_core::app_data_dir();

        let mut app = Self {
            session: EditorSession::new_untitled(),
            workspace: Workspace::new(),
            audio_store: AudioStore::default(),
            image_store: ImageStore::default(),
            transport: Transport::default(),
            visual_preview: VisualPreview::new(bundled_presets.clone(), ffmpeg.clone()),
            bundled_presets: bundled_presets.clone(),
            ffmpeg,
            export: None,
            preset_browser: PresetBrowser::new(bundled_presets),
            visualizer_browser: VisualizerBrowser::default(),
            visualizer_catalog: VisualizerCatalog::new(),
            is_about_window_open: false,
            is_project_settings_open: false,
            lyrics: LyricsWindow::default(),
            stem_job: None,
            settings: app_data
                .as_deref()
                .map(vizx_core::AppSettings::load)
                .unwrap_or_default(),
            autosaver: app_data.as_deref().map(vizx_core::Autosaver::new),
            recoveries: app_data
                .as_deref()
                .map(vizx_core::find_recoveries)
                .unwrap_or_default(),
            app_data,
            clipboard: None,
            is_shortcuts_open: false,
            graphics_summary: describe_graphics_adapter(creation_context),
            status_message: "Ready".to_owned(),
            requested_command: None,
            command_awaiting_confirmation: None,
            is_exit_confirmed: false,
            window_title: String::new(),
            has_maximized_on_startup: false,
            is_welcome_open: false,
        };
        if let Some(layout) = app.settings.panel_layout.clone() {
            app.workspace.restore_layout(&layout);
        }
        // Like Blender's splash screen; not over the offer to recover work.
        app.is_welcome_open = app.settings.show_welcome && app.recoveries.is_empty();
        app
    }

    /// Pre-render progress of the MilkDrop clips, for the status bar.
    pub fn prerender_progress(&self) -> Option<crate::visual_preview::PrerenderProgress> {
        self.visual_preview.progress()
    }

    /// The video export that is running or has just ended, if any.
    pub fn export(&self) -> Option<&Export> {
        self.export.as_ref()
    }

    /// Forgets a finished export (or stops a running one).
    pub fn close_export(&mut self) {
        self.export = None;
    }

    /// The FFmpeg program, or a status message explaining that it is
    /// missing.
    fn require_ffmpeg(&mut self, purpose: &str) -> Option<PathBuf> {
        if self.ffmpeg.is_none() {
            self.status_message = format!(
                "{purpose} needs FFmpeg, which was not found. Put ffmpeg next to VizX or \
                 install it."
            );
        }
        self.ffmpeg.clone()
    }

    /// Lets the user pick a video file and adds its picture to the project.
    fn import_video(&mut self, frame: &eframe::Frame, as_background: bool) {
        let Some(ffmpeg) = self.require_ffmpeg("A video file") else {
            return;
        };
        let Some(path) = rfd::FileDialog::new()
            .add_filter(
                "Video or animated GIF",
                &vizx_core::SUPPORTED_VIDEO_EXTENSIONS,
            )
            .set_parent(frame)
            .pick_file()
        else {
            return; // dialog cancelled
        };

        self.status_message = match self.session.import_video(&path, &ffmpeg, as_background) {
            Ok(_) => format!("Added video {}", path.display()),
            Err(problem) => format!("Could not import {}: {problem}", path.display()),
        };
    }

    /// Lets the user pick a font file for the text on the selected layer.
    fn choose_text_font(&mut self, frame: &eframe::Frame) {
        // The text on the selected layer, with its media id.
        let selected_text = self.session.selected_track().and_then(|track| {
            let selected_clip = self.session.selected_clip_id();
            let clips = track
                .clips
                .iter()
                .filter(|clip| selected_clip.is_none_or(|id| id == clip.id));
            clips.into_iter().find_map(|clip| {
                let source = self.session.project().media_source(clip.media)?;
                Some((source.id, source.text()?.clone()))
            })
        });
        let Some((media_id, mut text)) = selected_text else {
            self.status_message = "Select a layer with a text first".to_owned();
            return;
        };
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Font", &["ttf", "otf", "ttc"])
            .set_parent(frame)
            .pick_file()
        else {
            return; // dialog cancelled
        };

        text.font_path = path.to_string_lossy().into_owned();
        self.status_message = match self.session.set_text(media_id, text) {
            Ok(()) => format!("Using font {}", path.display()),
            Err(problem) => format!("Could not use that font: {problem}"),
        };
    }

    /// Starts separating the selected song clip into stems.
    fn separate_stems(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.stem_job {
            self.status_message = format!(
                "Still separating {}; wait for it to finish",
                job.song_name()
            );
            return;
        }
        let Some(clip_id) = self.session.selected_clip_id() else {
            self.status_message = "Select a song clip to separate".to_owned();
            return;
        };
        let Some((_, clip)) = self.session.project().clip(clip_id) else {
            return;
        };
        let Some(source) = self
            .session
            .project()
            .media_source(clip.media)
            .filter(|source| source.audio().is_some())
        else {
            self.status_message = "Only audio can be separated into stems".to_owned();
            return;
        };
        let Some(loaded) = self.audio_store.loaded(&source.path) else {
            self.status_message = "The song is still loading; try again in a moment".to_owned();
            return;
        };
        let ctx = ctx.clone();
        let job = vizx_core::StemJob::start(
            clip_id,
            std::sync::Arc::clone(&loaded.audio),
            Path::new(&source.path),
            vizx_core::StemEngine::best_available(),
            move || ctx.request_repaint(),
        );
        self.status_message = format!(
            "Separating {} into stems with {}...",
            job.song_name(),
            job.engine_label()
        );
        self.stem_job = Some(job);
    }

    /// Adds the stems once their separation has finished.
    fn collect_stems(&mut self) {
        let Some(job) = &self.stem_job else {
            return;
        };
        let Some(outcome) = job.finished() else {
            self.status_message = format!(
                "Separating {} into stems with {}: {:.0} %",
                job.song_name(),
                job.engine_label(),
                job.progress() * 100.0
            );
            return;
        };
        let (clip_id, song) = (job.clip_id(), job.song_name().to_owned());
        self.stem_job = None;
        self.status_message = match outcome
            .and_then(|stems| self.session.add_stems(clip_id, &stems))
        {
            Ok(tracks) => format!(
                "Separated {song} into {} stems on muted tracks; choose one under \"Reacts to\" of a visualizer",
                tracks.len()
            ),
            Err(problem) => format!("Could not separate {song}: {problem}"),
        };
    }

    /// The beats of all the project's audio, in seconds of project time.
    pub fn mix_beat_seconds(&self) -> Vec<f64> {
        let (beats, _) = vizx_core::beat_times(
            self.session.project(),
            &self.audio_store,
            vizx_project::AudioInput::Mix,
        );
        beats.iter().map(|beat| beat.seconds).collect()
    }

    /// Lets the user pick an .lrc or .srt file and adds its lines.
    fn import_lyrics_file(&mut self, frame: &eframe::Frame) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Timed lyrics or subtitles", &["lrc", "srt"])
            .set_parent(frame)
            .pick_file()
        else {
            return; // dialog cancelled
        };
        let lines = std::fs::read(&path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| vizx_core::parse_timed_lyrics(&String::from_utf8_lossy(&bytes)));
        match lines {
            Ok(lines) => {
                lyrics_window::add_timed_lines(self, &lines);
                self.status_message = format!("Added the lyrics of {}", path.display());
            }
            Err(problem) => {
                self.status_message = format!("Could not read {}: {problem}", path.display());
            }
        }
    }

    /// Lets the user pick a file of an image sequence (any of its frames)
    /// or a sprite sheet, and adds it as a frame animation.
    fn import_animation(&mut self, frame: &eframe::Frame, is_sequence: bool) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(if is_sequence {
                "Pick any frame of the image sequence"
            } else {
                "Pick the sprite sheet"
            })
            .add_filter("Image", &vizx_core::SUPPORTED_IMAGE_EXTENSIONS)
            .set_parent(frame)
            .pick_file()
        else {
            return; // dialog cancelled
        };
        let result = if is_sequence {
            self.session.import_image_sequence(&path)
        } else {
            self.session.import_sprite_sheet(&path)
        };
        self.status_message = match result {
            Ok(_) if is_sequence => format!("Added image sequence {}", path.display()),
            Ok(_) => format!(
                "Added sprite sheet {}; set its grid in Properties",
                path.display()
            ),
            Err(problem) => format!("Could not import {}: {problem}", path.display()),
        };
    }

    /// Asks where to save the video and starts exporting in the background.
    fn export_video(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if self
            .export
            .as_ref()
            .is_some_and(|export| export.result().is_none())
        {
            self.status_message = "An export is already running".to_owned();
            return;
        }
        let Some(ffmpeg) = self.require_ffmpeg("Export") else {
            return;
        };
        let Some(path) = rfd::FileDialog::new()
            .add_filter("MP4 video", &["mp4"])
            .set_file_name(format!("{}.mp4", self.session.project_name()))
            .set_parent(frame)
            .save_file()
        else {
            return; // dialog cancelled
        };
        self.start_export(ctx, &ffmpeg, &with_mp4_extension(path));
    }

    fn start_export(&mut self, ctx: &egui::Context, ffmpeg: &Path, output: &Path) {
        let job = ExportJob::from_project(
            self.session.project(),
            &self.audio_store,
            &mut self.visualizer_catalog,
            self.bundled_presets.as_deref(),
            ffmpeg,
            output,
        );
        match job {
            Ok(job) => {
                let ctx = ctx.clone();
                // Called on the export thread, so the progress bar moves.
                self.export = Some(Export::start(job, move || ctx.request_repaint()));
                self.status_message = format!("Exporting {}", output.display());
            }
            Err(problem) => self.status_message = format!("Cannot export: {problem}"),
        }
    }

    pub fn is_playing(&self) -> bool {
        self.transport.is_playing()
    }

    fn toggle_playback(&mut self) {
        if self.transport.is_playing() {
            self.transport.pause();
            self.status_message = "Paused".to_owned();
            return;
        }
        self.status_message = match self.transport.play(&mut self.session, &self.audio_store) {
            Ok(note) => note,
            Err(error) => format!("Cannot play: {error}"),
        };
    }

    /// Asks for a command to be executed at the end of this frame.
    pub fn request(&mut self, command: AppCommand) {
        self.requested_command = Some(command);
    }

    /// The command currently waiting for the unsaved-changes decision.
    pub fn command_awaiting_confirmation(&self) -> Option<AppCommand> {
        self.command_awaiting_confirmation
    }

    /// Called by the unsaved-changes dialog with the user's decision.
    pub fn resolve_unsaved_changes(
        &mut self,
        decision: unsaved_changes_dialog::Decision,
        ctx: &egui::Context,
        frame: &eframe::Frame,
    ) {
        let Some(command) = self.command_awaiting_confirmation.take() else {
            return;
        };

        match decision {
            unsaved_changes_dialog::Decision::Save => {
                // Only continue if saving really succeeded. If the user
                // cancels the file dialog, nothing is discarded.
                if self.save_project(frame) {
                    self.execute(command, ctx, frame);
                }
            }
            unsaved_changes_dialog::Decision::Discard => self.execute(command, ctx, frame),
            unsaved_changes_dialog::Decision::Cancel => {}
        }
    }

    fn run_requested_command(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(command) = self.requested_command.take() else {
            return;
        };

        if command.discards_current_project() && self.session.has_unsaved_changes() {
            self.command_awaiting_confirmation = Some(command);
        } else {
            self.execute(command, ctx, frame);
        }
    }

    fn execute(&mut self, command: AppCommand, ctx: &egui::Context, frame: &eframe::Frame) {
        match command {
            AppCommand::NewProject => {
                self.transport.pause();
                self.forget_autosave();
                self.session = EditorSession::new_untitled();
                self.status_message = "New project created".to_owned();
            }
            AppCommand::OpenProject => self.open_project(frame),
            AppCommand::OpenRecent(index) => {
                if let Some(path) = self.settings.recent_projects.get(index).cloned() {
                    self.open_project_at(&path);
                }
            }
            AppCommand::CopyClip => {
                self.clipboard = self.session.copy_selected_clip();
                self.status_message = if self.clipboard.is_some() {
                    "Clip copied; paste it with Ctrl+V at the playhead".to_owned()
                } else {
                    "Select a clip to copy it".to_owned()
                };
            }
            AppCommand::CutClip => match self.session.copy_selected_clip() {
                Some(copied) => {
                    self.clipboard = Some(copied);
                    self.session.delete_selection();
                    self.status_message =
                        "Clip cut; paste it with Ctrl+V at the playhead".to_owned();
                }
                None => self.status_message = "Select a clip to cut it".to_owned(),
            },
            AppCommand::PasteClip => {
                self.status_message = match &self.clipboard {
                    Some(copied) => match self.session.paste_clip(copied) {
                        Some(_) => "Clip pasted at the playhead".to_owned(),
                        None => "Could not paste the clip there".to_owned(),
                    },
                    None => "Nothing to paste; copy a clip first (Ctrl+C)".to_owned(),
                };
            }
            AppCommand::MovePlayheadFrames(frames) => self.session.step_playhead_frames(frames),
            AppCommand::MovePlayheadSeconds(seconds) => {
                let now = self.session.playhead_seconds();
                self.session.set_playhead_seconds(now + f64::from(seconds));
            }
            AppCommand::PreviousClipEdge => {
                self.session.jump_to_clip_edge(false);
            }
            AppCommand::NextClipEdge => {
                self.session.jump_to_clip_edge(true);
            }
            AppCommand::ShowShortcuts => self.is_shortcuts_open = true,
            AppCommand::ShowWelcome => self.is_welcome_open = true,
            AppCommand::StartFromTemplate(template) => self.start_from_template(template, frame),
            AppCommand::SaveProject => {
                self.save_project(frame);
            }
            AppCommand::SaveProjectAs => {
                self.save_project_as(frame);
            }
            AppCommand::ImportAudio => self.import_audio(frame),
            AppCommand::ImportImage => self.import_images(frame),
            AppCommand::ImportBackgroundImage => self.import_background_image(frame),
            AppCommand::ImportVideo => self.import_video(frame, false),
            AppCommand::ImportImageSequence => self.import_animation(frame, true),
            AppCommand::ImportSpriteSheet => self.import_animation(frame, false),
            AppCommand::ImportBackgroundVideo => self.import_video(frame, true),
            AppCommand::ExportVideo => self.export_video(ctx, frame),
            AppCommand::AddText => {
                self.status_message = match self.session.add_text() {
                    Ok(_) => "Added text; edit it in Properties".to_owned(),
                    Err(problem) => format!("Could not add text: {problem}"),
                };
            }
            AppCommand::OpenLyrics => {
                let project_seconds = self.session.project().settings.duration_seconds();
                self.lyrics.open(project_seconds);
            }
            AppCommand::ImportLyricsFile => self.import_lyrics_file(frame),
            AppCommand::SeparateStems => self.separate_stems(ctx),
            AppCommand::ChooseTextFont => self.choose_text_font(frame),
            AppCommand::SplitClip => {
                self.status_message = if self.session.split_selected_clip() {
                    "Clip split at the playhead".to_owned()
                } else {
                    "To split, select a clip and put the playhead inside it".to_owned()
                };
            }
            AppCommand::DuplicateClip => {
                self.status_message = if self.session.duplicate_selected_clip() {
                    "Clip duplicated".to_owned()
                } else {
                    "Select a clip to duplicate it".to_owned()
                };
            }
            AppCommand::TapBeat => {
                if let Err(reason) = self.session.tap_beat() {
                    self.status_message = format!("Cannot tap a beat: {reason}");
                }
            }
            AppCommand::Undo => {
                self.status_message = if self.session.undo() {
                    "Undone".to_owned()
                } else {
                    "Nothing to undo".to_owned()
                };
            }
            AppCommand::Redo => {
                self.status_message = if self.session.redo() {
                    "Redone".to_owned()
                } else {
                    "Nothing to redo".to_owned()
                };
            }
            AppCommand::DeleteSelection => {
                self.status_message = match self.session.delete_selection() {
                    Some(what) => format!("Deleted the {what}"),
                    None => "Nothing is selected".to_owned(),
                };
            }
            AppCommand::BrowseMilkdropPresets => self.preset_browser.open(ctx),
            AppCommand::AddMilkdropPreset => self.add_milkdrop_preset(frame),
            AppCommand::BrowseVisualizers => self.visualizer_browser.is_open = true,
            AppCommand::AddVisualizerFolder => self.add_visualizer_folder(frame),
            AppCommand::AddVisualLayer => {
                self.session.add_track(TrackKind::Visual);
            }
            AppCommand::AddAudioTrack => {
                self.session.add_track(TrackKind::Audio);
            }
            AppCommand::OpenProjectSettings => self.is_project_settings_open = true,
            AppCommand::TogglePlayback => self.toggle_playback(),
            AppCommand::StopPlayback => {
                self.transport.pause();
                self.session.set_playhead_seconds(0.0);
                self.status_message = "Stopped".to_owned();
            }
            AppCommand::GoToStart => self.session.set_playhead_seconds(0.0),
            AppCommand::GoToEnd => {
                let project_end = self.session.project().settings.duration_seconds();
                self.session.set_playhead_seconds(project_end);
            }
            AppCommand::Exit => {
                self.is_exit_confirmed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn open_project(&mut self, frame: &eframe::Frame) {
        let Some(path) = project_file_dialog(frame).pick_file() else {
            return; // dialog cancelled
        };
        self.open_project_at(&path);
    }

    /// Opens the project file at `path` in place of the current project.
    fn open_project_at(&mut self, path: &Path) {
        match EditorSession::open(path) {
            Ok(session) => {
                self.transport.pause();
                self.forget_autosave();
                self.session = session;
                self.remember_recent_project(path);
                self.status_message = format!("Opened {}", path.display());
            }
            Err(error) => {
                self.status_message = format!("Could not open {}: {error}", path.display());
            }
        }
    }

    /// Lets the user pick a MilkDrop preset file and adds it to the project.
    fn add_milkdrop_preset(&mut self, frame: &eframe::Frame) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("MilkDrop preset", &["milk"])
            .set_parent(frame)
            .pick_file()
        else {
            return; // dialog cancelled
        };

        self.status_message = match self.session.add_milkdrop_preset(&path) {
            Ok(_) => format!("Added preset {}", path.display()),
            Err(reason) => format!("Could not add {}: {reason}", path.display()),
        };
    }

    /// Lets the user pick one or more image files and imports each of them
    /// onto a visual layer of its own.
    fn import_images(&mut self, frame: &eframe::Frame) {
        let Some(paths) = rfd::FileDialog::new()
            .add_filter("Image", &vizx_core::SUPPORTED_IMAGE_EXTENSIONS)
            .set_parent(frame)
            .pick_files()
        else {
            return; // dialog cancelled
        };

        let mut imported_count = 0;
        let mut last_error = None;
        for path in &paths {
            match self.session.import_image(path) {
                Ok(_) => imported_count += 1,
                Err(problem) => {
                    last_error = Some(format!("Could not import {}: {problem}", path.display()));
                }
            }
        }
        self.status_message = match (last_error, imported_count) {
            (Some(error), _) => error,
            (None, 1) => format!("Imported {}", paths[0].display()),
            (None, count) => format!("Imported {count} images"),
        };
    }

    /// Lets the user pick one image and makes it the background.
    fn import_background_image(&mut self, frame: &eframe::Frame) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Image", &vizx_core::SUPPORTED_IMAGE_EXTENSIONS)
            .set_parent(frame)
            .pick_file()
        else {
            return; // dialog cancelled
        };

        self.status_message = match self.session.import_background_image(&path) {
            Ok(_) => format!("Added background {}", path.display()),
            Err(problem) => format!("Could not import {}: {problem}", path.display()),
        };
    }

    /// Lets the user pick the folder of a visualizer plugin and adds it to
    /// the project, if it is a readable plugin.
    fn add_visualizer_folder(&mut self, frame: &eframe::Frame) {
        let Some(folder) = rfd::FileDialog::new().set_parent(frame).pick_folder() else {
            return; // dialog cancelled
        };
        let stored_path = folder.to_string_lossy().into_owned();

        // Cloned so the catalog is free again before the session is changed.
        let manifest = self
            .visualizer_catalog
            .plugin(&stored_path)
            .manifest
            .clone();
        self.status_message = match manifest {
            Ok(manifest) => {
                self.session.add_visualizer_plugin(&stored_path);
                format!("Added visualizer {}", manifest.name)
            }
            Err(problem) => format!("Not a visualizer plugin: {problem}"),
        };
    }

    /// Lets the user pick one or more audio files and imports each of them.
    fn import_audio(&mut self, frame: &eframe::Frame) {
        let Some(paths) = rfd::FileDialog::new()
            .add_filter("Audio", &vizx_audio::SUPPORTED_EXTENSIONS)
            .set_parent(frame)
            .pick_files()
        else {
            return; // dialog cancelled
        };

        let mut imported_count = 0;
        let mut last_error = None;
        for path in &paths {
            match self.session.import_audio(path) {
                Ok(_) => imported_count += 1,
                Err(error) => {
                    last_error = Some(format!("Could not import {}: {error}", path.display()));
                }
            }
        }

        // A failure is the more important thing to report, even if other
        // files of the same selection were imported.
        self.status_message = match (last_error, imported_count) {
            (Some(error), _) => error,
            (None, 1) => format!("Imported {}", paths[0].display()),
            (None, count) => format!("Imported {count} audio files"),
        };
    }

    /// Saves to the project's file, asking for a location if it has none.
    /// Returns `true` if the project was written to disk.
    fn save_project(&mut self, frame: &eframe::Frame) -> bool {
        match self.session.file_path() {
            Some(path) => self.save_project_to(path.to_owned()),
            None => self.save_project_as(frame),
        }
    }

    fn save_project_as(&mut self, frame: &eframe::Frame) -> bool {
        let chosen_path = project_file_dialog(frame)
            // Suggest the current name, so "Save As" starts from it.
            .set_file_name(format!(
                "{}.{PROJECT_FILE_EXTENSION}",
                self.session.project_name()
            ))
            .save_file();

        match chosen_path {
            Some(path) => self.save_project_to(with_project_extension(path)),
            None => false, // dialog cancelled
        }
    }

    fn save_project_to(&mut self, path: PathBuf) -> bool {
        match self.session.save_as(&path) {
            Ok(()) => {
                self.forget_autosave();
                self.remember_recent_project(&path);
                self.status_message = format!("Saved {}", path.display());
                true
            }
            Err(error) => {
                self.status_message = format!("Could not save {}: {error}", path.display());
                false
            }
        }
    }

    /// Whether the welcome window opens at startup; stored right away.
    pub fn set_show_welcome(&mut self, show: bool) {
        self.settings.show_welcome = show;
        if let Some(folder) = &self.app_data {
            let _ = self.settings.save(folder);
        }
    }

    /// Starts a new project from `template`: a fresh project, then the
    /// files the template needs, picked by the user.
    fn start_from_template(&mut self, template: Template, frame: &eframe::Frame) {
        self.transport.pause();
        self.forget_autosave();
        self.session = EditorSession::new_untitled();
        if template == Template::Empty {
            self.status_message = "New project created".to_owned();
            return;
        }

        self.import_audio(frame);
        let has_song = self
            .session
            .project()
            .audio_tracks()
            .any(|track| !track.clips.is_empty());
        if !has_song {
            self.status_message =
                "New project created; add a song with right-click on the timeline > Add".to_owned();
            return;
        }
        match template {
            Template::Empty => {}
            Template::MusicVisualizer => {
                self.session
                    .add_visualizer_plugin("bundled-visualizers:/spectrum-bars");
                self.status_message =
                    "Press Space to play; change the visualizer's look in Properties".to_owned();
            }
            Template::LyricsVideo => {
                let project_seconds = self.session.project().settings.duration_seconds();
                self.lyrics.open(project_seconds);
                self.status_message = "Paste the lyrics into the Lyrics window".to_owned();
            }
            Template::CoverArtVideo => {
                self.import_background_image(frame);
                self.session
                    .add_visualizer_plugin("bundled-visualizers:/core");
                self.status_message =
                    "Resize the ring in the preview so the cover shows through its middle"
                        .to_owned();
            }
        }
    }

    /// Puts `path` at the top of File > Open Recent and stores the list.
    fn remember_recent_project(&mut self, path: &Path) {
        self.settings.remember_project(path);
        if let Some(folder) = &self.app_data {
            let _ = self.settings.save(folder);
        }
    }

    /// Removes the autosaved copy of the current project (it was saved,
    /// or deliberately replaced or closed).
    fn forget_autosave(&mut self) {
        if let Some(autosaver) = &mut self.autosaver {
            autosaver.clear();
        }
    }

    /// Autosaves the project now and then while it has unsaved changes.
    fn autosave(&mut self) {
        if let Some(autosaver) = &mut self.autosaver
            && let Err(problem) = autosaver.tick(&self.session, std::time::Instant::now())
        {
            self.status_message = format!("Autosave failed: {problem}");
        }
    }

    /// Opens an autosaved project left behind by a VizX that did not close
    /// properly.
    pub fn recover(&mut self, index: usize) {
        if index >= self.recoveries.len() {
            return;
        }
        let recovery = self.recoveries.remove(index);
        match EditorSession::recover(&recovery.autosave, recovery.project_path.clone()) {
            Ok(session) => {
                self.transport.pause();
                self.session = session;
                recovery.discard();
                self.status_message =
                    format!("Recovered {}; save it to keep it", recovery.project_name);
            }
            Err(error) => {
                self.status_message =
                    format!("Could not recover {}: {error}", recovery.project_name);
            }
        }
    }

    /// Deletes an autosaved project the user does not want back.
    pub fn discard_recovery(&mut self, index: usize) {
        if index < self.recoveries.len() {
            self.recoveries.remove(index).discard();
        }
    }

    fn handle_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};

        // `COMMAND` means Ctrl on Windows/Linux and Cmd on macOS. The
        // Ctrl+Shift+S entry must come before Ctrl+S, because Ctrl+S would
        // otherwise also match while Shift is held. The same goes for
        // Ctrl+Shift+M and Ctrl+M.
        let shortcuts = [
            (
                KeyboardShortcut::new(Modifiers::COMMAND, Key::K),
                AppCommand::SplitClip,
            ),
            (
                KeyboardShortcut::new(Modifiers::COMMAND, Key::D),
                AppCommand::DuplicateClip,
            ),
            (
                KeyboardShortcut::new(Modifiers::COMMAND, Key::T),
                AppCommand::AddText,
            ),
            (
                KeyboardShortcut::new(Modifiers::COMMAND, Key::L),
                AppCommand::OpenLyrics,
            ),
            (
                KeyboardShortcut::new(Modifiers::COMMAND, Key::E),
                AppCommand::ExportVideo,
            ),
            (
                KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::I),
                AppCommand::ImportImage,
            ),
            (
                KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::M),
                AppCommand::BrowseVisualizers,
            ),
            (
                KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::S),
                AppCommand::SaveProjectAs,
            ),
            (
                KeyboardShortcut::new(Modifiers::COMMAND, Key::S),
                AppCommand::SaveProject,
            ),
            (
                KeyboardShortcut::new(Modifiers::COMMAND, Key::O),
                AppCommand::OpenProject,
            ),
            (
                KeyboardShortcut::new(Modifiers::COMMAND, Key::N),
                AppCommand::NewProject,
            ),
            (
                KeyboardShortcut::new(Modifiers::COMMAND, Key::I),
                AppCommand::ImportAudio,
            ),
            (
                KeyboardShortcut::new(Modifiers::COMMAND, Key::M),
                AppCommand::BrowseMilkdropPresets,
            ),
        ];

        // Shortcuts are ignored while the confirmation dialog is open.
        if self.command_awaiting_confirmation.is_some() {
            return;
        }

        // Single keys control playback, but only while the user is not
        // typing into a text field, where Space and Home mean something else.
        if !ctx.egui_wants_keyboard_input() {
            for (key, command) in [
                (Key::Space, AppCommand::TogglePlayback),
                (Key::Delete, AppCommand::DeleteSelection),
                (Key::B, AppCommand::TapBeat),
                (Key::Home, AppCommand::GoToStart),
                (Key::End, AppCommand::GoToEnd),
                (Key::ArrowLeft, AppCommand::MovePlayheadFrames(-1)),
                (Key::ArrowRight, AppCommand::MovePlayheadFrames(1)),
                (Key::ArrowUp, AppCommand::PreviousClipEdge),
                (Key::ArrowDown, AppCommand::NextClipEdge),
                (Key::F1, AppCommand::ShowShortcuts),
            ] {
                if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, key)) {
                    self.request(command);
                }
            }
            for (key, command) in [
                (Key::ArrowLeft, AppCommand::MovePlayheadSeconds(-1)),
                (Key::ArrowRight, AppCommand::MovePlayheadSeconds(1)),
            ] {
                if ctx.input_mut(|input| input.consume_key(Modifiers::SHIFT, key)) {
                    self.request(command);
                }
            }
            // egui turns Ctrl+C, Ctrl+X and Ctrl+V into clipboard events.
            let clipboard_command = ctx.input(|input| {
                input.events.iter().find_map(|event| match event {
                    egui::Event::Copy => Some(AppCommand::CopyClip),
                    egui::Event::Cut => Some(AppCommand::CutClip),
                    egui::Event::Paste(_) => Some(AppCommand::PasteClip),
                    _ => None,
                })
            });
            if let Some(command) = clipboard_command {
                self.request(command);
            }
        }
        for (shortcut, command) in shortcuts {
            if ctx.input_mut(|input| input.consume_shortcut(&shortcut)) {
                self.request(command);
            }
        }

        // Undo and redo, but not while the user is typing: a text field has
        // its own undo for what is typed into it, and uses the same keys.
        if !ctx.egui_wants_keyboard_input() {
            let history_shortcuts = [
                // Ctrl+Shift+Z before Ctrl+Z, which would match it too.
                (
                    KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z),
                    AppCommand::Redo,
                ),
                (
                    KeyboardShortcut::new(Modifiers::COMMAND, Key::Y),
                    AppCommand::Redo,
                ),
                (
                    KeyboardShortcut::new(Modifiers::COMMAND, Key::Z),
                    AppCommand::Undo,
                ),
            ];
            for (shortcut, command) in history_shortcuts {
                if ctx.input_mut(|input| input.consume_shortcut(&shortcut)) {
                    self.request(command);
                }
            }
        }
    }

    /// Collects audio files that finished loading in the background and
    /// starts loading the ones the project needs but does not have yet.
    fn load_project_audio(&mut self, ctx: &egui::Context) {
        self.audio_store.collect_finished();

        // Only audio files are decoded; other media (presets) are not audio.
        let audio_sources = self
            .session
            .project()
            .media()
            .iter()
            .filter(|source| source.audio().is_some());
        for source in audio_sources {
            if self.audio_store.state(&source.path).is_none() {
                // The background thread calls this when it is done, so the
                // window redraws and shows the waveform without waiting for
                // the next mouse movement.
                let ctx = ctx.clone();
                self.audio_store
                    .request(&source.path, move || ctx.request_repaint());
            }
        }
    }

    /// The same for images: collects finished ones and starts loading the
    /// ones the project needs.
    fn load_project_images(&mut self, ctx: &egui::Context) {
        self.image_store.collect_finished();

        let image_sources = self
            .session
            .project()
            .media()
            .iter()
            .filter(|source| source.image().is_some());
        for source in image_sources {
            if self.image_store.state(&source.path).is_none() {
                let ctx = ctx.clone();
                self.image_store
                    .request(&source.path, move || ctx.request_repaint());
            }
        }
    }

    /// Intercepts the window's close button while there are unsaved changes
    /// and routes it through the same confirmation as File > Exit.
    fn handle_window_close_request(&mut self, ctx: &egui::Context) {
        let is_close_requested = ctx.input(|input| input.viewport().close_requested());

        if is_close_requested && !self.is_exit_confirmed && self.session.has_unsaved_changes() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.request(AppCommand::Exit);
        }
    }

    /// Maximizes the window once, on the first frame.
    ///
    /// Editors normally start maximized, and it keeps the window on screen
    /// when the display is smaller than the default 1440x900 size (e.g. a
    /// 1920x1080 laptop panel at 125% scaling). Requesting the maximized
    /// state together with a size at window creation left the window
    /// "maximized" but still at 1440x900 on Windows, so it is requested here
    /// instead, after the window exists.
    fn maximize_on_startup(&mut self, ctx: &egui::Context) {
        if !self.has_maximized_on_startup {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            self.has_maximized_on_startup = true;
        }
    }

    /// Keeps the OS window title in sync with the project name and its
    /// unsaved-changes state, e.g. `My Song* - VizX (Prototype)`.
    fn sync_window_title(&mut self, ctx: &egui::Context) {
        let unsaved_marker = if self.session.has_unsaved_changes() {
            "*"
        } else {
            ""
        };
        let title = format!(
            "{}{} - {}",
            self.session.project_name(),
            unsaved_marker,
            vizx_core::APP_NAME
        );

        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }
}

impl eframe::App for VizxApp {
    /// The window is closing for good (unsaved changes were dealt with):
    /// the autosaved copy is not needed.
    fn on_exit(&mut self) {
        self.forget_autosave();
        // The panel arrangement is kept for the next run.
        self.settings.panel_layout = self.workspace.layout();
        if let Some(folder) = &self.app_data {
            let _ = self.settings.save(folder);
        }
    }

    /// Called by eframe every time the window needs to be redrawn.
    ///
    /// egui is an "immediate mode" GUI: there are no persistent widget
    /// objects. Each frame this function describes the whole interface from
    /// the current state, and egui reports interactions back immediately.
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        self.maximize_on_startup(&ctx);
        self.handle_window_close_request(&ctx);
        self.transport.begin_frame(&mut self.session, &ctx);
        self.handle_keyboard_shortcuts(&ctx);
        self.load_project_audio(&ctx);
        self.load_project_images(&ctx);
        self.collect_stems();
        self.autosave();
        self.visual_preview.update(
            &ctx,
            &self.session,
            &self.audio_store,
            &mut self.visualizer_catalog,
            &self.image_store,
        );

        // Fixed bars are added first; each one takes space from an edge of
        // the window. Whatever remains is given to the dockable workspace.
        egui::Panel::top("menu_bar").show(ui, |ui| menu_bar::show(ui, self));
        egui::Panel::top("toolbar").show(ui, |ui| toolbar::show(ui, self));
        egui::Panel::bottom("status_bar").show(ui, |ui| status_bar::show(ui, self));

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                self.workspace.show(
                    ui,
                    &mut self.session,
                    &self.audio_store,
                    &self.visual_preview,
                    &mut self.visualizer_catalog,
                    &mut self.requested_command,
                );
            });

        about_window::show(&ctx, self);
        project_settings_window::show(&ctx, self);
        lyrics_window::show(&ctx, self);
        recovery_window::show(&ctx, self);
        welcome_window::show(&ctx, self);
        shortcuts_window::show(&ctx, self);
        preset_browser::show(&ctx, self);
        visualizer_browser::show(&ctx, self);
        export_window::show(&ctx, self);
        if let Some(decision) = unsaved_changes_dialog::show(&ctx, self) {
            self.resolve_unsaved_changes(decision, &ctx, frame);
        }

        // Commands requested anywhere above are executed here, after the
        // whole interface for this frame has been described.
        self.run_requested_command(&ctx, frame);
        // After everything that could change the project this frame. While
        // a mouse button is down, the user is in the middle of a drag.
        let is_user_busy = ctx.input(|input| input.pointer.any_down());
        self.session.settle_history(is_user_busy);
        self.transport.end_frame(&self.session);
        self.sync_window_title(&ctx);
    }
}

/// Returns `path` with the `.mp4` extension, appending it if it is missing.
fn with_mp4_extension(path: PathBuf) -> PathBuf {
    let has_extension = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"));
    if has_extension {
        return path;
    }
    let mut file_name = path.into_os_string();
    file_name.push(".mp4");
    PathBuf::from(file_name)
}

/// A native file dialog limited to VizX project files, attached to the main
/// window so it stays in front of it.
fn project_file_dialog(frame: &eframe::Frame) -> rfd::FileDialog {
    rfd::FileDialog::new()
        .add_filter("VizX Project", &[PROJECT_FILE_EXTENSION])
        .set_parent(frame)
}

fn describe_graphics_adapter(creation_context: &eframe::CreationContext<'_>) -> String {
    match &creation_context.wgpu_render_state {
        Some(render_state) => {
            let adapter_info = render_state.adapter.get_info();
            format!("{:?} - {}", adapter_info.backend, adapter_info.name)
        }
        None => "wgpu renderer not active".to_owned(),
    }
}
