use std::path::{Path, PathBuf};

use vizx_project::{Project, ProjectFileError};

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

    /// Starts a session by loading a project from a `.vzx` file.
    pub fn open(path: &Path) -> Result<Self, ProjectFileError> {
        Ok(Self {
            project: Project::load_from_file(path)?,
            file_path: Some(path.to_owned()),
            has_unsaved_changes: false,
        })
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
