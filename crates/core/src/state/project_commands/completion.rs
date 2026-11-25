//! Project I/O completion and pinned render handoff.
use super::*;
impl CoreState {
    pub(crate) fn project_io_finished(
        &mut self,
        id: JobId,
        result: Result<IoOutput, ErrorPayload>,
    ) {
        let Some(pending) = self.projects.pending.remove(&id) else {
            return;
        };
        if self.projects.switch == Some(id) {
            self.projects.switch = None;
        }
        let additional_import = matches!(&pending, Pending::AddImport { .. });
        let cancelled = self.jobs[self.job_indices[&id]].task.is_cancelled();
        let pending = match pending {
            Pending::Capture {
                config,
                reference,
                view,
                view_revision,
                pick,
                debug_albedo,
                debug_ao,
                camera,
            } => {
                let prepared = match result {
                    // Capture consumes the direct-preview package itself; no
                    // scene text or export manifest exists on this path.
                    Ok(IoOutput::Previewed { path, .. }) if !cancelled => Ok(path),
                    Ok(_) if cancelled => Err(ErrorPayload::new(
                        ErrorCode::Cancelled,
                        "project capture cancelled",
                    )),
                    Ok(_) => Err(ErrorPayload::new(
                        ErrorCode::OutputInvalid,
                        "project capture preview returned an invalid result",
                    )),
                    Err(error) => Err(error),
                };
                match prepared {
                    Ok(root) => {
                        let record = &mut self.jobs[self.job_indices[&id]];
                        record.job.phase = "capturing".into();
                        self.service_tasks.push_back(crate::services::ServiceTask {
                            task: record.task.clone(),
                            work: crate::services::Work::ProjectCapture {
                                root,
                                project: reference,
                                view,
                                view_revision,
                                pick,
                                debug_albedo,
                                debug_ao,
                                camera,
                            },
                            config,
                        });
                        self.emit_job(id);
                    }
                    Err(error) => self.service_finished(
                        id,
                        crate::services::WorkResult {
                            result: Err(error),
                            artifacts: vec![],
                        },
                    ),
                }
                return;
            }
            Pending::Render {
                config,
                reference,
                settings,
                view,
            } => {
                let prepared = match result {
                    Ok(IoOutput::Exported { path, .. }) if !cancelled => Ok(path),
                    Ok(_) if cancelled => Err(ErrorPayload::new(
                        ErrorCode::Cancelled,
                        "project render cancelled",
                    )),
                    Ok(_) => Err(ErrorPayload::new(
                        ErrorCode::OutputInvalid,
                        "project render export returned an invalid result",
                    )),
                    Err(error) => Err(error),
                };
                match prepared {
                    Ok(root) => {
                        let record = &mut self.jobs[self.job_indices[&id]];
                        record.job.phase = "rendering".into();
                        self.service_tasks.push_back(crate::services::ServiceTask {
                            task: record.task.clone(),
                            work: crate::services::Work::ProjectRender {
                                root,
                                project: reference,
                                settings,
                                view,
                                ipr: false,
                            },
                            config,
                        });
                        self.emit_job(id);
                    }
                    Err(error) => self.service_finished(
                        id,
                        crate::services::WorkResult {
                            result: Err(error),
                            artifacts: vec![],
                        },
                    ),
                }
                return;
            }
            Pending::Ipr {
                config,
                reference,
                settings,
                view,
            } => {
                let prepared = match result {
                    Ok(IoOutput::Exported { path, .. }) if !cancelled => Ok(path),
                    Ok(_) if cancelled => Err(ErrorPayload::new(
                        ErrorCode::Cancelled,
                        "ipr export cancelled",
                    )),
                    Ok(_) => Err(ErrorPayload::new(
                        ErrorCode::OutputInvalid,
                        "ipr export returned an invalid result",
                    )),
                    Err(error) => Err(error),
                };
                match prepared {
                    Ok(root) => {
                        let record = &mut self.jobs[self.job_indices[&id]];
                        record.job.phase = "rendering".into();
                        self.service_tasks.push_back(crate::services::ServiceTask {
                            task: record.task.clone(),
                            work: crate::services::Work::ProjectRender {
                                root,
                                project: reference,
                                settings,
                                view,
                                ipr: true,
                            },
                            config,
                        });
                        self.emit_job(id);
                    }
                    Err(error) => self.service_finished(
                        id,
                        crate::services::WorkResult {
                            result: Err(error),
                            artifacts: vec![],
                        },
                    ),
                }
                return;
            }
            pending => pending,
        };
        let result = (|| -> Result<Value, ErrorPayload> {
            match (pending, result?) {
                (
                    Pending::ArrayEdit {
                        reference,
                        store_id,
                        scene,
                        plan,
                    },
                    IoOutput::ArrayPrepared { lease },
                ) => {
                    if cancelled {
                        return Err(ErrorPayload::new(
                            ErrorCode::Cancelled,
                            "array edit cancelled before publication",
                        ));
                    }
                    // (windows, new location) was made against the
                    // pinned revision: any content change since is stale.
                    let current = self.projects.reference().ok_or_else(|| {
                        ErrorPayload::new(
                            ErrorCode::StaleProject,
                            "project closed before publication",
                        )
                    })?;
                    if current.project_id != reference.project_id
                        || self.projects.active.as_ref().unwrap().store_id != store_id
                    {
                        return Err(ErrorPayload::new(
                            ErrorCode::StaleProject,
                            "project identity changed",
                        )
                        .with_details(json!({"current":current})));
                    }
                    if current.revision != reference.revision {
                        return Err(ErrorPayload::new(
                            ErrorCode::RevisionConflict,
                            "project revision changed",
                        )
                        .with_details(json!({"current":current})));
                    }
                    let target = match &plan {
                        super::edits::ArrayEditPlan::Inline { target, .. }
                        | super::edits::ArrayEditPlan::DataAsset { target, .. } => target,
                    };
                    super::edits::scene_still_holds(
                        target,
                        scene,
                        self.projects
                            .active
                            .as_ref()
                            .unwrap()
                            .session
                            .active_scene(),
                    )?;
                    let edits = match plan {
                        super::edits::ArrayEditPlan::Inline { edits, .. } => edits,
                        plan => super::edits::data_asset_edits(plan, lease)?,
                    };
                    let active = self.projects.active.as_mut().unwrap();
                    let deletes = edits.iter().any(|edit| {
                        matches!(
                            edit,
                            project::Edit::DeleteItem {
                                item: project::ItemId::Asset(_)
                            }
                        )
                    });
                    if deletes && let Some(root) = active.asset_root.clone() {
                        // The old copy's bytes stay restorable by Undo.
                        active
                            .session
                            .ensure_temporary_payload_staging(std::path::Path::new(&root))
                            .map_err(crate::project_io::project_error)?;
                    }
                    let change = active
                        .session
                        .apply(&reference.revision, &edits)
                        .map_err(crate::project_io::project_error)?;
                    let document = active.session.document();
                    let created = change
                        .created_items
                        .iter()
                        .filter_map(|item| match item {
                            project::ItemId::Asset(id) => Some(
                                json!({"kind":"asset","id":id,"location":document.assets[id].location}),
                            ),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    self.ipr_project_changed();
                    Ok(
                        json!({"project":self.projects.reference(),"revision":change.revision,"created":created,"published":true}),
                    )
                }
                (
                    Pending::AddImport {
                        reference,
                        store_id,
                        folder,
                    },
                    IoOutput::ImportPrepared { batch, diagnostics },
                ) => {
                    if cancelled {
                        return Err(ErrorPayload::new(
                            ErrorCode::Cancelled,
                            "import cancelled before publication",
                        ));
                    }
                    // Opening or closing a scene while the job ran
                    // moves the activation but not the content revision, and
                    // must not stale the publication. Identity decides via
                    // project/revision/store instead of `check`: a closed
                    // project has no reference, and a reopened or
                    // Save-As-relocated one has a different store id.
                    let current = self.projects.reference().ok_or_else(|| {
                        ErrorPayload::new(
                            ErrorCode::StaleProject,
                            "project closed before publication",
                        )
                    })?;
                    if current.project_id != reference.project_id {
                        return Err(ErrorPayload::new(
                            ErrorCode::StaleProject,
                            "project identity changed",
                        )
                        .with_details(json!({"current":current})));
                    }
                    // Same semantics as `check` elsewhere: content edits
                    // report revision conflicts, as in the stale-revision case,
                    // while scene switches that leave the revision untouched
                    // publish below.
                    if current.revision != reference.revision {
                        return Err(ErrorPayload::new(
                            ErrorCode::RevisionConflict,
                            "project revision changed",
                        )
                        .with_details(json!({"current":current})));
                    }
                    let active = self.projects.active.as_ref().unwrap();
                    if active.store_id != store_id {
                        return Err(ErrorPayload::new(
                            ErrorCode::StaleProject,
                            "import belongs to a relocated project",
                        )
                        .with_details(json!({"current":current})));
                    }
                    let batch = batch
                        .placed_for(active.session.document(), folder.as_deref())
                        .map_err(crate::project_io::project_error)?;
                    let edit = project::Edit::ImportBatch { batch };
                    let mut validation =
                        project::ProjectSession::new(active.session.document().clone())
                            .map_err(crate::project_io::project_error)?;
                    validation
                        .apply(&validation.revision(), std::slice::from_ref(&edit))
                        .map_err(crate::project_io::project_error)?;
                    let document = validation.document();
                    let mismatches = pbrt_io::texture_type_mismatches(document);
                    if !mismatches.is_empty() {
                        return Err(invalid(
                            "import has incompatible texture parameters or references",
                        )
                        .with_details(
                            json!({"reason":"texture_type_mismatch","references":mismatches}),
                        ));
                    }
                    let nodes = document
                        .scenes
                        .values()
                        .flat_map(|scene| scene.nodes.keys().copied())
                        .chain(document.definition_nodes.keys().copied())
                        .collect::<Vec<_>>();
                    let resources = document.resources.keys().copied().collect::<Vec<_>>();
                    pbrt_io::check_exportable(document, &nodes, &resources)
                        .map_err(|error| invalid(error.to_string()))?;
                    let active = self.projects.active.as_mut().unwrap();
                    let change = active
                        .session
                        .apply(&reference.revision, &[edit])
                        .map_err(crate::project_io::project_error)?;
                    let document = active.session.document();
                    let created = change.created_items.iter().map(|item| match item {
                        project::ItemId::Scene(id) => json!({"kind":"scene","id":id,"location":document.scenes[id].location}),
                        project::ItemId::Resource(id) => json!({"kind":"resource","id":id,"location":document.resources[id].location}),
                        project::ItemId::Asset(id) => json!({"kind":"asset","id":id,"location":document.assets[id].location}),
                    }).collect::<Vec<_>>();
                    self.ipr_project_changed();
                    Ok(
                        json!({"project":self.projects.reference(),"revision":change.revision,"created":created,"diagnostics":diagnostics,"published":true}),
                    )
                }

                (
                    Pending::Switch { expected },
                    IoOutput::Loaded {
                        snapshot,
                        store_id,
                        path,
                        source_path,
                        unregistered,
                        published,
                        report,
                        mode,
                        asset_root,
                        staging_owner,
                    },
                ) => {
                    let accept = self.projects.check_context(&expected);
                    if accept.is_err() || cancelled {
                        self.projects.queue.push_back(IoCommand::Release(store_id));
                        if cancelled && published {
                            return Ok(json!({"path":path,"published":true,"activated":false}));
                        }
                        return Err(if cancelled {
                            ErrorPayload::new(ErrorCode::Cancelled, "project open cancelled")
                        } else {
                            accept.unwrap_err().with_details(json!({"current":self.projects.context(),"published":published,"path":path}))
                        });
                    }
                    let mut session = match ProjectSession::from_saved_snapshot(snapshot) {
                        Ok(session) => session,
                        Err(error) => {
                            self.projects.queue.push_back(IoCommand::Release(store_id));
                            return Err(crate::project_io::project_error(error));
                        }
                    };
                    if let Some(root) = asset_root.as_deref() {
                        if let Err(error) =
                            session.enable_temporary_payload_staging(std::path::Path::new(root))
                        {
                            self.projects.queue.push_back(IoCommand::Release(store_id));
                            return Err(crate::project_io::project_error(error));
                        }
                    }
                    if let Some(old) = self.projects.active.take() {
                        self.projects
                            .queue
                            .push_back(IoCommand::Release(old.store_id));
                    }
                    self.projects.advance_activation();
                    self.projects.active = Some(ActiveProject {
                        session,
                        store_id,
                        path: path.clone(),
                        source_path,
                        unregistered,
                        mode,
                        asset_root,
                        staging_owner,
                    });
                    Ok(
                        json!({"path":path,"mode":mode,"published":published,"activated":true,"project":self.projects.reference(),"report":report}),
                    )
                }
                (
                    Pending::Save {
                        store_id,
                        saved_scene,
                        reference,
                    },
                    IoOutput::Saved {
                        receipt,
                        registered,
                    },
                ) => {
                    let mut persist_scene = None;
                    if let Some(active) = self
                        .projects
                        .active
                        .as_mut()
                        .filter(|p| p.store_id == store_id)
                    {
                        active
                            .session
                            .mark_saved(&receipt)
                            .map_err(crate::project_io::project_error)?;
                        if active
                            .session
                            .merge_registered_items(&registered, &active.unregistered)
                        {
                            active.unregistered.clear();
                        }
                        if active.mode == dto::ProjectMode::Project
                            && active.session.active_scene() != saved_scene
                        {
                            persist_scene = Some(active.session.active_scene());
                        }
                    }
                    if let Some(scene) = persist_scene {
                        self.projects
                            .queue
                            .push_back(IoCommand::SetDefaultScene { store_id, scene });
                    }
                    Ok(
                        json!({"project":reference,"saved_revision":receipt.revision(),"generation":receipt.generation(),"content_id":receipt.content_id().as_str(),"published":true}),
                    )
                }
                (
                    Pending::Save {
                        store_id: old_store_id,
                        saved_scene,
                        reference,
                    },
                    IoOutput::SavedAs {
                        receipt,
                        registered,
                        store_id: new_store_id,
                        path,
                        relocated,
                    },
                ) => {
                    let active_matches = self
                        .projects
                        .active
                        .as_ref()
                        .is_some_and(|project| project.store_id == old_store_id);
                    if cancelled || !active_matches {
                        self.projects
                            .queue
                            .push_back(IoCommand::Release(new_store_id));
                        return Ok(json!({
                            "project":reference,
                            "saved_revision":receipt.revision(),
                            "generation":receipt.generation(),
                            "content_id":receipt.content_id().as_str(),
                            "path":path,
                            "relocated":false,
                            "activated":false,
                            "mode":dto::ProjectMode::Project,
                            "published":true,
                        }));
                    }
                    let active = self.projects.active.as_mut().unwrap();
                    let relocation = match relocated {
                        Some(relocated) => active.session.adopt_relocated_payloads(relocated),
                        None => active
                            .session
                            .relocate_payload_staging(&std::path::Path::new(&path).join("Assets")),
                    };
                    if let Err(error) = relocation {
                        self.projects
                            .queue
                            .push_back(IoCommand::Release(new_store_id));
                        return Err(crate::project_io::project_error(error));
                    }
                    let accepted = if active.mode == dto::ProjectMode::Viewer {
                        active
                            .session
                            .accept_saved_migration(registered.clone(), &receipt)
                    } else {
                        active.session.mark_saved(&receipt)
                    };
                    if let Err(error) = accepted {
                        self.projects
                            .queue
                            .push_back(IoCommand::Release(new_store_id));
                        return Err(crate::project_io::project_error(error));
                    }
                    active.store_id = new_store_id;
                    active.path = Some(path.clone());
                    active.asset_root = std::path::Path::new(&path)
                        .join("Assets")
                        .to_str()
                        .map(str::to_owned);
                    active.staging_owner = None;
                    if let Some(root) = active.asset_root.as_deref() {
                        active
                            .session
                            .set_payload_assets_root(std::path::Path::new(root));
                    }
                    active.source_path = None;
                    if active
                        .session
                        .merge_registered_items(&registered, &active.unregistered)
                    {
                        active.unregistered.clear();
                    }
                    active.mode = dto::ProjectMode::Project;
                    let current_scene = active.session.active_scene();
                    if current_scene != saved_scene {
                        self.projects.queue.push_back(IoCommand::SetDefaultScene {
                            store_id: new_store_id,
                            scene: current_scene,
                        });
                    }
                    self.projects
                        .queue
                        .push_back(IoCommand::Release(old_store_id));
                    let mode = dto::ProjectMode::Project;
                    let project = self.projects.reference().unwrap_or(reference);
                    Ok(json!({
                        "project":project,
                        "saved_revision":receipt.revision(),
                        "generation":receipt.generation(),
                        "content_id":receipt.content_id().as_str(),
                        "path":path,
                        "relocated":true,
                        "mode":mode,
                        "published":true,
                    }))
                }
                (Pending::Export { reference }, IoOutput::Exported { path, warnings }) => Ok(
                    json!({"project":reference,"scene_path":path,"published":true,
                           "warnings":export_warnings(&warnings)}),
                ),
                (
                    Pending::Preview {
                        reference,
                        snapshot,
                    },
                    IoOutput::Previewed {
                        path,
                        draws,
                        unsupported,
                        warnings,
                    },
                ) => {
                    // the succeeded preview is now the
                    // activation's one preview base; the next
                    // `project.preview_transforms` diffs against it and a
                    // mismatching base_revision just means fallback. A job
                    // that crossed an activation switch (its reference was
                    // pinned at request time) published for a context the
                    // caller has already discarded, so it is not a base.
                    if reference.activation == Revision::from_u64(self.projects.activation) {
                        self.projects.record_preview_base(snapshot);
                    }
                    // Counts come from the worker's typed manifest; the
                    // driver never reparses the published geometry.json.
                    let slot = std::path::Path::new(&path);
                    Ok(json!({
                        "project":reference,
                        "geometry_path": slot.join("geometry.json").to_string_lossy(),
                        "draws": draws,
                        "unsupported": unsupported,
                        "published": true,
                        "warnings": export_warnings(&warnings),
                    }))
                }
                _ => Err(ErrorPayload::new(
                    ErrorCode::OutputInvalid,
                    "project I/O result does not match job",
                )),
            }
        })();
        let result = result.map_err(|mut error| {
            if additional_import {
                let mut details = error
                    .details
                    .take()
                    .filter(Value::is_object)
                    .unwrap_or_else(|| json!({}));
                details["published"] = json!(false);
                if details.get("reason").is_none() {
                    // Machine-readable fallback: the wire name of the code
                    // ("stale_project"), never the Rust variant spelling.
                    details["reason"] = serde_json::to_value(&error.code)
                        .ok()
                        .filter(Value::is_string)
                        .unwrap_or_else(|| json!(format!("{:?}", error.code)));
                }
                error.details = Some(details);
            }
            error
        });
        let job = &mut self.jobs[self.job_indices[&id]].job;
        match result {
            Ok(value) => {
                job.state = JobState::Succeeded;
                job.phase = "complete".into();
                job.result = Some(value);
            }
            Err(error) => {
                job.state = if error.code == ErrorCode::Cancelled {
                    JobState::Cancelled
                } else {
                    JobState::Failed
                };
                job.phase = if job.state == JobState::Cancelled {
                    "cancelled"
                } else {
                    "failed"
                }
                .into();
                job.error = Some(error);
            }
        }
        self.emit_job(id);
        self.project_changed();
    }
}

/// The wire's `warnings` on export and preview results.
fn export_warnings(warnings: &[pbrt_io::ExportWarning]) -> Vec<serde_json::Value> {
    warnings
        .iter()
        .map(|warning| {
            json!({"code":warning.code,"resource_id":warning.resource,"message":warning.message})
        })
        .collect()
}
