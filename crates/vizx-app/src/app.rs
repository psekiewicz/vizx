use eframe::egui;
use vizx_core::EditorSession;

use crate::theme;
use crate::ui::workspace::Workspace;
use crate::ui::{about_window, menu_bar, status_bar, toolbar};

/// Top-level application state.
///
/// `session` is the shared editing state from `vizx-core`; everything else
/// here is purely about how the UI is currently arranged.
pub struct VizxApp {
    pub session: EditorSession,
    pub workspace: Workspace,
    pub is_about_window_open: bool,
    /// Description of the graphics adapter wgpu selected, for diagnostics.
    pub graphics_summary: String,
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
            is_about_window_open: false,
            graphics_summary: describe_graphics_adapter(creation_context),
            window_title: String::new(),
            has_maximized_on_startup: false,
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

    /// Replaces the open project with a new, empty one.
    pub fn start_new_project(&mut self) {
        self.session = EditorSession::new_untitled();
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
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.maximize_on_startup(ui.ctx());
        self.sync_window_title(ui.ctx());

        // Fixed bars are added first; each one takes space from an edge of
        // the window. Whatever remains is given to the dockable workspace.
        egui::Panel::top("menu_bar").show(ui, |ui| menu_bar::show(ui, self));
        egui::Panel::top("toolbar").show(ui, |ui| toolbar::show(ui, self));
        egui::Panel::bottom("status_bar").show(ui, |ui| status_bar::show(ui, self));

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.workspace.show(ui, &mut self.session));

        about_window::show(ui.ctx(), self);
    }
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
