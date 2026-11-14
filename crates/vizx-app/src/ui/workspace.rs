//! The dockable workspace: which panels exist and how they are arranged.

use eframe::egui;
use egui_dock::{DockArea, DockState, NodeIndex};
use vizx_core::{AudioStore, EditorSession, VisualizerCatalog};

use crate::app::AppCommand;
use crate::theme;
use crate::ui::panels;
use crate::ui::panels::timeline::TimelineView;
use crate::visual_preview::LayerView;

/// Identifies one dockable panel.
///
/// The docking system only stores these small values. The actual panel
/// content is drawn by [`PanelViewer`] from the shared [`EditorSession`], so
/// a panel never owns project data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PanelKind {
    MediaLibrary,
    Preview,
    Properties,
    Timeline,
}

impl PanelKind {
    /// Every panel, in the order shown in the View menu.
    pub const ALL: [Self; 4] = [
        Self::MediaLibrary,
        Self::Preview,
        Self::Properties,
        Self::Timeline,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::MediaLibrary => "Media Library",
            Self::Preview => "Preview",
            Self::Properties => "Properties",
            Self::Timeline => "Timeline",
        }
    }
}

/// Current arrangement of the dockable panels.
pub struct Workspace {
    dock_state: DockState<PanelKind>,
    /// Zoom and scroll position of the timeline panel.
    timeline_view: TimelineView,
}

impl Workspace {
    pub fn new() -> Self {
        Self {
            dock_state: default_layout(),
            timeline_view: TimelineView::default(),
        }
    }

    pub fn reset_layout(&mut self) {
        self.dock_state = default_layout();
    }

    pub fn is_open(&self, panel: PanelKind) -> bool {
        self.dock_state.find_tab(&panel).is_some()
    }

    /// Opens or closes a panel. A reopened panel is added as a tab to the
    /// currently focused group; the user can then drag it anywhere.
    pub fn set_open(&mut self, panel: PanelKind, should_be_open: bool) {
        match (self.dock_state.find_tab(&panel), should_be_open) {
            (Some(tab_path), false) => {
                self.dock_state.remove_tab(tab_path);
            }
            (None, true) => self.dock_state.push_to_focused_leaf(panel),
            _ => {} // already in the requested state
        }
    }

    /// Draws all panels. A panel that wants an application command executed
    /// (for example "open project settings") writes it to `requested_command`.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut EditorSession,
        audio_store: &AudioStore,
        visualizer_layers: &[LayerView<'_>],
        visualizer_catalog: &mut VisualizerCatalog,
        requested_command: &mut Option<AppCommand>,
    ) {
        let style = theme::dock_style(ui.style());
        let mut panel_viewer = PanelViewer {
            session,
            timeline_view: &mut self.timeline_view,
            audio_store,
            visualizer_layers,
            visualizer_catalog,
            requested_command,
        };

        DockArea::new(&mut self.dock_state)
            .style(style)
            .show_leaf_close_all_buttons(false)
            .show_leaf_collapse_buttons(false)
            .show_inside(ui, &mut panel_viewer);
    }
}

/// Builds the default editor layout:
///
/// ```text
/// +---------------+----------------------+--------------+
/// | Media Library |       Preview        |  Properties  |
/// +---------------+----------------------+--------------+
/// |                      Timeline                       |
/// +-----------------------------------------------------+
/// ```
fn default_layout() -> DockState<PanelKind> {
    let mut dock_state = DockState::new(vec![PanelKind::Preview]);
    let tree = dock_state.main_surface_mut();

    // Each split divides an existing area in two and returns the indices of
    // [the original area, the newly created area]. The fraction is the share
    // of space kept by the original area.
    let [upper_area, _timeline] =
        tree.split_below(NodeIndex::root(), 0.68, vec![PanelKind::Timeline]);
    let [upper_area, _media_library] =
        tree.split_left(upper_area, 0.18, vec![PanelKind::MediaLibrary]);
    let [_preview, _properties] = tree.split_right(upper_area, 0.76, vec![PanelKind::Properties]);

    dock_state
}

/// Tells egui_dock how to draw each [`PanelKind`].
///
/// It borrows the session only for the duration of one frame, which is why
/// it has a lifetime parameter (`'session`): the viewer cannot outlive the
/// data it points to.
struct PanelViewer<'session> {
    session: &'session mut EditorSession,
    timeline_view: &'session mut TimelineView,
    audio_store: &'session AudioStore,
    /// The visualizers at the playhead, bottom layer first.
    visualizer_layers: &'session [LayerView<'session>],
    visualizer_catalog: &'session mut VisualizerCatalog,
    requested_command: &'session mut Option<AppCommand>,
}

impl egui_dock::TabViewer for PanelViewer<'_> {
    type Tab = PanelKind;

    fn id(&mut self, panel: &mut PanelKind) -> egui::Id {
        egui::Id::new(*panel)
    }

    fn title(&mut self, panel: &mut PanelKind) -> egui::WidgetText {
        panel.title().into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, panel: &mut PanelKind) {
        match panel {
            PanelKind::MediaLibrary => {
                panels::media_library::show(ui, self.session);
            }
            PanelKind::Preview => {
                panels::preview::show(
                    ui,
                    self.session,
                    self.requested_command,
                    self.visualizer_layers,
                );
            }
            PanelKind::Properties => {
                panels::properties::show(ui, self.session, self.visualizer_catalog)
            }
            PanelKind::Timeline => {
                panels::timeline::show(
                    ui,
                    self.session,
                    self.timeline_view,
                    self.audio_store,
                    self.requested_command,
                );
            }
        }
    }

    /// The preview and timeline paint their own full-size content, so they
    /// manage scrolling themselves instead of using the default scroll bars.
    fn scroll_bars(&self, panel: &PanelKind) -> [bool; 2] {
        match panel {
            PanelKind::Preview | PanelKind::Timeline => [false, false],
            PanelKind::MediaLibrary | PanelKind::Properties => [false, true],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_contains_every_panel() {
        let workspace = Workspace::new();

        for panel in PanelKind::ALL {
            assert!(workspace.is_open(panel), "{panel:?} should be open");
        }
    }

    #[test]
    fn panel_can_be_closed_and_reopened() {
        let mut workspace = Workspace::new();

        workspace.set_open(PanelKind::Timeline, false);
        assert!(!workspace.is_open(PanelKind::Timeline));
        assert!(workspace.is_open(PanelKind::Preview));

        workspace.set_open(PanelKind::Timeline, true);
        assert!(workspace.is_open(PanelKind::Timeline));
    }

    #[test]
    fn opening_an_open_panel_does_not_duplicate_it() {
        let mut workspace = Workspace::new();

        workspace.set_open(PanelKind::Preview, true);

        let preview_tabs = workspace
            .dock_state
            .iter_all_tabs()
            .filter(|(_, panel)| **panel == PanelKind::Preview)
            .count();
        assert_eq!(preview_tabs, 1);
    }

    #[test]
    fn every_panel_can_be_closed_and_layout_reset_restores_them() {
        let mut workspace = Workspace::new();

        for panel in PanelKind::ALL {
            workspace.set_open(panel, false);
        }
        for panel in PanelKind::ALL {
            assert!(!workspace.is_open(panel));
        }

        workspace.reset_layout();
        for panel in PanelKind::ALL {
            assert!(workspace.is_open(panel));
        }
    }

    #[test]
    fn panel_reopens_when_all_panels_were_closed() {
        let mut workspace = Workspace::new();
        for panel in PanelKind::ALL {
            workspace.set_open(panel, false);
        }

        workspace.set_open(PanelKind::Properties, true);

        assert!(workspace.is_open(PanelKind::Properties));
    }
}
