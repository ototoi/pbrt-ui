//! Serial core driver; phase ordering and shutdown acknowledgements.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// One per admitted preview, so a long decode (a large image asset in
/// Contents) never holds the Inspector's request behind it.
const PREVIEW_WORKERS: usize = 2;

struct PreviewWork {
    request_id: String,
    run: Box<dyn FnOnce() -> Response + Send>,
    reply: Sender<Response>,
}

fn spawn_preview_worker(
    receiver: Arc<Mutex<Receiver<PreviewWork>>>,
    done: Sender<()>,
    stopping: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        loop {
            let next = receiver
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .recv();
            let Ok(work) = next else {
                break;
            };
            let request_id = work.request_id.clone();
            let response = if stopping.load(Ordering::Acquire) {
                Response::err(
                    request_id,
                    ErrorPayload::new(ErrorCode::IoError, "host is shutting down"),
                )
            } else {
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work.run)) {
                    Ok(response) => response,
                    Err(_) => Response::err(
                        request_id,
                        ErrorPayload::new(ErrorCode::WorkerCrashed, "preview worker panicked"),
                    ),
                }
            };
            let _ = work.reply.send(response);
            let _ = done.send(());
        }
    })
}

pub(super) fn run(mut state: CoreState, commands: Receiver<Command>) {
    let (service_tx, service_rx) = mpsc::sync_channel::<ServiceWork>(state.limits.command_queue);
    let (render_tx, render_rx) = mpsc::sync_channel::<ServiceWork>(state.limits.command_queue);
    let (done_tx, done_rx) = mpsc::channel();
    let (progress_tx, progress_rx) = mpsc::channel();
    let service_thread = spawn_service(service_rx, done_tx.clone(), progress_tx.clone());
    let render_thread = spawn_service(render_rx, done_tx, progress_tx);
    let (project_tx, project_rx) = mpsc::sync_channel(state.limits.command_queue);
    let (project_done_tx, project_done) = mpsc::channel();
    let project_thread = thread::spawn(move || crate::project_io::run(project_rx, project_done_tx));
    let (preview_tx, preview_rx) = mpsc::sync_channel::<PreviewWork>(1);
    let (preview_done_tx, preview_done_rx) = mpsc::channel();
    let preview_stopping = Arc::new(AtomicBool::new(false));
    let preview_rx = Arc::new(Mutex::new(preview_rx));
    let preview_threads: Vec<_> = (0..PREVIEW_WORKERS)
        .map(|_| {
            spawn_preview_worker(
                preview_rx.clone(),
                preview_done_tx.clone(),
                preview_stopping.clone(),
            )
        })
        .collect();
    let mut preview_in_flight = 0usize;
    let mut waiting: Vec<Waiting> = Vec::new();
    loop {
        while preview_done_rx.try_recv().is_ok() {
            preview_in_flight = preview_in_flight.saturating_sub(1);
        }
        while let Ok(completion) = project_done.try_recv() {
            match completion {
                crate::project_io::Completion::Started(id) => state.service_started(id),
                crate::project_io::Completion::Finished(id, result) => {
                    state.project_io_finished(id, result)
                }
                crate::project_io::Completion::DefaultSceneFailed {
                    store_id,
                    scene,
                    error,
                } => state.project_default_scene_failed(store_id, scene, error),
            }
        }
        while let Some(work) = state.pop_project_io() {
            match project_tx.try_send(work) {
                Ok(()) => {}
                Err(TrySendError::Full(work)) => {
                    state.return_project_io(work);
                    break;
                }
                Err(TrySendError::Disconnected(work)) => {
                    if let crate::state::project_commands::IoCommand::Run(task) = work {
                        state.project_io_finished(
                            task.control.job_id,
                            Err(ErrorPayload::new(
                                ErrorCode::WorkerCrashed,
                                "project I/O executor stopped",
                            )),
                        );
                    }
                }
            }
        }
        while let Ok((id, artifact)) = progress_rx.try_recv() {
            state.service_progress(id, artifact);
        }
        while let Ok((id, result)) = done_rx.try_recv() {
            if let Some(result) = result {
                state.service_finished(id, result);
            } else {
                state.service_started(id);
            }
        }
        // Export completion can enqueue a render without another client request.
        state.tick_ipr(Instant::now());
        let services: Vec<_> = state.take_services().collect();
        for work in services {
            let id = work.task.job_id;
            let sender = if matches!(work.work, crate::services::Work::ProjectRender { .. }) {
                &render_tx
            } else {
                &service_tx
            };
            if sender.try_send(ServiceWork { work }).is_err() {
                state.service_finished(
                    id,
                    crate::services::WorkResult {
                        result: Err(ErrorPayload::new(
                            ErrorCode::Busy,
                            "service queue unavailable",
                        )),
                        artifacts: Vec::new(),
                    },
                );
            }
        }
        let mut index = 0;
        while index < waiting.len() {
            let response = state.handle(waiting[index].request.clone());
            if response.error.is_some()
                || !response.result.as_ref().unwrap()["events"]
                    .as_array()
                    .unwrap()
                    .is_empty()
                || Instant::now() >= waiting[index].deadline
            {
                let pending = waiting.swap_remove(index);
                let _ = pending.reply.send(response);
            } else {
                index += 1;
            }
        }
        match commands.recv_timeout(Duration::from_millis(10)) {
            Ok(Command::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            #[cfg(test)]
            Ok(Command::TestPreview(run, reply)) => {
                let request_id = format!("test-preview-{preview_in_flight}");
                if preview_in_flight >= PREVIEW_WORKERS {
                    let _ = reply.send(Response::err(
                        request_id,
                        ErrorPayload::new(ErrorCode::Busy, "preview worker queue is full"),
                    ));
                    continue;
                }
                match preview_tx.try_send(PreviewWork {
                    request_id: request_id.clone(),
                    run,
                    reply,
                }) {
                    Ok(()) => preview_in_flight += 1,
                    Err(TrySendError::Full(work)) => {
                        let _ = work.reply.send(Response::err(
                            work.request_id,
                            ErrorPayload::new(ErrorCode::Busy, "preview worker queue is full"),
                        ));
                    }
                    Err(TrySendError::Disconnected(work)) => {
                        let _ = work.reply.send(Response::err(
                            work.request_id,
                            ErrorPayload::new(ErrorCode::WorkerCrashed, "preview worker stopped"),
                        ));
                    }
                }
            }
            #[cfg(test)]
            Ok(Command::TestShutdown(acknowledge)) => {
                preview_stopping.store(true, Ordering::Release);
                let _ = acknowledge.send(());
                break;
            }
            Ok(Command::Request(request, reply)) => {
                if matches!(
                    request.method,
                    Method::ProjectPreviewTexture
                        | Method::ProjectPreviewMaterial
                        | Method::ProjectPreviewAsset
                ) {
                    if let Err(response) = state.validate_request(&request) {
                        let _ = reply.send(*response);
                        continue;
                    }
                    if preview_in_flight >= PREVIEW_WORKERS {
                        let _ = reply.send(Response::err(
                            request.request_id,
                            ErrorPayload::new(ErrorCode::Busy, "preview worker queue is full"),
                        ));
                        continue;
                    }
                    match state.prepare_preview(&request) {
                        Ok(task) => {
                            let request_id = task.request_id.clone();
                            let cancellation = preview_stopping.clone();
                            match preview_tx.try_send(PreviewWork {
                                request_id,
                                run: Box::new(move || task.execute(&cancellation)),
                                reply,
                            }) {
                                Ok(()) => preview_in_flight += 1,
                                Err(TrySendError::Full(work)) => {
                                    let _ = work.reply.send(Response::err(
                                        work.request_id,
                                        ErrorPayload::new(
                                            ErrorCode::Busy,
                                            "preview worker queue is full",
                                        ),
                                    ));
                                }
                                Err(TrySendError::Disconnected(work)) => {
                                    let _ = work.reply.send(Response::err(
                                        work.request_id,
                                        ErrorPayload::new(
                                            ErrorCode::WorkerCrashed,
                                            "preview worker stopped",
                                        ),
                                    ));
                                }
                            }
                        }
                        Err(error) => {
                            let _ = reply.send(Response::err(request.request_id, error));
                        }
                    }
                    continue;
                }
                let response = state.handle(request.clone());
                let wait_ms = if request.method == Method::EventsRead && response.error.is_none() {
                    params::<ReadEventsParams>(&request).map_or(0, |p| p.wait_ms)
                } else {
                    0
                };
                if wait_ms > 0
                    && response.result.as_ref().unwrap()["events"]
                        .as_array()
                        .unwrap()
                        .is_empty()
                {
                    if waiting.len() >= state.limits.command_queue {
                        let _ = reply.send(Response::err(
                            request.request_id,
                            ErrorPayload::new(ErrorCode::Busy, "event wait queue is full"),
                        ));
                    } else {
                        waiting.push(Waiting {
                            request,
                            reply,
                            deadline: Instant::now() + Duration::from_millis(wait_ms),
                        });
                    }
                } else {
                    let _ = reply.send(response);
                }
            }
        }
    }
    preview_stopping.store(true, Ordering::Release);
    state.shutdown();
    drop(preview_tx);
    for thread in preview_threads {
        let _ = thread.join();
    }
    drop(project_tx);
    let _ = project_thread.join();
    drop(service_tx);
    drop(render_tx);
    let _ = service_thread.join();
    let _ = render_thread.join();
    for pending in waiting {
        let _ = pending.reply.send(Response::err(
            pending.request.request_id,
            ErrorPayload::new(ErrorCode::IoError, "host is shutting down"),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::commands::Limits;

    fn response(id: &str) -> Response {
        Response::ok(id, serde_json::json!({"ok":true}))
    }

    fn request(id: &str) -> Request {
        Request {
            protocol: protocol::PROTOCOL_VERSION.to_owned(),
            request_id: id.to_owned(),
            method: Method::AppGetState,
            params: serde_json::Value::Null,
        }
    }

    #[test]
    fn blocked_preview_worker_blocks_neither_core_nor_the_other_preview() {
        let (commands, receiver) = mpsc::sync_channel(8);
        let host = thread::spawn(move || run(CoreState::new(Limits::default()), receiver));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (first_tx, first_rx) = mpsc::channel();
        commands
            .send(Command::TestPreview(
                Box::new(move || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    response("first")
                }),
                first_tx,
            ))
            .unwrap();
        entered_rx.recv().unwrap();

        // The second worker starts while the first is still held.
        let (second_tx, second_rx) = mpsc::channel();
        let (hold_tx, hold_rx) = mpsc::channel();
        let (second_entered_tx, second_entered_rx) = mpsc::channel();
        commands
            .send(Command::TestPreview(
                Box::new(move || {
                    second_entered_tx.send(()).unwrap();
                    hold_rx.recv().unwrap();
                    response("second")
                }),
                second_tx,
            ))
            .unwrap();
        second_entered_rx.recv().unwrap();
        let (third_tx, third_rx) = mpsc::channel();
        commands
            .send(Command::TestPreview(
                Box::new(|| response("third")),
                third_tx,
            ))
            .unwrap();
        assert_eq!(
            third_rx.recv().unwrap().error.unwrap().code,
            ErrorCode::Busy
        );

        let (state_tx, state_rx) = mpsc::channel();
        commands
            .send(Command::Request(request("state-during-preview"), state_tx))
            .unwrap();
        assert!(state_rx.recv().unwrap().error.is_none());

        release_tx.send(()).unwrap();
        hold_tx.send(()).unwrap();
        assert!(first_rx.recv().unwrap().error.is_none());
        assert!(second_rx.recv().unwrap().error.is_none());
        commands.send(Command::Shutdown).unwrap();
        host.join().unwrap();
    }

    #[test]
    fn preview_worker_recovers_from_task_panic() {
        let (commands, receiver) = mpsc::sync_channel(8);
        let host = thread::spawn(move || run(CoreState::new(Limits::default()), receiver));
        let (panic_tx, panic_rx) = mpsc::channel();
        commands
            .send(Command::TestPreview(
                Box::new(|| panic!("synthetic preview panic")),
                panic_tx,
            ))
            .unwrap();
        assert_eq!(
            panic_rx.recv().unwrap().error.unwrap().code,
            ErrorCode::WorkerCrashed
        );
        let (next_tx, next_rx) = mpsc::channel();
        commands
            .send(Command::TestPreview(Box::new(|| response("next")), next_tx))
            .unwrap();
        assert!(next_rx.recv().unwrap().error.is_none());
        commands.send(Command::Shutdown).unwrap();
        host.join().unwrap();
    }

    #[test]
    fn shutdown_lets_both_running_previews_answer_and_joins_the_workers() {
        let (commands, receiver) = mpsc::sync_channel(8);
        let host = thread::spawn(move || run(CoreState::new(Limits::default()), receiver));
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let mut replies = Vec::new();
        for name in ["first", "second"] {
            let (entered_tx, entered_rx) = mpsc::channel();
            let (reply_tx, reply_rx) = mpsc::channel();
            let release = release_rx.clone();
            commands
                .send(Command::TestPreview(
                    Box::new(move || {
                        entered_tx.send(()).unwrap();
                        let _ = release.lock().unwrap().recv();
                        response(name)
                    }),
                    reply_tx,
                ))
                .unwrap();
            entered_rx.recv().unwrap();
            replies.push(reply_rx);
        }
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        commands.send(Command::TestShutdown(shutdown_tx)).unwrap();
        shutdown_rx.recv().unwrap();
        drop(release_tx);
        for reply in replies {
            assert!(reply.recv().unwrap().error.is_none());
        }
        host.join().unwrap();
    }
}
