use std::mem;
use std::time::{Duration, Instant};

use vizx_project::Project;

/// A change must have stayed untouched for this long before it becomes an
/// undo step of its own. Everything that happens faster (every pixel of a
/// drag, every letter typed) is part of the same step.
const SETTLE_TIME: Duration = Duration::from_millis(400);
/// How many steps can be undone. Older ones are forgotten.
const STEP_LIMIT: usize = 100;

/// Remembers earlier states of the project, for Undo and Redo.
///
/// It works by comparison instead of recording every action: once per frame
/// it is shown the project, and when the project has changed and come to
/// rest, the state before the change is kept as an undo step. A project is
/// a small piece of data (it refers to media files, it does not contain
/// them), so keeping whole copies is cheap, and no editing code can forget
/// to take part: whatever changes the project can be undone.
#[derive(Debug, Clone)]
pub struct History {
    /// The project as of the newest step.
    committed: Project,
    /// Earlier states, oldest first.
    undo_steps: Vec<Project>,
    /// States that were undone, most recently undone last.
    redo_steps: Vec<Project>,
    /// A change that has not become a step yet: the project as last seen,
    /// and when it last changed.
    pending: Option<(Project, Instant)>,
}

impl History {
    /// Starts a history in which `project` is the oldest known state.
    pub fn new(project: &Project) -> Self {
        Self {
            committed: project.clone(),
            undo_steps: Vec::new(),
            redo_steps: Vec::new(),
            pending: None,
        }
    }

    /// Shows the history the current project. Call once per frame.
    ///
    /// `is_user_busy` is true while the user is in the middle of something
    /// (a mouse button is held down): then nothing becomes a step yet.
    pub fn observe(&mut self, project: &Project, is_user_busy: bool, now: Instant) {
        if *project == self.committed {
            self.pending = None;
            return;
        }
        match &self.pending {
            // Still the same change as last frame: has it come to rest?
            Some((seen, since)) if seen == project => {
                if !is_user_busy && now.duration_since(*since) >= SETTLE_TIME {
                    self.commit(project);
                }
            }
            // Changed again (or for the first time): start waiting anew.
            _ => self.pending = Some((project.clone(), now)),
        }
    }

    /// Makes the current project the newest step.
    fn commit(&mut self, project: &Project) {
        if *project == self.committed {
            return;
        }
        let before = mem::replace(&mut self.committed, project.clone());
        self.undo_steps.push(before);
        if self.undo_steps.len() > STEP_LIMIT {
            self.undo_steps.remove(0);
        }
        // A new change makes what was undone unreachable.
        self.redo_steps.clear();
        self.pending = None;
    }

    pub fn can_undo(&self, project: &Project) -> bool {
        !self.undo_steps.is_empty() || *project != self.committed
    }

    pub fn can_redo(&self, project: &Project) -> bool {
        !self.redo_steps.is_empty() && *project == self.committed
    }

    /// Takes the project back one step. Returns `false` if there is nothing
    /// to undo.
    pub fn undo(&mut self, project: &mut Project) -> bool {
        // A change that has not settled yet is the first thing to undo.
        self.commit(project);
        let Some(previous) = self.undo_steps.pop() else {
            return false;
        };
        let undone = mem::replace(&mut self.committed, previous.clone());
        self.redo_steps.push(undone);
        *project = previous;
        true
    }

    /// Brings back the step that was undone last. Returns `false` if there
    /// is nothing to redo (also after any new change).
    pub fn redo(&mut self, project: &mut Project) -> bool {
        self.commit(project);
        let Some(next) = self.redo_steps.pop() else {
            return false;
        };
        let redone = mem::replace(&mut self.committed, next.clone());
        self.undo_steps.push(redone);
        *project = next;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_author(author: &str) -> Project {
        let mut project = Project::new();
        project.metadata.author = author.to_owned();
        project
    }

    /// Lets a change settle: seen once, then seen again after the wait.
    fn settle(history: &mut History, project: &Project, start: Instant) -> Instant {
        history.observe(project, false, start);
        let later = start + SETTLE_TIME;
        history.observe(project, false, later);
        later
    }

    #[test]
    fn settled_changes_can_be_undone_and_redone_in_order() {
        let mut project = with_author("a");
        let mut history = History::new(&project);
        let mut now = Instant::now();
        assert!(!history.can_undo(&project) && !history.can_redo(&project));

        for author in ["b", "c"] {
            project = with_author(author);
            now = settle(&mut history, &project, now);
        }

        assert!(history.undo(&mut project));
        assert_eq!(project.metadata.author, "b");
        assert!(history.undo(&mut project));
        assert_eq!(project.metadata.author, "a");
        assert!(!history.undo(&mut project));

        assert!(history.can_redo(&project));
        assert!(history.redo(&mut project));
        assert!(history.redo(&mut project));
        assert_eq!(project.metadata.author, "c");
        assert!(!history.redo(&mut project));
    }

    #[test]
    fn a_burst_of_changes_is_one_step() {
        let mut project = with_author("start");
        let mut history = History::new(&project);
        let mut now = Instant::now();

        // A drag: the project changes every frame while the button is
        // held, then rests.
        for (frame, author) in ["s1", "s2", "s3"].into_iter().enumerate() {
            project = with_author(author);
            now += Duration::from_millis(16);
            history.observe(&project, frame < 2, now);
        }
        // Resting, but the button is still down: no step yet.
        history.observe(&project, true, now + SETTLE_TIME);
        assert!(history.undo_steps.is_empty());
        history.observe(&project, false, now + SETTLE_TIME);
        assert_eq!(history.undo_steps.len(), 1);

        assert!(history.undo(&mut project));
        assert_eq!(project.metadata.author, "start");
    }

    #[test]
    fn an_unsettled_change_is_undone_first_and_a_new_change_ends_redo() {
        let mut project = with_author("a");
        let mut history = History::new(&project);
        let now = settle(&mut history, &with_author("b"), Instant::now());
        project = with_author("c");
        history.observe(&project, false, now);

        // "c" was typed a moment ago and has not settled.
        assert!(history.can_undo(&project));
        assert!(history.undo(&mut project));
        assert_eq!(project.metadata.author, "b");
        assert!(history.redo(&mut project));
        assert_eq!(project.metadata.author, "c");

        history.undo(&mut project);
        project = with_author("d");
        assert!(!history.can_redo(&project));
        assert!(!history.redo(&mut project));
        assert_eq!(project.metadata.author, "d");
    }

    #[test]
    fn only_the_newest_steps_are_kept() {
        let mut project = with_author("0");
        let mut history = History::new(&project);
        let mut now = Instant::now();

        for step in 1..=STEP_LIMIT + 5 {
            project = with_author(&step.to_string());
            now = settle(&mut history, &project, now);
        }

        assert_eq!(history.undo_steps.len(), STEP_LIMIT);
        while history.undo(&mut project) {}
        assert_eq!(project.metadata.author, "5");
    }
}
