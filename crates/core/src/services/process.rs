//! External process lifetime, cancellation and descendant reaping.
use super::*;
pub(super) fn run_process(
    mut command: Command,
    task: &LoadTask,
    log: &Path,
) -> Result<(), ErrorPayload> {
    let file = fs::File::create(log).map_err(fail)?;
    command
        .stdout(Stdio::from(file.try_clone().map_err(fail)?))
        .stderr(Stdio::from(file))
        .stdin(Stdio::null());
    let mut group = super::child_group::ChildGroup::spawn(command).map_err(fail)?;
    let mut terminate = None;
    loop {
        if let Some(status) = group.child.try_wait().map_err(fail)? {
            // Reap descendants left by a renderer/capture adapter as well.
            group.kill();
            if task.is_cancelled() {
                return Err(ErrorPayload::new(ErrorCode::Busy, "job cancelled"));
            }
            if status.success() {
                return Ok(());
            }
            let message = match last_error_line(log) {
                Some(line) => format!("process exited with {status}; {line}"),
                None => format!("process exited with {status}; see job log"),
            };
            return Err(ErrorPayload::new(ErrorCode::WorkerCrashed, message));
        }
        if task.is_cancelled() {
            let started = terminate.get_or_insert_with(|| {
                group.terminate();
                std::time::Instant::now()
            });
            if started.elapsed() > Duration::from_secs(2) {
                group.kill();
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn last_error_line(log: &Path) -> Option<String> {
    fs::read_to_string(log)
        .ok()?
        .lines()
        .filter(|line| line.contains("[ERROR]") || line.trim_start().starts_with("ERROR"))
        .map(str::trim)
        .last()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::last_error_line;
    #[test]
    fn extracts_last_error_line_from_renderer_log() {
        let directory = tempfile::tempdir().unwrap();
        let log = directory.path().join("render.log");
        std::fs::write(&log, "info\n[ERROR] first\nwarn\n[ERROR] final reason\n").unwrap();
        assert_eq!(
            last_error_line(&log).as_deref(),
            Some("[ERROR] final reason")
        );
    }
}
