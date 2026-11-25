//! Process/capture executor and request completion.
use super::*;
pub(super) fn spawn_service(
    service_rx: Receiver<ServiceWork>,
    done_tx: mpsc::Sender<(protocol::JobId, Option<crate::services::WorkResult>)>,
    progress: mpsc::Sender<(protocol::JobId, protocol::jobs::Artifact)>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        while let Ok(work) = service_rx.recv() {
            let id = work.work.task.job_id;
            let _ = done_tx.send((id, None));
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crate::services::execute(work.work, progress.clone())
            }))
            .unwrap_or_else(|_| crate::services::WorkResult {
                result: Err(ErrorPayload::new(
                    ErrorCode::WorkerCrashed,
                    "service adapter panicked",
                )),
                artifacts: Vec::new(),
            });
            let _ = done_tx.send((id, Some(result)));
        }
    })
}
