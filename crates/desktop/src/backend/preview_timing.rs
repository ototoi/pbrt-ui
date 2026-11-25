//! Opt-in timing records for native Scene preview work.
//!
//! Off by default: set `PBRT_UI_PREVIEW_TIMING=1` to emit one short stderr
//! line per stage. Lines are keyed by preview key plus job/frame ids only,
//! never by paths or scene content, so scratch measurements can be matched
//! against Core job timestamps without dragging diagnostics into user flows.
use std::time::{Duration, Instant};

/// Record the duration of one named stage. Cheap when disabled.
pub(crate) fn record(
    stage: &str,
    started: Instant,
    job_id: Option<&str>,
    preview_key: &str,
    frame_id: Option<u64>,
) {
    record_duration(stage, started.elapsed(), job_id, preview_key, frame_id);
}

/// Record a duration the caller already holds — a stage the scene-view
/// renderer measured and reported back. Same one-line form as `record`.
pub(crate) fn record_duration(
    stage: &str,
    duration: Duration,
    job_id: Option<&str>,
    preview_key: &str,
    frame_id: Option<u64>,
) {
    if !enabled() {
        return;
    }
    let stamp = std::time::SystemTime::UNIX_EPOCH
        .elapsed()
        .map(|since| since.as_millis())
        .unwrap_or_default();
    let job = job_id.unwrap_or("-");
    let frame = frame_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "-".into());
    eprintln!(
        "[preview-timing] t={stamp}ms key={preview_key} job={job} frame={frame} {stage} {duration:?}"
    );
}

fn enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("PBRT_UI_PREVIEW_TIMING").is_ok_and(|value| value != "0"))
}
