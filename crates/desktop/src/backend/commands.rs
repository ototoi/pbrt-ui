//! Display adapter phase; state stays owned by Display.
use super::*;
use std::collections::BTreeSet;

fn frame_request_is_fresh(
    requested: u64,
    current_frame: u64,
    dirty: bool,
    render_in_flight: bool,
    preview_ready: bool,
    preview_key: Option<&str>,
    current_state: Result<Value, String>,
) -> bool {
    requested == current_frame
        && !dirty
        && !render_in_flight
        && preview_ready
        && current_state.is_ok_and(|state| {
            Some(project_preview_key(
                &state["context"]["project"],
                &state["active_scene"],
            ))
            .as_deref()
                == preview_key
        })
}

impl Display {
    pub(super) fn handle_commands(&mut self, commands: impl IntoIterator<Item = Command>) {
        for command in commands {
            let render_request = matches!(&command, Command::Render(_));
            let result = match command {
                Command::Settings(settings,expected)=>request(&self.core,Method::RenderSetSettings,json!({"settings":settings,"expected_settings_revision":expected})).map(|_|()),
                Command::ResetLayout=>request(&self.core,Method::WorkspaceResetLayout,json!({})).map(|_|()),
                Command::Render(view) => request(&self.core, Method::AppGetState, json!({})).and_then(|state| {
                    let project = &state["project"]["context"]["project"];
                    if !project.is_object() {
                        return Err("Open a project or a PBRT scene to render".into());
                    }
                    let view_key = project_preview_key(project, &self.last_project["active_scene"]);
                    if view && (self.project_geometry.is_none() || self.project_preview_key.as_deref() != Some(view_key.as_str())) {
                        return Err("Wait for the current project preview before rendering its view".into());
                    }
                    let view = view.then(|| json!({
                        "camera":serde_json::to_value(self.camera.to_protocol()).unwrap_or(Value::Null),
                        "size":self.render_size(),"mode":"solid","time":self.project_time,
                    }));
                    // Render Scene: the scene's own camera and Film.
                    let camera = if view.is_some() { "view" } else { "scene" };
                    request(&self.core, Method::ProjectRender, json!({"project":project,"expected_settings_revision":state["settings_revision"],"view":view,"camera":camera}))
                }).map(|_|()),
                Command::CancelRender=>request(&self.core,Method::JobList,json!({"limit":1000})).and_then(|jobs|{
                    if let Some(job)=jobs["jobs"].as_array().and_then(|jobs|jobs.iter().rev().find(|job|job["kind"]=="render" && ["queued","running","cancelling"].contains(&job["state"].as_str().unwrap_or("")))) {
                        request(&self.core,Method::JobCancel,json!({"job_id":job["job_id"]}))?;
                    } Ok(())
                }),
                Command::CancelJob(job_id)=>request(&self.core,Method::JobCancel,json!({"job_id":job_id})).map(|_|()),
                Command::Export(id,path)=>request(&self.core,Method::ArtifactExport,json!({"artifact_id":id,"absolute_path":path.to_str().ok_or("non UTF-8 export path").unwrap_or(""),"overwrite":false})).map(|_|()),
                Command::Stop=>{self.stopping=true;return;},
                Command::Mode(value) => {
                    self.mode = value;
                    self.dirty = true;
                    self.view_due = Some(Instant::now() + Duration::from_millis(300));
                    let _ = self.tx.send(Update::Mode(value));
                    Ok(())
                }
                Command::Overlays(visible) => {
                    if self.overlays_visible != visible {
                        self.overlays_visible = visible;
                        self.dirty = true;
                    }
                    Ok(())
                }
                Command::Time(value) => {
                    self.project_time = value;
                    self.dirty = true;
                    self.view_due = Some(Instant::now() + Duration::from_millis(300));
                    let _ = self.tx.send(Update::Time(value));
                    Ok(())
                }
                Command::Resize(size) => {
                    let size = fit_viewport(size);
                    if size != self.size {
                        self.size = size;
                        self.dirty = true;
                        // Renders, IPR and the shared view follow the aspect.
                        self.arm_ipr_submit();
                    }
                    Ok(())
                }
                Command::Orbit(x,y) => if self.project_geometry.is_some() { self.camera.orbit(x,y).map(|()|{self.dirty=true;self.arm_ipr_submit();}) } else { Ok(()) },
                Command::RetargetAt { frame_id: requested, xy } => {
                    let current = request(&self.core, Method::ProjectGetState, json!({}));
                    if frame_request_is_fresh(requested, self.frame_id, self.dirty, self.render_in_flight, self.preview_ready, self.project_preview_key.as_deref(), current)
                        && let Some(frame) = &self.last_frame
                    {
                        if let Ok(pick) = frame.pick_at(xy) {
                            if let Some(node) = self.project_pick_nodes.get(&pick) {
                                let ids = BTreeSet::from([node.clone()]);
                                if let Some(bounds) = self.project_geometry.as_ref().and_then(|geometry| crate::project_preview::selected_bounds(geometry, &self.project_pick_nodes, &self.project_object_bounds, self.project_time, &ids)) {
                                    let centre = bounds.center();
                                    self.camera.retarget_depth(centre).map(|()| { self.dirty = true; self.arm_ipr_submit(); })
                                } else {
                                    Ok(())
                                }
                            } else { Ok(()) }
                        } else { Ok(()) }
                    } else { Ok(()) }
                }
                Command::Pan(x,y) => if self.project_geometry.is_some() { self.camera.pan(x,y).map(|()|{self.dirty=true;self.arm_ipr_submit();}) } else { Ok(()) },
                Command::Fly(delta) => if self.project_geometry.is_some() { self.camera.fly_relative(delta).map(|()|{self.dirty=true;self.arm_ipr_submit();}) } else { Ok(()) },
                Command::Dolly(value) => if self.project_geometry.is_some() { self.camera.dolly(value).map(|()|{self.dirty=true;self.arm_ipr_submit();}) } else { Ok(()) },
                Command::GizmoPreview(delta) => {
                    self.gizmo_delta = delta;
                    self.apply_gizmo_delta()
                }
                Command::FrameAll => {
                    if let Some(geometry) = &self.project_geometry {
                        self.camera = ViewerCamera::frame_up(geometry.bounds.as_ref(), self.size[0] as f64 / self.size[1].max(1) as f64, self.scene_up()); self.camera.mirror = self.scene_mirror();
                        self.dirty = true;
                        self.arm_ipr_submit();
                        Ok(())
                    } else {
                        Err("Wait for the project preview before framing".into())
                    }
                }
                Command::ViewSceneCamera => {
                    if let Some(camera) = self.scene_camera {
                        self.camera = camera;
                        self.camera.mirror = self.scene_mirror();
                        self.dirty = true;
                        self.arm_ipr_submit();
                        Ok(())
                    } else {
                        Err("Wait for the project preview before aligning to the scene camera".into())
                    }
                }
                Command::ViewSelectedCamera { project, node_id } => {
                    (|| {
                        let id = uuid::Uuid::parse_str(&node_id).map_err(|error| error.to_string())?;
                        let node = request(&self.core, Method::ProjectGetNode, json!({"project":project,"node_id":id}))?;
                        if node["object"] != "camera" { return Err("Select a camera to align the view to".into()); }
                        let world = node["components"].as_array().into_iter().flatten().find(|component| component["kind"] == "transform").map(|component| &component["world"]["start"]).ok_or("The selected camera has no evaluated world transform")?;
                        let m: [f64; 16] = serde_json::from_value(world.clone()).map_err(|error| error.to_string())?;
                        let camera_parameters = camera_parameter_page(&self.core, &project, &node_id).unwrap_or_else(|_| json!({"parameters":[]}));
                        let parameter = |name: &str| camera_parameters["parameters"].as_array().into_iter().flatten().find(|p| p["name"] == name).and_then(|p| first_preview_number(&p["values"]));
                        let screenwindow = camera_parameters["parameters"].as_array().into_iter().flatten().find(|p| p["name"] == "screenwindow").and_then(|p| preview_numbers(&p["values"])).and_then(|values| <[f64; 4]>::try_from(values).ok());
                        // `halffov` wins when declared, as Align Camera to View writes it then.
                        let fov = parameter("halffov").map(|half| half * 2.).or_else(|| parameter("fov")).unwrap_or(90.);
                        let scene_id = self.last_project["active_scene"].as_str().unwrap_or_default();
                        let aspect = film_aspect(&self.core, &project, scene_id).unwrap_or(self.size[0] as f64 / self.size[1].max(1) as f64);
                        let distance = ((0..3).map(|i| (self.camera.eye[i] - self.camera.target[i]).powi(2)).sum::<f64>()).sqrt().max(1e-3);
                        let (eye, target, camera_up, _) = crate::editor::view_from_camera(&m, fov, aspect, distance).ok_or("The selected camera's transform is degenerate")?;
                        let vertical = protocol::view::camera_fov_y(Some(fov), screenwindow, parameter("frameaspectratio"), Some(aspect));
                        // Orbit turns about the camera's up, so keep the scene's up axis
                        // rather than the camera's own (tilted) up; a rolled camera loses its roll.
                        self.camera.eye=eye; self.camera.target=target; self.camera.up=up_for_view(eye, target, self.scene_up().vector(), camera_up); self.camera.fov_y_degrees=vertical;
                        let bounds = self.project_geometry.as_ref().and_then(|geometry| geometry.bounds.as_ref());
                        (self.camera.near, self.camera.far) = clipping_for_eye(eye, bounds);
                        self.camera.mirror=self.scene_mirror();
                        self.dirty=true; self.arm_ipr_submit(); Ok(())
                    })()
                }
                // The F key frames the selection; with nothing
                // selected it frames the whole scene, like Frame All.
                Command::FrameSelection => match self.selected_project_node.clone() {
                    Some(id) => match project_descendants(
                        &self.core,
                        &self.last_project["context"]["project"],
                        &id,
                    ) {
                        Ok(descendants) => {
                            let mut ids = BTreeSet::from([id]);
                            ids.extend(descendants);
                            if let Some(bounds) = self.project_geometry.as_ref().and_then(|geometry| crate::project_preview::selected_bounds(geometry,&self.project_pick_nodes,&self.project_object_bounds,self.project_time,&ids)) {
                                self.camera = ViewerCamera::frame_up(Some(&bounds),self.size[0] as f64 / self.size[1].max(1) as f64, self.scene_up()); self.camera.mirror = self.scene_mirror();
                                self.dirty = true;
                                self.arm_ipr_submit();
                                Ok(())
                            } else { Err("The selection has no preview geometry".into()) }
                        }
                        Err(error) => Err(error),
                    },
                    None => match self.project_geometry.as_ref() {
                        Some(geometry) => {
                            self.camera = ViewerCamera::frame_up(geometry.bounds.as_ref(), self.size[0] as f64 / self.size[1].max(1) as f64, self.scene_up()); self.camera.mirror = self.scene_mirror();
                            self.dirty = true;
                            self.arm_ipr_submit();
                            Ok(())
                        }
                        None => Err("Wait for the project preview before framing".into()),
                    },
                },
                Command::Pick { frame_id: requested, xy }=>{
                    let current = request(&self.core,Method::ProjectGetState,json!({}));
                    if frame_request_is_fresh(requested, self.frame_id, self.dirty, self.render_in_flight, self.preview_ready, self.project_preview_key.as_deref(), current)
                        && let Some(frame) = &self.last_frame
                    {
                            match frame.pick_at(xy) {
                                Ok(pick) => {
                                    self.selected_project_node = self.project_pick_nodes.get(&pick).cloned();
                                    let details = if let Some(id) = self.selected_project_node.clone() { self.node_details(&self.last_project["context"]["project"].clone(),&id) } else { Ok(json!({"message":"Select a project node"})) };
                                    match details {
                                        Ok(details) => {
                                            // A viewport pick also reveals the node in the Hierarchy.
                                            if let Some(node) = details["node"]["node_id"].as_str() {
                                                let path = details["node"]["path"].as_array().into_iter().flatten().filter_map(|id| id.as_str().map(str::to_owned)).collect();
                                                let _ = self.tx.send(Update::RevealPicked(node.to_owned(), path));
                                            }
                                            let _ = self.tx.send(Update::ProjectInspector(details));
                                        }
                                        Err(error) => {let _=self.tx.send(Update::Status(error));}
                                    }
                                }
                                Err(error) => {let _=self.tx.send(Update::Status(error));}
                            }
                    }
                    Ok(())
                }
                Command::Project(method, params) => {
                    // Create, open and import all replace the document,
                    // so none keeps the viewport camera.
                    let import = matches!(
                        &method,
                        Method::ProjectCreate | Method::ProjectImport | Method::ProjectOpen
                    );
                    // These additions go to the project that is already open. They
                    // publish into the same document, so the camera, the
                    // active scene and the whole layout stay untouched.
                    let add_import = matches!(
                        &method,
                        Method::ProjectImportScene | Method::ProjectImportResource
                    );
                    let save = method == Method::ProjectSave;
                    let export = method == Method::ProjectExport;
                    let foreground_label = match &method {
                        Method::ProjectCreate => Some("New project"),
                        Method::ProjectOpen => Some("Open project"),
                        Method::ProjectImport => Some("Import scene"),
                        Method::ProjectImportScene => Some("Add scene"),
                        Method::ProjectImportResource => Some("Import resources"),
                        _ => None,
                    };
                    if import {
                        // A plain import frames the new document; only
                        // Reload Source carries the camera over.
                        self.preserve_camera = None;
                    }
                    let result = request_error(&self.core, method, params);
                    match result {
                        Ok(response) => {
                            if let (Some(label), Some(job_id)) =
                                (foreground_label, response["job_id"].as_str())
                            {
                                self.foreground_jobs.retain(|(id, _, _)| id != job_id);
                                self.foreground_jobs
                                    .push((job_id.to_owned(), label.to_owned(), 0));
                            }
                            if import || add_import {
                                // The import report arrives in the job result, so
                                // keep every job id and read the diagnostics when
                                // each settles.
                                if let Some(job_id) = response["job_id"].as_str() {
                                    self.import_jobs.push(job_id.to_owned());
                                }
                            }
                            if save
                                && let Some(job_id) = response["job_id"].as_str() {
                                    self.save_job = Some(job_id.to_owned());
                                }
                            if export
                                && let Some(job_id) = response["job_id"].as_str() {
                                    self.export_jobs.push(job_id.to_owned());
                                }
                            Ok(())
                        }
                        // The window asks before replacing a dirty document; a
                        // racing edit lands here instead of the status line.
                        Err(error) if error.code == protocol::ErrorCode::UnsavedChanges => {
                            let _ = self.tx.send(Update::UnsavedChanges);
                            Ok(())
                        }
                        // A save refused before its job starts settles like a failed job.
                        Err(error) if save => {
                            let _ = self.tx.send(Update::SaveSettled {
                                ok: false,
                                error: Some(error.message),
                            });
                            Ok(())
                        }
                        Err(error) => Err(error.message),
                    }
                }
                Command::ReloadViewer(source) => {
                    let source_owner =
                        camera_preservation::ProjectOwner::from_ref(
                            &self.last_project["context"]["project"],
                        );
                    self.preserve_camera = None;
                    let params = json!({
                        "source_path":source,
                        "expected_current":self.last_project["context"].clone(),
                    });
                    match request_error(&self.core, Method::ProjectImport, params) {
                        Ok(response) => {
                            let job_id = response["job_id"].as_str().unwrap_or_default();
                            if !job_id.is_empty() {
                                self.import_jobs.push(job_id.to_owned());
                                self.foreground_jobs.push((job_id.to_owned(), "Reload scene".into(), 0));
                                self.preserve_camera = source_owner.map(|source_owner| {
                                    camera_preservation::CameraPreservation::new(
                                        job_id.to_owned(),
                                        source_owner,
                                    )
                                });
                            }
                            Ok(())
                        }
                        Err(error) => Err(error.message),
                    }
                }
                Command::ProjectSelect { project, node_id } => {
                    self.selected_project_node = Some(node_id.clone());
                    match self.node_details(&project,&node_id) {
                        Ok(details)=>{let _=self.tx.send(Update::ProjectInspector(details));Ok(())}
                        Err(error)=>{self.selected_project_node=None;let _=self.tx.send(Update::ProjectInspector(json!({"message":error})));Err(error)}
                    }
                }
                Command::ProjectDeselect => {
                    self.selected_project_node = None;
                    let _ = self.tx.send(Update::ProjectDeselected);
                    Ok(())
                }
                Command::ProjectSearch { project, query, offset, limit } => {
                    // Revision switches race with in-flight searches; answer
                    // only for the project the display thread currently shows.
                    if self.last_project["context"]["project"] != project {
                        Ok(())
                    } else {
                        match project_search(&self.core, &project, &query, offset, limit) {
                            Ok(page) => {
                                let _ = self.tx.send(Update::ProjectSearch {
                                    query,
                                    offset,
                                    page,
                                });
                                Ok(())
                            }
                            Err(error) => {
                                // Clear the pending state even on failure; the
                                // status line carries the reason.
                                let _ = self.tx.send(Update::ProjectSearch {
                                    query,
                                    offset,
                                    page: Value::Null,
                                });
                                Err(error)
                            }
                        }
                    }
                }
                Command::ProjectSearchContents { project, query, generation, offset, limit } => {
                    if self.last_project["context"]["project"] != project {
                        Ok(())
                    } else {
                        let result = project_search_contents(&self.core, &project, &query, offset, limit);
                        let update = Update::ProjectSearchContents {
                            project,
                            query,
                            generation,
                            offset,
                            page: result.as_ref().cloned().unwrap_or(Value::Null),
                            error: result.as_ref().err().cloned(),
                        };
                        let _ = self.tx.send(update);
                        result.map(|_| ())
                    }
                }
                Command::ProjectExpand { project, node_id, offset } => {
                    // Stale expansions arrive from previous revisions; drop them
                    // quietly so the UI is not torn by late pages.
                    if self.last_project["context"]["project"] != project {
                        Ok(())
                    } else {
                        match project_nodes_page(
                            &self.core,
                            &project,
                            &Some(node_id.clone()),
                            offset as usize,
                            1000,
                        ) {
                            Ok(page) => {
                                let _ = self.tx.send(Update::ProjectChildren {
                                    parent: node_id,
                                    offset,
                                    nodes: page["nodes"].as_array().cloned().unwrap_or_default(),
                                    next_offset: page["next_offset"].as_u64(),
                                });
                                Ok(())
                            }
                            Err(error) => Err(error),
                        }
                    }
                }
                Command::ProjectContents { project, folder, offset, limit } => {
                    if self.last_project["context"]["project"] != project {
                        Ok(())
                    } else {
                        let page = project_contents_page(&self.core, &project, &folder, offset, limit);
                        let failed = page.as_ref().err().cloned();
                        let _ = self.tx.send(Update::ProjectContents {
                            project,
                            folder,
                            offset,
                            page: page.unwrap_or(Value::Null),
                        });
                        failed.map_or(Ok(()), Err)
                    }
                }
                // Static: r4's table, summarized once.
                Command::GetResourceSchemas => {
                    let summary = resource_schema_summary(&self.core);
                    let failed = summary.as_ref().err().cloned();
                    let _ = self.tx.send(Update::ResourceSchemas(
                        summary.unwrap_or(Value::Null),
                    ));
                    failed.map_or(Ok(()), Err)
                }
                // A node editor needs no file; its graph comes later.
                Command::OpenEditor { project, entry, request_id } if entry["kind"] == "resource" => {
                    let _ = self.tx.send(Update::EditorReady { project, request_id, entry, path: Ok(None) });
                    Ok(())
                }
                Command::OpenEditor { project, entry, request_id } if entry["kind"] == "parameter" => {
                    let _ = self.tx.send(Update::EditorReady { project, request_id, entry, path: Ok(None) });
                    Ok(())
                }
                Command::OpenEditor { project, entry, request_id } => {
                    let asset = request(&self.core, Method::ProjectGetAsset, json!({"project":project,"asset_id":entry["id"]}));
                    let path = asset
                        .map(|asset| asset["path"].as_str().map(str::to_owned))
                        .map_err(|error| protocol::ErrorPayload::new(protocol::ErrorCode::IoError, error));
                    let _ = self.tx.send(Update::EditorReady { project, request_id, entry, path });
                    Ok(())
                }
                Command::CheckItems { project, ids } => {
                    if self.last_project["context"]["project"] != project {
                        Ok(())
                    } else {
                        let missing = ids
                            .into_iter()
                            .filter(|id| {
                                // An editor tab shows an asset or a resource.
                                let gone = |method, params| {
                                    request_error(&self.core, method, params)
                                        .is_err_and(|error| error.code == protocol::ErrorCode::NotFound)
                                };
                                gone(Method::ProjectGetAsset, json!({"project":project,"asset_id":id}))
                                    && gone(Method::ProjectGetResource, json!({"project":project,"resource_id":id}))
                            })
                            .collect::<Vec<_>>();
                        if !missing.is_empty() {
                            let _ = self.tx.send(Update::ItemsMissing { project, ids: missing });
                        }
                        Ok(())
                    }
                }
                // A node editor's graph and its selected node.
                Command::ResourceGraph { project, resource_id } => {
                    let graph = request_error(&self.core, Method::ProjectGetResourceGraph, json!({"project":project,"resource_id":resource_id}));
                    let failed = graph.as_ref().err().map(|error| error.message.clone());
                    // The code rides beside the message: the node editor tells a
                    // gone resource from a raced revision.
                    let _ = self.tx.send(Update::ResourceGraph {
                        project,
                        resource_id,
                        graph: graph.unwrap_or_else(|error| json!({"error":error.message,"error_code":error.code})),
                    });
                    failed.map_or(Ok(()), Err)
                }
                // Queue it; the display loop submits one at a time so
                // this handler never blocks the thread waiting on core.
                Command::Preview { project, id, kind } => {
                    self.queue_preview(project, id, kind);
                    Ok(())
                }
                Command::ThumbnailsVisible { project, generation, candidates } => {
                    self.thumbnail_project = project;
                    self.thumbnail_generation = generation;
                    self.thumbnail_candidates = candidates.into_iter().take(256).collect();
                    Ok(())
                }
                Command::ThumbnailDone(request_id) => {
                    if self.thumbnail_flight.as_ref().is_some_and(|flight| flight.request_id == request_id) {
                        self.thumbnail_flight = None;
                    }
                    Ok(())
                }
                // The array values review: the table reads its
                // own windows, whatever the storage keeps today.
                Command::GetParameterValues { project, target, name, offset, limit, form, group_page } => {
                    let page = request(&self.core, Method::ProjectGetParameterValues, json!({
                        "project":project,"target":target,"name":name,
                        "offset":offset,"limit":limit}));
                    let _ = self.tx.send(Update::ParameterValues {
                        project, form, group_page, page,
                    });
                    Ok(())
                }
                Command::GetSpectrumTable { project, resource_id } => {
                    // Read both tables whole — the storage kinds from
                    // the parameter preview, the numbers in windows — and
                    // answer once, so the editor gets one payload.
                    let target = json!({"kind":"resource","id":resource_id});
                    let parameters = request(
                        &self.core,
                        Method::ProjectGetParameters,
                        json!({"project":project,"target":target}),
                    );
                    let fetch = |name: &str, parameters: &Value| -> Result<(Vec<f64>, bool), String> {
                        // `array` storage means set_parameter cannot
                        // rewrite the table (`set_parameter_values` is the
                        // editor's write path there).
                        let declared = parameters["parameters"]
                            .as_array()
                            .and_then(|rows| rows.iter().find(|row| row["name"] == name))
                            .ok_or_else(|| format!("the resource has no {name} sample table"))?
                            .clone();
                        let storage_array = declared["values"]["kind"] == "array";
                        let mut floats = Vec::new();
                        let mut offset = 0usize;
                        loop {
                            let page = request(
                                &self.core,
                                Method::ProjectGetParameterValues,
                                json!({"project":project,"target":target,"name":name,
                                       "offset":offset,"limit":4096}),
                            )
                            .map_err(|error| error.to_owned())?;
                            for value in page["values"]["value"]
                                .as_array()
                                .cloned()
                                .unwrap_or_default()
                            {
                                if value["kind"] != "finite" {
                                    return Err("spectrum samples must be finite".into());
                                }
                                floats.push(value["value"].as_f64().ok_or_else(|| {
                                    "a sample is not a finite number".to_owned()
                                })?);
                            }
                            match page["next_offset"].as_u64() {
                                Some(next) => offset = next as usize,
                                None => break,
                            }
                        }
                        Ok((floats, storage_array))
                    };
                    // Both calls need the parameter listing: pass a clone.
                    let parameters = parameters.map_err(|error| error.to_owned());
                    let table = parameters.and_then(|parameters| {
                        let (wavelengths, wavelengths_array) =
                            fetch("wavelengths", &parameters)?;
                        let (values, values_array) = fetch("values", &parameters)?;
                        if wavelengths.len() != values.len() {
                            return Err("wavelengths and values sample counts differ".into());
                        }
                        Ok(json!({
                            "wavelengths":wavelengths, "values":values,
                            "storage": if wavelengths_array || values_array { "array" } else { "floats" },
                        }))
                    });
                    let _ = self.tx.send(Update::SpectrumTable {
                        project,
                        resource_id,
                        table,
                    });
                    Ok(())
                }
                Command::SpectrumSwatch { project, resource_id, role } => {
                    let answer = request(
                        &self.core,
                        Method::ProjectSpectrumColor,
                        json!({"project":project,
                               "target":{"kind":"resource","id":resource_id},
                               "parameter":"values","role":role}),
                    );
                    let _ = self.tx.send(Update::SpectrumSwatch {
                        project,
                        resource_id,
                        role,
                        answer,
                    });
                    Ok(())
                }
                // One parameter row's colour, for its own role
                // when it picked one — the core derives the rest from the
                // schema, so the box and Lit can never disagree. Handled
                // inline like every other small read: queued, answered
                // through the update channel, and never waited on by
                // the GPUI thread.
                Command::ParameterSpectrumColour { project, target, parameter, role } => {
                    let mut params = json!({"project":project,"target":target,
                                             "parameter":parameter});
                    if let Some(role) = role.as_deref() {
                        params["role"] = json!(role);
                    }
                    let answer =
                        request(&self.core, Method::ProjectSpectrumColor, params);
                    let _ = self.tx.send(Update::ParameterSpectrumColour {
                        project,
                        target,
                        parameter,
                        role,
                        answer,
                    });
                    Ok(())
                }
                Command::ListReferences { project, item } => {
                    // Find where this item is used. The backend caches past
                    // answers per revision, so the limit pays for itself.
                    let (kind, id) = &item;
                    let calls = [
                        json!({"project":project,"item":{"kind":kind,"id":id},
                               "offset":0,"limit":100}),
                        json!({"project":project,"item":{"kind":kind,"id":id},
                               "offset":100,"limit":100}),
                    ];
                    let mut list = json!({"total":0,"references":[]});
                    for call in calls {
                        match request(&self.core, Method::ProjectListReferences, call) {
                            Ok(page) => {
                                let total = page["total"].as_u64().unwrap_or(0);
                                let rows = page["references"]
                                    .as_array()
                                    .cloned()
                                    .unwrap_or_default();
                                if let Some(array) = list["references"].as_array_mut() {
                                    array.extend(rows);
                                    list["total"] = json!(total);
                                }
                            }
                            Err(error) => {
                                list = json!({"error":error});
                                break;
                            }
                        }
                        match list["total"].as_u64() {
                            Some(total) if list["references"].as_array().map_or(0, |a| a.len()) < total as usize => continue,
                            _ => break,
                        }
                    }
                    let _ = self.tx.send(Update::ListReferences {
                        project,
                        item,
                        list,
                    });
                    Ok(())
                }
                Command::ResourceDetails { project, resource_id } => {
                    let entry = json!({"kind":"resource","id":resource_id});
                    let details = self
                        .items(&project)
                        .and_then(|items| project_item_details_with(&self.core, &project, &entry, items));
                    let failed = details.as_ref().err().cloned();
                    let _ = self.tx.send(Update::ResourceDetails {
                        project,
                        resource_id,
                        details: details.unwrap_or_else(|error| json!({"error":error})),
                    });
                    failed.map_or(Ok(()), Err)
                }
                Command::InspectContents { project, entry } => {
                    self.selected_project_node = None;
                    match project_item_details(&self.core, &project, &entry) {
                        Ok(details) => {
                            let _ = self.tx.send(Update::ProjectInspector(details));
                            Ok(())
                        }
                        Err(error) => {
                            let _ = self.tx.send(Update::ProjectInspector(json!({"item":entry,"error":error})));
                            Err(error)
                        }
                    }
                }
                Command::PlanImplementation { project, target, implementation, form } => {
                    match request(&self.core, Method::ProjectPlanImplementation, json!({"project":project,"target":target,"implementation":implementation})) {
                        Ok(plan) => {
                            let _ = self.tx.send(Update::ImplementationPlan { project, form, plan });
                            Ok(())
                        }
                        Err(error) => {
                            let _ = self.tx.send(Update::EditSettled { form, error: Some(error.clone()) });
                            Err(error)
                        }
                    }
                }
                Command::Submit { method, params, form } => {
                    let error = request_error(&self.core, method, params).err().map(|error| {
                        if error.code == protocol::ErrorCode::ReadOnly {
                            "Viewer is read-only — Save as Project to edit".to_owned()
                        } else {
                            error.message
                        }
                    });
                    if let Some(error) = &error {
                        let _ = self.tx.send(Update::Log(format!("Edit rejected: {error}")));
                    }
                    let _ = self.tx.send(Update::EditSettled { form, error });
                    Ok(())
                }
                Command::SubmitSequence { method, steps, form } => {
                    let mut error = None;
                    for mut params in steps {
                        match request(&self.core, Method::ProjectGetState, json!({})) {
                            Ok(state) => params["project"] = state["context"]["project"].clone(),
                            Err(message) => {
                                error = Some(message);
                                break;
                            }
                        }
                        match request_error(&self.core, method.clone(), params) {
                            Ok(result) => {
                                if let Some(job) = result["job_id"].as_str()
                                    && let Err(message) = wait_for_job(&self.core, job)
                                {
                                    error = Some(message);
                                    break;
                                }
                            }
                            Err(refused) => {
                                error = Some(if refused.code == protocol::ErrorCode::ReadOnly {
                                    "Viewer is read-only — Save as Project to edit".to_owned()
                                } else {
                                    refused.message
                                });
                                break;
                            }
                        }
                    }
                    if let Some(error) = &error {
                        let _ = self.tx.send(Update::Log(format!("Edit rejected: {error}")));
                    }
                    let _ = self.tx.send(Update::EditSettled { form, error });
                    Ok(())
                }
                Command::Apply { project, edits, follow, form } => {
                    let editor_ask = match &follow {
                        Follow::SelectResourceInEditor { ask, .. } => Some(*ask),
                        _ => None,
                    };
                    let result = match request_error(&self.core, Method::ProjectApply, json!({"project":project,"edits":edits})) {
                        Ok(result) => {
                            self.follow_apply(&result, follow);
                            if let Some(selected) = self.selected_project_node.clone() {
                                match self.node_details(&result["project"], &selected) {
                                    Ok(details) => {
                                        let _ = self.tx.send(Update::ProjectInspector(details));
                                    }
                                    Err(_error) => {
                                        self.selected_project_node = None;
                                        let _ = self.tx.send(Update::ProjectInspector(
                                            json!({"message":"Select a project node"}),
                                        ));
                                    }
                                }
                            }
                            Ok(())
                        }
                        Err(error) if error.code == protocol::ErrorCode::ReadOnly => {
                            Err("Viewer is read-only — Save as Project to edit".to_owned())
                        }
                        Err(error) => Err(error.message),
                    };
                    if let (Err(_), Some(ask)) = (&result, editor_ask) {
                        let _ = self.tx.send(Update::EditorCreateFailed(ask));
                    }
                    if result.is_err() && form.as_deref() == Some(crate::editor::GIZMO_FORM) && self.gizmo_delta.take().is_some() {
                        // A refused gizmo commit: the selection goes back.
                        let _ = self.apply_gizmo_delta();
                    }
                    if let Some(form) = form {
                        // The field shows its own outcome; the Console keeps
                        // a line for every rejection.
                        if let Err(error) = &result {
                            let _ = self.tx.send(Update::Log(format!("Edit rejected: {error}")));
                        }
                        let _ = self.tx.send(Update::EditSettled { form, error: result.as_ref().err().cloned() });
                    }
                    result
                }
                Command::AlignCamera { project, scene_id, camera_id, coordinate_system, view, view_fov_y_degrees, viewport_aspect } => {
                    let aligned = (|| {
                        let camera_uuid = uuid::Uuid::parse_str(&camera_id).map_err(|error| error.to_string())?;
                        let camera = request(&self.core, Method::ProjectGetNode,
                            json!({"project":project,"node_id":camera_uuid}))?;
                        let implementation = camera["data"]["implementation"].as_str().unwrap_or_default();
                        let aspect = film_aspect(&self.core, &project, &scene_id).unwrap_or(viewport_aspect);
                        let parameters = camera_parameter_page(&self.core, &project, &camera_id)
                            .unwrap_or_else(|_| json!({"parameters":[]}));
                        let edits = crate::editor::align_camera_edits_for_type(
                            implementation, &camera_id, coordinate_system, view,
                            view_fov_y_degrees, aspect, &parameters,
                        ).ok_or("The current view cannot orient a camera")?;
                        request(&self.core, Method::ProjectApply, json!({"project":project,"edits":edits}))
                    })();
                    match aligned {
                        Ok(result) => {
                            if let Some(selected) = self.selected_project_node.clone()
                                && let Ok(details) = self.node_details(&result["project"], &selected)
                            {
                                let _ = self.tx.send(Update::ProjectInspector(details));
                            }
                            Ok(())
                        }
                        Err(error) => Err(error),
                    }
                }
            };
            if let Err(error) = result {
                if render_request {
                    let _ = self.tx.send(Update::RenderRequestFailed(error.clone()));
                }
                let _ = self.tx.send(Update::Status(error));
            }
        }
    }

    /// Inspector details for a node, with this revision's Contents items for
    /// links and selectors (listed once per revision).
    pub(super) fn node_details(&mut self, project: &Value, node_id: &str) -> Result<Value, String> {
        let items = self.items(project)?;
        project_node_details(&self.core, project, node_id, &items)
    }

    /// This revision's Contents items, listed once per revision.
    fn items(&mut self, project: &Value) -> Result<Value, String> {
        Ok(match &self.project_items {
            Some((reference, items)) if reference == project => items.clone(),
            _ => {
                let items = project_items(&self.core, project)?;
                self.project_items = Some((project.clone(), items.clone()));
                items
            }
        })
    }

    /// Open a created scene, or pick the created object to reveal: a shape,
    /// light or instance before its wrapper group.
    fn follow_apply(&mut self, result: &Value, follow: Follow) {
        let created = result["created"].as_array().cloned().unwrap_or_default();
        let project = result["project"].clone();
        match follow {
            Follow::Nothing => {}
            Follow::OpenScene => {
                if let Some(scene) = created.iter().find(|item| item["kind"] == "scene")
                    && let Err(error) = request(
                        &self.core,
                        Method::ProjectOpenScene,
                        json!({"project":project,"scene":scene["id"]}),
                    )
                {
                    let _ = self.tx.send(Update::Status(error));
                }
            }
            Follow::RevealFirstCreated { parent } => {
                if let Some(node) = created
                    .iter()
                    .find(|item| item["kind"] == "node")
                    .and_then(|item| item["id"].as_str())
                {
                    let _ = self.tx.send(Update::Created {
                        node: node.to_owned(),
                        wrapper: None,
                        parent,
                    });
                }
            }
            Follow::Reveal { parent } => {
                let mut candidate: Option<(usize, String, Option<String>)> = None;
                for item in created.iter().filter(|item| item["kind"] == "node") {
                    let Some(id) = item["id"].as_str() else {
                        continue;
                    };
                    let Ok(node) = request(
                        &self.core,
                        Method::ProjectGetNode,
                        json!({"project":project,"node_id":id}),
                    ) else {
                        continue;
                    };
                    let data = &node["data"];
                    let next = reveal_candidate(id, data);
                    if let Some(next) = next
                        && candidate.as_ref().is_none_or(|current| next.0 < current.0)
                    {
                        candidate = Some(next);
                    }
                }
                if let Some((_, node, wrapper)) = candidate {
                    let _ = self.tx.send(Update::Created {
                        node,
                        wrapper,
                        parent,
                    });
                }
            }
            // Contents ▸ New / the node editor's background
            // create): select the created resource in its
            // folder; the Contents flow enters rename, the editor ask
            // reports itself to the editor instead.
            Follow::SelectResource { folder } => {
                self.select_created_resource(&project, &created, &folder, None);
            }
            Follow::SelectResourceInEditor { folder, ask } => {
                self.select_created_resource(&project, &created, &folder, Some(ask));
            }
        }
    }

    /// Publish the apply's created resource as the `Update::CreatedResource`
    /// the window selects: the entry located in the folder the edit aimed
    /// at (a big folder pages), `editor_ask` marking which flow asked —
    /// Contents ▸ New (None) enters rename; the node editor's does not,
    /// and hears of a failure too.
    fn select_created_resource(
        &self,
        project: &Value,
        created: &[Value],
        folder: &str,
        editor_ask: Option<u64>,
    ) {
        let failed = || {
            if let Some(ask) = editor_ask {
                let _ = self.tx.send(Update::EditorCreateFailed(ask));
            }
        };
        let Some(resource) = created
            .iter()
            .find(|item| item["kind"] == "resource" && item["id"].is_string())
        else {
            failed();
            return;
        };
        match created_resource_entry(&self.core, project, folder, &resource["id"]) {
            Ok(entry) => {
                let _ = self.tx.send(Update::CreatedResource {
                    project: project.clone(),
                    entry,
                    editor_ask,
                });
            }
            Err(error) => {
                failed();
                let _ = self.tx.send(Update::Status(error));
            }
        }
    }

    /// Read the import report once its job settles and put diagnostics in the
    /// Console. `project.open` has no report; its lines are simply empty.
    /// additions carry their diagnostics at the top of the result and
    /// finish with a status line naming what they added. Every pending job
    /// gets a poll of its own - imports can run in parallel, and each one's
    /// diagnostics must reach the Console.
    pub(super) fn poll_import_report(&mut self) {
        for index in (0..self.import_jobs.len()).rev() {
            let Ok(job) = request(
                &self.core,
                Method::JobGet,
                json!({"job_id": self.import_jobs[index]}),
            ) else {
                // An unanswerable id just drops.
                self.import_jobs.swap_remove(index);
                continue;
            };
            let job_id = self.import_jobs[index].clone();
            match job["state"].as_str() {
                Some("succeeded") => {
                    self.complete_camera_reload(&job_id, &job["result"]);
                    let report = crate::editor::document::import_report(&job["result"]);
                    let lines = crate::editor::document::import_diagnostic_lines(report);
                    let warnings = crate::editor::document::import_warning_count(report);
                    for line in lines {
                        let _ = self.tx.send(Update::Log(line));
                    }
                    let _ = self.tx.send(Update::ImportWarnings(warnings));
                    if let Some(added) = crate::editor::document::import_summary(&job["result"]) {
                        let _ = self.tx.send(Update::Status(added));
                    }
                    self.import_jobs.swap_remove(index);
                }
                Some("failed") | Some("cancelled") => {
                    // A failed reload never produced a preview; frame normally
                    // next time. Another import's failure leaves its intent.
                    self.fail_camera_reload(&job_id);
                    let message = import_failure_message(&job);
                    // Status is overwritten by the next poll, so the result
                    // also goes to the Console, where it stays.
                    let _ = self.tx.send(Update::Status(message.clone()));
                    let _ = self.tx.send(Update::Log(message));
                    self.import_jobs.swap_remove(index);
                }
                _ => {}
            }
        }
    }

    /// Put what each settled export left out (its `warnings`) in the
    /// Console. A failure already shows in the job list.
    pub(super) fn poll_export_warnings(&mut self) {
        for index in (0..self.export_jobs.len()).rev() {
            let Ok(job) = request(
                &self.core,
                Method::JobGet,
                json!({"job_id": self.export_jobs[index]}),
            ) else {
                self.export_jobs.swap_remove(index);
                continue;
            };
            match job["state"].as_str() {
                Some("succeeded") => {
                    for line in crate::editor::document::export_warning_lines(&job["result"]) {
                        let _ = self.tx.send(Update::Log(line));
                    }
                    self.export_jobs.swap_remove(index);
                }
                Some("failed") | Some("cancelled") => {
                    self.export_jobs.swap_remove(index);
                }
                _ => {}
            }
        }
    }

    /// Report an in-place save once its job settles, so the window can run the
    /// action the unsaved-changes dialog held back.
    pub(super) fn poll_save_job(&mut self) {
        let Some(job_id) = self.save_job.clone() else {
            return;
        };
        let Ok(job) = request(&self.core, Method::JobGet, json!({"job_id": job_id})) else {
            self.save_job = None;
            return;
        };
        match job["state"].as_str() {
            Some("succeeded") => {
                self.save_job = None;
                let _ = self.tx.send(Update::SaveSettled {
                    ok: true,
                    error: None,
                });
            }
            Some("failed") | Some("cancelled") => {
                let error = job["error"]["message"].as_str().map(str::to_owned);
                self.save_job = None;
                let _ = self.tx.send(Update::SaveSettled { ok: false, error });
            }
            _ => {}
        }
    }
}

fn reveal_candidate(id: &str, data: &Value) -> Option<(usize, String, Option<String>)> {
    match (data["kind"].as_str(), data["entity"].as_str()) {
        (Some("entity"), Some("shape")) => Some((0, id.to_owned(), None)),
        (Some("entity"), Some("light")) => Some((1, id.to_owned(), None)),
        (Some("instance"), _) => Some((2, id.to_owned(), None)),
        (Some("scope"), _) if data["owned_object"].is_string() => data["owned_object"]
            .as_str()
            .map(|owner| (0, owner.to_owned(), Some(id.to_owned()))),
        (Some("scope"), _) => Some((3, id.to_owned(), None)),
        _ => None,
    }
}

fn clipping_for_eye(eye: [f64; 3], bounds: Option<&protocol::geometry::Bounds>) -> (f64, f64) {
    let (center, radius) = bounds
        .map(|bounds| (bounds.center(), bounds.half_diagonal().max(1e-3)))
        .unwrap_or(([0.; 3], 1.));
    let distance = (0..3)
        .map(|axis| (eye[axis] - center[axis]).powi(2))
        .sum::<f64>()
        .sqrt();
    ((distance - radius * 2.).max(0.001), distance + radius * 2.)
}

/// Keep the scene axis as screen-up unless it would make the view basis
/// degenerate; top-down views need the camera's own world-space up vector.
fn up_for_view(
    eye: [f64; 3],
    target: [f64; 3],
    scene_up: [f64; 3],
    camera_up: [f64; 3],
) -> [f64; 3] {
    let forward = std::array::from_fn::<_, 3, _>(|i| target[i] - eye[i]);
    let length = |vector: [f64; 3]| vector.iter().map(|value| value * value).sum::<f64>().sqrt();
    let forward_length = length(forward);
    let scene_length = length(scene_up);
    let parallel = forward_length > 0.
        && scene_length > 0.
        && (forward
            .iter()
            .zip(scene_up)
            .map(|(a, b)| a * b)
            .sum::<f64>()
            / (forward_length * scene_length))
            .abs()
            > 0.999;
    if parallel { camera_up } else { scene_up }
}

#[cfg(test)]
mod tests {
    use super::{clipping_for_eye, reveal_candidate, up_for_view};
    use serde_json::json;

    #[test]
    fn created_reveal_targets_visible_owner_for_shapes_lights_instances_and_groups() {
        let wrapper =
            reveal_candidate("wrapper", &json!({"kind":"scope", "owned_object":"shape"})).unwrap();
        let shape = reveal_candidate("shape", &json!({"kind":"entity", "entity":"shape"})).unwrap();
        assert_eq!(wrapper, (0, "shape".into(), Some("wrapper".into())));
        assert_eq!(shape, (0, "shape".into(), None));
        assert_eq!(
            reveal_candidate("light", &json!({"kind":"entity", "entity":"light"})).unwrap(),
            (1, "light".into(), None)
        );
        assert_eq!(
            reveal_candidate("instance", &json!({"kind":"instance"})).unwrap(),
            (2, "instance".into(), None)
        );
        assert_eq!(
            reveal_candidate("group", &json!({"kind":"scope"})).unwrap(),
            (3, "group".into(), None)
        );
    }

    #[test]
    fn top_down_camera_uses_camera_up_when_scene_up_is_parallel() {
        let up = up_for_view([0., 2., 0.], [0., 0., 0.], [0., 1., 0.], [0., 0., 1.]);
        assert_eq!(up, [0., 0., 1.]);
        let forward = [0., -2., 0.];
        let cross = [
            forward[1] * up[2] - forward[2] * up[1],
            forward[2] * up[0] - forward[0] * up[2],
            forward[0] * up[1] - forward[1] * up[0],
        ];
        assert!(cross.iter().map(|value| value * value).sum::<f64>() >= 1e-18);
    }

    #[test]
    fn selected_camera_clipping_contains_distant_scene_bounds() {
        let bounds = protocol::geometry::Bounds {
            min: [-1., -1., -1.],
            max: [1., 1., 1.],
        };
        let (near, far) = clipping_for_eye([0., 0., 1000.], Some(&bounds));
        assert!(near > 990.);
        assert!(far > 1000.);
        assert!(near < far);
    }
}

/// Scale a viewport down, keeping its aspect, to at most 4096 pixels a side
/// and the renderer's pixel budget; each side is at least 16.
fn fit_viewport([width, height]: [u32; 2]) -> [u32; 2] {
    let (w, h) = (width.max(1) as f64, height.max(1) as f64);
    let scale = (4096. / w.max(h))
        .min((scene_view::preview::MAX_PREVIEW_PIXELS as f64 / (w * h)).sqrt())
        .min(1.);
    [w, h].map(|side| ((side * scale).floor() as u32).max(16))
}

impl Display {
    /// Draw the selection's draws moved by `gizmo_delta`, or as
    /// uploaded when there is none. The renderer update runs on the GPU
    /// worker; a request keyed to a superseded scene is dropped there
    /// This ensures a drag never lands on freshly installed geometry.
    pub(super) fn apply_gizmo_delta(&mut self) -> Result<(), String> {
        let Some(draws) = self
            .project_geometry
            .as_ref()
            .map(|geometry| geometry.draws.clone())
        else {
            return Ok(());
        };
        let mut draws = draws;
        if let Some(delta) = self.gizmo_delta {
            for draw in draws
                .iter_mut()
                .filter(|draw| self.highlight.contains(&draw.pick_id))
            {
                for matrix in &mut draw.world_from_object {
                    *matrix = multiply(&delta, matrix);
                }
            }
        }
        if let Some(key) = self.project_preview_key.clone() {
            let _ = self.gpu.requests.send(gpu::GpuRequest::UpdateDraws {
                key,
                draws,
                adopt: None,
            });
        }
        self.dirty = true;
        Ok(())
    }
}

/// Row-major `a · b`.
fn multiply(a: &[f64; 16], b: &[f64; 16]) -> [f64; 16] {
    crate::project_preview::row_major_product(*a, *b)
}

/// Block until a job is terminal; the error is the job's own message.
fn wait_for_job(core: &CoreHandle, job: &str) -> Result<(), String> {
    for _ in 0..1200 {
        let state = request(core, Method::JobGet, json!({"job_id": job}))?;
        match state["state"].as_str() {
            Some("succeeded") => return Ok(()),
            Some("failed" | "cancelled") => {
                return Err(state["error"]["message"]
                    .as_str()
                    .unwrap_or("the edit job did not finish")
                    .to_owned());
            }
            _ => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    Err("the edit job timed out".into())
}

/// One settled import's failure text: a failed import left the
/// previously open project in place, and the reason alone does not say
/// that. The message names the file the job carries as its root path —
/// in every desktop flow that is the imported source; a flow that ever
/// passes a destination would need core to keep the source as the
/// canonical root. A cancelled import says only that it was cancelled.
pub(super) fn import_failure_message(job: &Value) -> String {
    let cancelled =
        job["state"] == "cancelled" || job["error"]["code"].as_str() == Some("cancelled");
    if cancelled {
        return "Import cancelled".to_owned();
    }
    let reason = job["error"]["message"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| "Import did not publish".into());
    let file = std::path::Path::new(job["root_path"].as_str().unwrap_or_default())
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if file.is_empty() {
        format!("Import failed: {reason}. The open project is unchanged.")
    } else {
        format!("Import failed: {file}: {reason}. The open project is unchanged.")
    }
}

#[cfg(test)]
mod freshness_tests {
    use super::*;

    #[test]
    fn frame_request_requires_the_displayed_clean_preview() {
        let state = json!({"context":{"project":{}},"active_scene":null});
        let key = project_preview_key(&state["context"]["project"], &state["active_scene"]);
        let current_state = || Ok(state.clone());

        assert!(frame_request_is_fresh(
            7,
            7,
            false,
            false,
            true,
            Some(&key),
            current_state(),
        ));
        assert!(!frame_request_is_fresh(
            6,
            7,
            false,
            false,
            true,
            Some(&key),
            current_state(),
        ));
        assert!(!frame_request_is_fresh(
            7,
            7,
            true,
            false,
            true,
            Some(&key),
            current_state(),
        ));
        assert!(!frame_request_is_fresh(
            7,
            7,
            false,
            true,
            true,
            Some(&key),
            current_state(),
        ));
    }
}
