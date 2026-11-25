//! Optimistic feedback while the display thread submits a render request.
use std::collections::HashSet;
use std::time::{Duration, Instant};

const START_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Default)]
pub(super) struct RenderStart {
    known_render_jobs: HashSet<String>,
    deadline: Option<Instant>,
}

impl RenderStart {
    pub(super) fn begin(&mut self, jobs: &serde_json::Value, now: Instant) {
        self.known_render_jobs = render_job_ids(jobs);
        self.deadline = Some(now + START_TIMEOUT);
    }

    pub(super) fn active(&self) -> bool {
        self.deadline.is_some()
    }

    pub(super) fn observe_jobs(&mut self, jobs: &serde_json::Value, now: Instant) -> bool {
        if !self.active() {
            return false;
        }
        let arrived = render_job_ids(jobs)
            .iter()
            .any(|job_id| !self.known_render_jobs.contains(job_id));
        if arrived || self.deadline.is_some_and(|deadline| now >= deadline) {
            self.clear();
            true
        } else {
            false
        }
    }

    pub(super) fn request_failed(&mut self) -> bool {
        let was_active = self.active();
        self.clear();
        was_active
    }

    pub(super) fn expire(&mut self, now: Instant) -> bool {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.clear();
            true
        } else {
            false
        }
    }

    fn clear(&mut self) {
        self.deadline = None;
        self.known_render_jobs.clear();
    }
}

fn render_job_ids(jobs: &serde_json::Value) -> HashSet<String> {
    jobs["jobs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|job| job["kind"] == "render")
        .filter_map(|job| job["job_id"].as_str().map(str::to_owned))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::RenderStart;
    use std::time::{Duration, Instant};

    #[test]
    fn stays_starting_until_a_new_render_job_arrives() {
        let now = Instant::now();
        let existing = serde_json::json!({"jobs":[
            {"kind":"render","job_id":"old"}, {"kind":"ipr","job_id":"ipr"}
        ]});
        let mut state = RenderStart::default();
        state.begin(&existing, now);
        assert!(state.active());
        assert!(!state.observe_jobs(&existing, now + Duration::from_secs(1)));
        assert!(state.observe_jobs(
            &serde_json::json!({"jobs":[
                {"kind":"render","job_id":"old"}, {"kind":"render","job_id":"new"}
            ]}),
            now + Duration::from_secs(1)
        ));
        assert!(!state.active());
    }

    #[test]
    fn request_error_and_timeout_release_starting_state() {
        let now = Instant::now();
        let jobs = serde_json::json!({"jobs":[]});
        let mut state = RenderStart::default();
        state.begin(&jobs, now);
        assert!(state.request_failed());
        assert!(!state.active());
        state.begin(&jobs, now);
        assert!(state.expire(now + Duration::from_secs(30)));
        assert!(!state.active());
    }
}
