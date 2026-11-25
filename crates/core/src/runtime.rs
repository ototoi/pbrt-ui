use crate::state::{CoreState, params};
use protocol::commands::{Limits, ReadEventsParams};
use protocol::{ErrorCode, ErrorPayload, Method, Request, Response};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

enum Command {
    Request(Request, Sender<Response>),
    #[cfg(test)]
    TestPreview(Box<dyn FnOnce() -> Response + Send>, Sender<Response>),
    #[cfg(test)]
    TestShutdown(Sender<()>),
    Shutdown,
}
#[derive(Clone)]
pub struct CoreHandle {
    sender: SyncSender<Command>,
}
impl CoreHandle {
    /// Enqueue without blocking the UI. Queue saturation is reported as busy.
    pub fn submit(&self, request: Request) -> Result<Receiver<Response>, ErrorPayload> {
        let (sender, receiver) = mpsc::channel();
        self.sender
            .try_send(Command::Request(request, sender))
            .map_err(|error| match error {
                TrySendError::Full(_) => {
                    ErrorPayload::new(ErrorCode::Busy, "core command queue is full")
                }
                TrySendError::Disconnected(_) => {
                    ErrorPayload::new(ErrorCode::IoError, "core is stopped")
                }
            })?;
        Ok(receiver)
    }
    pub fn request(&self, request: Request) -> Response {
        let id = request.request_id.clone();
        match self.submit(request) {
            Ok(receiver) => receiver.recv().unwrap_or_else(|_| {
                Response::err(
                    id,
                    ErrorPayload::new(ErrorCode::IoError, "core stopped before responding"),
                )
            }),
            Err(error) => Response::err(id, error),
        }
    }
}
pub struct CoreHost {
    handle: CoreHandle,
    thread: Option<JoinHandle<()>>,
}
impl CoreHost {
    pub fn start(limits: Limits) -> Self {
        Self::start_with_services(limits, None)
    }
    pub fn start_with_services(
        limits: Limits,
        services: Option<crate::services::ServiceConfig>,
    ) -> Self {
        let mut state = CoreState::new(limits.clone());
        if let Some(config) = services {
            state.configure_services(config);
        }
        let (sender, receiver) = mpsc::sync_channel(limits.command_queue);
        let handle = CoreHandle { sender };
        let thread = thread::spawn(move || run(state, receiver));
        Self {
            handle,
            thread: Some(thread),
        }
    }
    pub fn handle(&self) -> CoreHandle {
        self.handle.clone()
    }
}
impl Drop for CoreHost {
    fn drop(&mut self) {
        let _ = self.handle.sender.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
struct Waiting {
    request: Request,
    reply: Sender<Response>,
    deadline: Instant,
}
struct ServiceWork {
    work: crate::services::ServiceTask,
}

mod driver;
mod service_executor;
use driver::run;
use service_executor::spawn_service;
