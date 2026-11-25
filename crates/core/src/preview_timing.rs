//! Opt-in timing records for the serial project I/O executor.
//!
//! Off by default: set `PBRT_UI_PREVIEW_TIMING=1` to emit one short stderr
//! line per stage of a `project.preview` job, in the same family as the
//! desktop backend's records,
//! so scratch measurements pair up across the pipeline. Lines are keyed by
//! job id only, never by paths or scene content. The helper is a deliberate
//! tiny duplicate: the desktop one carries preview-key and frame fields
//! core does not have, and core must not depend on desktop.
use std::time::{Duration, Instant};

/// Enables the records; unset or `0` leaves the executor silent.
const SWITCH: &str = "PBRT_UI_PREVIEW_TIMING";

/// Record the duration of one named stage. Cheap when disabled.
pub(crate) fn record(stage: &str, started: Instant, job_id: protocol::JobId) {
    emit(stage, started.elapsed(), job_id, "");
}

/// Record the total duration of one synchronous method,
/// which has no job to name; the line carries a `-` in the job position so
/// it stays column-aligned with the per-stage records.
pub(crate) fn record_method(stage: &str, started: Instant) {
    emit(stage, started.elapsed(), "-", "");
}

/// Record a stage that consults the geometry cache, with this job's share
/// of the cache's cumulative hit/miss counters on the line, and of the
/// Count of PLY files the stage actually opened. The cache lock is
/// held for the whole stage, so the deltas are exactly this job's.
pub(crate) fn record_geometry_cache(
    stage: &str,
    started: Instant,
    job_id: protocol::JobId,
    hits: usize,
    misses: usize,
    asset_reads: usize,
) {
    emit(
        stage,
        started.elapsed(),
        job_id,
        &format!(" cache_hits={hits} cache_misses={misses} asset_reads={asset_reads}"),
    );
}

fn emit(stage: &str, duration: Duration, job_id: impl std::fmt::Display, extra: &str) {
    if !enabled() {
        return;
    }
    let stamp = std::time::SystemTime::UNIX_EPOCH
        .elapsed()
        .map(|since| since.as_millis())
        .unwrap_or_default();
    eprintln!("[preview-timing] t={stamp}ms job={job_id} {stage} {duration:?}{extra}");
}

fn enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| switch(std::env::var(SWITCH).ok().as_deref()))
}

/// The switch's policy on `PBRT_UI_PREVIEW_TIMING`'s value: anything but
/// unset or `0` enables — the same rule as the desktop records.
fn switch(value: Option<&str>) -> bool {
    value.is_some_and(|value| value != "0")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_switch_is_off_when_unset_or_zero() {
        assert!(!switch(None));
        assert!(!switch(Some("0")));
    }

    #[test]
    fn timing_switch_accepts_any_other_value() {
        assert!(switch(Some("1")));
        assert!(switch(Some("true")));
        // Matched with the desktop helper: any value other than "0" enables.
        assert!(switch(Some("")));
    }
}
