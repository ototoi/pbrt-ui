//! Display adapter phase; state stays owned by Display.
use super::*;
impl Display {
    pub(super) fn poll_project_preview(&mut self) {
        if let Some((job_id, _directory)) = self.project_preview_job.clone()
            && let Ok(job) = request(&self.core, Method::JobGet, json!({"job_id":job_id}))
        {
            match job["state"].as_str().unwrap_or("") {
                "succeeded" => {
                    // The heavy package work runs on the GPU worker
                    // This loop only dispatches the install
                    // and applies its result later, in poll_gpu.
                    let preview_key = self.project_preview_key.clone().unwrap_or_default();
                    let geometry_path = job["result"]["geometry_path"].as_str().map(PathBuf::from);
                    let slot = geometry_path
                        .as_ref()
                        .and_then(|path| path.parent())
                        .map(|slot| slot.to_path_buf());
                    let failure = if geometry_path.is_none() {
                        Some("preview job omitted its geometry path")
                    } else if slot.is_none() {
                        Some("geometry.json has no slot")
                    } else {
                        None
                    };
                    if let Some(message) = failure {
                        let text = format!("Project preview parse: {message}");
                        self.fail_current_camera_preview();
                        let _ = self.tx.send(Update::PreviewUnavailable(Some(text.clone())));
                        let _ = self.tx.send(Update::Status(text));
                    } else {
                        self.enqueue_install(preview_key, job_id, slot.unwrap(), None);
                    }
                    self.project_preview_job = None;
                }
                "failed" | "cancelled" => {
                    let cancelled =
                        job["state"] == "cancelled" || job["error"]["code"] == "cancelled";
                    let message = if cancelled {
                        "Preview cancelled".to_owned()
                    } else {
                        format!(
                            "Preview failed: {}",
                            job["error"]["message"].as_str().unwrap_or("unknown error")
                        )
                    };
                    let _ = self
                        .tx
                        .send(Update::PreviewUnavailable(Some(message.clone())));
                    let _ = self.tx.send(Update::Status(message));
                    self.fail_current_camera_preview();
                    self.project_preview_job = None;
                }
                _ => {}
            }
        }
    }

    /// Queue the package install on the GPU worker. One install runs at a
    /// time; a newer package replaces anything still waiting (latest wins).
    /// `manifest` overrides the slot's `geometry.json` — the
    /// transform fast path's retargeted package; `None` reads the slot.
    pub(super) fn enqueue_install(
        &mut self,
        key: String,
        job_id: String,
        slot: PathBuf,
        manifest: Option<Box<protocol::geometry::GeometryManifest>>,
    ) {
        if self.install_in_flight {
            self.queued_install = Some((key, job_id, slot, manifest));
        } else {
            let _ = self.gpu.requests.send(gpu::GpuRequest::Install {
                key,
                job_id,
                slot,
                manifest,
            });
            self.install_in_flight = true;
        }
    }

    /// Consume the queued install after a terminal result. Entries whose
    /// scene moved on while they waited are discarded, not sent.
    pub(super) fn flush_queued_install(&mut self) {
        if let Some((key, job_id, slot, manifest)) = self.queued_install.take()
            && Some(&key) == self.project_preview_key.as_ref()
        {
            let _ = self.gpu.requests.send(gpu::GpuRequest::Install {
                key,
                job_id,
                slot,
                manifest,
            });
            self.install_in_flight = true;
        }
    }

    /// Apply a finished install: everything the old inline success branch
    /// set, minus the heavy work the GPU worker paid for.
    /// The active scene's up axis from the last project state;
    /// +Y when none is reported.
    pub(super) fn scene_up(&self) -> protocol::project::UpAxis {
        serde_json::from_value(self.last_project["active_up"].clone())
            .unwrap_or(protocol::project::UpAxis::PosY)
    }

    /// Whether the active scene is right-handed, so the viewport mirrors
    /// like the scene's cameras do in the render.
    pub(super) fn scene_mirror(&self) -> bool {
        self.last_project["active_handedness"] == "right"
    }

    pub(super) fn apply_installed(&mut self, data: gpu::InstalledData) {
        let gpu::InstalledData {
            key,
            manifest,
            bindings,
            bounds,
            lit_notes,
            diagnostics,
            slot,
            keeps_transform_base,
        } = data;
        self.log_lit_notes(&lit_notes);
        self.log_preview_warnings(&diagnostics);
        self.install_pick_nodes(bindings);
        self.project_object_bounds = bounds;
        let owner_ref = &self.last_project["context"]["project"];
        let owner = camera_preservation::ProjectOwner::from_ref(owner_ref);
        let owner_key = owner.as_ref().map(|owner| owner.key()).unwrap_or_default();
        let manifest_camera = ViewerCamera::from_manifest_up(
            &manifest,
            self.size[0] as f64 / self.size[1].max(1) as f64,
            self.scene_up(),
        );
        self.scene_camera = Some(manifest_camera);
        if self.project_camera_owner.as_deref() != Some(&owner_key) {
            // Reload Source keeps the camera the user was looking through
            // across the new activation; a freshly opened document frames
            // its manifest camera.
            if let Some(owner) = owner.as_ref() {
                self.choose_camera_for_preview(owner, manifest_camera);
            } else {
                self.camera = manifest_camera;
            }
            self.camera.mirror = self.scene_mirror();
            self.project_camera_owner = Some(owner_key);
            // The shared view pins to the exact activation, so every freshly
            // displayed project must schedule a
            // sync or external captures keep the manifest framing.
            self.view_due = Some(Instant::now() + Duration::from_millis(300));
        }
        if !manifest.supported_times.contains(&self.project_time) {
            self.project_time = manifest.supported_times.first().copied().unwrap_or(0.);
            self.view_due = Some(Instant::now() + Duration::from_millis(300));
        }
        let times = manifest.supported_times.clone();
        let draws = manifest.draws.len();
        // One number for the status line: the manifest's unsupported draws
        // plus the package's warnings. Info
        // records are shown in the Console but never
        // count — the project keeps what they name.
        let unsupported = manifest.unsupported.len()
            + diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.is_warning())
                .count();
        let base_draws = manifest.draws.clone();
        let base_lights = manifest.lights.clone();
        let optical_depth = camera_medium_optical_depth(&self.project_tree_snapshot, &manifest);
        if let Some(tree) = self.project_tree_snapshot.as_object_mut() {
            if let Some(value) = optical_depth {
                tree.insert("camera_medium_optical_depth".into(), json!(value));
            } else {
                tree.remove("camera_medium_optical_depth");
            }
            let _ = self
                .tx
                .send(Update::ProjectTree(self.project_tree_snapshot.clone()));
        }
        self.project_geometry = Some(manifest);
        // The transform fast path compares
        // against this install — the untouched draws AND lights, the
        // package slot a light move re-installs from, and the revision the
        // preview key was built from; every job's package replaces the
        // record wholesale. An OVERRIDE install (the fast path's own
        // light move, keeps_transform_base) does not replace it: that
        // manifest is the pre-move base retargeted, and the base must
        // stay exact so the next move still applies to it.
        if !keeps_transform_base {
            self.project_preview_base =
                preview_key_revision(&key).map(|revision| crate::project_preview::TransformBase {
                    revision,
                    draws: base_draws,
                    lights: base_lights,
                    slot,
                });
        }
        // The edited scene replaces a committed gizmo drag.
        self.gizmo_delta = None;
        self.preview_ready = true;
        self.dirty = true;
        let _ = self.tx.send(Update::ProjectPreview {
            key,
            times,
            time: self.project_time,
        });
        let _ = self.tx.send(Update::Status(format!(
            "Project preview: {draws} draws · {unsupported} unsupported"
        )));
    }

    /// Send the uploaded scene's Lit notes to the Console, skipping those
    /// already sent for the previous upload — and repeats within one
    /// upload, which two shapes approximated the same way produce.
    pub(super) fn log_lit_notes(&mut self, notes: &[scene_view::preview::LitNote]) {
        let mut lines = Vec::new();
        for note in notes {
            let material = note
                .material
                .as_ref()
                .map(|m| format!(" ({m})"))
                .unwrap_or_default();
            let draws = if note.draws > 0 {
                format!(" · {} draws", note.draws)
            } else {
                String::new()
            };
            let line = format!("Lit approximation: {}{material}{draws}", note.reason);
            if !lines.contains(&line) {
                lines.push(line);
            }
        }
        if tree_uses_medium(&self.project_tree_snapshot) {
            lines.push("Lit approximation: participating media are not shown in Lit".into());
        }
        for line in &lines {
            if !self.logged_lit_notes.contains(line) {
                let _ = self.tx.send(Update::Log(line.clone()));
            }
        }
        self.logged_lit_notes = lines;
    }

    /// Send the installed package's warnings (e.g. a mesh dropped for
    /// exceeding the preview asset budget) to the Console, skipping lines
    /// already sent for the previous package: a preview runs after every
    /// edit, and the same warning must not repeat. Repeats within one
    /// package (two shapes referencing the same broken asset) print once.
    /// Informational notes about media the preview cannot draw print the same way,
    /// without counting as unsupported.
    pub(super) fn log_preview_warnings(&mut self, diagnostics: &[scene_parse::Diagnostic]) {
        let mut lines = Vec::new();
        for diagnostic in diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.is_warning() || diagnostic.is_info())
        {
            let line = format!("Preview: {}: {}", diagnostic.code, diagnostic.message);
            if !lines.contains(&line) {
                lines.push(line);
            }
        }
        for line in &lines {
            if !self.logged_preview_warnings.contains(line) {
                let _ = self.tx.send(Update::Log(line.clone()));
            }
        }
        self.logged_preview_warnings = lines;
    }

    /// A preview key change within the
    /// same project, activation and scene asks the core for transform-only
    /// deltas first (`project.preview_transforms`, against the base
    /// revision of the installed preview). A fast answer retargets the
    /// installed geometry without a preview job. Deltas naming shapes
    /// take the gizmo's `UpdateDraws` path, keyed to the installed
    /// scene and adopting the new key — carrying the BASE draws with the
    /// deltas applied; the displayed manifest follows with recomputed
    /// placement bounds. A delta naming a base LIGHT moves
    /// something `UpdateDraws` cannot express — Lit assembles its
    /// lights from the manifest — so the answer instead re-installs the
    /// base package's slot with the moved draws and lights as a manifest
    /// override: `update_scene` rebuilds the Lit lights while the GPU
    /// keeps its meshes and lit textures. Either way a committed gizmo
    /// delta clears in the same update, so a released drag does not jump
    /// back for a frame, and the window learns the new key with no badge
    /// and no job. Everything else — the gates themselves, `fast:false`
    /// (whatever its reason), a malformed answer, a request error, a
    /// delta the answered samples cannot retarget or that names nothing
    /// installed — answers `false`, and the caller rebuilds the preview
    /// in full, unchanged.
    pub(super) fn try_transform_fast_path(
        &mut self,
        key: &str,
        reference: &Value,
        activation_changed: bool,
        scene_changed: bool,
    ) -> bool {
        // Same project, activation and scene; the geometry displayed for
        // this activation is installed and ready; nothing about the preview
        // pipeline is in flight — a job, a cleanup, an install or its
        // queued successor would supersede the draws the deltas retarget.
        if activation_changed
            || scene_changed
            || !self.preview_ready
            || self.project_geometry.is_none()
            || self.project_preview_job.is_some()
            || !self.project_preview_cleanup.is_empty()
            || self.install_in_flight
            || self.queued_install.is_some()
        {
            return false;
        }
        let Some(installed) = self.project_preview_key.clone() else {
            return false;
        };
        let Some(base) = self.project_preview_base.clone() else {
            return false;
        };
        let started = Instant::now();
        // The base revision the installed draws were keyed to. Any
        // answer but a fast one — `fast:false` with whatever reason, a
        // request error, a malformed answer — is simply "rebuild".
        let answer = match request(
            &self.core,
            Method::ProjectPreviewTransforms,
            json!({"project": reference, "base_revision": base.revision}),
        ) {
            Ok(answer) => answer,
            Err(_) => return false,
        };
        if answer["fast"] != true {
            return false;
        }
        let Ok(deltas) = serde_json::from_value::<
            BTreeMap<String, crate::project_preview::TransformDelta>,
        >(answer["deltas"].clone()) else {
            return false;
        };
        // A delta naming a base light moves something
        // `UpdateDraws` cannot express — Lit assembles its lights from the
        // manifest, not from the draws — so the answer re-installs the
        // base package with the moved lights as a
        // manifest override, through the same install pipeline a job's
        // package uses. The request is marked as an override so
        // `apply_installed` keeps the base it was computed from:
        // the on-disk package stays the base revision's, so the base —
        // not the last move — stays what the next move applies to).
        // A light an earlier override moved also re-installs when the answer
        // no longer names it, or it stays stale.
        let names_a_base_light = base.lights.iter().any(|light| {
            light
                .entity_id
                .as_ref()
                .is_some_and(|entity| deltas.contains_key(entity.as_str()))
        }) || self
            .project_geometry
            .as_ref()
            .is_some_and(|shown| shown.lights != base.lights);
        if names_a_base_light {
            let Some((draws, lights)) = crate::project_preview::retarget_transform_base(
                &base,
                &deltas,
                &self.project_object_bounds,
            ) else {
                // Mid-time samples the answer cannot express, or a delta
                // naming nothing installed: only a rebuild can tell.
                return false;
            };
            // The manifest to show: the displayed geometry clone (meshes,
            // materials, textures, camera — a move changes placements only)
            // with the BASE's retargeted draws and lights.
            let mut manifest = self
                .project_geometry
                .clone()
                .expect("the gates hold an installed geometry");
            manifest.draws = draws;
            manifest.lights = lights;
            if let Some(bounds) = crate::project_preview::draws_world_bounds(&manifest.draws) {
                manifest.bounds = Some(bounds);
            }
            // The key advances before the window learns it, the install
            // carries it so the worker's result matches on arrival, and
            // the install carries the manifest so the slot's
            // geometry.json is not read.
            self.gizmo_delta = None;
            self.project_preview_key = Some(key.to_owned());
            self.enqueue_install(
                key.to_owned(),
                "transform-fast-path".to_owned(),
                base.slot.clone(),
                Some(Box::new(manifest.clone())),
            );
            self.project_geometry = Some(manifest);
            self.dirty = true;
            // The window learns the scene's key without any job to wait
            // for — the badge never appears and the next frame clears
            // `updating`.
            let _ = self.tx.send(Update::ProjectPreview {
                key: key.to_owned(),
                times: self
                    .project_geometry
                    .as_ref()
                    .map(|geometry| geometry.supported_times.clone())
                    .unwrap_or_default(),
                time: self.project_time,
            });
            preview_timing::record("transform_fast_path", started, None, key, None);
            return true;
        }
        let Some(draws) = crate::project_preview::apply_transform_deltas(
            &base.draws,
            &deltas,
            &self.project_object_bounds,
        ) else {
            // Mid-time samples the answer cannot express retarget nothing.
            return false;
        };
        // The gizmo's path: the request is keyed to the
        // scene the draws were computed against and the worker
        // adopts the key the scene now represents.
        if self
            .gpu
            .requests
            .send(gpu::GpuRequest::UpdateDraws {
                key: installed,
                draws: draws.clone(),
                adopt: Some(key.to_owned()),
            })
            .is_err()
        {
            return false;
        }
        // The manifest the UI uses — picking by id, FrameSelection's
        // selection bounds, FrameAll — follows the move; the meshes, pick
        // ids and entities are the base's.
        if let Some(geometry) = self.project_geometry.as_mut() {
            geometry.draws = draws;
            if let Some(bounds) = crate::project_preview::draws_world_bounds(&geometry.draws) {
                geometry.bounds = Some(bounds);
            }
        }
        // A committed gizmo delta clears in the same update as the new
        // draws, so a released drag does not jump back for a frame.
        self.gizmo_delta = None;
        self.project_preview_key = Some(key.to_owned());
        self.dirty = true;
        // The window learns the scene's key without any job to wait for —
        // the badge never appears and the next frame clears `updating`.
        let _ = self.tx.send(Update::ProjectPreview {
            key: key.to_owned(),
            times: self
                .project_geometry
                .as_ref()
                .map(|geometry| geometry.supported_times.clone())
                .unwrap_or_default(),
            time: self.project_time,
        });
        preview_timing::record("transform_fast_path", started, None, key, None);
        true
    }
}

/// Estimate homogeneous camera-medium optical depth to the scene bounds center.
pub(super) fn camera_medium_optical_depth(
    tree: &Value,
    manifest: &protocol::geometry::GeometryManifest,
) -> Option<f64> {
    fn camera(nodes: &[Value]) -> Option<f64> {
        for node in nodes {
            if node["object"] == "camera" && node["render_camera"] == true {
                return node["medium_sigma_t"].as_f64();
            }
            if let Some(value) = node["children"]
                .as_array()
                .and_then(|children| camera(children))
            {
                return Some(value);
            }
        }
        None
    }
    let sigma_t = camera(tree["nodes"].as_array()?)?;
    let bounds = manifest.bounds.as_ref()?;
    let scene_center = [
        (bounds.min[0] + bounds.max[0]) * 0.5,
        (bounds.min[1] + bounds.max[1]) * 0.5,
        (bounds.min[2] + bounds.max[2]) * 0.5,
    ];
    let camera = manifest.camera.as_ref()?;
    let pose = camera.camera_from_world;
    let camera_position = [pose[3], pose[7], pose[11]];
    let distance = (0..3)
        .map(|index| (scene_center[index] - camera_position[index]).powi(2))
        .sum::<f64>()
        .sqrt();
    let estimate = sigma_t * distance;
    estimate.is_finite().then_some(estimate)
}

fn tree_uses_medium(tree: &Value) -> bool {
    if tree["nodes"]
        .as_array()
        .is_some_and(|nodes| nodes.iter().any(|node| node["medium_used"] == true))
    {
        return true;
    }
    fn visit(nodes: &[Value]) -> bool {
        nodes.iter().any(|node| {
            if node["kind"] == "definition_section" {
                return false;
            }
            (matches!(node["object"].as_str(), Some("camera" | "shape"))
                && node["medium_bound"] == true)
                || node["children"]
                    .as_array()
                    .is_some_and(|children| visit(children))
        })
    }
    tree["nodes"].as_array().is_some_and(|nodes| visit(nodes))
}

#[cfg(test)]
mod medium_hint_tests {
    use super::{camera_medium_optical_depth, tree_uses_medium};
    use protocol::geometry::{Bounds, CameraRecord, GeometryManifest, TessellationSettings};
    use serde_json::json;

    #[test]
    fn medium_usage_only_counts_camera_and_shape_bindings() {
        assert!(tree_uses_medium(
            &json!({"nodes":[{"object":"camera","medium_bound":true}]})
        ));
        assert!(tree_uses_medium(
            &json!({"nodes":[{"object":"group","children":[{"object":"shape","medium_bound":true}]}]})
        ));
        assert!(!tree_uses_medium(
            &json!({"nodes":[{"object":"group","medium_bound":true}]})
        ));
        assert!(!tree_uses_medium(
            &json!({"nodes":[{"object":"shape","medium_bound":false}]})
        ));
    }

    #[test]
    fn an_uninstanced_definition_does_not_change_scene_medium_status() {
        let tree = json!({"nodes":[
            {"id":"scene","object":"scene","medium_used":false,"children":[]},
            {"id":"@object-definitions","kind":"definition_section","children":[
                {"id":"shape","object":"shape","medium_bound":true,"children":[]}
            ]}
        ]});
        assert!(!tree_uses_medium(&tree));
    }

    #[test]
    fn camera_optical_depth_uses_sigma_t_and_distance_to_bounds_center() {
        let tree = json!({"nodes":[{"object":"camera","render_camera":true,"medium_sigma_t":2.0}]});
        let manifest = GeometryManifest {
            version: 1,
            tessellation: TessellationSettings::default(),
            supported_times: vec![0.],
            bounds: Some(Bounds {
                min: [0.; 3],
                max: [2.; 3],
            }),
            meshes: vec![],
            draws: vec![],
            unsupported: vec![],
            camera: Some(CameraRecord {
                kind: "perspective".into(),
                camera_from_world: [
                    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
                ],
                look_at: None,
                fov_degrees: None,
                screenwindow: None,
                frame_aspect_ratio: None,
                shutter: [0., 1.],
                film_resolution: None,
                film_exposure: None,
            }),
            blobs: Default::default(),
            materials: vec![],
            lights: vec![],
            textures: vec![],
        };
        assert!(
            (camera_medium_optical_depth(&tree, &manifest).unwrap() - 2. * 3_f64.sqrt()).abs()
                < 1e-9
        );
    }
}
