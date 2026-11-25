//! Unsaved-changes confirmation: what the dialog buttons do.
//!
//! The window owns the dialog and the pending action; the decisions live here
//! so they can be tested without a window (GPUI's recursion limit forbids tests
//! in `editor/session.rs`).
use std::path::{Path, PathBuf};

/// An operation that replaces or ends the current document. It waits while the
/// unsaved-changes dialog asks whether to save first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingAction {
    OpenProject(PathBuf),
    ImportScene(PathBuf),
    CreateProject(NewProject),
    CloseDocument,
    CloseWindow,
    Quit,
}

/// A resolved New Project: what `project.create` takes. The title is
/// the dialog's, or the folder's name when the dialog left it empty
///.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewProject {
    pub path: PathBuf,
    pub title: String,
}

/// Resolve the New Project dialog's folder and title into the
/// `project.create` pair. An empty title takes the folder's name, like
/// the CLI default. The picker only offers existing folders while
/// `project.create` wants a new or empty one, so a folder that already
/// holds something gets a subfolder named after the title. `None` when
/// the folder names nothing usable; the server
/// owns the deeper validation.
pub fn resolve_new_project(folder: &str, title: &str) -> Option<NewProject> {
    let folder = folder.trim();
    let title = title.trim();
    let name = Path::new(folder).file_name()?.to_str()?;
    let title = if title.is_empty() { name } else { title };
    let mut path = PathBuf::from(folder);
    let occupied = std::fs::read_dir(&path).is_ok_and(|mut entries| entries.next().is_some());
    if occupied {
        path.push(project::presets::file_stem(title).unwrap_or_else(|| "Project".to_owned()));
    }
    Some(NewProject {
        path,
        title: title.to_owned(),
    })
}

/// The dialog's three buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsavedChoice {
    Save,
    Discard,
    Cancel,
}

/// What to do after the user presses a button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogStep {
    /// Do nothing.
    Nothing,
    /// Save in place, then run the action when the save job settles.
    SaveFirst(PendingAction),
    /// Run the action now with `discard_unsaved: true`.
    Run(PendingAction, bool),
}

pub fn choose(choice: UnsavedChoice, action: PendingAction) -> DialogStep {
    match choice {
        UnsavedChoice::Save => DialogStep::SaveFirst(action),
        UnsavedChoice::Discard => DialogStep::Run(action, true),
        UnsavedChoice::Cancel => DialogStep::Nothing,
    }
}

/// What to do once the in-place save settles. A failed save keeps the document
/// and drops the pending action; the status line carries the error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AfterSave {
    Run(PendingAction),
    Drop,
    Nothing,
}

pub fn after_save(ok: bool, pending: Option<PendingAction>) -> AfterSave {
    match (ok, pending) {
        (true, Some(action)) => AfterSave::Run(action),
        (false, Some(_)) => AfterSave::Drop,
        (_, None) => AfterSave::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action() -> PendingAction {
        PendingAction::OpenProject(PathBuf::from("/tmp/next"))
    }

    #[test]
    fn save_keeps_the_action_until_the_save_settles() {
        assert_eq!(
            choose(UnsavedChoice::Save, action()),
            DialogStep::SaveFirst(action())
        );
    }

    #[test]
    fn discard_runs_the_action_with_discard_unsaved() {
        assert_eq!(
            choose(UnsavedChoice::Discard, action()),
            DialogStep::Run(action(), true)
        );
    }

    #[test]
    fn cancel_does_nothing() {
        assert_eq!(choose(UnsavedChoice::Cancel, action()), DialogStep::Nothing);
    }

    #[test]
    fn a_successful_save_runs_the_pending_action() {
        assert_eq!(after_save(true, Some(action())), AfterSave::Run(action()));
        assert_eq!(after_save(true, None), AfterSave::Nothing);
    }

    #[test]
    fn a_failed_save_drops_the_pending_action() {
        assert_eq!(after_save(false, Some(action())), AfterSave::Drop);
        assert_eq!(after_save(false, None), AfterSave::Nothing);
    }

    /// Resolve the New Project dialog's folder and title into the path
    /// `project.create` takes — an untouched title takes the folder's
    /// name — and folders that name nothing are
    /// refused client-side.
    #[test]
    fn new_project_resolution_titles_from_the_folder_name() {
        assert_eq!(
            resolve_new_project("/home/me/My Room/", ""),
            Some(NewProject {
                path: PathBuf::from("/home/me/My Room"),
                title: "My Room".to_owned(),
            })
        );
        assert_eq!(
            resolve_new_project(" /home/me/room ", "  My Room  "),
            Some(NewProject {
                path: PathBuf::from("/home/me/room"),
                title: "My Room".to_owned(),
            })
        );
        assert_eq!(resolve_new_project("", "T"), None);
        assert_eq!(resolve_new_project("   ", "T"), None);
        assert_eq!(resolve_new_project("/", "T"), None);
        assert_eq!(resolve_new_project("..", "T"), None);
        // A folder that already holds something gets a titled subfolder.
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("note.txt"), "x").unwrap();
        let folder = temp.path().to_str().unwrap();
        assert_eq!(
            resolve_new_project(folder, "My Room: v1").unwrap().path,
            temp.path().join("My Room_ v1")
        );
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(
            resolve_new_project(empty.path().to_str().unwrap(), "T")
                .unwrap()
                .path,
            empty.path()
        );
    }
}
