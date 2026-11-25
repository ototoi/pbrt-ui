//! Resident capture adapter session. One spawned `pbrt-ui-capture --server`
//! child owns the GPU device; requests are newline-delimited JSON with a
//! reader thread so callers can bound their wait. Falls back to one-shot
//! adapter runs whenever the session cannot be established or breaks.
use super::*;
use std::{
    io::{BufRead, BufReader, Write},
    process::{ChildStdin, Command, Stdio},
    sync::mpsc,
    time::Duration,
};
const READY_TIMEOUT: Duration = Duration::from_secs(3);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
/// Pause between cancellation checks while waiting for a render reply.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

pub struct CaptureDaemon {
    group: super::child_group::ChildGroup,
    stdin: ChildStdin,
    replies: mpsc::Receiver<String>,
}

impl CaptureDaemon {
    /// Spawn `program --server`, wait for the ready line, and confirm the
    /// protocol handshake. Any failure leaves the child killed.
    pub fn spawn(program: &Path) -> Result<Self, ErrorPayload> {
        let mut command = Command::new(program);
        command
            .arg("--server")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut group = super::child_group::ChildGroup::spawn(command).map_err(fail)?;
        let child = &mut group.child;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| fail("no daemon stdout"))?;
        let stdin = child.stdin.take().ok_or_else(|| fail("no daemon stdin"))?;
        let (tx, replies) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                if reader
                    .read_line(&mut line)
                    .ok()
                    .filter(|count| *count > 0)
                    .is_none()
                {
                    return;
                }
                if tx.send(line.trim_end().to_owned()).is_err() {
                    return;
                }
            }
        });
        let mut session = Self {
            group,
            stdin,
            replies,
        };
        let ready = session.next_line(READY_TIMEOUT)?;
        let parsed: Value = serde_json::from_str(&ready)
            .map_err(|_| fail("capture daemon sent a malformed ready line"))?;
        if parsed["ready"] != true || parsed["protocol"] != 1 {
            session.kill();
            return Err(fail("capture daemon readiness handshake failed"));
        }
        Ok(session)
    }

    /// One capture request; the reply reports where the PNG and pick buffer
    /// were written. `geometry_key` skipping happens inside the daemon. The
    /// wait observes cancellation like the one-shot adapter run does; a
    /// cancelled request kills the daemon session immediately so cancelled
    /// jobs neither block on a stuck adapter nor retain late artifacts.
    pub fn capture(
        &mut self,
        task: &crate::state::LoadTask,
        geometry_key: &str,
        spec: &CaptureSpec,
        output: &Path,
    ) -> Result<Value, ErrorPayload> {
        let mut line = serde_json::to_string(&json!({
            "geometry_key": geometry_key,
            "spec": spec,
            "output": output.display().to_string(),
        }))
        .map_err(fail)?;
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|_| self.stdin.flush())
            .map_err(|error| {
                self.kill();
                fail(format!("capture daemon write failed: {error}"))
            })?;
        let reply = self
            .next_reply(task)?
            .ok_or_else(|| ErrorPayload::new(ErrorCode::Busy, "job cancelled"))?;
        let parsed: Value =
            serde_json::from_str(&reply).map_err(|_| fail("malformed daemon reply"))?;
        if parsed["ok"] == true {
            return Ok(parsed);
        }
        Err(ErrorPayload::new(
            ErrorCode::OutputInvalid,
            format!("capture adapter: {}", parsed["error"]),
        ))
    }

    fn next_line(&mut self, timeout: Duration) -> Result<String, ErrorPayload> {
        match self.replies.recv_timeout(timeout) {
            Ok(line) => Ok(line),
            Err(_) => {
                self.kill();
                Err(fail("capture adapter timed out waiting for a render reply"))
            }
        }
    }

    /// Next request reply line, or `None` when the job was cancelled while
    /// waiting (which also abandons the daemon session).
    fn next_reply(
        &mut self,
        task: &crate::state::LoadTask,
    ) -> Result<Option<String>, ErrorPayload> {
        let deadline = std::time::Instant::now() + REQUEST_TIMEOUT;
        loop {
            if task.is_cancelled() {
                self.kill();
                return Ok(None);
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                self.kill();
                return Err(fail("capture adapter timed out waiting for a render reply"));
            }
            match self.replies.recv_timeout(remaining.min(POLL_INTERVAL)) {
                Ok(line) => return Ok(Some(line)),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.kill();
                    return Err(fail("capture adapter exited before replying to a render"));
                }
            }
        }
    }

    /// Kill the adapter now; safe to call more than once.
    pub fn kill(&mut self) {
        self.group.kill();
        let _ = self.group.child.wait();
    }
}
impl Drop for CaptureDaemon {
    fn drop(&mut self) {
        // The daemon lives as long as requests come in; a core host shutdown
        // takes it down via PDEATHSIG (the Job Object on Windows) as well.
        self.kill();
    }
}
