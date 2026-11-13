use std::path::{Path, PathBuf};

use vizx_project::Project;

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
}

impl EditorSession {
    /// Starts a session with a new, never-saved project.
    pub fn new_untitled() -> Self {
        Self {
            project: Project::new("Untitled Project"),
            file_path: None,
            has_unsaved_changes: false,
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
}

#[cfg(test)]
mod tests {
    use super::*;

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

        session.edit_project(|project| {
            project.metadata.name = "Renamed".to_owned();
            true
        });
        assert!(session.has_unsaved_changes());
        assert_eq!(session.project().metadata.name, "Renamed");
    }
}
