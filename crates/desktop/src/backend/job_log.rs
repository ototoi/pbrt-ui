//! Console lines for job ends: one line per job the display watched settle.
//! Pure and window-free so the rules stay testable.
use super::*;
use std::collections::HashMap;

/// Job kinds the user starts; every end is logged in the Console. The
/// automatic kinds — project.preview, capture, pick, and anything not
/// listed here — log failures only, so the per-edit jobs do not flood the
/// Console.
const USER_KINDS: &[&str] = &[
    "project.open",
    "project.import",
    "project.import_scene",
    "project.import_resource",
    "project.create",
    "project.save",
    "project.export",
    "project.set_parameter_values",
    "export",
    "render",
    "ipr",
];

/// A job that moved to a terminal state in the freshly polled list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct JobEnd<'a> {
    pub(super) job_id: &'a str,
    pub(super) kind: &'a str,
    /// The terminal state: succeeded, failed or cancelled.
    pub(super) state: &'a str,
    pub(super) error: Option<&'a str>,
}

fn terminal_state(job: &Value) -> Option<&str> {
    let state = job["state"].as_str()?;
    matches!(state, "succeeded" | "failed" | "cancelled").then_some(state)
}

/// The jobs that settled since the last-seen list: unfinished before,
/// terminal now. A job that appears settled in the first list after a
/// (re)connect is not an observed end, so startup never replays history;
/// a job that leaves the list without a state is not one either.
pub(super) fn settled_jobs<'a>(previous: &Value, current: &'a Value) -> Vec<JobEnd<'a>> {
    let previous_states: HashMap<&str, &str> = previous["jobs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|job| Some((job["job_id"].as_str()?, job["state"].as_str()?)))
        .collect();
    let Some(current_jobs) = current["jobs"].as_array() else {
        return Vec::new();
    };
    let mut settled = Vec::new();
    for job in current_jobs {
        let (Some(id), Some(state), Some(kind)) = (
            job["job_id"].as_str(),
            terminal_state(job),
            job["kind"].as_str(),
        ) else {
            continue;
        };
        // Only a watched transition counts as the job's end.
        if !matches!(
            previous_states.get(id).copied(),
            Some("queued" | "running" | "cancelling")
        ) {
            continue;
        }
        settled.push(JobEnd {
            job_id: id,
            kind,
            state,
            error: job["error"]["message"].as_str(),
        });
    }
    settled
}

/// Whether — and as what text — a settled job end reaches the Console.
/// Failures are logged for every kind, with their error text; outcomes
/// (succeeded, cancelled) are logged for user-started kinds only. A
/// cancelled IPR job while IPR stays enabled is per-edit relaunch churn —
/// the successor debounces behind the trigger and a preempting final render
/// holds it back for the whole render — so `ipr_enabled` decides: turning
/// IPR off is the operation's end and logs once.
pub(super) fn console_end_line(
    job: &JobEnd,
    elapsed: Option<Duration>,
    ipr_enabled: bool,
) -> Option<String> {
    if job.kind == "ipr" && job.state == "cancelled" && job.error.is_none() && ipr_enabled {
        return None;
    }
    if !USER_KINDS.contains(&job.kind) && job.state != "failed" {
        return None;
    }
    let mut line = format!("{} · {}", job.kind, job.state);
    if let Some(elapsed) = elapsed {
        line.push_str(&format!(" ({})", elapsed_text(elapsed)));
    }
    if let Some(error) = job.error {
        line.push_str(&format!(" · {error}"));
    }
    Some(line)
}

/// The duration shown on a Console line: one decimal below a minute,
/// whole minutes above, whole hours above that.
pub(super) fn elapsed_text(elapsed: Duration) -> String {
    if elapsed.as_secs_f64() < 60. {
        format!("{:.1} s", elapsed.as_secs_f64())
    } else {
        let total = elapsed.as_secs();
        let (hours, minutes) = (total / 3600, (total % 3600) / 60);
        let seconds = total % 60;
        if hours > 0 {
            format!("{hours}h {minutes:02}m {seconds:02}s")
        } else {
            format!("{minutes}m {seconds:02}s")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn settled_jobs_are_only_the_observed_transitions() {
        let previous = json!({"jobs":[
            {"job_id":"a","kind":"render","state":"running","phase":"rendering"},
            {"job_id":"b","kind":"export","state":"succeeded","phase":"complete"},
            {"job_id":"c","kind":"project.preview","state":"running","phase":"export"}
        ]});
        let current = json!({"jobs":[
            {"job_id":"d","kind":"project.open","state":"succeeded","phase":"done"},
            {"job_id":"a","kind":"render","state":"failed","phase":"failed",
             "error":{"message":"renderer exited with code 1"}},
            {"job_id":"b","kind":"export","state":"succeeded","phase":"complete"},
            {"job_id":"c","kind":"project.preview","state":"running","phase":"export"}
        ]});
        let settled = settled_jobs(&previous, &current);
        // Only `a` moved from active to terminal: `b` was already settled,
        // `d` was never seen active.
        assert_eq!(
            settled,
            vec![JobEnd {
                job_id: "a",
                kind: "render",
                state: "failed",
                error: Some("renderer exited with code 1"),
            }]
        );
        // A job that leaves the list without a terminal state is not an
        // observed end either.
        let vanished = json!({"jobs":[]});
        assert!(settled_jobs(&current, &vanished).is_empty());
    }

    #[test]
    fn an_ipr_cancel_while_ipr_stays_on_is_relaunch_churn() {
        // Edits and preempting final renders cancel the running IPR job and
        // re-arm; the successor debounces hundreds of milliseconds behind
        // the trigger, so at settle time no
        // successor is around. A cancel while IPR stays on is the churn the
        // Console must not flood with; IPR turned off is the operation's end
        // and logs once.
        let churned = JobEnd {
            job_id: "a",
            kind: "ipr",
            state: "cancelled",
            error: None,
        };
        assert_eq!(
            console_end_line(&churned, Some(Duration::from_secs(2)), true),
            None
        );
        let toggled_off = JobEnd {
            job_id: "a",
            kind: "ipr",
            state: "cancelled",
            error: None,
        };
        assert_eq!(
            console_end_line(&toggled_off, Some(Duration::from_secs(5)), false),
            Some("ipr · cancelled (5.0 s)".to_owned())
        );
    }

    #[test]
    fn a_failed_ipr_end_logs_even_while_ipr_is_on() {
        // Failures log for every kind, with their error text — even with
        // IPR still enabled and a relaunch coming.
        let failed = JobEnd {
            job_id: "b",
            kind: "ipr",
            state: "failed",
            error: Some("renderer crashed"),
        };
        assert_eq!(
            console_end_line(&failed, None, true),
            Some("ipr · failed · renderer crashed".to_owned())
        );
    }

    #[test]
    fn user_operations_log_every_end() {
        let ended = |kind: &'static str, state: &'static str| JobEnd {
            job_id: "a",
            kind,
            state,
            error: None,
        };
        assert_eq!(
            console_end_line(
                &ended("project.open", "succeeded"),
                Some(Duration::from_millis(1180)),
                false
            ),
            Some("project.open · succeeded (1.2 s)".to_owned())
        );
        assert_eq!(
            console_end_line(
                &ended("render", "cancelled"),
                Some(Duration::from_secs(5)),
                false
            ),
            Some("render · cancelled (5.0 s)".to_owned())
        );
        assert_eq!(
            console_end_line(
                &ended("export", "failed"),
                Some(Duration::from_millis(400)),
                false
            ),
            Some("export · failed (0.4 s)".to_owned())
        );
        // Every user-started kind logs: the io operations (open, import,
        // add-scene/import-resource, create, save, export a copy, array
        // apply), artifact export, render, IPR.
        assert_eq!(
            console_end_line(&ended("project.import_scene", "succeeded"), None, false),
            Some("project.import_scene · succeeded".to_owned())
        );
        assert_eq!(
            console_end_line(&ended("project.import_resource", "succeeded"), None, false),
            Some("project.import_resource · succeeded".to_owned())
        );
        assert_eq!(
            console_end_line(&ended("project.export", "succeeded"), None, false),
            Some("project.export · succeeded".to_owned())
        );
        assert_eq!(
            console_end_line(
                &ended("project.set_parameter_values", "succeeded"),
                None,
                false
            ),
            Some("project.set_parameter_values · succeeded".to_owned())
        );
        // Without a measured duration the line stands without one.
        assert_eq!(
            console_end_line(&ended("project.save", "succeeded"), None, false),
            Some("project.save · succeeded".to_owned())
        );
    }

    #[test]
    fn failures_log_with_the_error_text_whatever_the_kind() {
        let failed_preview = JobEnd {
            job_id: "a",
            kind: "project.preview",
            state: "failed",
            error: Some("export failed: no renderer"),
        };
        assert_eq!(
            console_end_line(&failed_preview, Some(Duration::from_millis(830)), false),
            Some("project.preview · failed (0.8 s) · export failed: no renderer".to_owned())
        );
        // A failure without a message still names its kind.
        let failed_export = JobEnd {
            job_id: "b",
            kind: "export",
            state: "failed",
            error: None,
        };
        assert_eq!(
            console_end_line(&failed_export, None, false),
            Some("export · failed".to_owned())
        );
        // Kinds the UI does not know log failures too.
        let failed_new_kind = JobEnd {
            job_id: "c",
            kind: "project.reindex",
            state: "failed",
            error: Some("boom"),
        };
        assert_eq!(
            console_end_line(&failed_new_kind, None, false),
            Some("project.reindex · failed · boom".to_owned())
        );
    }

    #[test]
    fn automatic_jobs_log_failures_only() {
        let ended = |kind: &'static str, state: &'static str| JobEnd {
            job_id: "a",
            kind,
            state,
            error: None,
        };
        assert_eq!(
            console_end_line(&ended("project.preview", "succeeded"), None, false),
            None
        );
        assert_eq!(
            console_end_line(&ended("capture", "succeeded"), None, false),
            None
        );
        assert_eq!(
            console_end_line(&ended("pick", "succeeded"), None, false),
            None
        );
        assert_eq!(
            console_end_line(&ended("project.preview", "cancelled"), None, false),
            None
        );
        assert_eq!(
            console_end_line(&ended("project.reindex", "succeeded"), None, false),
            None
        );
    }

    #[test]
    fn elapsed_keeps_one_decimal_below_a_minute_and_minutes_above() {
        assert_eq!(elapsed_text(Duration::from_millis(1183)), "1.2 s");
        assert_eq!(elapsed_text(Duration::from_secs(4)), "4.0 s");
        assert_eq!(elapsed_text(Duration::from_secs(59)), "59.0 s");
        assert_eq!(elapsed_text(Duration::from_millis(75_400)), "1m 15s");
        assert_eq!(elapsed_text(Duration::from_secs(60)), "1m 00s");
        assert_eq!(elapsed_text(Duration::from_secs(3_725)), "1h 02m 05s");
    }
}
