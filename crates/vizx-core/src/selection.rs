use vizx_project::TrackId;

/// What the user currently has selected in the editor.
///
/// Selection is editing state, not project data: it is shared by all panels
/// (the timeline sets it, the Properties panel shows it) but it is never
/// saved into the project file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Selection {
    #[default]
    Nothing,
    Track(TrackId),
}
