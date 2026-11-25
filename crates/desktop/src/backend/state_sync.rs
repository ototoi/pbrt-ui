//! Display adapter phase; state stays owned by Display.
use super::*;

fn node_selection_is_gone(error: &protocol::ErrorPayload) -> bool {
    error.code == protocol::ErrorCode::NotFound
}

pub(super) fn should_foreground_project_preview(
    activation_changed: bool,
    scene_changed: bool,
) -> bool {
    activation_changed || scene_changed
}

pub(super) fn preview_slot_path(
    reference: &Value,
    project_folder: Option<&std::path::Path>,
    cache_root: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    let sanitized: String = reference["project_id"]
        .as_str()?
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let activation = reference["activation"]
        .as_u64()
        .or_else(|| reference["activation"].as_str()?.parse().ok())?;
    let base = project_folder
        .map(|folder| folder.join("Library/Preview"))
        .or_else(|| cache_root.map(|root| root.join("Library/Preview")))?;
    Some(base.join(format!(
        "pbrt-ui-preview-{}-{sanitized}-{activation}",
        std::process::id()
    )))
}

impl Display {
    pub(super) fn sync_state(&mut self) {
        if let Ok(state) = request(&self.core, Method::AppGetState, json!({})) {
            let project = state.get("project").cloned().unwrap_or(Value::Null);
            if project != self.last_project {
                let previous = self.last_project["context"]["project"].clone();
                let previous_folder = self.last_project["path"]
                    .as_str()
                    .map(std::path::PathBuf::from);
                let previous_scene = self.last_project["active_scene"].clone();
                let previous_up = self.scene_up();
                self.last_project = project;
                // An Up edit turns the orbit axis without moving the view;
                // a view along the new axis is re-framed instead.
                let up = self.scene_up();
                let current = &self.last_project["context"]["project"];
                let same_activation = current["project_id"] == previous["project_id"]
                    && current["activation"] == previous["activation"];
                if up != previous_up && same_activation {
                    self.camera.up = up.vector();
                    if !protocol::view::camera_is_valid(&self.camera.to_protocol()) {
                        self.camera = ViewerCamera::frame_up(
                            self.project_geometry
                                .as_ref()
                                .and_then(|g| g.bounds.as_ref()),
                            self.size[0] as f64 / self.size[1].max(1) as f64,
                            up,
                        );
                    }
                    self.dirty = true;
                    self.arm_ipr_submit();
                }
                // Handedness edits flip the viewport without moving it.
                let mirror = self.scene_mirror();
                if self.camera.mirror != mirror {
                    self.camera.mirror = mirror;
                    self.dirty = true;
                    self.arm_ipr_submit();
                }
                match serde_json::from_value::<protocol::project::ProjectState>(
                    self.last_project.clone(),
                ) {
                    Ok(state) => {
                        let render_camera_label = state.active_scene.and_then(|scene| {
                            request(
                                &self.core,
                                Method::ProjectGetNode,
                                json!({"project":self.last_project["context"]["project"],"node_id":scene}),
                            )
                            .ok()
                            .and_then(|root| root["components"].as_array().cloned())
                            .and_then(|components| {
                                components
                                    .into_iter()
                                    .find(|component| component["kind"] == "render_settings")
                            })
                            .and_then(|settings| {
                                let selected = settings["camera"].as_str()?;
                                let cameras =
                                    settings["cameras"].as_array().cloned().unwrap_or_default();
                                let camera = cameras.iter().find(|camera| camera["id"] == selected)?;
                                let label = camera["label"].as_str().unwrap_or("Camera");
                                if cameras
                                    .iter()
                                    .filter(|camera| camera["label"] == label)
                                    .count()
                                    > 1
                                {
                                    Some(format!("{label} ({})", &selected[..8.min(selected.len())]))
                                } else {
                                    Some(label.to_owned())
                                }
                            })
                        });
                        let _ = self.tx.send(Update::RenderCamera(render_camera_label));
                        let _ = self.tx.send(Update::Project(state));
                    }
                    Err(error) => {
                        let _ = self
                            .tx
                            .send(Update::Status(format!("Project state: {error}")));
                    }
                }
                self.reconcile_camera_preservation();
                // An owned snapshot keeps `&mut self` methods callable below
                // while this reads live.
                let reference = self.last_project["context"]["project"].clone();
                let activation_changed = previous["project_id"] != reference["project_id"]
                    || previous["activation"] != reference["activation"];
                let scene_changed = previous_scene != self.last_project["active_scene"];
                if activation_changed {
                    // Retire the previous activation's completed slot now.
                    // An in-flight publisher is left for PreviewCleanup,
                    // which removes both paths only after its terminal state.
                    let previous_cache = previous_folder
                        .is_none()
                        .then(project::cache_directory)
                        .transpose()
                        .ok()
                        .flatten();
                    if let Some(previous_slot) = preview_slot_path(
                        &previous,
                        previous_folder.as_deref(),
                        previous_cache.as_deref(),
                    ) {
                        let previous_path = previous_slot.to_string_lossy();
                        let is_busy = self
                            .project_preview_job
                            .as_ref()
                            .is_some_and(|(_, directory)| directory == previous_path.as_ref())
                            || self
                                .project_preview_cleanup
                                .iter()
                                .any(|entry| entry.directory == previous_path.as_ref());
                        if !is_busy {
                            let started = Instant::now();
                            preview_sweep::remove_slot_and_store(&previous_slot);
                            preview_timing::record(
                                "remove_slot",
                                started,
                                None,
                                &project_preview_key(&previous, &previous_scene),
                                None,
                            );
                        }
                    }
                    self.selected_project_node = None;
                    self.project_pick_nodes.clear();
                    self.project_tree_snapshot = Value::Null;
                    self.project_object_bounds.clear();
                    self.project_camera_owner = None;
                    // The renderer lives on the GPU worker; drops are requests.
                    let _ = self.gpu.requests.send(gpu::GpuRequest::Clear);
                    let _ = self.tx.send(Update::ProjectInspector(
                        json!({"message":"Select a project node"}),
                    ));
                }
                if reference.is_object() && self.last_project["active_scene"].is_null() {
                    self.foreground_preview_pending = false;
                    self.preview_ready = false;
                    for entry in &mut self.project_preview_cleanup {
                        entry.queued_request = None;
                    }
                    self.project_geometry = None;
                    self.project_tree_snapshot = json!({"nodes":[]});
                    self.project_preview_key = Some(project_preview_key(
                        &reference,
                        &self.last_project["active_scene"],
                    ));
                    if let Some((job_id, directory)) = self.project_preview_job.take() {
                        let _ = request(&self.core, Method::JobCancel, json!({"job_id":job_id}));
                        self.project_preview_cleanup.push(PreviewCleanup {
                            job_id,
                            directory,
                            queued_request: None,
                        });
                    }
                    let _ = self.gpu.requests.send(gpu::GpuRequest::Clear);
                    let _ = self.tx.send(Update::PreviewUnavailable(None));
                    let _ = self.tx.send(Update::ProjectTree(json!({"nodes":[]})));
                    if let Ok(scenes) = request(
                        &self.core,
                        Method::ProjectListScenes,
                        json!({"project":reference}),
                    ) {
                        let _ = self.tx.send(Update::ProjectScenes(scenes));
                    }
                    if let Ok(presets) = request(
                        &self.core,
                        Method::ProjectListPresets,
                        json!({"project":reference}),
                    ) {
                        let _ = self.tx.send(Update::ProjectPresets(presets));
                    }
                    // Contents and Resource inspectors are requested independently.
                }
                if reference.is_object() && !self.last_project["active_scene"].is_null() {
                    let key = project_preview_key(&reference, &self.last_project["active_scene"]);
                    // A key change within one
                    // activation goes to the transform fast path first; a
                    // fast answer retargets the installed draws without a
                    // preview job, and the full rebuild below stays the
                    // fallback for everything it declines.
                    let transform_fast_path = self.project_preview_key.as_deref() != Some(&key)
                        && self.try_transform_fast_path(
                            &key,
                            &reference,
                            activation_changed,
                            scene_changed,
                        );
                    if self.project_preview_key.as_deref() != Some(&key) && !transform_fast_path {
                        self.foreground_preview_pending |=
                            should_foreground_project_preview(activation_changed, scene_changed);
                        self.preview_ready = false;
                        self.project_geometry = None;
                        // One stable slot per project activation:
                        // repeated edits publish over it instead of paying a
                        // fresh directory every revision. The activation change
                        // moves to a new slot and cleanup retires the old one.
                        // A reference without its id or activation keeps the
                        // old default name instead of stopping the display.
                        let project_folder =
                            self.last_project["path"].as_str().map(std::path::Path::new);
                        let cache_root = project_folder
                            .is_none()
                            .then(project::cache_directory)
                            .transpose()
                            .ok()
                            .flatten();
                        let directory =
                            preview_slot_path(&reference, project_folder, cache_root.as_deref());
                        let Some(directory) = directory else {
                            let _ = self.tx.send(Update::Status(
                                "Project preview needs an available cache directory".into(),
                            ));
                            return;
                        };
                        if !preview_sweep::initialize_base(
                            directory.parent().unwrap(),
                            project_folder.is_none(),
                        ) {
                            let _ = self.tx.send(Update::Status(
                                "Project preview directory is unavailable".into(),
                            ));
                            return;
                        }
                        let destination = directory.to_string_lossy().into_owned();
                        let content_store = preview_sweep::store_path_for_slot(&directory)
                            .to_string_lossy()
                            .into_owned();
                        // The slot may only be cleared and republished once
                        // every in-flight job on it is terminal: a cancelled
                        // publisher past its final checkpoint would otherwise
                        // keep the replacement's exclusive publish from
                        // landing. Park the request with the running job, or
                        // with the cleanup entry already waiting on this slot.
                        let parked = match self
                            .project_preview_cleanup
                            .iter_mut()
                            .find(|entry| entry.directory == destination)
                        {
                            Some(entry) => {
                                // Supersede whatever request was parked here.
                                entry.queued_request = Some((reference.clone(), destination));
                                true
                            }
                            None if self.project_preview_job.is_some() => {
                                let (job_id, job_directory) =
                                    self.project_preview_job.take().unwrap();
                                let _ = request(
                                    &self.core,
                                    Method::JobCancel,
                                    json!({"job_id":job_id}),
                                );
                                self.project_preview_cleanup.push(PreviewCleanup {
                                    job_id,
                                    directory: job_directory,
                                    queued_request: Some((reference.clone(), destination)),
                                });
                                true
                            }
                            None => false,
                        };
                        if parked {
                            // The retry queue owns the next request; nothing
                            // claims the key until the slot is free.
                            self.project_preview_key = None;
                            let _ = self
                                .tx
                                .send(Update::Status("Preparing native project preview…".into()));
                        } else {
                            // Exports publish by renaming onto this slot; clear
                            // any previous revision's bundle first.
                            let remove_started = Instant::now();
                            let _ = std::fs::remove_dir_all(&directory);
                            preview_timing::record("remove_slot", remove_started, None, &key, None);
                            if let Some(destination) = directory.to_str() {
                                match request(
                                    &self.core,
                                    Method::ProjectPreview,
                                    json!({"project":reference,"destination":destination,"store":content_store,"stage_assets":false}),
                                ) {
                                    Ok(started) => {
                                        if let Some(job_id) = started["job_id"].as_str() {
                                            self.project_preview_job = Some((
                                                job_id.to_owned(),
                                                directory.to_string_lossy().into_owned(),
                                            ));
                                            self.project_preview_key = Some(key);
                                            if self.foreground_preview_pending {
                                                self.foreground_jobs.push((
                                                    job_id.to_owned(),
                                                    "Preparing preview".into(),
                                                    0,
                                                ));
                                                self.foreground_preview_pending = false;
                                            }
                                            let _ = self.tx.send(Update::Status(
                                                "Preparing native project preview…".into(),
                                            ));
                                        }
                                    }
                                    Err(error) => {
                                        self.foreground_preview_pending = false;
                                        self.fail_current_camera_preview();
                                        let _ = self.tx.send(Update::PreviewUnavailable(Some(
                                            format!("Project preview export: {error}"),
                                        )));
                                        self.project_preview_key = Some(key);
                                        let _ = self.tx.send(Update::Status(format!(
                                            "Project preview export: {error}"
                                        )));
                                    }
                                }
                            }
                        }
                    }
                    if reference != previous {
                        // Lazy hierarchy: the Scene root loads with Camera
                        // and World content; the World scope is hidden, while
                        // deeper pages arrive on expansion.
                        match project_roots(&self.core, &reference) {
                            Ok(records) => {
                                let mut nodes = records
                                    .iter()
                                    .map(crate::project_preview::tree_node)
                                    .collect::<Vec<_>>();
                                let mut tree = json!({"nodes":[]});
                                if let Some(scene) = nodes
                                    .first()
                                    .and_then(|n| n["id"].as_str())
                                    .map(str::to_owned)
                                {
                                    tree["nodes"] = json!(nodes.drain(..).collect::<Vec<_>>());
                                    if let Ok(page) = project_nodes_page(
                                        &self.core,
                                        &reference,
                                        &Some(scene.clone()),
                                        0,
                                        1000,
                                    ) {
                                        crate::project_preview::merge_project_children(
                                            &mut tree,
                                            &scene,
                                            0,
                                            page["nodes"]
                                                .as_array()
                                                .map(Vec::as_slice)
                                                .unwrap_or(&[]),
                                            page["next_offset"].as_u64(),
                                        );
                                    }
                                }
                                if let Ok(definitions) =
                                    project_definition_roots(&self.core, &reference)
                                {
                                    tree["nodes"].as_array_mut().map(|roots| {
                                        roots.push(json!({
                                            "id":"@object-definitions",
                                            "label":"Object Definitions",
                                            "kind":"definition_section",
                                            "object":"definition_section",
                                            "child_count":definitions.len(),
                                            "loaded":true,
                                            "children":definitions,
                                            "more_offset":null,
                                        }))
                                    });
                                }
                                let mut tree = tree;
                                if let Some(geometry) = self.project_geometry.as_ref()
                                    && let Some(depth) =
                                        crate::backend::project_preview::camera_medium_optical_depth(
                                            &tree, geometry,
                                        )
                                {
                                    tree["camera_medium_optical_depth"] = json!(depth);
                                }
                                self.project_tree_snapshot = tree.clone();
                                let _ = self.tx.send(Update::ProjectTree(tree));
                            }
                            Err(error) => {
                                let _ = self
                                    .tx
                                    .send(Update::Status(format!("Project hierarchy: {error}")));
                            }
                        }
                        match request(
                            &self.core,
                            Method::ProjectListScenes,
                            json!({"project":reference}),
                        ) {
                            Ok(scenes) => {
                                let _ = self.tx.send(Update::ProjectScenes(scenes));
                            }
                            Err(error) => {
                                let _ = self
                                    .tx
                                    .send(Update::Status(format!("Project scenes: {error}")));
                            }
                        }
                        if activation_changed {
                            if let Ok(presets) = request(
                                &self.core,
                                Method::ProjectListPresets,
                                json!({"project":reference}),
                            ) {
                                let _ = self.tx.send(Update::ProjectPresets(presets));
                            }
                        }
                    }
                } else if !reference.is_object() {
                    self.foreground_preview_pending = false;
                    if let Some((job_id, directory)) = self.project_preview_job.take() {
                        let _ = request(&self.core, Method::JobCancel, json!({"job_id":job_id}));
                        self.project_preview_cleanup.push(PreviewCleanup {
                            job_id,
                            directory,
                            queued_request: None,
                        });
                    }
                    self.project_preview_key = None;
                    self.project_camera_owner = None;
                    self.project_time = 0.;
                    self.project_geometry = None;
                    self.preview_ready = false;
                    let _ = self.gpu.requests.send(gpu::GpuRequest::Clear);
                    self.selected_project_node = None;
                    let _ = self.tx.send(Update::ProjectTree(Value::Null));
                    let _ = self.tx.send(Update::ProjectInspector(
                        json!({"message":"Select a project node"}),
                    ));
                }
                if let (Some(node_id), true) = (
                    self.selected_project_node.as_deref(),
                    reference.is_object() && reference != previous,
                ) {
                    let reference = reference.clone();
                    let node_id = node_id.to_owned();
                    match self.node_details(&reference, &node_id) {
                        Ok(details) => {
                            let _ = self.tx.send(Update::ProjectInspector(details));
                        }
                        Err(error) => {
                            // Confirm absence at the guarded revision; a stale query
                            // must never clear a still valid selection.
                            if matches!(
                                request_error(&self.core, Method::ProjectGetNode,
                                    json!({"project":reference,"node_id":node_id})),
                                Err(ref error) if node_selection_is_gone(error)
                            ) {
                                self.selected_project_node = None;
                                let _ = self.tx.send(Update::ProjectInspector(
                                    json!({"message":"Select a project node"}),
                                ));
                            }
                            // Other failures may be transient; retain the selection.
                            let _ = self
                                .tx
                                .send(Update::Status(format!("Project inspector: {error}")));
                        }
                    }
                }
            }
            if state != self.last_state {
                if !self.last_state.is_null()
                    && state["layout_revision"] != self.last_state["layout_revision"]
                {
                    let _ = self.tx.send(Update::LayoutReset);
                }

                let status = if state["project"]["context"]["project"].is_object() {
                    let mode = if state["project"]["read_only"] == true {
                        "Viewer"
                    } else {
                        "Project"
                    };
                    format!(
                        "{mode}: {} · {} directive nodes · {} resources{} · native preview {}",
                        state["project"]["title"].as_str().unwrap_or("Untitled"),
                        state["project"]["node_count"].as_u64().unwrap_or(0),
                        state["project"]["resource_count"].as_u64().unwrap_or(0),
                        if state["project"]["dirty"] == true {
                            " · unsaved changes"
                        } else {
                            ""
                        },
                        if self.preview_ready {
                            "ready"
                        } else if self.project_preview_job.is_some() {
                            "updating"
                        } else {
                            "unavailable"
                        }
                    )
                } else {
                    "Open a project or a PBRT scene to begin".into()
                };
                let _ = self.tx.send(Update::Status(status));
                if state["ipr"] != self.last_state["ipr"] {
                    let _ = self.tx.send(Update::Ipr(state["ipr"].clone()));
                }
                if state["quit_requested"] == true && self.last_state["quit_requested"] != true {
                    let _ = self.tx.send(Update::Quit);
                }
                self.last_state = state;
            }
        }
    }
}

#[cfg(test)]
mod selection_tests {
    use super::node_selection_is_gone;
    use protocol::{ErrorCode, ErrorPayload};

    #[test]
    fn only_a_confirmed_missing_node_clears_inspector_selection() {
        assert!(node_selection_is_gone(&ErrorPayload::new(
            ErrorCode::NotFound,
            "node not found",
        )));
        assert!(!node_selection_is_gone(&ErrorPayload::new(
            ErrorCode::Busy,
            "temporary failure",
        )));
    }
}
