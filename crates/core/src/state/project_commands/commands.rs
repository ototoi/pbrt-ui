//! Revision-checked project operation acceptance.
use super::*;
impl CoreState {
    pub(crate) fn handle_project(&mut self, request: &Request) -> Result<Value, ErrorPayload> {
        if super::queues_project_io(&request.method)
            && self.projects.pending.len() >= self.limits.command_queue
        {
            return Err(ErrorPayload::new(
                ErrorCode::Busy,
                "project I/O queue is full",
            ));
        }
        match request.method {
            Method::ProjectGetState => {
                let _: EmptyParams = params(request)?;
                Ok(json!(self.projects.state(self.capture_attached())))
            }
            Method::ProjectOpenScene => {
                let p: dto::OpenProjectSceneParams = params(request)?;
                self.projects.check(&p.project)?;
                let active = self.projects.active.as_ref().unwrap();
                if !active
                    .session
                    .document()
                    .scenes
                    .contains_key(&project::SceneId(p.scene))
                {
                    return Err(ErrorPayload::new(ErrorCode::NotFound, "scene not found"));
                }
                if active.session.active_scene() == Some(project::SceneId(p.scene)) {
                    return Ok(json!(self.projects.state(self.capture_attached())));
                }
                if active.mode == dto::ProjectMode::Project
                    && self.projects.queue.len() >= self.limits.command_queue
                {
                    return Err(ErrorPayload::new(
                        ErrorCode::Busy,
                        "project I/O queue is full",
                    ));
                }
                let active = self.projects.active.as_mut().unwrap();
                active
                    .session
                    .open_scene(project::SceneId(p.scene))
                    .map_err(crate::project_io::project_error)?;
                let store_id = active.store_id;
                let mode = active.mode;
                self.projects.advance_activation();
                self.project_view = None;
                if mode == dto::ProjectMode::Project {
                    self.projects.queue.push_back(IoCommand::SetDefaultScene {
                        store_id,
                        scene: Some(project::SceneId(p.scene)),
                    });
                }
                let state = self.projects.state(self.capture_attached());
                self.emit("project.changed", None, None, json!(state));
                self.ipr_project_changed();
                Ok(json!(state))
            }
            Method::ProjectCloseScene => {
                let p: dto::ProjectParams = params(request)?;
                self.projects.check(&p.project)?;
                let active = self.projects.active.as_mut().unwrap();
                if active.session.active_scene().is_none() {
                    return Ok(json!(self.projects.state(self.capture_attached())));
                }
                if self.projects.queue.len() >= self.limits.command_queue {
                    return Err(ErrorPayload::new(
                        ErrorCode::Busy,
                        "project I/O queue is full",
                    ));
                }
                active.session.close_scene();
                if active.mode == dto::ProjectMode::Project {
                    self.projects.queue.push_back(IoCommand::SetDefaultScene {
                        store_id: active.store_id,
                        scene: None,
                    });
                }
                self.projects.advance_activation();
                self.project_view = None;
                let state = self.projects.state(self.capture_attached());
                self.emit("project.changed", None, None, json!(state));
                self.ipr_project_changed();
                Ok(json!(state))
            }
            Method::ProjectSetParameterValues => {
                let p: dto::SetParameterValuesParams = params(request)?;
                self.projects.check(&p.project)?;
                if self.projects.is_read_only() {
                    return Err(read_only_error());
                }
                let active = self.projects.active.as_ref().unwrap();
                let plan = edits::plan_array_edit(
                    active.session.document(),
                    active.session.active_scene(),
                    &p,
                )?;
                // The data asset's copy happens on the I/O executor, which
                // also owns every file move a save makes.
                let data = match &plan {
                    edits::ArrayEditPlan::Inline { .. } => None,
                    edits::ArrayEditPlan::DataAsset {
                        old,
                        patches,
                        new_id,
                        ..
                    } => Some(super::ArrayCopy {
                        asset: old.id,
                        location: old.location.clone(),
                        size: old.size,
                        sha256: old.sha256.clone(),
                        patches: patches.clone(),
                        new_id: *new_id,
                    }),
                };
                let store_id = active.store_id;
                let snapshot = active.session.snapshot();
                let path = match p.target {
                    dto::ParameterTarget::Node(id) | dto::ParameterTarget::Resource(id) => {
                        format!("{id}:{}", p.name)
                    }
                };
                Ok(self.project_job(
                    "project.set_parameter_values",
                    path,
                    Some(p.project.clone()),
                    Work::ArrayEdit {
                        store_id,
                        snapshot,
                        data,
                    },
                    Pending::ArrayEdit {
                        reference: p.project,
                        store_id,
                        scene: active.session.active_scene(),
                        plan,
                    },
                ))
            }
            Method::ProjectImportScene | Method::ProjectImportResource => {
                let p: dto::AddImportParams = params(request)?;
                self.projects.check(&p.project)?;
                if self.projects.is_read_only() {
                    return Err(read_only_error());
                }
                absolute(&p.source_path)?;
                let active = self.projects.active.as_ref().unwrap();
                let scene = request.method == Method::ProjectImportScene;
                if !scene && p.folder.is_none() {
                    return Err(invalid("a resource import requires a destination folder"));
                }
                if let Some(folder) = &p.folder
                    && !folder.is_empty()
                    && !active.session.document().folders.contains(folder)
                {
                    return Err(invalid("import destination folder does not exist"));
                }
                let store_id = active.store_id;
                Ok(self.project_job(
                    if scene {
                        "project.import_scene"
                    } else {
                        "project.import_resource"
                    },
                    p.source_path.clone(),
                    Some(p.project.clone()),
                    Work::AddImport {
                        source: p.source_path,
                        scene,
                        staging_base: crate::import_prepare::staging_base(
                            (active.mode == dto::ProjectMode::Project)
                                .then_some(active.path.as_deref())
                                .flatten()
                                .map(std::path::Path::new)
                                .filter(|path| path.is_dir()),
                            &project::cache_directory()
                                .map_err(|error| invalid(error.to_string()))?,
                        ),
                    },
                    Pending::AddImport {
                        reference: p.project,
                        store_id,
                        folder: p.folder,
                    },
                ))
            }
            Method::ProjectImport => {
                let p: dto::ImportProjectParams = params(request)?;
                absolute(&p.source_path)?;
                if let Some(destination) = &p.destination {
                    absolute(destination)?;
                }
                self.projects.may_switch(
                    &p.expected_current,
                    p.discard_unsaved,
                    self.capture_attached(),
                )?;
                let result = self.project_job(
                    "project.import",
                    p.destination
                        .clone()
                        .unwrap_or_else(|| p.source_path.clone()),
                    self.projects.reference(),
                    Work::Import {
                        source: p.source_path,
                        destination: p.destination,
                    },
                    Pending::Switch {
                        expected: p.expected_current,
                    },
                );
                self.projects.switch =
                    Some(serde_json::from_value(result["job_id"].clone()).unwrap());
                self.project_changed();
                Ok(result)
            }
            Method::ProjectOpen => {
                let p: dto::OpenProjectParams = params(request)?;
                absolute(&p.path)?;
                self.projects.may_switch(
                    &p.expected_current,
                    p.discard_unsaved,
                    self.capture_attached(),
                )?;
                let result = self.project_job(
                    "project.open",
                    p.path.clone(),
                    self.projects.reference(),
                    Work::Open { path: p.path },
                    Pending::Switch {
                        expected: p.expected_current,
                    },
                );
                self.projects.switch =
                    Some(serde_json::from_value(result["job_id"].clone()).unwrap());
                self.project_changed();
                Ok(result)
            }
            Method::ProjectCreate => {
                let p: dto::CreateProjectParams = params(request)?;
                absolute(&p.path)?;
                validate_new_title(&p.title)?;
                let coordinate_system = match p.coordinate_system {
                    Some(choice) => project::CoordinateSystem::new(
                        super::edits::up_axis(choice.up()),
                        super::edits::wire_handedness(choice.handedness()),
                    ),
                    None => project::CoordinateSystem::default(),
                };
                self.projects.may_switch(
                    &p.expected_current,
                    p.discard_unsaved,
                    self.capture_attached(),
                )?;
                let result = self.project_job(
                    "project.create",
                    p.path.clone(),
                    self.projects.reference(),
                    Work::Create {
                        path: p.path,
                        title: p.title,
                        coordinate_system,
                    },
                    Pending::Switch {
                        expected: p.expected_current,
                    },
                );
                self.projects.switch =
                    Some(serde_json::from_value(result["job_id"].clone()).unwrap());
                self.project_changed();
                Ok(result)
            }
            Method::ProjectClose => {
                let p: dto::CloseProjectParams = params(request)?;
                self.projects.may_switch(
                    &p.expected_current,
                    p.discard_unsaved,
                    self.capture_attached(),
                )?;
                if let Some(old) = self.projects.active.take() {
                    self.projects
                        .queue
                        .push_back(IoCommand::Release(old.store_id));
                    self.projects.advance_activation();
                }
                Ok(self.project_changed())
            }
            Method::ProjectSave => {
                let p: dto::SaveProjectParams = params(request)?;
                self.projects.check(&p.project)?;
                let active = self.projects.active.as_mut().unwrap();
                if let Some(destination) = &p.destination {
                    validate_save_destination(destination)?;
                } else if active.mode == dto::ProjectMode::Viewer {
                    return Err(invalid(
                        "a viewer must be saved to an explicit project folder",
                    ));
                }
                let token = active.store_id;
                let snapshot = active
                    .session
                    .snapshot_for_io()
                    .map_err(crate::project_io::project_error)?;
                let work = Work::Save {
                    store_id: token,
                    snapshot: snapshot.clone(),
                    destination: p.destination.clone(),
                    retained: if p.destination.is_some() {
                        active.session.retained_payloads()
                    } else {
                        Vec::new()
                    },
                };
                let job_path = p
                    .destination
                    .clone()
                    .or_else(|| active.path.clone())
                    .or_else(|| active.source_path.clone())
                    .unwrap_or_default();
                Ok(self.project_job(
                    "project.save",
                    job_path,
                    Some(p.project.clone()),
                    work,
                    Pending::Save {
                        store_id: token,
                        saved_scene: snapshot.scene(),
                        reference: p.project,
                    },
                ))
            }
            Method::ProjectExport => {
                let p: dto::ExportProjectParams = params(request)?;
                self.projects.check(&p.project)?;
                absolute(&p.destination)?;
                let active = self.projects.active.as_mut().unwrap();
                let snapshot = snapshot_for_scene(active, p.scene)?;
                let work = Work::Export {
                    store_id: active.store_id,
                    snapshot,
                    destination: p.destination.clone(),
                };
                Ok(self.project_job(
                    "project.export",
                    p.destination,
                    Some(p.project.clone()),
                    work,
                    Pending::Export {
                        reference: p.project,
                    },
                ))
            }
            Method::ProjectPreview => {
                let p: dto::PreviewProjectParams = params(request)?;
                self.projects.check(&p.project)?;
                absolute(&p.destination)?;
                if let Some(store) = &p.store {
                    absolute(store)?;
                }
                let active = self.projects.active.as_mut().unwrap();
                // the pinned snapshot rides in both the
                // job (which builds the package from it) and the pending
                // record (which makes it the activation's preview base when
                // the job succeeds).
                let snapshot = active
                    .session
                    .snapshot_for_io()
                    .map_err(crate::project_io::project_error)?;
                let work = Work::Preview {
                    store_id: active.store_id,
                    snapshot: snapshot.clone(),
                    destination: p.destination.clone(),
                    store: p.store.clone(),
                    // Share the service tessellation cache so the preview
                    // refresh keeps working across edits like the desktop's
                    // own cache did; headless commands fall back to a
                    // job-local cache.
                    geometry_cache: self
                        .service_config
                        .as_ref()
                        .map(|config| config.geometry_cache.clone()),
                    // External callers keep the self-contained bundle
                    //.
                    stage_assets: p.stage_assets,
                };
                Ok(self.project_job(
                    "project.preview",
                    p.destination,
                    Some(p.project.clone()),
                    work,
                    Pending::Preview {
                        reference: p.project,
                        snapshot,
                    },
                ))
            }
            Method::ProjectPreviewTransforms => {
                let p: dto::ProjectPreviewTransformsParams = params(request)?;
                self.projects.check(&p.project)?;
                // synchronous and read-only — what the
                // displayed preview needs after a transform-only edit.
                Ok(self.project_preview_transforms(&p))
            }
            Method::ProjectRender => {
                let p: dto::RenderProjectParams = params(request)?;
                self.projects.check(&p.project)?;
                revision(
                    &p.expected_settings_revision,
                    self.settings_revision,
                    "settings",
                )?;
                if let Some(view) = &p.view {
                    crate::view::validate(&view.camera)?;
                    if view.size.iter().any(|n| *n == 0 || *n > 8192)
                        || u64::from(view.size[0]) * u64::from(view.size[1]) > 16_777_216
                        || !view.time.is_finite()
                    {
                        return Err(invalid("render view size or time exceeds limits"));
                    }
                }
                // Without a view the scene's own camera and Film render;
                // camera "view" asks for the stored shared view instead.
                let camera = p.camera.unwrap_or(if p.view.is_some() {
                    dto::RenderCamera::View
                } else {
                    dto::RenderCamera::Scene
                });
                if camera == dto::RenderCamera::Scene && p.view.is_some() {
                    return Err(invalid("a scene-camera render takes no view"));
                }
                if camera == dto::RenderCamera::Scene {
                    let active = self.projects.active.as_mut().unwrap();
                    let snapshot = snapshot_for_scene(active, p.scene)?;
                    let scene = snapshot
                        .scene_view()
                        .map_err(crate::project_io::project_error)?
                        .scene;
                    if scene.render_camera.is_none_or(|id| {
                        !matches!(
                            scene.nodes.get(&id).map(|node| &node.data),
                            Some(project::NodeKind::Entity {
                                entity: project::EntityKind::Camera,
                                ..
                            })
                        )
                    }) {
                        return Err(invalid("No camera is set for rendering")
                            .with_details(json!({"reason":"no_render_camera","scene_id":scene.id,"action":"open_render_settings"})));
                    }
                }
                if !self.capabilities.render || self.service_config.is_none() {
                    return Err(ErrorPayload::new(
                        ErrorCode::Unsupported,
                        crate::state::ipr::RENDERER_NOT_CONFIGURED,
                    ));
                }
                if self
                    .jobs
                    .iter()
                    .any(|record| record.job.kind == "render" && !record.job.state.is_terminal())
                {
                    return Err(ErrorPayload::new(
                        ErrorCode::Busy,
                        "a render job is already active",
                    ));
                }
                let view = p.view.clone().or_else(|| {
                    (camera == dto::RenderCamera::View).then_some(())?;
                    (p.scene.is_none_or(|scene| {
                        Some(scene)
                            == self
                                .projects
                                .active
                                .as_ref()
                                .and_then(|a| a.session.active_scene().map(|id| id.0))
                    }))
                    .then(|| self.stored_project_view().map(|(view, _)| view.clone()))
                    .flatten()
                });
                // Asked for the shared view but there is none (never set, or
                // another scene): say so rather than render the scene camera.
                if camera == dto::RenderCamera::View && view.is_none() {
                    return Err(
                        invalid("no shared view for this scene; set one or pass a view")
                            .with_details(json!({"reason":"no_shared_view"})),
                    );
                }
                self.ipr_preempt();
                let active = self.projects.active.as_mut().unwrap();
                let config = self.service_config.as_ref().unwrap().for_project(
                    (active.mode == dto::ProjectMode::Project)
                        .then_some(active.path.as_deref())
                        .flatten()
                        .map(std::path::Path::new)
                        .filter(|path| path.is_dir()),
                )?;
                let destination = config
                    .directory
                    .path()
                    .join(format!("project-input-{}", Uuid::new_v4()))
                    .to_str()
                    .ok_or_else(|| invalid("non UTF-8 render path"))?
                    .to_owned();
                let snapshot = snapshot_for_scene(active, p.scene)?;
                let work = Work::Export {
                    store_id: active.store_id,
                    snapshot,
                    destination,
                };
                let job_path = active
                    .path
                    .clone()
                    .or_else(|| active.source_path.clone())
                    .unwrap_or_default();
                Ok(self.project_job(
                    "render",
                    job_path,
                    Some(p.project.clone()),
                    work,
                    Pending::Render {
                        config: config.clone(),
                        reference: p.project,
                        settings: self.settings.clone(),
                        view,
                    },
                ))
            }
            Method::ProjectCapture | Method::ProjectPick => {
                let (project, view, pick, scene, debug_albedo, debug_ao, camera) =
                    if request.method == Method::ProjectPick {
                        let p: dto::PickProjectParams = params(request)?;
                        if p.normalized_xy
                            .iter()
                            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                        {
                            return Err(invalid("pick coordinates must be within 0..1"));
                        }
                        (
                            p.project,
                            p.view,
                            Some(p.normalized_xy),
                            p.scene,
                            false,
                            false,
                            None,
                        )
                    } else {
                        let p: dto::CaptureProjectParams = params(request)?;
                        if p.debug_albedo && p.debug_ao {
                            return Err(invalid("debug_albedo and debug_ao are exclusive"));
                        }
                        // The camera picks the view source like render's; an
                        // explicit view would override it, so refuse both
                        //.
                        match (p.camera, p.view.is_some()) {
                            (Some(dto::RenderCamera::Scene), true) => {
                                return Err(invalid("a scene-camera capture takes no view"));
                            }
                            (Some(dto::RenderCamera::View), true) => {
                                return Err(invalid("a shared-view capture takes no view"));
                            }
                            _ => {}
                        }
                        (
                            p.project,
                            p.view,
                            None,
                            p.scene,
                            p.debug_albedo,
                            p.debug_ao,
                            p.camera,
                        )
                    };
                // Without an explicit view, captures reuse the stored shared
                // native view, mirroring the viewer's core-owned camera; a
                // scene-camera capture keeps only its mode and time.
                let active_scene = self
                    .projects
                    .active
                    .as_ref()
                    .and_then(|active| active.session.active_scene().map(|id| id.0));
                let view = view.or_else(|| {
                    (scene.is_none_or(|scene| Some(scene) == active_scene))
                        .then(|| self.stored_project_view().map(|(view, _)| view.clone()))
                        .flatten()
                });
                self.start_capture(project, view, pick, scene, debug_albedo, debug_ao, camera)
            }
            Method::ProjectGetView => {
                let p: dto::GetProjectViewParams = params(request)?;
                self.projects.check(&p.project)?;
                let stored = self.stored_project_view();
                Ok(json!({
                    "project": p.project,
                    "view": stored.map(|(view, _)| view),
                    "view_revision": protocol::Revision::from_u64(
                        stored.map_or(0, |(_, revision)| *revision)
                    ),
                }))
            }
            Method::ProjectSetCamera => {
                let p: dto::SetProjectCameraParams = params(request)?;
                self.projects.check(&p.project)?;
                let stored = self.stored_project_view();
                revision(
                    &p.expected_view_revision,
                    stored.map_or(0, |(_, revision)| *revision),
                    "view",
                )?;
                crate::view::validate(&p.camera)?;
                let mut view =
                    stored.map_or_else(|| crate::view::initial(None), |(view, _)| view.clone());
                view.camera = p.camera;
                let revision = self.update_project_view(view)?;
                Ok(json!({
                    "project": p.project,
                    "view": self.stored_project_view().unwrap().0,
                    "view_revision": protocol::Revision::from_u64(revision),
                }))
            }
            Method::ProjectConfigureView => {
                let p: dto::ConfigureProjectViewParams = params(request)?;
                self.projects.check(&p.project)?;
                let stored = self.stored_project_view();
                revision(
                    &p.expected_view_revision,
                    stored.map_or(0, |(_, revision)| *revision),
                    "view",
                )?;
                let mut view =
                    stored.map_or_else(|| crate::view::initial(None), |(view, _)| view.clone());
                if let Some(size) = p.size {
                    if size.iter().any(|n| *n == 0 || *n > 8192)
                        || u64::from(size[0]) * u64::from(size[1]) > 16_777_216
                    {
                        return Err(invalid("project view size exceeds limits"));
                    }
                    view.size = size;
                }
                if let Some(mode) = p.mode {
                    view.mode = mode;
                }
                if let Some(time) = p.time {
                    if !time.is_finite() {
                        return Err(invalid("project view time must be finite"));
                    }
                    view.time = time;
                }
                let revision = self.update_project_view(view)?;
                Ok(json!({
                    "project": p.project,
                    "view": self.stored_project_view().unwrap().0,
                    "view_revision": protocol::Revision::from_u64(revision),
                }))
            }
            Method::ProjectApply => {
                let p: dto::ApplyProjectParams = params(request)?;
                if self.projects.is_read_only() {
                    return Err(read_only_error());
                }
                self.projects.check(&p.project)?;
                if p.edits.is_empty() || p.edits.len() > 64 {
                    return Err(invalid("project transaction requires 1 to 64 edits"));
                }
                if let Some(capture) = &p.capture {
                    self.check_capture_ready(&p.project, &capture.view, None)?;
                }
                let active = self.projects.active.as_ref().unwrap();
                let active_scene = active.session.active_scene();
                let mut validation_session =
                    project::ProjectSession::from_saved_snapshot(active.session.snapshot())
                        .map_err(crate::project_io::project_error)?;
                let mut edits = Vec::with_capacity(p.edits.len());
                // Touched nodes and resources across the transaction, for the
                // apply-time exportability check.
                let mut touched_nodes = std::collections::BTreeSet::new();
                let mut touched_resources = std::collections::BTreeSet::new();
                // per changed target, the kept parameters the new
                // implementation's table does not know.
                let mut kept_unknown = Vec::new();
                for wire_edit in &p.edits {
                    validate_item_edit(
                        wire_edit,
                        validation_session.document(),
                        // the delete checks read the session's
                        // per-revision cached index.
                        &validation_session.reference_index(),
                        validation_session.active_scene(),
                    )?;
                    let planned = match wire_edit {
                        dto::ProjectEdit::SetImplementation {
                            target,
                            implementation,
                            parameters: dto::ImplementationParameters::KeepCompatible,
                        } => {
                            let edits::ImplementationPlan { edit, change, .. } =
                                edits::plan_implementation(
                                    validation_session.document(),
                                    validation_session.active_scene(),
                                    target.clone(),
                                    implementation,
                                )?;
                            if let Some(change) = change {
                                // keeps unknown parameters; the ones the
                                // new implementation cannot export would only
                                // fail the check below, so name them here.
                                if !change.breaks_reference.is_empty() {
                                    return Err(invalid(format!(
                                        "{} cannot export these parameters; remove them first: {}",
                                        change.implementation,
                                        change.breaks_reference.join(", ")
                                    ))
                                    .with_details(json!({
                                        "reason":"breaks_reference",
                                        "target":target,
                                        "parameters":change.breaks_reference,
                                    })));
                                }
                                if !change.kept_unknown.is_empty() {
                                    kept_unknown.push(json!({
                                        "target":target,
                                        "parameters":change.kept_unknown,
                                    }));
                                }
                            }
                            vec![edit]
                        }
                        // Resolve against the table into
                        // set_parameter edits.
                        dto::ProjectEdit::ConnectTexture { .. }
                        | dto::ProjectEdit::DisconnectTexture { .. } => edits::plan_connection(
                            validation_session.document(),
                            validation_session.active_scene(),
                            wire_edit,
                        )?,
                        dto::ProjectEdit::SetParameterValues { .. } => edits::plan_inline_values(
                            wire_edit,
                            validation_session.document(),
                            validation_session.active_scene(),
                        )?,
                        _ => vec![edits::convert(wire_edit.clone())?],
                    };
                    for edit in planned {
                        let revision = validation_session.revision();
                        let change = validation_session
                            .apply(&revision, std::slice::from_ref(&edit))
                            .map_err(crate::project_io::project_error)?;
                        touched_nodes.extend(change.nodes.iter().copied());
                        touched_nodes.extend(change.created.iter().copied());
                        for item in &change.created_items {
                            if let project::ItemId::Scene(id) = item {
                                touched_nodes.extend(
                                    validation_session
                                        .document()
                                        .scenes
                                        .get(id)
                                        .map(|scene| {
                                            scene.nodes.keys().copied().collect::<Vec<_>>()
                                        })
                                        .unwrap_or_default(),
                                );
                            }
                        }
                        touched_resources.extend(change.resources.iter().copied());
                        touched_resources.extend(change.created_items.iter().filter_map(|item| {
                            match item {
                                project::ItemId::Resource(id) => Some(*id),
                                _ => None,
                            }
                        }));
                        edits.push(edit);
                    }
                }
                // The touch set may name nodes a later edit in the same
                // transaction removed again; the final document is what must
                // stay exportable.
                let document = validation_session.document();
                touched_nodes.retain(|id| {
                    document
                        .scenes
                        .values()
                        .any(|scene| scene.nodes.contains_key(id))
                        || document.definition_nodes.contains_key(id)
                });
                touched_resources.retain(|id| document.resources.contains_key(id));
                if edits.iter().any(|edit| match edit {
                    project::Edit::SetTextureValueType { resource, .. } => {
                        document
                            .resources
                            .get(resource)
                            .and_then(|value| value.texture_value_type)
                            != active
                                .session
                                .document()
                                .resources
                                .get(resource)
                                .and_then(|value| value.texture_value_type)
                    }
                    _ => false,
                }) {
                    let mismatches = pbrt_io::texture_type_mismatches(document);
                    if !mismatches.is_empty() {
                        return Err(invalid(
                            "texture value type conflicts with project references",
                        )
                        .with_details(
                            json!({"reason":"texture_type_mismatch","references":mismatches}),
                        ));
                    }
                    touched_resources.extend(document.resources.keys().copied());
                    touched_nodes.extend(
                        document
                            .scenes
                            .values()
                            .flat_map(|scene| scene.nodes.keys().copied()),
                    );
                    touched_nodes.extend(document.definition_nodes.keys().copied());
                }
                pbrt_io::check_exportable(
                    document,
                    &touched_nodes.into_iter().collect::<Vec<_>>(),
                    &touched_resources.into_iter().collect::<Vec<_>>(),
                )
                .map_err(|error| {
                    invalid(format!("edit leaves the project non-exportable: {error}"))
                        .with_details(json!({"reason":"not_exportable"}))
                })?;
                let active = self.projects.active.as_mut().unwrap();
                if edits.iter().any(|edit| {
                    matches!(
                        edit,
                        project::Edit::DeleteItem {
                            item: project::ItemId::Asset(_)
                        }
                    )
                }) && let Some(root) = active.asset_root.as_deref()
                {
                    active
                        .session
                        .ensure_temporary_payload_staging(std::path::Path::new(root))
                        .map_err(crate::project_io::project_error)?;
                }
                let change = active
                    .session
                    .apply(&p.project.revision, &edits)
                    .map_err(crate::project_io::project_error)?;
                if active_scene != active.session.active_scene() {
                    self.projects.advance_activation();
                    self.project_view = None;
                }
                let changed = change.revision != p.project.revision;
                let created = change
                    .created
                    .iter()
                    .map(|id| json!({"kind":"node","id":id}))
                    .chain(change.created_items.iter().filter_map(|item| match item {
                        project::ItemId::Scene(id) => Some(json!({"kind":"scene","id":id})),
                        // creating a resource reports it like scenes do.
                        project::ItemId::Resource(id) => Some(json!({"kind":"resource","id":id})),
                        _ => None,
                    }))
                    .collect::<Vec<_>>();
                let mut result = json!({"project":self.projects.reference(),"nodes":change.nodes,"resources":change.resources,"created":created,"kept_unknown":kept_unknown});
                if changed {
                    self.project_changed();
                }
                if let Some(capture) = p.capture {
                    let reference = self.projects.reference().unwrap();
                    let capture_view = capture
                        .view
                        .clone()
                        .or_else(|| self.stored_project_view().map(|(view, _)| view.clone()));
                    match self.start_capture(
                        reference,
                        capture_view,
                        None,
                        None,
                        false,
                        false,
                        None,
                    ) {
                        Ok(job) => result["capture_job_id"] = job["job_id"].clone(),
                        Err(error) => result["capture_error"] = json!(error),
                    }
                }
                Ok(result)
            }
            Method::ProjectUndo | Method::ProjectRedo => {
                let p: dto::ProjectParams = params(request)?;
                if self.projects.is_read_only() {
                    return Err(read_only_error());
                }
                self.projects.check(&p.project)?;
                let active = self.projects.active.as_mut().unwrap();
                let previous_scene = active.session.active_scene();
                let change = if request.method == Method::ProjectUndo {
                    active.session.undo(&p.project.revision)
                } else {
                    active.session.redo(&p.project.revision)
                }
                .map_err(crate::project_io::project_error)?;
                if previous_scene != active.session.active_scene() {
                    self.projects.advance_activation();
                    self.project_view = None;
                }
                let result = json!({"project":self.projects.reference(),"nodes":change.nodes,"resources":change.resources,"kept_unknown":[]});
                self.project_changed();
                Ok(result)
            }
            _ => self.project_query(request),
        }
    }
}

fn validate_item_edit(
    edit: &dto::ProjectEdit,
    document: &project::ProjectDocument,
    references: &project::ReferenceIndex,
    active_scene: Option<project::SceneId>,
) -> Result<(), ErrorPayload> {
    use dto::{ProjectEdit as Edit, ProjectItemId as Item};
    let item_id = |item: Item| match item {
        Item::Scene(id) => project::ItemId::Scene(project::SceneId(id)),
        Item::Resource(id) => project::ItemId::Resource(project::ResourceId(id)),
        Item::Asset(id) => project::ItemId::Asset(project::AssetId(id)),
    };
    let ensure_scene_removable = |id: uuid::Uuid| -> Result<(), ErrorPayload> {
        if !document.scenes.contains_key(&project::SceneId(id)) {
            return Err(ErrorPayload::new(ErrorCode::NotFound, "scene not found"));
        }
        Ok(())
    };
    match edit {
        Edit::DeleteScene { scene } => ensure_scene_removable(*scene)?,
        Edit::DeleteItem {
            item: Item::Scene(scene),
        } => ensure_scene_removable(*scene)?,
        Edit::DeleteItem { item } => {
            let id = item_id(*item);
            let exists = match id {
                project::ItemId::Resource(id) => document.resources.contains_key(&id),
                project::ItemId::Asset(id) => document.assets.contains_key(&id),
                project::ItemId::Scene(_) => unreachable!(),
            };
            if !exists {
                return Err(ErrorPayload::new(
                    ErrorCode::NotFound,
                    "project item not found",
                ));
            }
            let found = references.references(id);
            if !found.is_empty() {
                // the refusal's head rows name their sources - the same
                // rows `project.list_references` serves - so the dialog can
                // show where the item is used.
                let parents = super::queries::reference_parents(document);
                let definitions = super::queries::definition_parents(document);
                let examples = found
                    .iter()
                    .take(10)
                    .map(|reference| {
                        super::queries::reference_row(
                            document,
                            references,
                            &parents,
                            &definitions,
                            reference,
                        )
                    })
                    .collect::<Vec<_>>();
                return Err(invalid("cannot delete a referenced project item").with_details(
                    json!({"reason":"referenced","references":{"count":found.len(),"items":examples}}),
                ));
            }
        }
        Edit::CreatePreset { preset, parent, .. } => {
            if project::presets::builtin(project::ResourceId(*preset)).is_none() {
                return Err(ErrorPayload::new(ErrorCode::NotFound, "preset not found"));
            }
            let view = document
                .scene_view(active_scene)
                .map_err(|error| invalid(error.to_string()))?;
            let parent = if *parent == view.scene.id.0 {
                view.scene.world
            } else {
                project::NodeId(*parent)
            };
            match view.node(parent).map(|node| &node.data) {
                None => {
                    return Err(ErrorPayload::new(
                        ErrorCode::NotFound,
                        "preset parent not found",
                    ));
                }
                Some(project::NodeKind::Scope {
                    owned_object: Some(_),
                    ..
                }) => {
                    return Err(invalid("preset parent is not visible in the scene graph"));
                }
                Some(project::NodeKind::Scope { .. }) => {}
                Some(_) => {
                    return Err(invalid("preset parent is not a group")
                        .with_details(json!({"reason":"parent_not_group"})));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// A new project's title names the project and stamps the initial scene's
/// location (`Scenes/<title>.scene`), so it must stay one path segment.
#[allow(clippy::result_large_err)] // the wire's error payload, like the savers below
fn validate_new_title(title: &str) -> Result<(), ErrorPayload> {
    if title.trim().is_empty() || title.len() > 128 {
        return Err(invalid("project titles are 1 to 128 characters"));
    }
    if title.contains('/') || title.contains('\\') || title.contains('\0') {
        return Err(invalid(
            "project titles hold no path separators (the title stamps Scenes/<title>.scene)",
        ));
    }
    Ok(())
}

fn read_only_error() -> ErrorPayload {
    ErrorPayload::new(ErrorCode::ReadOnly, "viewer projects are read-only")
        .with_details(json!({"mode":"viewer","hint":"project.save with destination"}))
}

fn validate_save_destination(destination: &str) -> Result<(), ErrorPayload> {
    absolute(destination)?;
    let path = std::path::Path::new(destination);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid("Save As destination needs a UTF-8 folder name"))?
        .to_ascii_lowercase();
    if name.ends_with(".pbrt") || name.ends_with(".pbrt.gz") {
        return Err(invalid("PBRT files must be created with project.export"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Save As destination has no parent"))?;
    if !parent.is_dir() {
        return Err(invalid("Save As parent must be an existing directory"));
    }
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(invalid("Save As destination must be a directory"));
        }
        Ok(_)
            if std::fs::read_dir(path)
                .map_err(|error| invalid(error.to_string()))?
                .next()
                .is_some() =>
        {
            return Err(invalid("Save As destination directory is not empty"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(invalid(error.to_string())),
    }
    Ok(())
}

impl CoreState {
    /// Validates everything a capture needs so callers can fail before mutating the project.
    pub(super) fn check_capture_ready(
        &mut self,
        project: &dto::ProjectRef,
        view: &Option<ViewState>,
        scene: Option<protocol::SceneId>,
    ) -> Result<(), ErrorPayload> {
        self.projects.check(project)?;
        let active = self.projects.active.as_mut().unwrap();
        snapshot_for_scene(active, scene)?;
        if self
            .service_config
            .as_ref()
            .is_none_or(|c| c.capture.is_none())
        {
            return Err(ErrorPayload::new(
                ErrorCode::Unsupported,
                "capture adapter unavailable",
            ));
        }
        if self.projects.pending.len() >= self.limits.command_queue {
            return Err(ErrorPayload::new(
                ErrorCode::Busy,
                "project I/O queue is full",
            ));
        }
        if let Some(view) = view {
            crate::view::validate(&view.camera)?;
            if view.size.iter().any(|n| *n == 0 || *n > 8192)
                || u64::from(view.size[0]) * u64::from(view.size[1]) > 16_777_216
                || !view.time.is_finite()
            {
                return Err(invalid("capture view size or time exceeds limits"));
            }
        }
        Ok(())
    }
    /// Queues a direct-preview package that ends in a raster capture (or pick)
    /// of the pinned project snapshot.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_capture(
        &mut self,
        project: dto::ProjectRef,
        view: Option<ViewState>,
        pick: Option<[f64; 2]>,
        scene: Option<protocol::SceneId>,
        debug_albedo: bool,
        debug_ao: bool,
        camera: Option<dto::RenderCamera>,
    ) -> Result<Value, ErrorPayload> {
        self.check_capture_ready(&project, &view, scene)?;
        let active = self.projects.active.as_mut().unwrap();
        let config = self.service_config.as_ref().unwrap().for_project(
            (active.mode == dto::ProjectMode::Project)
                .then_some(active.path.as_deref())
                .flatten()
                .map(std::path::Path::new)
                .filter(|path| path.is_dir()),
        )?;
        let destination = config
            .directory
            .path()
            .join(format!("project-preview-{}", Uuid::new_v4()))
            .to_str()
            .ok_or_else(|| invalid("non UTF-8 capture path"))?
            .to_owned();
        let geometry_cache = config.geometry_cache.clone();
        let snapshot = snapshot_for_scene(active, scene)?;
        let work = Work::Preview {
            store_id: active.store_id,
            snapshot,
            destination,
            store: None,
            // The capture/pick slot reads only the manifest and blobs
            // resolve assets in place.
            geometry_cache: Some(geometry_cache),
            stage_assets: false,
        };
        let path = active
            .path
            .clone()
            .or_else(|| active.source_path.clone())
            .unwrap_or_default();
        Ok(self.project_job(
            if pick.is_some() { "pick" } else { "capture" },
            path,
            Some(project.clone()),
            work,
            Pending::Capture {
                config: config.clone(),
                reference: project,
                view,
                view_revision: protocol::Revision::from_u64(
                    self.stored_project_view()
                        .map_or(0, |(_, revision)| *revision),
                ),
                pick,
                debug_albedo,
                debug_ao,
                camera,
            },
        ))
    }

    /// The stored shared view when it belongs to the current activation.
    pub(crate) fn stored_project_view(&self) -> Option<(&ViewState, &u64)> {
        let (context, view, revision) = self.project_view.as_ref()?;
        (context == &self.projects.context()).then_some((view, revision))
    }

    /// Store the shared native view for the current activation, advancing its
    /// revision (fresh activations start at one).
    fn update_project_view(&mut self, view: ViewState) -> Result<u64, ErrorPayload> {
        let revision = self
            .stored_project_view()
            .map_or(1, |(_, revision)| revision + 1);
        self.project_view = Some((self.projects.context(), view, revision));
        Ok(revision)
    }
}

fn snapshot_for_scene(
    active: &mut ActiveProject,
    scene: Option<protocol::SceneId>,
) -> Result<ProjectSnapshot, ErrorPayload> {
    let scene = scene
        .map(project::SceneId)
        .or_else(|| active.session.active_scene())
        .ok_or_else(|| {
            invalid("no active scene").with_details(json!({"reason":"no_active_scene"}))
        })?;
    active
        .session
        .snapshot_for_io()
        .map_err(crate::project_io::project_error)?
        .with_scene(scene)
        .map_err(|_| ErrorPayload::new(ErrorCode::NotFound, "scene not found"))
}
