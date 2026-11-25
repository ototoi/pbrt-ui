//! Jobs panel rows: import, save, export, render and IPR jobs, newest first.
//! Pure so it can be tested without a window.
use gpui_kit::{Rgba, rgb};
use std::collections::HashMap;
use std::time::{Duration, Instant};
/// Background preview exports are an implementation detail of the Scene view.
const HIDDEN_KINDS: &[&str] = &["project.preview", "pick", "capture"];
const MAX_ROWS: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobRow {
    pub job_id: String,
    pub kind: String,
    pub state: String,
    pub phase: String,
    pub detail: String,
    pub active: bool,
}

/// Newest first, at most [`MAX_ROWS`].
pub fn job_rows(jobs: &serde_json::Value) -> Vec<JobRow> {
    jobs["jobs"]
        .as_array()
        .into_iter()
        .flatten()
        .rev()
        .filter(|job| !HIDDEN_KINDS.contains(&job["kind"].as_str().unwrap_or("")))
        .filter_map(|job| {
            let state = job["state"].as_str()?.to_owned();
            let detail = job["error"]["message"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| {
                    job["root_path"]
                        .as_str()
                        .filter(|path| !path.is_empty())
                        .map(|path| path.rsplit('/').next().unwrap_or(path).to_owned())
                })
                .unwrap_or_default();
            Some(JobRow {
                job_id: job["job_id"].as_str()?.to_owned(),
                kind: job["kind"].as_str().unwrap_or("job").to_owned(),
                active: matches!(state.as_str(), "queued" | "running" | "cancelling"),
                phase: job["phase"].as_str().unwrap_or("").to_owned(),
                state,
                detail,
            })
        })
        .take(MAX_ROWS)
        .collect()
}

/// Remember when the desktop first saw each active job; the core reports no
/// timestamps. Settled jobs keep their final elapsed time.
pub fn track_elapsed(
    rows: &[JobRow],
    seen: &mut HashMap<String, (Instant, Option<Duration>)>,
    now: Instant,
) {
    for row in rows {
        let entry = seen.entry(row.job_id.clone()).or_insert((now, None));
        if !row.active && entry.1.is_none() {
            entry.1 = Some(now.duration_since(entry.0));
        }
    }
    seen.retain(|id, _| rows.iter().any(|row| &row.job_id == id));
}

pub fn elapsed_label(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    }
}

pub fn foreground_debounce_elapsed(started: Instant, now: Instant) -> bool {
    now.duration_since(started) >= Duration::from_millis(300)
}

/// The icon shown left of a job's state text in the Jobs panel.
///
/// Active states spin; the settled states
/// get a result icon. Unknown states keep the panel's active-state fallback
/// and spin, matching the state colours below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobStateIcon {
    Spinner,
    Check,
    Cross,
    CircleSlash,
}

pub fn state_icon(state: &str) -> JobStateIcon {
    match state {
        "succeeded" => JobStateIcon::Check,
        "failed" => JobStateIcon::Cross,
        "cancelled" => JobStateIcon::CircleSlash,
        _ => JobStateIcon::Spinner,
    }
}

/// The colour of a job's state text and its icon, as the Jobs panel shows
/// them today: reddish orange for failures, grey for cancellations, green
/// for successes, yellow while active or unknown.
pub fn state_color(state: &str) -> Rgba {
    match state {
        "failed" => rgb(0xd99a74),
        "cancelled" | "cancelling" => rgb(0x9aa0aa),
        "succeeded" => rgb(0x8fc79a),
        _ => rgb(0xe0b050),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn job_rows_are_newest_first_and_hide_preview_exports() {
        let jobs = json!({"jobs":[
            {"job_id":"a","kind":"project.import","state":"succeeded","phase":"done","root_path":"/s/box.pbrt"},
            {"job_id":"b","kind":"project.preview","state":"running","phase":"export"},
            {"job_id":"c","kind":"render","state":"running","phase":"rendering","root_path":""},
            {"job_id":"d","kind":"project.save","state":"failed","phase":"write","error":{"message":"disk full"}}
        ]});
        let rows = job_rows(&jobs);
        assert_eq!(
            rows.iter()
                .map(|row| row.job_id.as_str())
                .collect::<Vec<_>>(),
            ["d", "c", "a"]
        );
        assert_eq!(rows[0].detail, "disk full");
        assert!(rows[1].active);
        assert_eq!(rows[2].detail, "box.pbrt");
        assert!(!rows[2].active);
    }

    #[test]
    fn elapsed_time_freezes_when_a_job_settles() {
        let start = Instant::now();
        let mut seen = HashMap::new();
        let mut row = JobRow {
            job_id: "a".into(),
            kind: "render".into(),
            state: "running".into(),
            phase: String::new(),
            detail: String::new(),
            active: true,
        };
        track_elapsed(std::slice::from_ref(&row), &mut seen, start);
        row.active = false;
        track_elapsed(
            std::slice::from_ref(&row),
            &mut seen,
            start + Duration::from_secs(75),
        );
        assert_eq!(seen["a"].1, Some(Duration::from_secs(75)));
        assert_eq!(elapsed_label(Duration::from_secs(75)), "1m 15s");
        track_elapsed(&[], &mut seen, start);
        assert!(seen.is_empty());
    }

    #[test]
    fn foreground_job_waits_300ms_before_becoming_visible() {
        let start = Instant::now();
        assert!(!foreground_debounce_elapsed(
            start,
            start + Duration::from_millis(299)
        ));
        assert!(foreground_debounce_elapsed(
            start,
            start + Duration::from_millis(300)
        ));
    }

    #[test]
    fn active_states_spin_and_settled_states_get_result_icons() {
        assert_eq!(state_icon("running"), JobStateIcon::Spinner);
        assert_eq!(state_icon("queued"), JobStateIcon::Spinner);
        assert_eq!(state_icon("cancelling"), JobStateIcon::Spinner);
        assert_eq!(state_icon("succeeded"), JobStateIcon::Check);
        assert_eq!(state_icon("failed"), JobStateIcon::Cross);
        assert_eq!(state_icon("cancelled"), JobStateIcon::CircleSlash);
        // Unknown states keep the panel's active colour fallback, so they spin.
        assert_eq!(state_icon("unknown-state"), JobStateIcon::Spinner);
    }

    #[test]
    fn state_icon_colors_match_the_state_text_colors() {
        assert_eq!(state_color("failed"), rgb(0xd99a74));
        assert_eq!(state_color("cancelled"), rgb(0x9aa0aa));
        assert_eq!(state_color("cancelling"), rgb(0x9aa0aa));
        assert_eq!(state_color("succeeded"), rgb(0x8fc79a));
        assert_eq!(state_color("running"), rgb(0xe0b050));
        assert_eq!(state_color("queued"), rgb(0xe0b050));
        assert_eq!(state_color("unknown-state"), rgb(0xe0b050));
    }
}
