//! Typed parameters and results for implemented methods. Every method rejects
//! unknown parameters; each method has an explicit DTO.

use crate::{ErrorPayload, JobId, Revision, SceneId, SessionId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyParams {}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelloParams {
    pub client_name: String,
    pub supported_protocols: Vec<String>,
}

/// `app.quit`: end the host (the desktop app or `pbrt-ui-host`) after the
/// same unsaved-changes guard as `project.close`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuitParams {
    #[serde(default)]
    pub discard_unsaved: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobParams {
    pub job_id: JobId,
}

fn default_limit() -> usize {
    100
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListJobsParams {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadEventsParams {
    pub after: Revision,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub wait_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub command_queue: usize,
    pub request_records: usize,
    pub retained_events: usize,
    pub max_line_bytes: usize,
    pub max_page_size: usize,
    pub max_event_wait_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            command_queue: 256,
            request_records: 100_000,
            retained_events: 10_000,
            max_line_bytes: crate::MAX_LINE_BYTES,
            max_page_size: 1000,
            max_event_wait_ms: 30_000,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub project_management: bool,
    pub project_editing: bool,
    pub project_export: bool,
    pub scene_capture: bool,
    pub render: bool,
    pub ipr: bool,
    pub workspace_layout: bool,
    pub project: ProjectCapabilities,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectCapabilities {
    pub viewer_mode: bool,
    pub multi_scene: bool,
    pub folder_format: bool,
    pub scene_graph: bool,
    pub presets: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    #[serde(default)]
    pub project: Option<crate::project::ProjectRef>,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    pub job_id: JobId,
    pub kind: String,
    pub state: JobState,
    pub phase: String,
    pub root_path: String,
    pub scene_id: Option<SceneId>,
    pub error: Option<ErrorPayload>,
    #[serde(default)]
    pub artifacts: Vec<crate::ArtifactId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppState {
    pub project: crate::project::ProjectState,
    pub session_id: SessionId,
    pub settings_revision: Revision,
    pub event_sequence: Revision,
    pub layout_revision: Revision,
    pub ipr: crate::jobs::IprStatus,
    /// `app.quit` was accepted; the host is shutting down.
    #[serde(default)]
    pub quit_requested: bool,
}
