use std::path::PathBuf;

use eframe::egui;
use vizx_core::{EditorSession, WaveformStore};
use vizx_project::{PROJECT_FILE_EXTENSION, with_project_extension};

use crate::theme;
use crate::ui::workspace::Workspace;
use crate::ui::{
    about_window, menu_bar, project_settings_window, status_bar, toolbar, unsaved_changes_dialog,
};

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
    SaveProject,
    SaveProjectAs,
    ImportAudio,
    OpenProjectSettings,
    Exit,
}

impl AppCommand {
    /// Whether the command throws away the currently open project, in which
    /// case unsaved changes must be confirmed first.
    fn discards_current_project(self) -> bool {
        matches!(self, Self::NewProject | Self::OpenProject | Self::Exit)
    }
}

/// Top-level application state.
///
/// `session` is the shared editing state from `vizx-core`; everything else
/// here is purely about how the UI is currently arranged.
pub struct VizxApp {
    pub session: EditorSession,
    pub workspace: Workspace,
    /// Waveforms of the project's audio files, computed in the background.
    waveforms: WaveformStore,
    pub is_about_window_open: bool,
    pub is_project_settings_open: bool,
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

        Self {
            session: EditorSession::new_untitled(),
            workspace: Workspace::new(),
            waveforms: WaveformStore::default(),
            is_about_window_open: false,
            is_project_settings_open: false,
            graphics_summary: describe_graphics_adapter(creation_context),
            status_message: "Ready".to_owned(),
            requested_command: None,
            command_awaiting_confirmation: None,
            is_exit_confirmed: false,
            window_title: String::new(),
            has_maximized_on_startup: false,
        }
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
                self.session = EditorSession::new_untitled();
                self.status_message = "New project created".to_owned();
            }
            AppCommand::OpenProject => self.open_project(frame),
            AppCommand::SaveProject => {
                self.save_project(frame);
            }
            AppCommand::SaveProjectAs => {
                self.save_project_as(frame);
            }
            AppCommand::ImportAudio => self.import_audio(frame),
            AppCommand::OpenProjectSettings => self.is_project_settings_open = true,
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

        match EditorSession::open(&path) {
            Ok(session) => {
                self.session = session;
                self.status_message = format!("Opened {}", path.display());
            }
            Err(error) => {
                self.status_message = format!("Could not open {}: {error}", path.display());
            }
        }
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
            .set_file_name(self.session.project().suggested_file_name())
            .save_file();

        match chosen_path {
            Some(path) => self.save_project_to(with_project_extension(path)),
            None => false, // dialog cancelled
        }
    }

    fn save_project_to(&mut self, path: PathBuf) -> bool {
        match self.session.save_as(&path) {
            Ok(()) => {
                self.status_message = format!("Saved {}", path.display());
                true
            }
            Err(error) => {
                self.status_message = format!("Could not save {}: {error}", path.display());
                false
            }
        }
    }

    fn handle_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};

        // `COMMAND` means Ctrl on Windows/Linux and Cmd on macOS. The
        // Ctrl+Shift+S entry must come before Ctrl+S, because Ctrl+S would
        // otherwise also match while Shift is held.
        let shortcuts = [
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
        ];

        // Shortcuts are ignored while the confirmation dialog is open.
        if self.command_awaiting_confirmation.is_some() {
            return;
        }
        for (shortcut, command) in shortcuts {
            if ctx.input_mut(|input| input.consume_shortcut(&shortcut)) {
                self.request(command);
            }
        }
    }

    /// Collects waveforms finished in the background and starts computing
    /// the ones the project needs but does not have yet.
    fn update_waveforms(&mut self, ctx: &egui::Context) {
        self.waveforms.collect_finished();

        for source in self.session.project().media() {
            if self.waveforms.state(&source.path).is_none() {
                // The background thread calls this when it is done, so the
                // window redraws and shows the waveform without waiting for
                // the next mouse movement.
                let ctx = ctx.clone();
                self.waveforms
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
            self.session.project().metadata.name,
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
    /// Called by eframe every time the window needs to be redrawn.
    ///
    /// egui is an "immediate mode" GUI: there are no persistent widget
    /// objects. Each frame this function describes the whole interface from
    /// the current state, and egui reports interactions back immediately.
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        self.maximize_on_startup(&ctx);
        self.handle_window_close_request(&ctx);
        self.handle_keyboard_shortcuts(&ctx);
        self.update_waveforms(&ctx);

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
                    &self.waveforms,
                    &mut self.requested_command,
                );
            });

        about_window::show(&ctx, self);
        project_settings_window::show(&ctx, self);
        if let Some(decision) = unsaved_changes_dialog::show(&ctx, self) {
            self.resolve_unsaved_changes(decision, &ctx, frame);
        }

        // Commands requested anywhere above are executed here, after the
        // whole interface for this frame has been described.
        self.run_requested_command(&ctx, frame);
        self.sync_window_title(&ctx);
    }
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
