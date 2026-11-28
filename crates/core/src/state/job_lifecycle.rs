//! Command handling within the single CoreState owner.
use super::*;
impl CoreState {
    pub(crate) fn configure_services(&mut self, config: ServiceConfig) {
        self.capabilities.workspace_layout = config.gui;
        self.capabilities.scene_capture = config.capture.is_some();
        self.capabilities.render = config.renderer.is_some();
        self.capabilities.ipr = config.renderer.is_some();
        self.service_config = Some(config);
    }
    pub(crate) fn take_services(&mut self) -> impl Iterator<Item = ServiceTask> + '_ {
        self.service_tasks.drain(..)
    }
    pub(super) fn service_task(
        &mut self,
        work: Work,
        kind: &str,
    ) -> Result<(JobId, ServiceTask), ErrorPayload> {
        let config = self
            .service_config
            .clone()
            .ok_or_else(|| ErrorPayload::new(ErrorCode::Unsupported, "job services unavailable"))?;
        let project_folder = self.projects.project_folder();
        let config = config.for_project(project_folder)?;
        if self
            .jobs
            .iter()
            .any(|record| record.job.kind == kind && !record.job.state.is_terminal())
        {
            return Err(ErrorPayload::new(
                ErrorCode::Busy,
                "a job of this kind is already active",
            ));
        }
        let id = Uuid::new_v4();
        let task = LoadTask {
            scene_id: SceneId::default(),
            scene_revision: Revision::initial(),
            job_id: id,
            absolute_path: String::new(),
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        {
            let job = Job {
                project: None,
                result: None,
                job_id: id,
                kind: kind.into(),
                state: JobState::Queued,
                phase: "queued".into(),
                root_path: String::new(),
                scene_id: None,
                error: None,
                artifacts: Vec::new(),
            };
            self.job_indices.insert(id, self.jobs.len());
            self.jobs.push(JobRecord {
                job,
                task: task.clone(),
            });
            self.emit_job(id);
        }
        Ok((id, ServiceTask { task, work, config }))
    }
    pub(crate) fn service_finished(&mut self, id: JobId, mut result: WorkResult) {
        let Some(&index) = self.job_indices.get(&id) else {
            return;
        };
        let cancelled = self.jobs[index].task.is_cancelled();
        for artifact in result.artifacts {
            self.jobs[index].job.artifacts.push(artifact.artifact_id);
            self.artifacts.insert(artifact.artifact_id, artifact);
        }
        if self.jobs[index].job.kind == "pick"
            && let (Ok(value), Some(pinned)) =
                (&mut result.result, self.jobs[index].job.project.clone())
            && let Some(node) = value["node_id"]
                .as_str()
                .and_then(|id| uuid::Uuid::parse_str(id).ok())
        {
            value["node"] = self.project_node_summary(node, &pinned);
        }
        let job = &mut self.jobs[index].job;
        if cancelled {
            job.state = JobState::Cancelled;
            job.phase = "cancelled".into();
        } else {
            match result.result {
                Ok(value) => {
                    job.state = JobState::Succeeded;
                    job.phase = "complete".into();
                    job.result = Some(value);
                }
                Err(error) => {
                    job.state = JobState::Failed;
                    job.phase = "failed".into();
                    job.error = Some(error);
                }
            }
        }
        if job.kind == ipr::IPR_JOB_KIND {
            self.ipr_finalized(id);
        }
        self.emit_job(id);
    }
    pub(crate) fn service_started(&mut self, id: JobId) {
        let Some(&index) = self.job_indices.get(&id) else {
            return;
        };
        if self.jobs[index].job.state == JobState::Queued {
            self.jobs[index].job.state = JobState::Running;
            self.jobs[index].job.phase = "executing".into();
            self.emit_job(id);
        }
    }
    pub(crate) fn service_progress(&mut self, id: JobId, artifact: Artifact) {
        let Some(&index) = self.job_indices.get(&id) else {
            return;
        };
        if self.jobs[index].job.state.is_terminal() {
            return;
        }
        if self.jobs[index].job.kind == ipr::IPR_JOB_KIND {
            // Progressive frames replace the previous frame; they never
            // accumulate as retained artifacts.
            self.ipr_frame_replaced(id, artifact);
            return;
        }
        // A partial snapshot replaces the previous one, so a long render
        // keeps publishing without accumulating previews.
        if artifact.metadata["partial"] == true {
            let superseded: Vec<_> = self.jobs[index]
                .job
                .artifacts
                .extract_if(.., |id| {
                    self.artifacts
                        .get(id)
                        .is_some_and(|a| a.metadata["partial"] == true)
                })
                .collect();
            for id in superseded {
                if let Some(artifact) = self.artifacts.remove(&id) {
                    let _ = std::fs::remove_file(&artifact.path);
                }
            }
        }
        self.jobs[index].job.artifacts.push(artifact.artifact_id);
        self.artifacts.insert(artifact.artifact_id, artifact);
        self.emit_job(id);
    }
    pub fn job(&self, job_id: JobId) -> Result<&Job, ErrorPayload> {
        self.job_indices
            .get(&job_id)
            .map(|index| &self.jobs[*index].job)
            .ok_or_else(|| {
                ErrorPayload::new(ErrorCode::NotFound, "job not found")
                    .with_target(job_id.to_string())
            })
    }
    pub(super) fn cancel_job(&mut self, job_id: JobId) {
        let index = self.job_indices[&job_id];
        let record = &mut self.jobs[index];
        if record.job.state.is_terminal() || record.job.state == JobState::Cancelling {
            return;
        }
        record.task.cancel();
        // Even a queued job may have started in the executor before its
        // Started notification is processed. Wait for executor acknowledgement.
        record.job.state = JobState::Cancelling;
        record.job.phase = "cancelling".into();
        self.emit_job(job_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::Work;

    /// A final render's partial previews replace each other (one file kept),
    /// while ordinary artifacts still accumulate.
    #[test]
    fn partial_previews_replace_the_previous_one() {
        let mut core = CoreState::new(Limits::default());
        core.configure_services(ServiceConfig {
            gui: false,
            capture: None,
            renderer: Some("/unused-renderer".into()),
            directory: Arc::new(tempfile::tempdir().unwrap()),
            geometry_cache: Default::default(),
            capture_daemon: Default::default(),
            project_directories: Default::default(),
        });
        let work = Work::ProjectRender {
            root: "/published/scene.pbrt".into(),
            project: protocol::project::ProjectRef {
                project_id: Uuid::new_v4(),
                revision: Revision::initial(),
                activation: Revision::initial(),
            },
            settings: protocol::jobs::RenderSettings::default(),
            view: None,
            ipr: false,
        };
        let (id, _) = core.service_task(work, "render").unwrap();
        let directory = tempfile::tempdir().unwrap();
        let artifact = |name: &str, partial: bool| {
            let path = directory.path().join(name);
            std::fs::write(&path, name).unwrap();
            Artifact {
                artifact_id: Uuid::new_v4(),
                path: path.to_string_lossy().into_owned(),
                mime: "image/png".into(),
                size: 1,
                sha256: "0".repeat(64),
                metadata: json!({"kind":"render_preview","partial":partial}),
            }
        };
        let manifest = artifact("export.json", false);
        let manifest_id = manifest.artifact_id;
        core.service_progress(id, manifest);
        core.service_progress(id, artifact("progress-000.png", true));
        let second = artifact("progress-001.png", true);
        let second_id = second.artifact_id;
        core.service_progress(id, second);
        assert_eq!(
            core.job(id).unwrap().artifacts,
            vec![manifest_id, second_id]
        );
        assert!(!directory.path().join("progress-000.png").exists());
        assert!(directory.path().join("progress-001.png").exists());
        assert_eq!(core.artifacts.len(), 2);
    }
}
