use std::path::{Path, PathBuf};

use vizx_project::{Project, ProjectFileError, Track, TrackId, TrackKind};

use crate::Selection;

/// The state of one open project while it is being edited.
///
/// There is exactly one session in the application and every UI panel is a
/// view of it. Panels never keep their own copy of project data, which is
/// what keeps the editor unified instead of a set of separate tools.
#[derive(Debug, Clone)]
pub struct EditorSession {
    project: Project,
    /// Where the project is stored on disk. `None` until it is first saved.
    file_path: Option<PathBuf>,
    has_unsaved_changes: bool,
    selection: Selection,
}

impl EditorSession {
    /// Starts a session with a new, never-saved project.
    pub fn new_untitled() -> Self {
        Self {
            project: Project::with_starter_tracks("Untitled Project"),
            file_path: None,
            has_unsaved_changes: false,
            selection: Selection::Nothing,
        }
    }

    /// Starts a session by loading a project from a `.vzx` file.
    pub fn open(path: &Path) -> Result<Self, ProjectFileError> {
        Ok(Self {
            project: Project::load_from_file(path)?,
            file_path: Some(path.to_owned()),
            has_unsaved_changes: false,
            selection: Selection::Nothing,
        })
    }

    pub fn selection(&self) -> Selection {
        self.selection
    }

    /// Changes the selection. Selecting is not an edit, so it does not mark
    /// the project as changed.
    pub fn select(&mut self, selection: Selection) {
        self.selection = selection;
    }

    /// The selected track, if a track is selected and still exists.
    pub fn selected_track(&self) -> Option<&Track> {
        match self.selection {
            Selection::Track(id) => self.project.track(id),
            Selection::Nothing => None,
        }
    }

    /// Adds a track at the bottom of the timeline and selects it.
    pub fn add_track(&mut self, kind: TrackKind) -> TrackId {
        let id = self.project.add_default_named_track(kind);
        self.has_unsaved_changes = true;
        self.selection = Selection::Track(id);
        id
    }

    /// Removes a track, clearing the selection if it was selected.
    pub fn remove_track(&mut self, id: TrackId) {
        if self.project.remove_track(id) {
            self.has_unsaved_changes = true;
            if self.selection == Selection::Track(id) {
                self.selection = Selection::Nothing;
            }
        }
    }

    /// Read-only access to the project.
    pub fn project(&self) -> &Project {
        &self.project
    }

    /// Applies an edit to the project and records that it changed.
    ///
    /// All modifications go through this method so the "unsaved changes"
    /// flag cannot be forgotten. It is also the natural place to hook in
    /// undo/redo later. The closure returns `true` if it changed anything.
    pub fn edit_project(&mut self, edit: impl FnOnce(&mut Project) -> bool) {
        if edit(&mut self.project) {
            self.has_unsaved_changes = true;
        }
    }

    pub fn file_path(&self) -> Option<&Path> {
        self.file_path.as_deref()
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.has_unsaved_changes
    }

    /// Saves the project to `path` and makes that its file from now on.
    ///
    /// If writing fails, the session is left exactly as it was: it keeps its
    /// previous path and still counts as having unsaved changes.
    pub fn save_as(&mut self, path: &Path) -> Result<(), ProjectFileError> {
        self.project.save_to_file(path)?;
        self.file_path = Some(path.to_owned());
        self.has_unsaved_changes = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique path in the system temp directory for one test.
    fn temporary_project_path(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "vizx-session-{test_name}-{}.vzx",
            std::process::id()
        ))
    }

    fn rename_project(session: &mut EditorSession, name: &str) {
        session.edit_project(|project| {
            project.metadata.name = name.to_owned();
            true
        });
    }

    #[test]
    fn new_session_is_unsaved_on_disk_but_has_no_pending_changes() {
        let session = EditorSession::new_untitled();

        assert!(session.file_path().is_none());
        assert!(!session.has_unsaved_changes());
    }

    #[test]
    fn only_real_edits_mark_the_session_as_changed() {
        let mut session = EditorSession::new_untitled();

        session.edit_project(|_project| false);
        assert!(!session.has_unsaved_changes());

        rename_project(&mut session, "Renamed");
        assert!(session.has_unsaved_changes());
        assert_eq!(session.project().metadata.name, "Renamed");
    }

    #[test]
    fn adding_a_track_selects_it_and_marks_the_project_changed() {
        let mut session = EditorSession::new_untitled();
        let track_count_before = session.project().tracks().len();

        let id = session.add_track(TrackKind::Text);

        assert_eq!(session.project().tracks().len(), track_count_before + 1);
        assert_eq!(session.selection(), Selection::Track(id));
        assert_eq!(session.selected_track().unwrap().kind, TrackKind::Text);
        assert!(session.has_unsaved_changes());
    }

    #[test]
    fn removing_the_selected_track_clears_the_selection() {
        let mut session = EditorSession::new_untitled();
        let id = session.add_track(TrackKind::Audio);

        session.remove_track(id);

        assert_eq!(session.selection(), Selection::Nothing);
        assert!(session.selected_track().is_none());
    }

    #[test]
    fn selecting_does_not_count_as_a_change() {
        let mut session = EditorSession::new_untitled();
        let id = session.project().tracks()[0].id;

        session.select(Selection::Track(id));

        assert!(!session.has_unsaved_changes());
    }

    #[test]
    fn saving_remembers_the_path_and_clears_unsaved_changes() {
        let path = temporary_project_path("save");
        let mut session = EditorSession::new_untitled();
        rename_project(&mut session, "Saved");

        let result = session.save_as(&path);
        let _ = std::fs::remove_file(&path);

        result.unwrap();
        assert_eq!(session.file_path(), Some(path.as_path()));
        assert!(!session.has_unsaved_changes());
    }

    #[test]
    fn saved_project_can_be_opened_again() {
        let path = temporary_project_path("reopen");
        let mut session = EditorSession::new_untitled();
        rename_project(&mut session, "Reopened");
        session.save_as(&path).unwrap();

        let reopened = EditorSession::open(&path);
        let _ = std::fs::remove_file(&path);

        let reopened = reopened.unwrap();
        assert_eq!(reopened.project(), session.project());
        assert_eq!(reopened.file_path(), Some(path.as_path()));
        assert!(!reopened.has_unsaved_changes());
    }

    #[test]
    fn failed_save_leaves_the_session_unchanged() {
        // A directory that does not exist, so the write must fail.
        let path = std::env::temp_dir()
            .join("vizx-missing-directory")
            .join("project.vzx");
        let mut session = EditorSession::new_untitled();
        rename_project(&mut session, "Not saved");

        assert!(session.save_as(&path).is_err());
        assert!(session.file_path().is_none());
        assert!(session.has_unsaved_changes());
    }
}
