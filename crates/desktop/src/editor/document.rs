//! Document mode and menu decisions derived from the typed project state.
//!
//! The GUI keeps `protocol::project::ProjectState` as received from the core and
//! derives this view once per update. Pure decisions live here so they can be
//! tested without a window (GPUI's recursion limit forbids tests in
//! `editor/session.rs`).
use protocol::ids::Revision;
use protocol::project::{ProjectContext, ProjectMode, ProjectRef, ProjectState};
use std::path::PathBuf;

/// What the window currently shows. `ProjectState` still owns the wire values;
/// this only classifies them for menus, panels and the Inspector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentState {
    Empty,
    Viewer {
        project: ProjectRef,
        title: String,
        source_path: PathBuf,
    },
    Project {
        project: ProjectRef,
        title: String,
        path: PathBuf,
        dirty: bool,
    },
}

impl DocumentState {
    pub fn from_state(state: &ProjectState) -> Self {
        let Some(project) = state.context.project.clone() else {
            return Self::Empty;
        };
        let title = state.title.clone().unwrap_or_default();
        // A reference without a mode is only possible from an older core; the
        // read-only flag is the safest classification available there.
        let mode = state.mode.unwrap_or(if state.read_only {
            ProjectMode::Viewer
        } else {
            ProjectMode::Project
        });
        match mode {
            ProjectMode::Viewer => Self::Viewer {
                project,
                title,
                source_path: path_of(state.source_path.as_deref().or(state.path.as_deref())),
            },
            ProjectMode::Project => Self::Project {
                project,
                title,
                path: path_of(state.path.as_deref().or(state.source_path.as_deref())),
                dirty: state.dirty,
            },
        }
    }

    pub fn is_compatibility(&self) -> bool {
        matches!(self, Self::Viewer { source_path, .. } if source_path.file_name().is_some_and(|name| name == "project.pbrtui.json"))
    }

    pub fn is_read_only(&self) -> bool {
        matches!(self, Self::Viewer { .. })
    }

    pub fn project(&self) -> Option<&ProjectRef> {
        match self {
            Self::Empty => None,
            Self::Viewer { project, .. } | Self::Project { project, .. } => Some(project),
        }
    }
}

/// The state before the core reports any project.
pub fn empty_project_state() -> ProjectState {
    ProjectState {
        context: ProjectContext {
            activation: Revision::initial(),
            project: None,
        },
        path: None,
        mode: None,
        source_path: None,
        read_only: false,
        active_scene: None,
        active_up: None,
        active_handedness: None,
        default_scene: None,
        scene_count: 0,
        title: None,
        saved_revision: None,
        dirty: false,
        can_undo: false,
        can_redo: false,
        node_count: 0,
        resource_count: 0,
        asset_count: 0,
        switch_job_id: None,
        preview_connected: false,
    }
}

/// Where `Save` sends the document. A cancelled picker sends nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveAction {
    Disabled,
    /// Viewer: a destination picker is required (`Save as Project`).
    ChooseDestination,
    /// Project: save to the folder already in use.
    InPlace,
}

pub fn save_action(document: &DocumentState) -> SaveAction {
    match document {
        DocumentState::Empty => SaveAction::Disabled,
        DocumentState::Viewer { .. } => SaveAction::ChooseDestination,
        DocumentState::Project { .. } => SaveAction::InPlace,
    }
}

/// Whether a request that replaces the document (Open, Import, Close, Quit)
/// must ask about unsaved changes first. A Viewer is never dirty.
pub fn needs_unsaved_confirmation(document: &DocumentState) -> bool {
    matches!(document, DocumentState::Project { dirty: true, .. })
}

/// Files the `Open PBRT Scene` picker accepts.
pub fn is_pbrt_scene_path(path: &std::path::Path) -> bool {
    path.to_str().is_some_and(|name| {
        let name = name.to_ascii_lowercase();
        name.ends_with(".pbrt") || name.ends_with(".pbrt.gz")
    })
}

/// Console lines for the diagnostics an import response reports. `project.open`
/// reports `null`; `project.import` carries the report.
pub fn import_diagnostic_lines(report: &serde_json::Value) -> Vec<String> {
    code_message_lines(&report["diagnostics"])
}

/// What an export left out (its result's `warnings`), as Console lines.
pub fn export_warning_lines(result: &serde_json::Value) -> Vec<String> {
    code_message_lines(&result["warnings"])
}

/// `code: message` for each entry of a diagnostics or warnings list.
fn code_message_lines(list: &serde_json::Value) -> Vec<String> {
    list.as_array()
        .into_iter()
        .flatten()
        .map(|diagnostic| {
            format!(
                "{}: {}",
                diagnostic["code"].as_str().unwrap_or("diagnostic"),
                diagnostic["message"].as_str().unwrap_or("")
            )
        })
        .collect()
}

/// Warnings in an import report. The status bar counts these; errors keep the
/// document from opening and arrive as a failed job instead.
pub fn import_warning_count(report: &serde_json::Value) -> usize {
    report["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|diagnostic| {
            diagnostic["severity"]
                .as_str()
                .is_none_or(|severity| severity == "warning")
        })
        .count()
}

/// Where a settled import job keeps its diagnostics: project switches wrap
/// them in `report`, while additions to the open project carry them at
/// the top of the result. The result is safe to index either way.
pub fn import_report(result: &serde_json::Value) -> &serde_json::Value {
    if result["diagnostics"].is_array() {
        result
    } else {
        &result["report"]
    }
}

/// One-line status for a settled import that added items to the open
/// project. Project switches and `project.open` carry no `created` list and
/// keep their existing status.
pub fn import_summary(result: &serde_json::Value) -> Option<String> {
    let created = result["created"].as_array()?;
    match created.len() {
        0 => Some("Imported with no new items".into()),
        count => Some(format!(
            "Imported {} item{}",
            count,
            if count == 1 { "" } else { "s" }
        )),
    }
}

/// A folder name the Save-as-Project picker can start from.
pub fn suggested_project_name(title: &str) -> String {
    let mut name = String::new();
    for character in title.trim().chars() {
        if character.is_alphanumeric() || matches!(character, '-' | '_' | '.') {
            name.push(character);
        } else if character.is_whitespace() {
            name.push('-');
        } else {
            name.push('_');
        }
    }
    let name = name.trim_matches(['-', '_', '.']);
    if name.is_empty() {
        "project".into()
    } else {
        name.to_owned()
    }
}

fn path_of(path: Option<&str>) -> PathBuf {
    path.map(PathBuf::from).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::ids::Revision;
    use protocol::project::{ProjectContext, ProjectMode, ProjectRef, ProjectState};
    use std::path::PathBuf;

    fn state(mode: Option<ProjectMode>, dirty: bool, read_only: bool) -> ProjectState {
        ProjectState {
            context: ProjectContext {
                activation: Revision::from_u64(3),
                project: Some(ProjectRef {
                    project_id: uuid::Uuid::from_u128(7),
                    revision: Revision::from_u64(11),
                    activation: Revision::from_u64(3),
                }),
            },
            path: Some("/tmp/saved-project".into()),
            mode,
            source_path: Some("/tmp/source.pbrt".into()),
            read_only,
            active_scene: None,
            active_up: None,
            active_handedness: None,
            default_scene: None,
            scene_count: 1,
            title: Some("My Scene".into()),
            saved_revision: Some(Revision::from_u64(11)),
            dirty,
            can_undo: dirty,
            can_redo: false,
            node_count: 2,
            resource_count: 0,
            asset_count: 0,
            switch_job_id: None,
            preview_connected: true,
        }
    }

    #[test]
    fn document_state_empty_without_a_project_reference() {
        let mut empty = state(None, false, false);
        empty.context.project = None;
        assert_eq!(DocumentState::from_state(&empty), DocumentState::Empty);
    }

    #[test]
    fn document_state_viewer_uses_the_source_path() {
        let document = DocumentState::from_state(&state(Some(ProjectMode::Viewer), false, true));
        assert!(document.is_read_only());
        assert_eq!(
            document,
            DocumentState::Viewer {
                project: state(Some(ProjectMode::Viewer), false, true)
                    .context
                    .project
                    .unwrap(),
                title: "My Scene".into(),
                source_path: PathBuf::from("/tmp/source.pbrt"),
            }
        );
        assert!(document.project().is_some());
    }

    #[test]
    fn document_state_project_keeps_path_and_dirty() {
        let document = DocumentState::from_state(&state(Some(ProjectMode::Project), true, false));
        assert!(!document.is_read_only());
        assert_eq!(
            document,
            DocumentState::Project {
                project: state(Some(ProjectMode::Project), true, false)
                    .context
                    .project
                    .unwrap(),
                title: "My Scene".into(),
                path: PathBuf::from("/tmp/saved-project"),
                dirty: true,
            }
        );
    }

    #[test]
    fn save_requires_a_destination_for_a_viewer_and_is_in_place_for_a_project() {
        assert_eq!(save_action(&DocumentState::Empty), SaveAction::Disabled);
        assert_eq!(
            save_action(&DocumentState::from_state(&state(
                Some(ProjectMode::Viewer),
                false,
                true
            ))),
            SaveAction::ChooseDestination
        );
        assert_eq!(
            save_action(&DocumentState::from_state(&state(
                Some(ProjectMode::Project),
                true,
                false
            ))),
            SaveAction::InPlace
        );
    }

    #[test]
    fn only_a_dirty_project_confirms_before_replacing_the_document() {
        assert!(!needs_unsaved_confirmation(&DocumentState::Empty));
        assert!(!needs_unsaved_confirmation(&DocumentState::from_state(
            &state(Some(ProjectMode::Viewer), false, true)
        )));
        assert!(!needs_unsaved_confirmation(&DocumentState::from_state(
            &state(Some(ProjectMode::Project), false, false)
        )));
        assert!(needs_unsaved_confirmation(&DocumentState::from_state(
            &state(Some(ProjectMode::Project), true, false)
        )));
    }

    #[test]
    fn pbrt_scene_paths_include_gzip() {
        assert!(is_pbrt_scene_path(std::path::Path::new("scene.pbrt")));
        assert!(is_pbrt_scene_path(std::path::Path::new("SCENE.PBRT.GZ")));
        assert!(!is_pbrt_scene_path(std::path::Path::new("project")));
        assert!(!is_pbrt_scene_path(std::path::Path::new("notes.txt")));
    }

    #[test]
    fn export_warnings_become_console_lines() {
        let result = serde_json::json!({"scene_path": "/out/scene.pbrt", "warnings": [
            {"code": "imagemap_without_image_skipped", "resource_id": "r1",
             "message": "imagemap texture \"Pending\" has no image and was left out"},
        ]});
        assert_eq!(
            export_warning_lines(&result),
            vec![
                "imagemap_without_image_skipped: imagemap texture \"Pending\" has no image and was left out"
            ]
        );
        assert!(export_warning_lines(&serde_json::json!({"warnings": []})).is_empty());
    }

    #[test]
    fn import_diagnostics_become_console_lines() {
        let report = serde_json::json!({
            "diagnostics": [
                {"code": "unsupported_directive", "message": "ConcatTransform"},
                {"code": "missing_include", "message": "missing.pbrt"},
            ]
        });
        assert_eq!(
            import_diagnostic_lines(&report),
            vec![
                "unsupported_directive: ConcatTransform",
                "missing_include: missing.pbrt",
            ]
        );
        assert!(import_diagnostic_lines(&serde_json::Value::Null).is_empty());
    }

    #[test]
    fn import_warnings_count_only_warnings_and_tolerate_missing_severity() {
        let report = serde_json::json!({"diagnostics": [
            {"severity": "warning", "code": "a", "message": ""},
            {"severity": "error", "code": "b", "message": ""},
            {"severity": "info", "code": "d", "message": ""},
            {"code": "c", "message": ""},
        ]});
        assert_eq!(import_warning_count(&report), 2);
        // Info records never
        // count as warnings.
        let info_only = serde_json::json!({"diagnostics": [
            {"severity": "info", "code": "d", "message": ""},
        ]});
        assert_eq!(import_warning_count(&info_only), 0);
        assert_eq!(import_warning_count(&serde_json::Value::Null), 0);
    }

    /// additions put diagnostics at the top of the job result; project
    /// switches wrap them in `report`. Both shapes must produce lines.
    #[test]
    fn import_report_takes_diagnostics_from_both_result_shapes() {
        let wrapped = serde_json::json!({"report": {"diagnostics": [
            {"code": "wrapped", "message": ""}
        ]}});
        let direct = serde_json::json!({"created": [], "diagnostics": [
            {"code": "direct", "message": ""}
        ]});
        assert_eq!(
            import_diagnostic_lines(import_report(&wrapped)),
            vec!["wrapped: "]
        );
        assert_eq!(
            import_diagnostic_lines(import_report(&direct)),
            vec!["direct: "]
        );
        assert_eq!(
            import_warning_count(import_report(&serde_json::Value::Null)),
            0
        );
    }

    #[test]
    fn import_summary_names_the_added_item_count() {
        let single =
            serde_json::json!({"created": [{"kind": "scene", "id": "s", "location": "l"}]});
        assert_eq!(import_summary(&single).as_deref(), Some("Imported 1 item"));
        let pair = serde_json::json!({"created": [
            {"kind": "resource", "id": "r", "location": "l"},
            {"kind": "asset", "id": "a", "location": "l"},
        ]});
        assert_eq!(import_summary(&pair).as_deref(), Some("Imported 2 items"));
        assert_eq!(import_summary(&serde_json::json!({})), None);
        assert_eq!(
            import_summary(&serde_json::json!({"created": []})),
            Some("Imported with no new items".into())
        );
    }

    #[test]
    fn suggested_project_name_is_a_safe_folder_name() {
        assert_eq!(suggested_project_name("My Scene"), "My-Scene");
        assert_eq!(suggested_project_name("  "), "project");
        assert_eq!(suggested_project_name("../escape"), "escape");
        assert_eq!(suggested_project_name("a/b:c"), "a_b_c");
    }
}
