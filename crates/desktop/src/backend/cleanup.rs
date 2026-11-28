//! Preview export cancellation and temporary-directory lifecycle.
use super::*;
impl Display {
    pub(super) fn poll_cleanup(&mut self) {
        // A retired job's slot must not be owned by the in-flight
        // replacement that reuses the same directory.
        let active_slot = self
            .project_preview_job
            .as_ref()
            .map(|(_, directory)| directory.clone());
        let project_folder = self.last_project["path"].as_str().map(std::path::Path::new);
        let current_slot = if project_folder.is_some() {
            super::state_sync::preview_slot_path(
                &self.last_project["context"]["project"],
                project_folder,
                None,
            )
        } else {
            project::cache_directory().ok().and_then(|cache_root| {
                super::state_sync::preview_slot_path(
                    &self.last_project["context"]["project"],
                    None,
                    Some(&cache_root),
                )
            })
        };
        // Side effects needing a whole-Display borrow run after the retain
        // closure: the closure already owns &mut self.project_preview_cleanup.
        let mut parked_preview_failed = false;
        self.project_preview_cleanup.retain(|entry| {
            let terminal = request(&self.core, Method::JobGet, json!({"job_id":entry.job_id}))
                .ok()
                .and_then(|job| job["state"].as_str().map(str::to_owned))
                .is_some_and(|state| {
                    matches!(state.as_str(), "succeeded" | "failed" | "cancelled")
                });
            if terminal {
                let retired_is_current = active_slot.as_deref() == Some(entry.directory.as_str());
                if !retired_is_current {
                    let slot = std::path::Path::new(&entry.directory);
                    // The current activation's store outlives its jobs; only
                    // activation change or shutdown retires it.
                    if current_slot.as_deref() == Some(slot) {
                        let _ = std::fs::remove_dir_all(slot);
                    } else {
                        preview_sweep::remove_slot_and_store(slot);
                    }
                }
                // Issue the parked replacement only when this display still
                // wants it: a newer refresh that already started (or
                // concluded) owns the slot instead.
                if let (Some((reference, destination)), false) =
                    (&entry.queued_request, retired_is_current)
                    && self.project_preview_job.is_none()
                        && self.project_preview_key.is_none()
                        && reference == &self.last_project["context"]["project"]
                    {
                        match request(
                            &self.core,
                            Method::ProjectPreview,
                            json!({"project":reference,"destination":destination,"store":preview_sweep::store_path_for_slot(std::path::Path::new(destination)).to_string_lossy(),"stage_assets":false}),
                        ) {
                            Ok(started) => {
                                if let Some(job_id) = started["job_id"].as_str() {
                                    self.project_preview_job =
                                        Some((job_id.to_owned(), destination.clone()));
                                    self.project_preview_key = Some(project_preview_key(
                                        reference,
                                        &self.last_project["active_scene"],
                                    ));
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
                                parked_preview_failed = true;
                                let _ = self.tx.send(Update::PreviewUnavailable(Some(format!(
                                    "Project preview export: {error}"
                                ))));
                                self.project_preview_key = Some(project_preview_key(
                                    reference,
                                    &self.last_project["active_scene"],
                                ));
                                let _ = self.tx.send(Update::Status(format!(
                                    "Project preview export: {error}"
                                )));
                            }
                        }
                    }
            }
            !terminal
        });
        if parked_preview_failed {
            self.fail_current_camera_preview();
        }
    }
    pub(super) fn cleanup(&mut self) {
        if let Some((job_id, directory)) = self.project_preview_job.take() {
            self.project_preview_cleanup.push(PreviewCleanup {
                job_id,
                directory,
                queued_request: None,
            });
        }
        for entry in &self.project_preview_cleanup {
            let _ = request(
                &self.core,
                Method::JobCancel,
                json!({"job_id":entry.job_id}),
            );
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !self.project_preview_cleanup.is_empty() && std::time::Instant::now() < deadline {
            self.project_preview_cleanup.retain(|entry| {
                let terminal = request(&self.core, Method::JobGet, json!({"job_id":entry.job_id}))
                    .ok()
                    .is_some_and(|job| {
                        matches!(
                            job["state"].as_str(),
                            Some("succeeded" | "failed" | "cancelled")
                        )
                    });
                if terminal {
                    preview_sweep::remove_slot_and_store(std::path::Path::new(&entry.directory));
                }
                !terminal
            });
            if !self.project_preview_cleanup.is_empty() {
                thread::sleep(Duration::from_millis(20));
            }
        }
    }
}
