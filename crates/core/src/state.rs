//! Deterministic state transitions. File/process/socket I/O belongs to adapters.
mod app_commands;
mod artifact_commands;
mod event_commands;
mod ipr;
mod job_commands;
mod job_lifecycle;
pub(crate) mod project_commands;
mod render_commands;
mod workspace_commands;

use crate::services::{ServiceConfig, ServiceTask, Work, WorkResult};
use protocol::commands::*;
use protocol::jobs::*;
use protocol::view::*;
use protocol::{
    ErrorCode, ErrorPayload, Event, JobId, Method, Request, Response, Revision, SceneId,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use uuid::Uuid;

#[derive(Clone)]
pub struct LoadTask {
    pub scene_id: SceneId,
    pub scene_revision: Revision,
    pub job_id: JobId,
    pub absolute_path: String,
    cancelled: Arc<AtomicBool>,
}
impl LoadTask {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
struct JobRecord {
    job: Job,
    task: LoadTask,
}
pub struct CoreState {
    projects: project_commands::Projects,
    layout_revision: u64,
    pub(crate) service_config: Option<ServiceConfig>,
    service_tasks: VecDeque<ServiceTask>,
    artifacts: HashMap<protocol::ArtifactId, Artifact>,
    quit_requested: bool,
    settings: RenderSettings,
    settings_revision: u64,
    /// Shared native view of the active project lifecycle: context pins it to
    /// one project activation, the counter is the projects' view revision.
    pub(crate) project_view: Option<(protocol::project::ProjectContext, ViewState, u64)>,
    pub(crate) limits: Limits,
    capabilities: Capabilities,
    session_id: Uuid,
    jobs: Vec<JobRecord>,
    job_indices: HashMap<JobId, usize>,
    requests: HashMap<String, (Vec<u8>, Response)>,
    events: VecDeque<Event>,
    sequence: u64,
    ipr: ipr::Ipr,
}
pub(crate) fn invalid(message: impl Into<String>) -> ErrorPayload {
    ErrorPayload::new(ErrorCode::InvalidRequest, message)
}
pub(crate) fn params<T: DeserializeOwned>(request: &Request) -> Result<T, ErrorPayload> {
    let value = if request.params.is_null() {
        json!({})
    } else {
        request.params.clone()
    };
    serde_json::from_value(value).map_err(|error| invalid(error.to_string()))
}
fn changing(method: &Method) -> bool {
    matches!(
        method,
        Method::ProjectImport
            | Method::ProjectImportScene
            | Method::ProjectImportResource
            | Method::ProjectOpen
            | Method::ProjectCreate
            | Method::ProjectClose
            | Method::ProjectSave
            | Method::ProjectExport
            | Method::ProjectPreview
            | Method::ProjectRender
            | Method::ProjectCapture
            | Method::ProjectPick
            | Method::ProjectOpenScene
            | Method::ProjectCloseScene
            | Method::ProjectApply
            | Method::ProjectSetParameterValues
            | Method::ProjectUndo
            | Method::ProjectRedo
            | Method::RenderSetSettings
            | Method::RenderIprSet
            | Method::JobCancel
            | Method::ArtifactExport
            | Method::WorkspaceResetLayout
            | Method::AppQuit
    )
}
fn revision(expected: &Revision, current: u64, name: &str) -> Result<(), ErrorPayload> {
    if expected.to_u64() != Some(current) {
        Err(ErrorPayload::new(
            ErrorCode::RevisionConflict,
            format!("{name} revision changed"),
        )
        .with_details(json!({"expected":expected,"current":current.to_string()})))
    } else {
        Ok(())
    }
}
impl CoreState {
    /// True when the service host is attached with a capture adapter, i.e.
    /// shared capture APIs can serve native project geometry.
    fn capture_attached(&self) -> bool {
        self.service_config
            .as_ref()
            .is_some_and(|config| config.capture.is_some())
    }

    pub fn new(limits: Limits) -> Self {
        assert!(
            limits.command_queue > 0 && limits.retained_events > 0 && limits.request_records > 0
        );
        Self {
            projects: project_commands::Projects::default(),
            layout_revision: 0,
            service_config: None,
            service_tasks: VecDeque::new(),
            artifacts: HashMap::new(),
            quit_requested: false,
            settings: RenderSettings::default(),
            settings_revision: 0,
            project_view: None,
            limits,
            capabilities: Capabilities {
                project_management: true,
                project_editing: true,
                project_export: true,
                scene_capture: false,
                render: false,
                ipr: false,
                workspace_layout: false,
                project: protocol::commands::ProjectCapabilities {
                    viewer_mode: true,
                    multi_scene: true,
                    folder_format: true,
                    scene_graph: true,
                    presets: true,
                },
            },
            session_id: Uuid::new_v4(),
            jobs: Vec::new(),
            job_indices: HashMap::new(),
            requests: HashMap::new(),
            events: VecDeque::new(),
            sequence: 0,
            ipr: ipr::Ipr::default(),
        }
    }
    pub fn snapshot(&self) -> AppState {
        AppState {
            project: self.projects.state(self.capture_attached()),
            session_id: self.session_id,
            settings_revision: Revision::from_u64(self.settings_revision),
            event_sequence: Revision::from_u64(self.sequence),
            layout_revision: Revision::from_u64(self.layout_revision),
            ipr: self.ipr_status(),
            quit_requested: self.quit_requested,
        }
    }
    pub(crate) fn validate_request(
        &self,
        request: &Request,
    ) -> Result<(Vec<u8>, bool), Box<Response>> {
        if request.protocol != protocol::PROTOCOL_VERSION {
            return Err(Box::new(Response::err(
                request.request_id.clone(),
                ErrorPayload::new(ErrorCode::UnsupportedProtocol, "unsupported protocol"),
            )));
        }
        if request.request_id.is_empty() || request.request_id.len() > 256 {
            return Err(Box::new(Response::err(
                request.request_id.clone(),
                invalid("request_id must contain 1..256 UTF-8 bytes"),
            )));
        }
        let operation = match protocol::ndjson::canonical_operation(request) {
            Ok(operation) => operation,
            Err(error) => {
                return Err(Box::new(Response::err(
                    request.request_id.clone(),
                    invalid(error.to_string()),
                )));
            }
        };
        if let Some((previous, response)) = self.requests.get(&request.request_id) {
            return if previous == &operation {
                Err(Box::new(response.clone()))
            } else {
                Err(Box::new(Response::err(
                    request.request_id.clone(),
                    ErrorPayload::new(
                        ErrorCode::RequestIdConflict,
                        "request_id already identifies a different operation",
                    ),
                )))
            };
        }
        let record = changing(&request.method);
        if record && self.requests.len() >= self.limits.request_records {
            return Err(Box::new(Response::err(
                request.request_id.clone(),
                ErrorPayload::new(ErrorCode::Busy, "session request record limit reached"),
            )));
        }
        Ok((operation, record))
    }
    pub fn handle(&mut self, request: Request) -> Response {
        let (operation, record) = match self.validate_request(&request) {
            Ok(value) => value,
            Err(response) => return *response,
        };
        let response = match self.dispatch(&request) {
            Ok(result) => Response::ok(&request.request_id, result),
            Err(error) => Response::err(&request.request_id, error),
        };
        if record {
            self.requests
                .insert(request.request_id, (operation, response.clone()));
        }
        response
    }
    fn dispatch(&mut self, request: &Request) -> Result<Value, ErrorPayload> {
        match request.method {
            Method::ProjectGetState
            | Method::ProjectImport
            | Method::ProjectImportScene
            | Method::ProjectImportResource
            | Method::ProjectOpen
            | Method::ProjectCreate
            | Method::ProjectClose
            | Method::ProjectSave
            | Method::ProjectExport
            | Method::ProjectPreview
            | Method::ProjectPreviewTransforms
            | Method::ProjectRender
            | Method::ProjectCapture
            | Method::ProjectPick
            | Method::ProjectListNodes
            | Method::ProjectListContents
            | Method::ProjectListReferences
            | Method::ProjectListScenes
            | Method::ProjectOpenScene
            | Method::ProjectCloseScene
            | Method::ProjectListPresets
            | Method::ProjectSearchNodes
            | Method::ProjectSearchContents
            | Method::ProjectGetView
            | Method::ProjectSetCamera
            | Method::ProjectConfigureView
            | Method::ProjectGetNode
            | Method::ProjectGetResource
            | Method::ProjectGetAsset
            | Method::ProjectGetCache
            | Method::ProjectPutCache
            | Method::ProjectGetParameters
            | Method::ProjectGetParameterValues
            | Method::ProjectSetParameterValues
            | Method::ProjectPlanImplementation
            | Method::ProjectGetResourceGraph
            | Method::ProjectSpectrumColor
            | Method::ProjectPreviewTexture
            | Method::ProjectPreviewMaterial
            | Method::ProjectPreviewAsset
            | Method::ProjectApply
            | Method::ProjectUndo
            | Method::ProjectRedo => self.handle_project(request),
            // Static information: the parameter
            // table does not depend on an open project.
            Method::ProjectGetParameterSchema => self.handle_parameter_schema(request),
            // the builtin spectrum names, static like the table.
            Method::ProjectListSpectra => {
                let _: protocol::commands::EmptyParams = params(request)?;
                Ok(json!({"spectra":pbrt_io::BUILTIN_SPECTRA}))
            }
            Method::AppHello
            | Method::AppGetState
            | Method::AppGetCapabilities
            | Method::AppQuit => self.handle_app(request),
            Method::RenderGetSettings | Method::RenderSetSettings => self.handle_render(request),
            Method::RenderIprSet | Method::RenderIprSubmit => self.handle_ipr(request),
            Method::WorkspaceResetLayout => self.handle_workspace(request),
            Method::ArtifactGet | Method::ArtifactExport => self.handle_artifact(request),
            Method::JobGet | Method::JobCancel | Method::JobList => self.handle_jobs(request),
            Method::EventsRead => self.handle_events(request),
        }
    }
    fn check_limit(&self, limit: usize) -> Result<(), ErrorPayload> {
        if limit == 0 || limit > self.limits.max_page_size {
            Err(invalid("limit must be within 1..max_page_size"))
        } else {
            Ok(())
        }
    }
    fn emit_job(&mut self, job_id: JobId) {
        let job = self.job(job_id).unwrap().clone();
        self.emit("job.changed", job.scene_id, Some(job_id), json!(job));
    }
    fn emit(
        &mut self,
        kind: &str,
        scene_id: Option<SceneId>,
        job_id: Option<JobId>,
        payload: Value,
    ) {
        self.sequence += 1;
        self.events.push_back(Event {
            sequence: self.sequence.to_string(),
            kind: kind.into(),
            scene_id,
            job_id,
            payload: Some(payload),
        });
        while self.events.len() > self.limits.retained_events {
            self.events.pop_front();
        }
    }
    pub(crate) fn shutdown(&mut self) {
        for record in &self.jobs {
            record.task.cancel();
        }
    }
}
