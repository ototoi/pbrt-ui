use super::*;
use project::{ProjectSession, ProjectSnapshot, SaveReceipt};
use protocol::project as dto;
mod asset_preview;
mod commands;
mod completion;
mod edits;
mod material_preview;
pub(crate) mod preview_task;
mod preview_transforms;
mod queries;
mod resource_graph;
mod scene_graph;
mod spectrum_color;
mod texture_preview;

/// Methods that queue work on the serial project I/O executor: a full
/// queue answers Busy instead of piling up plans and snapshots.
pub(crate) fn queues_project_io(method: &Method) -> bool {
    matches!(
        method,
        Method::ProjectImport
            | Method::ProjectImportScene
            | Method::ProjectImportResource
            | Method::ProjectSetParameterValues
            | Method::ProjectOpen
            | Method::ProjectCreate
            | Method::ProjectSave
            | Method::ProjectExport
            | Method::ProjectPreview
            | Method::ProjectRender
            | Method::ProjectCapture
            | Method::ProjectPick
            | Method::ProjectOpenScene
            | Method::ProjectCloseScene
    )
}
/// A data asset's patched copy, made on the I/O executor.
pub(crate) struct ArrayCopy {
    pub asset: project::AssetId,
    /// For the error when its file is gone.
    pub location: String,
    pub size: u64,
    pub sha256: String,
    pub patches: Vec<(u64, Vec<u8>)>,
    pub new_id: project::AssetId,
}
pub(crate) enum Work {
    /// `project.set_parameter_values`: `data` for a data asset; an inline
    /// array has nothing to copy.
    ArrayEdit {
        store_id: Uuid,
        snapshot: ProjectSnapshot,
        data: Option<ArrayCopy>,
    },
    AddImport {
        source: String,
        scene: bool,
        staging_base: std::path::PathBuf,
    },
    Import {
        source: String,
        destination: Option<String>,
    },
    Open {
        path: String,
    },
    /// `project.create`: build a fresh editable folder project.
    Create {
        path: String,
        title: String,
        coordinate_system: project::CoordinateSystem,
    },
    Save {
        store_id: Uuid,
        snapshot: ProjectSnapshot,
        destination: Option<String>,
        /// Save As: the session's retained payloads, relinked into the new
        /// project's stage on the worker.
        retained: Vec<Arc<project::PayloadLease>>,
    },
    Export {
        store_id: Uuid,
        snapshot: ProjectSnapshot,
        destination: String,
    },
    Preview {
        store_id: Uuid,
        snapshot: ProjectSnapshot,
        destination: String,
        store: Option<String>,
        /// Shared tessellation cache when a service host is attached;
        /// `None` falls back to a job-local cache.
        geometry_cache: Option<std::sync::Arc<std::sync::Mutex<scene_parse::GeometryCache>>>,
        /// Stage a verified `assets/` copy for external
        /// callers' self-contained bundle) or resolve asset files in
        /// place from the store (the app's own slots).
        stage_assets: bool,
    },
}
pub(crate) struct IoTask {
    pub control: LoadTask,
    pub work: Work,
}
pub(crate) enum IoCommand {
    Run(IoTask),
    Release(Uuid),
    SetDefaultScene {
        store_id: Uuid,
        scene: Option<project::SceneId>,
    },
}
pub(crate) enum IoOutput {
    ArrayPrepared {
        lease: Option<Arc<project::PayloadLease>>,
    },
    ImportPrepared {
        batch: project::PreparedImport,
        diagnostics: Vec<project::ImportDiagnostic>,
    },
    Loaded {
        snapshot: ProjectSnapshot,
        store_id: Uuid,
        path: Option<String>,
        source_path: Option<String>,
        unregistered: Vec<String>,
        published: bool,
        report: Option<pbrt_io::ImportReport>,
        mode: dto::ProjectMode,
        /// The store's `Assets/` folder (Viewer staging included); None for a
        /// format 1 store, whose assets are content-addressed objects.
        asset_root: Option<String>,
        /// Keeps Viewer import staging alive after the I/O store is released
        /// while immutable preview work still reads its assets.
        staging_owner: Option<Arc<tempfile::TempDir>>,
    },
    Saved {
        receipt: SaveReceipt,
        registered: project::ProjectDocument,
    },
    SavedAs {
        receipt: SaveReceipt,
        registered: project::ProjectDocument,
        store_id: Uuid,
        path: String,
        /// None when the relink failed; acceptance then retries it in place.
        relocated: Option<project::RelocatedPayloads>,
    },
    /// `warnings`: what the export left out (the wire's `warnings`).
    Exported {
        path: String,
        warnings: Vec<pbrt_io::ExportWarning>,
    },
    Previewed {
        path: String,
        draws: usize,
        unsupported: usize,
        warnings: Vec<pbrt_io::ExportWarning>,
    },
}
struct ActiveProject {
    session: ProjectSession,
    store_id: Uuid,
    path: Option<String>,
    source_path: Option<String>,
    unregistered: Vec<String>,
    mode: dto::ProjectMode,
    asset_root: Option<String>,
    staging_owner: Option<Arc<tempfile::TempDir>>,
}
enum Pending {
    ArrayEdit {
        reference: dto::ProjectRef,
        store_id: Uuid,
        /// The active scene was made in (node targets need it).
        scene: Option<project::SceneId>,
        plan: edits::ArrayEditPlan,
    },
    AddImport {
        reference: dto::ProjectRef,
        store_id: Uuid,
        folder: Option<String>,
    },
    Switch {
        expected: dto::ProjectContext,
    },
    Save {
        store_id: Uuid,
        saved_scene: Option<project::SceneId>,
        reference: dto::ProjectRef,
    },
    Export {
        reference: dto::ProjectRef,
    },
    Preview {
        reference: dto::ProjectRef,
        /// the snapshot the job previews becomes the
        /// activation's preview base when the job succeeds.
        snapshot: ProjectSnapshot,
    },
    Capture {
        config: crate::services::ServiceConfig,
        reference: dto::ProjectRef,
        view: Option<ViewState>,
        view_revision: protocol::Revision,
        pick: Option<[f64; 2]>,
        debug_albedo: bool,
        debug_ao: bool,
        /// Capture's camera selection; pick always keeps `None`.
        camera: Option<dto::RenderCamera>,
    },
    Render {
        config: crate::services::ServiceConfig,
        reference: dto::ProjectRef,
        settings: RenderSettings,
        view: Option<ViewState>,
    },
    Ipr {
        config: crate::services::ServiceConfig,
        reference: dto::ProjectRef,
        settings: RenderSettings,
        view: Option<ViewState>,
    },
}
#[derive(Default)]
pub(crate) struct Projects {
    active: Option<ActiveProject>,
    activation: u64,
    switch: Option<JobId>,
    pending: HashMap<JobId, Pending>,
    queue: VecDeque<IoCommand>,
    /// the snapshot of the last successful app preview
    /// job of the current activation — the one base
    /// `project.preview_transforms` diffs against. One slot: advancing the
    /// activation clears it; a mismatching revision just means fallback.
    preview_base: Option<preview_transforms::PreviewBase>,
}
impl Projects {
    pub(crate) fn project_folder(&self) -> Option<&std::path::Path> {
        self.active.as_ref().and_then(|active| {
            (active.mode == dto::ProjectMode::Project)
                .then(|| active.path.as_deref())
                .flatten()
                .map(std::path::Path::new)
                .filter(|path| path.is_dir())
        })
    }
    /// Advance the activation counter — a project switch, or opening /
    /// closing / otherwise moving the active scene — dropping what the
    /// previous activation pinned.
    fn advance_activation(&mut self) {
        self.activation += 1;
        self.preview_base = None;
    }
    /// Remember the snapshot of one successful `Pending::Preview` job as
    /// the preview base of the current activation.
    pub(crate) fn record_preview_base(&mut self, snapshot: ProjectSnapshot) {
        self.preview_base = Some(preview_transforms::PreviewBase {
            activation: self.activation,
            snapshot,
        });
    }
    pub(crate) fn is_active(&self) -> bool {
        self.active.is_some()
    }
    pub(crate) fn is_read_only(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|project| project.mode == dto::ProjectMode::Viewer)
    }
    pub(crate) fn contains_scene(&self, scene: project::SceneId) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.session.document().scenes.contains_key(&scene))
    }
    pub(crate) fn active_scene(&self) -> Option<project::SceneId> {
        self.active
            .as_ref()
            .and_then(|active| active.session.active_scene())
    }
    pub(crate) fn reference(&self) -> Option<dto::ProjectRef> {
        self.active.as_ref().map(|p| dto::ProjectRef {
            project_id: p.session.document().id.0,
            revision: p.session.revision(),
            activation: Revision::from_u64(self.activation),
        })
    }
    fn context(&self) -> dto::ProjectContext {
        dto::ProjectContext {
            activation: Revision::from_u64(self.activation),
            project: self.reference(),
        }
    }
    pub(crate) fn state(&self, preview_connected: bool) -> dto::ProjectState {
        let p = self.active.as_ref();
        dto::ProjectState {
            context: self.context(),
            path: p.and_then(|p| p.path.clone()),
            mode: p.map(|p| p.mode),
            source_path: p.and_then(|p| p.source_path.clone()),
            read_only: p.is_some_and(|p| p.mode == dto::ProjectMode::Viewer),
            active_scene: p.and_then(|p| p.session.active_scene().map(|id| id.0)),
            active_up: p.and_then(|p| {
                let document = p.session.document();
                let scene = document.scene(p.session.active_scene()?)?;
                serde_json::from_value(serde_json::json!(scene.coordinate_system.up)).ok()
            }),
            active_handedness: p.and_then(|p| {
                let document = p.session.document();
                let scene = document.scene(p.session.active_scene()?)?;
                serde_json::from_value(serde_json::json!(scene.coordinate_system.handedness)).ok()
            }),
            default_scene: p.and_then(|p| p.session.document().default_scene.map(|id| id.0)),
            scene_count: p.map_or(0, |p| p.session.document().scenes.len() as u64),
            title: p.map(|p| p.session.document().title.clone()),
            saved_revision: p.and_then(|p| p.session.saved_revision()),
            dirty: p.is_some_and(|p| p.mode == dto::ProjectMode::Project && p.session.is_dirty()),
            can_undo: p
                .is_some_and(|p| p.mode == dto::ProjectMode::Project && p.session.can_undo()),
            can_redo: p
                .is_some_and(|p| p.mode == dto::ProjectMode::Project && p.session.can_redo()),
            node_count: p.map_or(0, |p| {
                let document = p.session.document();
                document
                    .scene_view(p.session.active_scene())
                    .map_or(0, |view| {
                        (view.scene.nodes.len() + document.definition_nodes.len()) as u64
                    })
            }),
            resource_count: p.map_or(0, |p| p.session.document().resources.len() as u64),
            asset_count: p.map_or(0, |p| p.session.document().assets.len() as u64),
            switch_job_id: self.switch,
            // Native geometry is served through the shared capture APIs
            // whenever a service host with a capture adapter is attached.
            preview_connected,
        }
    }
    fn check_context(&self, expected: &dto::ProjectContext) -> Result<(), ErrorPayload> {
        if self.context() != *expected {
            return Err(
                ErrorPayload::new(ErrorCode::RevisionConflict, "current project changed")
                    .with_details(json!({"current":self.context(),"expected":expected})),
            );
        }
        Ok(())
    }
    pub(crate) fn check(&self, expected: &dto::ProjectRef) -> Result<(), ErrorPayload> {
        let current = self
            .reference()
            .ok_or_else(|| ErrorPayload::new(ErrorCode::StaleProject, "no active project"))?;
        if expected.project_id != current.project_id || expected.activation != current.activation {
            return Err(
                ErrorPayload::new(ErrorCode::StaleProject, "project activation changed")
                    .with_details(json!({"current":current})),
            );
        }
        if expected.revision != current.revision {
            return Err(
                ErrorPayload::new(ErrorCode::RevisionConflict, "project revision changed")
                    .with_details(json!({"current":current})),
            );
        }
        Ok(())
    }
    fn may_switch(
        &self,
        expected: &dto::ProjectContext,
        discard: bool,
        preview_connected: bool,
    ) -> Result<(), ErrorPayload> {
        self.check_context(expected)?;
        if self.switch.is_some() {
            return Err(ErrorPayload::new(
                ErrorCode::Busy,
                "a project switch is already pending",
            ));
        }
        match self.unsaved_changes(discard, preview_connected) {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    /// The unsaved-changes guard shared by project switches and `app.quit`.
    pub(super) fn unsaved_changes(
        &self,
        discard: bool,
        preview_connected: bool,
    ) -> Option<ErrorPayload> {
        (self.active.as_ref().is_some_and(|p| p.session.is_dirty()) && !discard).then(|| {
            ErrorPayload::new(
                ErrorCode::UnsavedChanges,
                "save the project or explicitly discard unsaved changes",
            )
            .with_details(json!(self.state(preview_connected)))
        })
    }
}
fn absolute(path: &str) -> Result<(), ErrorPayload> {
    if !Path::new(path).is_absolute() || path.contains('\0') {
        Err(invalid(
            "project paths must be absolute UTF-8 paths without NUL",
        ))
    } else {
        Ok(())
    }
}
impl CoreState {
    pub(crate) fn project_default_scene_failed(
        &mut self,
        store_id: Uuid,
        scene: Option<project::SceneId>,
        error: ErrorPayload,
    ) {
        if self
            .projects
            .active
            .as_ref()
            .is_some_and(|active| active.store_id == store_id)
        {
            self.emit(
                "project.persistence_error",
                None,
                None,
                json!({"operation":"set_default_scene","scene":scene,"error":error}),
            );
        }
    }

    pub(crate) fn pop_project_io(&mut self) -> Option<IoCommand> {
        self.projects.queue.pop_front()
    }
    pub(crate) fn return_project_io(&mut self, command: IoCommand) {
        self.projects.queue.push_front(command);
    }
    fn project_job(
        &mut self,
        kind: &str,
        path: String,
        reference: Option<dto::ProjectRef>,
        work: Work,
        pending: Pending,
    ) -> Value {
        let id = Uuid::new_v4();
        let control = LoadTask {
            scene_id: Uuid::nil(),
            scene_revision: Revision::initial(),
            job_id: id,
            absolute_path: path.clone(),
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        self.job_indices.insert(id, self.jobs.len());
        self.jobs.push(JobRecord {
            job: Job {
                job_id: id,
                kind: kind.into(),
                state: JobState::Queued,
                phase: "queued".into(),
                root_path: path,
                scene_id: None,
                error: None,
                artifacts: vec![],
                project: reference,
                result: None,
            },
            task: control.clone(),
        });
        self.projects.pending.insert(id, pending);
        self.projects
            .queue
            .push_back(IoCommand::Run(IoTask { control, work }));
        self.emit_job(id);
        json!({"job_id":id})
    }
    fn project_changed(&mut self) -> Value {
        let value = json!(self.projects.state(self.capture_attached()));
        self.emit("project.changed", None, None, value.clone());
        self.ipr_project_changed();
        value
    }
    pub(crate) fn ipr_project_launch(
        &mut self,
        view: Option<ViewState>,
        scene: Option<protocol::SceneId>,
    ) -> Option<JobId> {
        let config = self
            .service_config
            .as_ref()?
            .for_project(self.projects.project_folder())
            .ok()?;
        let reference = self.projects.reference()?;
        let active = self.projects.active.as_ref()?;
        if self.jobs.iter().any(|record| {
            record.job.kind == crate::state::ipr::IPR_JOB_KIND && !record.job.state.is_terminal()
        }) {
            return None;
        }
        let destination = config
            .directory
            .path()
            .join(format!("project-input-{}", Uuid::new_v4()))
            .to_str()?
            .to_owned();
        let snapshot = active
            .session
            .snapshot_for_io()
            .ok()?
            .with_scene(
                scene
                    .map(project::SceneId)
                    .or_else(|| active.session.active_scene())?,
            )
            .ok()?;
        let value = self.project_job(
            crate::state::ipr::IPR_JOB_KIND,
            active
                .path
                .clone()
                .or_else(|| active.source_path.clone())
                .unwrap_or_default(),
            Some(reference.clone()),
            Work::Export {
                store_id: active.store_id,
                snapshot,
                destination,
            },
            Pending::Ipr {
                config: config.clone(),
                reference,
                settings: self.ipr_render_settings(),
                view,
            },
        );
        serde_json::from_value(value.get("job_id")?.clone()).ok()
    }
}

#[cfg(test)]
mod import_completion_tests;
#[cfg(test)]
mod preview_transforms_tests;
#[cfg(test)]
mod scene_graph_tests;
#[cfg(test)]
mod tests;
