use vizx_project::{ClipId, TrackId};

/// What the user currently has selected in the editor.
///
/// Selection is editing state, not project data: it is shared by all panels
/// (the timeline sets it, the Properties panel shows it) but it is never
/// saved into the project file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Selection {
    #[default]
    Nothing,
    /// A whole track (a visual layer or an audio track).
    Track(TrackId),
    /// One clip on a track. Its track counts as selected too wherever only
    /// tracks matter.
    Clip(ClipId),
}
