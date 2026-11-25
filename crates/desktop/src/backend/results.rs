//! Display adapter phase; state stays owned by Display.
use super::*;

/// The received-pixel fraction of a running render's partial snapshot,
/// keyed by the job that published it (final renders key by their job
/// id, IPR frames by the active job). None for artifacts without it.
fn received_fraction(artifact: &Value, job_id: &Value) -> Option<(String, f64)> {
    Some((
        job_id.as_str()?.to_owned(),
        artifact["metadata"]["received"].as_f64()?,
    ))
}

pub(super) fn foreground_job_row(job_id: &str, operation: &str, job: &Value) -> Option<Value> {
    let state = job["state"].as_str()?;
    if !matches!(state, "queued" | "running" | "cancelling") {
        return None;
    }
    Some(json!({
        "job_id":job_id,
        "operation":operation,
        "state":state,
        "phase":job["phase"].as_str().unwrap_or(""),
        "can_cancel":state != "cancelling",
    }))
}

impl Display {
    pub(super) fn poll_results(&mut self) {
        let mut foreground = Vec::new();
        self.foreground_jobs
            .retain_mut(|(job_id, operation, misses)| {
                match request(&self.core, Method::JobGet, json!({"job_id":job_id})) {
                    Ok(job) => {
                        *misses = 0;
                        if let Some(row) = foreground_job_row(job_id, operation, &job) {
                            foreground.push(row);
                            true
                        } else {
                            false
                        }
                    }
                    Err(_) => {
                        *misses = misses.saturating_add(1);
                        if *misses < 5 {
                            foreground.push(json!({
                                "job_id":job_id,
                                "operation":operation,
                                "state":"running",
                                "phase":"Checking status…",
                                "can_cancel":false,
                            }));
                            true
                        } else {
                            false
                        }
                    }
                }
            });
        let foreground = json!({"jobs":foreground});
        if foreground != self.last_foreground_jobs {
            let _ = self.tx.send(Update::ForegroundJobs(foreground.clone()));
            self.last_foreground_jobs = foreground;
        }
        if let Ok(settings) = request(&self.core, Method::RenderGetSettings, json!({})) {
            if settings != self.last_settings {
                let _ = self.tx.send(Update::Settings(settings.clone()));
                self.last_settings = settings;
            }
        }
        if let Ok(jobs) = request(&self.core, Method::JobList, json!({"limit":1000})) {
            if jobs != self.last_jobs {
                // The Console hears one line per job end watched here — the
                // single place the backend already observes job changes. IPR
                // decides its cancels by whether IPR is still on (the state
                // sync above keeps this current within one poll).
                let ipr_enabled = self.last_state["ipr"]["enabled"].as_bool().unwrap_or(false);
                let now = Instant::now();
                for end in job_log::settled_jobs(&self.last_jobs, &jobs) {
                    let elapsed = self
                        .job_ages
                        .remove(end.job_id)
                        .map(|start| start.elapsed());
                    if let Some(line) = job_log::console_end_line(&end, elapsed, ipr_enabled) {
                        let _ = self.tx.send(Update::Log(line));
                    }
                }
                // Each unfinished job's first sight starts its elapsed
                // clock; ids that left the list are forgotten.
                if let Some(records) = jobs["jobs"].as_array() {
                    for job in records {
                        if let Some(id) = job["job_id"].as_str()
                            && !matches!(
                                job["state"].as_str(),
                                Some("succeeded" | "failed" | "cancelled")
                            )
                        {
                            self.job_ages.entry(id.to_owned()).or_insert(now);
                        }
                    }
                    let present: std::collections::HashSet<&str> = records
                        .iter()
                        .filter_map(|job| job["job_id"].as_str())
                        .collect();
                    self.job_ages.retain(|id, _| present.contains(id.as_str()));
                }
                if let Some(records) = jobs["jobs"].as_array() {
                    'images: for job in records.iter().rev().take(20) {
                        let active = &self.last_project["context"]["project"];
                        if active.is_object() {
                            if job["project"]["project_id"] != active["project_id"]
                                || job["project"]["activation"] != active["activation"]
                            {
                                continue;
                            }
                        } else if job["project"].is_object() {
                            continue;
                        }
                        if job["kind"] == "ipr" {
                            // Only the current preview job's frames display; stale previews are reaped core-side.
                            if job["job_id"] != self.last_state["ipr"]["active_job_id"] {
                                continue;
                            }
                            if let Some(id) = job["artifacts"].as_array().and_then(|ids| ids.last())
                            {
                                if self.last_artifact.as_ref() == Some(id) {
                                    break 'images;
                                }
                                if let Ok(artifact) = request(
                                    &self.core,
                                    Method::ArtifactGet,
                                    json!({"artifact_id":id}),
                                ) {
                                    if artifact["mime"] == "image/png" {
                                        if let Some(path) = artifact["path"].as_str() {
                                            if let Ok(image) = image::open(path) {
                                                let rgba = image.into_rgba8();
                                                let _ = self.tx.send(Update::RenderImage {
                                                    project: job["project"].clone(),
                                                    artifact_id: None,
                                                    width: rgba.width(),
                                                    height: rgba.height(),
                                                    rgba: rgba.into_raw(),
                                                    received: received_fraction(
                                                        &artifact,
                                                        &job["job_id"],
                                                    ),
                                                });
                                                self.last_artifact = Some(id.clone());
                                            }
                                        }
                                        break 'images;
                                    }
                                }
                            }
                        }
                        if job["kind"] == "capture" || job["kind"] == "render" {
                            if let Some(ids) = job["artifacts"].as_array() {
                                for id in ids.iter().rev() {
                                    if let Ok(artifact) = request(
                                        &self.core,
                                        Method::ArtifactGet,
                                        json!({"artifact_id":id}),
                                    ) {
                                        if artifact["mime"] == "image/png" {
                                            if self.last_artifact.as_ref() == Some(id) {
                                                break 'images;
                                            }
                                            if let Some(path) = artifact["path"].as_str() {
                                                if let Ok(image) = image::open(path) {
                                                    let rgba = image.into_rgba8();
                                                    // A partial preview is replaced by
                                                    // the next one, so it is shown but
                                                    // not offered for export.
                                                    let partial =
                                                        artifact["metadata"]["partial"] == true;
                                                    let _ = self.tx.send(Update::RenderImage {
                                                        project: job["project"].clone(),
                                                        artifact_id: (!partial)
                                                            .then(|| {
                                                                serde_json::from_value(id.clone())
                                                                    .ok()
                                                            })
                                                            .flatten(),
                                                        width: rgba.width(),
                                                        height: rgba.height(),
                                                        rgba: rgba.into_raw(),
                                                        received: received_fraction(
                                                            &artifact,
                                                            &job["job_id"],
                                                        ),
                                                    });
                                                    self.last_artifact = Some(id.clone());
                                                }
                                            }
                                            break 'images;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                let _ = self.tx.send(Update::Jobs(jobs.clone()));
                self.last_jobs = jobs;
            }
        }
    }
}
