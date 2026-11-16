//! The dockable workspace: which panels exist and how they are arranged.

use eframe::egui;
use egui_dock::{DockArea, DockState, NodeIndex};
use vizx_core::{AudioStore, EditorSession, VisualizerCatalog};

use crate::app::AppCommand;
use crate::theme;
use crate::thumbnails::Thumbnails;
use crate::ui::panels;
use crate::ui::panels::timeline::TimelineView;
use crate::visual_preview::VisualPreview;

/// Identifies one dockable panel.
///
/// The docking system only stores these small values. The actual panel
/// content is drawn by [`PanelViewer`] from the shared [`EditorSession`], so
/// a panel never owns project data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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

/// What the panels draw from besides the session.
pub struct PanelResources<'a> {
    pub audio_store: &'a AudioStore,
    pub visual_preview: &'a VisualPreview,
    pub visualizer_catalog: &'a mut VisualizerCatalog,
    pub thumbnails: &'a mut Thumbnails,
}

/// Current arrangement of the dockable panels.
pub struct Workspace {
    dock_state: DockState<PanelKind>,
    /// Zoom and scroll position of the timeline panel.
    timeline_view: TimelineView,
    /// Closed panels and a panel they shared a tab group with, so they can
    /// go back into that group when reopened.
    closed_next_to: Vec<(PanelKind, PanelKind)>,
}

impl Workspace {
    pub fn new() -> Self {
        Self {
            dock_state: default_layout(),
            timeline_view: TimelineView::default(),
            closed_next_to: Vec::new(),
        }
    }

    /// The panel arrangement, to be stored between runs.
    pub fn layout(&self) -> Option<serde_json::Value> {
        serde_json::to_value(&self.dock_state).ok()
    }

    /// Brings back an arrangement stored by [`Self::layout`]. One that cannot
    /// be read (from another VizX version, say) leaves the default layout.
    pub fn restore_layout(&mut self, layout: &serde_json::Value) {
        if let Ok(dock_state) = serde_json::from_value::<DockState<PanelKind>>(layout.clone()) {
            // Every panel must still be reachable; otherwise keep the default.
            if PanelKind::ALL
                .iter()
                .any(|panel| dock_state.find_tab(panel).is_some())
            {
                self.dock_state = dock_state;
            }
        }
    }

    pub fn reset_layout(&mut self) {
        self.dock_state = default_layout();
    }

    pub fn is_open(&self, panel: PanelKind) -> bool {
        self.dock_state.find_tab(&panel).is_some()
    }

    /// Opens or closes a panel. A reopened panel goes back where it was: as
    /// a tab next to a panel it shared a group with, or else on its side of
    /// the default layout (Media Library left, Properties right, Timeline
    /// below, Preview in the middle).
    pub fn set_open(&mut self, panel: PanelKind, should_be_open: bool) {
        match (self.dock_state.find_tab(&panel), should_be_open) {
            (Some(tab_path), false) => {
                let companion = self
                    .dock_state
                    .iter_all_tabs()
                    .find(|(path, other)| {
                        **other != panel
                            && path.surface == tab_path.surface
                            && path.node == tab_path.node
                    })
                    .map(|(_, other)| *other);
                self.closed_next_to.retain(|(closed, _)| *closed != panel);
                if let Some(companion) = companion {
                    self.closed_next_to.push((panel, companion));
                }
                self.dock_state.remove_tab(tab_path);
            }
            (None, true) => self.reopen(panel),
            _ => {} // already in the requested state
        }
    }

    fn reopen(&mut self, panel: PanelKind) {
        // Back into the group it shared with another panel.
        let companion = self
            .closed_next_to
            .iter()
            .find(|(closed, _)| *closed == panel)
            .map(|(_, companion)| *companion);
        self.closed_next_to.retain(|(closed, _)| *closed != panel);
        if let Some(path) = companion.and_then(|companion| self.dock_state.find_tab(&companion)) {
            self.dock_state
                .set_focused_node_and_surface(egui_dock::NodePath::new(path.surface, path.node));
            self.dock_state.push_to_focused_leaf(panel);
            return;
        }

        let tree = self.dock_state.main_surface_mut();
        let node_of = |tree: &egui_dock::Tree<PanelKind>, other: PanelKind| {
            tree.find_tab(&other).map(|(node, _)| node)
        };
        if tree.is_empty() {
            self.dock_state.push_to_focused_leaf(panel);
            return;
        }
        let root = NodeIndex::root();
        let preview = node_of(tree, PanelKind::Preview);
        match panel {
            PanelKind::Timeline => {
                tree.split_below(root, 0.68, vec![panel]);
            }
            PanelKind::MediaLibrary => match preview {
                Some(node) => {
                    tree.split_left(node, 0.22, vec![panel]);
                }
                None => {
                    tree.split_left(root, 0.18, vec![panel]);
                }
            },
            PanelKind::Properties => match preview {
                Some(node) => {
                    tree.split_right(node, 0.72, vec![panel]);
                }
                None => {
                    tree.split_right(root, 0.8, vec![panel]);
                }
            },
            PanelKind::Preview => {
                match (
                    node_of(tree, PanelKind::Properties),
                    node_of(tree, PanelKind::MediaLibrary),
                ) {
                    (Some(properties), _) => {
                        tree.split_left(properties, 0.7, vec![panel]);
                    }
                    (None, Some(library)) => {
                        tree.split_right(library, 0.25, vec![panel]);
                    }
                    (None, None) => {
                        tree.split_above(root, 0.6, vec![panel]);
                    }
                }
            }
        }
    }

    /// Draws all panels. A panel that wants an application command executed
    /// (for example "open project settings") writes it to `requested_command`.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut EditorSession,
        resources: PanelResources<'_>,
        requested_command: &mut Option<AppCommand>,
    ) {
        let PanelResources {
            audio_store,
            visual_preview,
            visualizer_catalog,
            thumbnails,
        } = resources;
        let style = theme::dock_style(ui.style());
        let mut panel_viewer = PanelViewer {
            session,
            timeline_view: &mut self.timeline_view,
            audio_store,
            visual_preview,
            visualizer_catalog,
            thumbnails,
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
    /// The composed picture and the clips in it.
    visual_preview: &'session VisualPreview,
    visualizer_catalog: &'session mut VisualizerCatalog,
    thumbnails: &'session mut Thumbnails,
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
                    self.visual_preview,
                );
            }
            PanelKind::Properties => {
                panels::properties::show(
                    ui,
                    self.session,
                    self.visualizer_catalog,
                    self.audio_store,
                    self.requested_command,
                );
            }
            PanelKind::Timeline => {
                panels::timeline::show(
                    ui,
                    self.session,
                    self.timeline_view,
                    self.audio_store,
                    self.thumbnails,
                    self.visualizer_catalog,
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
    fn a_reopened_panel_gets_its_own_area_again() {
        let mut workspace = Workspace::new();
        let node_of = |workspace: &Workspace, panel| workspace.dock_state.find_tab(&panel).unwrap().node;

        workspace.set_open(PanelKind::Properties, false);
        workspace.set_open(PanelKind::Properties, true);
        // Not a tab in the focused group, but a group of its own.
        assert_ne!(
            node_of(&workspace, PanelKind::Properties),
            node_of(&workspace, PanelKind::Preview)
        );

        // A panel that shared a group goes back into that group.
        let preview = workspace.dock_state.find_tab(&PanelKind::Preview).unwrap();
        workspace
            .dock_state
            .set_focused_node_and_surface(egui_dock::NodePath::new(preview.surface, preview.node));
        workspace.set_open(PanelKind::Timeline, false);
        workspace.dock_state.push_to_focused_leaf(PanelKind::Timeline);
        workspace.set_open(PanelKind::Timeline, false);
        workspace.set_open(PanelKind::Timeline, true);
        assert_eq!(
            node_of(&workspace, PanelKind::Timeline),
            node_of(&workspace, PanelKind::Preview)
        );
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
