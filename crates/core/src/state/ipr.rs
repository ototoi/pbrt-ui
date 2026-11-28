//! Interactive progressive rendering (IPR): explicit On/Off, latest state only.
//!
//! IPR starts automatically when a committed project revision, the camera, or
//! IPR settings change, after a debounce window coalesces the edits into one
//! job that renders the latest state. An IPR job never cancels a final
//! render; a final render request preempts IPR instead, and IPR launch
//! requests that arrive while a final render is active coalesce into a single
//! launch once the final render is terminal. Progressive frames replace the
//! previous frame instead of accumulating artifacts.
use super::*;
use std::time::{Duration, Instant};

/// Why a render is refused when no renderer was found: what to do about it.
pub(crate) const RENDERER_NOT_CONFIGURED: &str = "renderer is not configured: place pbrt-r4 \
     next to the executable or start it with --renderer-path";

pub(crate) const IPR_JOB_KIND: &str = "ipr";
const IPR_SAMPLES_LIMIT: u32 = 256;
const IPR_DEBOUNCE_LIMIT: u64 = 5_000;

pub(crate) struct IprParams {
    project: Option<protocol::project::ProjectRef>,
    view: Option<ViewState>,
    scene: Option<protocol::SceneId>,
}

#[derive(Default)]
pub(crate) struct Ipr {
    enabled: bool,
    settings: IprSettings,
    params: Option<IprParams>,
    armed: bool,
    armed_at: Option<Instant>,
    generation: u64,
    active_job: Option<JobId>,
    launched_generation: u64,
    frame: Option<(JobId, protocol::ArtifactId)>,
}

fn validate(settings: &IprSettings) -> Result<(), ErrorPayload> {
    if settings.samples_per_pixel == 0
        || settings.samples_per_pixel > IPR_SAMPLES_LIMIT
        || settings.threads.is_some_and(|n| n == 0 || n > 1024)
        || settings
            .max_depth
            .is_some_and(|depth| depth == 0 || depth > 1024)
        || settings.debounce_ms < 10
        || settings.debounce_ms > IPR_DEBOUNCE_LIMIT
    {
        return Err(invalid("ipr settings exceed limits"));
    }
    Ok(())
}

impl CoreState {
    pub(crate) fn ipr_status(&self) -> IprStatus {
        let active = self.ipr.active_job.filter(|id| {
            self.job(*id)
                .ok()
                .is_some_and(|job| !job.state.is_terminal())
        });
        IprStatus {
            enabled: self.ipr.enabled,
            settings: self.ipr.settings.clone(),
            active_job_id: active,
            waiting_for_final_render: self.ipr.enabled
                && self.ipr.armed
                && active.is_none()
                && self.ipr.params.is_some()
                && self.final_render_active(),
        }
    }

    pub(crate) fn handle_ipr(&mut self, request: &Request) -> Result<Value, ErrorPayload> {
        match request.method {
            Method::RenderIprSet => {
                let p: IprSetParams = params(request)?;
                validate(&p.settings)?;
                if !self.capabilities.render {
                    return Err(ErrorPayload::new(
                        ErrorCode::Unsupported,
                        RENDERER_NOT_CONFIGURED,
                    ));
                }
                self.ipr.enabled = p.enabled;
                self.ipr.settings = p.settings;
                if p.enabled {
                    if self.projects.is_active() {
                        self.ipr_rearm_current_project();
                    }
                } else {
                    self.ipr.armed = false;
                    self.ipr.armed_at = None;
                    for index in 0..self.jobs.len() {
                        if self.jobs[index].job.kind == IPR_JOB_KIND
                            && !self.jobs[index].job.state.is_terminal()
                        {
                            let id = self.jobs[index].job.job_id;
                            self.cancel_job(id);
                        }
                    }
                }
                let status = self.ipr_status();
                self.emit("ipr.changed", None, None, json!(&status));
                Ok(serde_json::to_value(&status).map_err(|error| invalid(error.to_string()))?)
            }
            Method::RenderIprSubmit => {
                let p: IprSubmitParams = params(request)?;
                if !self.ipr.enabled {
                    return Err(ErrorPayload::new(
                        ErrorCode::Unsupported,
                        "ipr is not enabled",
                    ));
                }
                if let Some(reference) = &p.project {
                    self.projects.check(reference)?;
                }
                if let Some(view) = &p.view
                    && (view.size[0] == 0
                        || view.size[1] == 0
                        || view.size[0] > 8192
                        || view.size[1] > 8192
                        || u64::from(view.size[0]) * u64::from(view.size[1]) > 16_777_216)
                {
                    return Err(invalid("ipr view size exceeds limits"));
                }
                let project = if self.projects.is_active() {
                    Some(self.projects.reference().unwrap())
                } else {
                    None
                };
                if let Some(scene) = p.scene {
                    if !self.projects.is_active() {
                        return Err(invalid("scene can be specified only for an active project"));
                    }
                    if !self.projects.contains_scene(project::SceneId(scene)) {
                        return Err(ErrorPayload::new(ErrorCode::NotFound, "scene not found"));
                    }
                }
                // Native projects without an explicit push reuse the stored
                // shared view, matching project.render and project.capture.
                let view = project.as_ref().and_then(|_| {
                    p.view.clone().or_else(|| {
                        let requested_scene = p.scene.map(project::SceneId);
                        let active_scene = self.projects.active_scene();
                        (requested_scene.is_none() || requested_scene == active_scene)
                            .then(|| self.stored_project_view().map(|(view, _)| view.clone()))
                            .flatten()
                    })
                });
                self.ipr.params = Some(IprParams {
                    project,
                    view,
                    scene: p.scene,
                });
                self.ipr.armed = true;
                self.ipr.armed_at = Some(Instant::now());
                self.ipr.generation += 1;
                let status = self.ipr_status();
                Ok(serde_json::to_value(&status).map_err(|error| invalid(error.to_string()))?)
            }
            _ => unreachable!("command routed to the wrong handler"),
        }
    }

    /// A project revision moved: coalesce into one launch for the latest state.
    fn ipr_rearm_current_project(&mut self) {
        let Some(current) = self.projects.reference() else {
            return;
        };
        if self
            .ipr
            .params
            .as_ref()
            .is_some_and(|p| p.project.as_ref() == Some(&current))
        {
            return;
        }
        let previous = self.ipr.params.take();
        let view = previous.as_ref().and_then(|p| p.view.clone());
        let same_project = previous.as_ref().is_some_and(|params| {
            params.project.as_ref().is_some_and(|project| {
                project.project_id == current.project_id && project.activation == current.activation
            })
        });
        let scene = same_project
            .then(|| previous.and_then(|params| params.scene))
            .flatten()
            .filter(|scene| self.projects.contains_scene(project::SceneId(*scene)));
        self.ipr.params = Some(IprParams {
            project: Some(current),
            view,
            scene,
        });
        self.ipr.armed = true;
        self.ipr.armed_at = Some(Instant::now());
        self.ipr.generation += 1;
    }

    pub(crate) fn ipr_project_changed(&mut self) {
        if self.ipr.enabled && self.projects.is_active() {
            self.ipr_rearm_current_project();
        }
    }

    /// A final render started: IPR yields the renderer slot to it. Armed or
    /// running launches are preempted; once the final render is terminal the
    /// coalesced relaunch of the latest state proceeds.
    pub(crate) fn ipr_preempt(&mut self) {
        let mut preempted = false;
        for index in 0..self.jobs.len() {
            if self.jobs[index].job.kind == IPR_JOB_KIND
                && !self.jobs[index].job.state.is_terminal()
                && self.jobs[index].job.state != JobState::Cancelling
            {
                let id = self.jobs[index].job.job_id;
                self.cancel_job(id);
                preempted = true;
            }
        }
        if preempted && self.ipr.enabled && self.ipr.params.is_some() {
            self.ipr.armed = true;
            self.ipr.armed_at = Some(Instant::now());
        }
    }

    /// A client cancelled an IPR job explicitly: drop the pending relaunch.
    pub(crate) fn ipr_job_cancelled(&mut self, id: JobId) {
        if self
            .job(id)
            .ok()
            .is_some_and(|job| job.kind == IPR_JOB_KIND)
        {
            self.ipr.armed = false;
            self.ipr.armed_at = None;
        }
    }

    pub(crate) fn ipr_finalized(&mut self, id: JobId) {
        if self.ipr.active_job == Some(id) {
            self.ipr.active_job = None;
        }
        if self.ipr.frame.is_some_and(|(job, _)| job == id) {
            self.ipr_clear_frame();
        }
    }

    /// Replace the single progressive frame of an IPR job. Superseded frame
    /// artifacts and their files are removed so previews never accumulate.
    pub(crate) fn ipr_frame_replaced(&mut self, id: JobId, artifact: Artifact) {
        let Some(&index) = self.job_indices.get(&id) else {
            return;
        };
        if let Some((frame_job, previous)) = self.ipr.frame.take()
            && frame_job == id
            && let Some(superseded) = self.artifacts.remove(&previous)
        {
            let _ = std::fs::remove_file(&superseded.path);
        }
        self.ipr.frame = Some((id, artifact.artifact_id));
        self.jobs[index].job.artifacts = vec![artifact.artifact_id];
        self.artifacts.insert(artifact.artifact_id, artifact);
        self.emit_job(id);
    }

    fn ipr_clear_frame(&mut self) {
        if let Some((job, id)) = self.ipr.frame.take() {
            if let Some(&index) = self.job_indices.get(&job) {
                self.jobs[index].job.artifacts.clear();
            }
            if let Some(frame) = self.artifacts.remove(&id) {
                let _ = std::fs::remove_file(&frame.path);
            }
        }
    }

    fn final_render_active(&self) -> bool {
        self.jobs
            .iter()
            .any(|record| record.job.kind == "render" && !record.job.state.is_terminal())
    }

    pub(crate) fn ipr_render_settings(&self) -> RenderSettings {
        RenderSettings {
            samples_per_pixel: Some(self.ipr.settings.samples_per_pixel),
            threads: self.ipr.settings.threads,
            gpu: false,
            max_depth: self.ipr.settings.max_depth,
        }
    }

    /// Debounce, staleness, and final-render priority for IPR launches.
    pub(crate) fn tick_ipr(&mut self, now: Instant) {
        if !self.ipr.enabled {
            return;
        }
        if let Some(id) = self.ipr.active_job {
            let Some(job) = self.job(id).ok().cloned() else {
                self.ipr.active_job = None;
                return;
            };
            if job.state.is_terminal() {
                self.ipr.active_job = None;
            } else if job.state != JobState::Cancelling {
                let stale_generation = self.ipr.launched_generation != self.ipr.generation;
                let stale_target = job.project != self.projects.reference();
                if stale_generation || stale_target {
                    self.cancel_job(id);
                    self.ipr.armed = true;
                    self.ipr.armed_at = Some(now);
                }
            }
            return;
        }
        if !self.ipr.armed {
            return;
        }
        let renderable = match self.ipr.params.as_ref().map(|p| &p.project) {
            Some(Some(_)) => self.projects.is_active(),
            Some(None) | None => false,
        };
        if !renderable || !self.capabilities.render {
            return;
        }
        if self.final_render_active() {
            return;
        }
        let debounce = Duration::from_millis(self.ipr.settings.debounce_ms);
        if self
            .ipr
            .armed_at
            .is_none_or(|at| now.saturating_duration_since(at) < debounce)
        {
            return;
        }
        self.ipr_launch(now);
    }

    fn ipr_launch(&mut self, now: Instant) {
        let Some(params) = self.ipr.params.take() else {
            return;
        };
        let launched = if params.project.is_some() && self.projects.is_active() {
            let view = params.view.clone();
            self.ipr_project_launch(view.clone(), params.scene)
                .map(|id| {
                    (
                        id,
                        IprParams {
                            project: self.projects.reference(),
                            view,
                            scene: params.scene,
                        },
                    )
                })
        } else {
            None
        };
        match launched {
            Some((id, params)) => {
                self.ipr.params = Some(params);
                self.ipr.active_job = Some(id);
                self.ipr.launched_generation = self.ipr.generation;
                self.ipr.armed = false;
                self.ipr.armed_at = None;
            }
            None => {
                self.ipr.params = Some(params);
                self.ipr.armed_at = Some(now);
            }
        }
    }
}

#[cfg(test)]
mod tests;
