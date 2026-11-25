//! Keep a Viewer camera only for the project activation produced by its reload.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProjectOwner {
    project_id: String,
    activation: String,
}

impl ProjectOwner {
    pub(super) fn from_ref(project: &Value) -> Option<Self> {
        Some(Self {
            project_id: project["project_id"].as_str()?.to_owned(),
            activation: project["activation"].as_str()?.to_owned(),
        })
    }

    pub(super) fn key(&self) -> String {
        format!("{}:{}", self.project_id, self.activation)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CameraPreservation {
    pub job_id: String,
    source: ProjectOwner,
    target: Option<ProjectOwner>,
}

impl CameraPreservation {
    pub(super) fn new(job_id: String, source: ProjectOwner) -> Self {
        Self {
            job_id,
            source,
            target: None,
        }
    }

    pub(super) fn set_target(&mut self, job_id: &str, project: &Value) -> bool {
        if self.job_id != job_id {
            return false;
        }
        let Some(target) = ProjectOwner::from_ref(project) else {
            return false;
        };
        self.target = Some(target);
        true
    }

    fn target_matches(&self, owner: &ProjectOwner) -> bool {
        self.target.as_ref() == Some(owner)
    }

    fn fail_reload(&self, job_id: &str) -> bool {
        self.job_id == job_id
    }

    fn fail_preview_for(&self, owner: &ProjectOwner) -> bool {
        self.target_matches(owner)
    }
}

impl Display {
    /// Resolve a target from the exact reload job if the state poll beat the
    /// import-result poll. A still-running job leaves the intent pending.
    pub(super) fn reconcile_camera_preservation(&mut self) {
        let Some(intent) = self.preserve_camera.clone() else {
            return;
        };
        let current_ref = &self.last_project["context"]["project"];
        let Some(owner) = ProjectOwner::from_ref(current_ref) else {
            self.preserve_camera = None;
            return;
        };
        if self.last_project["active_scene"].is_null() {
            self.preserve_camera = None;
            return;
        }
        if intent.target_matches(&owner) {
            return;
        }
        if owner == intent.source {
            return;
        }

        let Ok(job) = request(&self.core, Method::JobGet, json!({"job_id":intent.job_id})) else {
            return;
        };
        match job["state"].as_str() {
            Some("succeeded") => {
                let target = job["result"]["project"].clone();
                if self
                    .preserve_camera
                    .as_mut()
                    .is_some_and(|pending| pending.set_target(&intent.job_id, &target))
                {
                    if !self
                        .preserve_camera
                        .as_ref()
                        .is_some_and(|pending| pending.target_matches(&owner))
                    {
                        self.preserve_camera = None;
                    }
                } else {
                    self.preserve_camera = None;
                }
            }
            Some("failed" | "cancelled") => self.preserve_camera = None,
            _ => {}
        }
    }

    /// Consume the intent only for the exact activation created by the reload.
    pub(super) fn take_camera_preservation_for(&mut self, owner: &ProjectOwner) -> bool {
        self.reconcile_camera_preservation();
        let matches = self
            .preserve_camera
            .as_ref()
            .is_some_and(|intent| intent.target_matches(owner));
        if matches {
            self.preserve_camera = None;
            return true;
        }
        if self
            .preserve_camera
            .as_ref()
            .is_some_and(|intent| intent.target.is_some() || intent.source != *owner)
        {
            self.preserve_camera = None;
        }
        false
    }

    pub(super) fn complete_camera_reload(&mut self, job_id: &str, result: &Value) {
        let target = &result["project"];
        if self
            .preserve_camera
            .as_mut()
            .is_some_and(|intent| intent.set_target(job_id, target))
        {
            self.reconcile_camera_preservation();
        }
    }

    pub(super) fn fail_camera_reload(&mut self, job_id: &str) {
        if self
            .preserve_camera
            .as_ref()
            .is_some_and(|intent| intent.fail_reload(job_id))
        {
            self.preserve_camera = None;
        }
    }

    pub(super) fn fail_camera_preview_for(&mut self, owner: &ProjectOwner) {
        self.reconcile_camera_preservation();
        if self
            .preserve_camera
            .as_ref()
            .is_some_and(|intent| intent.fail_preview_for(owner))
        {
            self.preserve_camera = None;
        }
    }

    pub(super) fn choose_camera_for_preview(
        &mut self,
        owner: &ProjectOwner,
        manifest_camera: ViewerCamera,
    ) -> bool {
        if self.take_camera_preservation_for(owner) {
            true
        } else {
            self.camera = manifest_camera;
            false
        }
    }

    pub(super) fn fail_current_camera_preview(&mut self) {
        if let Some(owner) = ProjectOwner::from_ref(&self.last_project["context"]["project"]) {
            self.fail_camera_preview_for(&owner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: &str, activation: &str) -> Value {
        json!({"project_id":id,"activation":activation,"revision":"1"})
    }

    #[test]
    fn camera_intent_requires_the_reload_job_and_exact_target_activation() {
        let source = ProjectOwner::from_ref(&project("a", "1")).unwrap();
        let target = project("b", "2");
        let mut intent = CameraPreservation::new("reload-1".into(), source);
        assert!(!intent.set_target("other-job", &target));
        assert!(intent.set_target("reload-1", &target));
        assert!(intent.target_matches(&ProjectOwner::from_ref(&target).unwrap()));
        assert!(!intent.target_matches(&ProjectOwner::from_ref(&project("b", "3")).unwrap()));
    }

    #[test]
    fn old_preview_errors_and_old_job_failures_do_not_clear_a_new_intent() {
        let source = ProjectOwner::from_ref(&project("a", "1")).unwrap();
        let mut intent = CameraPreservation::new("reload-1".into(), source);
        let target = project("b", "2");
        intent.set_target("reload-1", &target);
        let unrelated = ProjectOwner::from_ref(&project("c", "3")).unwrap();
        assert!(!intent.fail_preview_for(&unrelated));
        assert!(!intent.fail_reload("reload-0"));
        assert!(intent.fail_preview_for(&ProjectOwner::from_ref(&target).unwrap()));
        assert!(intent.fail_reload("reload-1"));
    }

    #[test]
    fn a_pending_intent_does_not_match_an_unrelated_project() {
        let source = ProjectOwner::from_ref(&project("a", "1")).unwrap();
        let intent = CameraPreservation::new("reload-1".into(), source);
        let other = ProjectOwner::from_ref(&project("c", "3")).unwrap();
        assert!(!intent.target_matches(&other));
    }
}
