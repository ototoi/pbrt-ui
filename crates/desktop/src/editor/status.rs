//! Window title and status-bar values derived from the document state.
//!
//! Pure formatting lives here so it can be tested without a window.
use super::document::DocumentState;
use protocol::project::ProjectRef;
use scene_view::PreviewMode;

/// A window title uses the format `<title> — Viewer — pbrt-ui`,
/// `<project> / <scene> ● — pbrt-ui`, or `pbrt-ui` when empty.
pub fn window_title(document: &DocumentState, scene: Option<&str>) -> String {
    if document.is_compatibility() {
        return format!(
            "{} — Compatibility (read only) — pbrt-ui",
            document_label(document, scene)
        );
    }
    match document {
        DocumentState::Empty => "pbrt-ui".into(),
        DocumentState::Viewer { title, .. } => {
            if title.is_empty() {
                "Viewer — pbrt-ui".into()
            } else {
                format!("{title} — Viewer — pbrt-ui")
            }
        }
        DocumentState::Project { dirty, .. } => {
            let mut label = document_label(document, scene);
            if *dirty {
                label.push_str(" ●");
            }
            format!("{label} — pbrt-ui")
        }
    }
}

/// The project's title with its active scene. The dirty mark is separate so
/// the status bar can draw it as its own element.
fn document_label(document: &DocumentState, scene: Option<&str>) -> String {
    match document {
        DocumentState::Empty => String::new(),
        DocumentState::Viewer { title, .. } => title.clone(),
        DocumentState::Project { title, .. } => {
            let mut label = title.clone();
            if let Some(scene) = scene.filter(|scene| !scene.is_empty() && *scene != title) {
                label = format!("{label} / {scene}");
            }
            label
        }
    }
}

/// The values the status bar renders: badge, title, dirty mark, running job,
/// import warnings and display mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusBar {
    pub badge: Option<&'static str>,
    pub title: String,
    pub dirty: bool,
    pub job: Option<String>,
    pub warnings: usize,
    pub mode: &'static str,
}

pub fn status_bar(
    document: &DocumentState,
    scene: Option<&str>,
    jobs: &serde_json::Value,
    project: Option<&ProjectRef>,
    warnings: usize,
    mode: PreviewMode,
) -> StatusBar {
    StatusBar {
        badge: match document {
            DocumentState::Empty => None,
            DocumentState::Viewer { .. } if document.is_compatibility() => Some("COMPATIBILITY"),
            DocumentState::Viewer { .. } => Some("VIEWER"),
            DocumentState::Project { .. } => Some("PROJECT"),
        },
        title: document_label(document, scene),
        dirty: matches!(document, DocumentState::Project { dirty: true, .. }),
        job: active_job(jobs, project),
        warnings,
        mode: mode_label(mode),
    }
}

/// The active scene's label from the root of the project tree. The first node
/// is the scene itself.
pub fn scene_label(tree: &serde_json::Value) -> Option<String> {
    tree["nodes"]
        .as_array()?
        .first()
        .filter(|node| node["kind"].as_str() == Some("scene"))
        .and_then(|node| node["label"].as_str())
        .filter(|label| !label.is_empty())
        .map(str::to_owned)
}

/// The newest queued/running job for the current document, as `kind · phase`.
pub fn active_job(jobs: &serde_json::Value, project: Option<&ProjectRef>) -> Option<String> {
    jobs["jobs"]
        .as_array()
        .into_iter()
        .flatten()
        .rev()
        .find(|job| {
            matches!(
                job["state"].as_str(),
                Some("queued" | "running" | "cancelling")
            ) && job_belongs_to(job, project)
        })
        .map(|job| {
            format!(
                "{} · {}",
                job["kind"].as_str().unwrap_or("job"),
                job["phase"].as_str().unwrap_or("running")
            )
        })
}

fn job_belongs_to(job: &serde_json::Value, project: Option<&ProjectRef>) -> bool {
    match project {
        None => job["project"].is_null(),
        Some(project) => {
            job["project"]["project_id"].as_str() == Some(project.project_id.to_string().as_str())
                && job["project"]["activation"].as_str() == Some(project.activation.as_str())
        }
    }
}

pub fn mode_label(mode: PreviewMode) -> &'static str {
    match mode {
        PreviewMode::Solid => "Solid",
        PreviewMode::Normal => "Normal",
        PreviewMode::Wireframe => "Wireframe",
        PreviewMode::Lit => "Lit",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::ids::Revision;
    use protocol::project::ProjectRef;
    use std::path::PathBuf;

    fn project() -> ProjectRef {
        ProjectRef {
            project_id: uuid::Uuid::from_u128(7),
            revision: Revision::from_u64(11),
            activation: Revision::from_u64(3),
        }
    }

    fn viewer() -> DocumentState {
        DocumentState::Viewer {
            project: project(),
            title: "cornell-box".into(),
            source_path: PathBuf::from("/tmp/cornell-box.pbrt"),
        }
    }

    fn saved_project(dirty: bool) -> DocumentState {
        DocumentState::Project {
            project: project(),
            title: "MyRoom".into(),
            path: PathBuf::from("/tmp/MyRoom"),
            dirty,
        }
    }

    #[test]
    fn titles_name_the_mode_and_scene() {
        assert_eq!(window_title(&DocumentState::Empty, None), "pbrt-ui");
        assert_eq!(
            window_title(&viewer(), None),
            "cornell-box — Viewer — pbrt-ui"
        );
        assert_eq!(
            window_title(&saved_project(false), Some("Kitchen")),
            "MyRoom / Kitchen — pbrt-ui"
        );
        assert_eq!(
            window_title(&saved_project(true), Some("Kitchen")),
            "MyRoom / Kitchen ● — pbrt-ui"
        );
        assert_eq!(
            window_title(&saved_project(false), Some("MyRoom")),
            "MyRoom — pbrt-ui"
        );
    }

    #[test]
    fn status_bar_reports_badge_dirty_and_mode() {
        let bar = status_bar(
            &saved_project(true),
            Some("Kitchen"),
            &serde_json::json!({"jobs": []}),
            Some(&project()),
            2,
            PreviewMode::Wireframe,
        );
        assert_eq!(bar.badge, Some("PROJECT"));
        assert_eq!(bar.title, "MyRoom / Kitchen");
        assert!(bar.dirty);
        assert_eq!(bar.warnings, 2);
        assert_eq!(bar.mode, "Wireframe");
        assert_eq!(bar.job, None);

        let viewer_bar = status_bar(
            &viewer(),
            None,
            &serde_json::json!({"jobs": []}),
            Some(&project()),
            0,
            PreviewMode::Solid,
        );
        assert_eq!(viewer_bar.badge, Some("VIEWER"));
        assert!(!viewer_bar.dirty);
    }

    #[test]
    fn active_job_skips_finished_and_foreign_jobs() {
        let jobs = serde_json::json!({"jobs": [
            {"kind": "render", "state": "running", "phase": "rendering",
             "project": {"project_id": "other", "activation": "3"}},
            {"kind": "project.save", "state": "succeeded", "phase": "complete",
             "project": {"project_id": "00000000-0000-0000-0000-000000000007", "activation": "3"}},
            {"kind": "project.preview", "state": "running", "phase": "publishing",
             "project": {"project_id": "00000000-0000-0000-0000-000000000007", "activation": "3"}}
        ]});
        assert_eq!(
            active_job(&jobs, Some(&project())).as_deref(),
            Some("project.preview · publishing")
        );
        assert_eq!(active_job(&serde_json::json!({"jobs": []}), None), None);
    }

    #[test]
    fn scene_label_reads_the_root_scene_node() {
        let tree = serde_json::json!({"nodes": [
            {"node_id": "1", "kind": "scene", "label": "Kitchen"},
            {"node_id": "2", "kind": "scope", "label": "Shapes"},
        ]});
        assert_eq!(scene_label(&tree).as_deref(), Some("Kitchen"));
        assert_eq!(
            scene_label(&serde_json::json!({"nodes": [{"kind": "scope", "label": "x"}]})),
            None
        );
        assert_eq!(scene_label(&serde_json::json!({"nodes": []})), None);
        assert_eq!(scene_label(&serde_json::Value::Null), None);
    }

    #[test]
    fn mode_labels_match_the_toolbar() {
        assert_eq!(mode_label(PreviewMode::Solid), "Solid");
        assert_eq!(mode_label(PreviewMode::Normal), "Normal");
        assert_eq!(mode_label(PreviewMode::Wireframe), "Wireframe");
        assert_eq!(mode_label(PreviewMode::Lit), "Lit");
    }
}
