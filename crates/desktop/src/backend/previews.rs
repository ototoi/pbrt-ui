//! Inspector and node editor preview requests. The old `Command::Preview`
//! handler blocked the display thread on `request`; requests are queued
//! here instead, submitted one at a time and polled like Contents
//! thumbnails, so a large texture or sphere evaluation never freezes the
//! display loop.
use super::*;
use protocol::{ErrorCode, ErrorPayload};
use std::sync::mpsc::TryRecvError;

/// Queue cap; the oldest request answers Busy when it is exceeded (the
/// session backs off 250ms and re-asks, like any other Busy reply).
const PREVIEW_QUEUE_LIMIT: usize = 64;

impl Display {
    /// Queue one preview for `project`; entries for other revisions can
    /// never be submitted by this thread and go first (no reply owed: the
    /// window asks again at its own revision), duplicates stay out, and a
    /// full queue answers the oldest request with Busy.
    pub(super) fn queue_preview(&mut self, project: Value, id: String, kind: PreviewKind) {
        self.preview_queue
            .retain(|(queued, _, _)| queued == &project);
        // The one in flight counts too: its reply answers this request.
        let in_flight = self.preview_flight.as_ref().is_some_and(|flight| {
            flight.project == project && flight.id == id && flight.kind == kind
        });
        if in_flight
            || self
                .preview_queue
                .iter()
                .any(|(_, queued, queued_kind)| queued == &id && *queued_kind == kind)
        {
            return;
        }
        self.preview_queue.push_back((project, id, kind));
        while self.preview_queue.len() > PREVIEW_QUEUE_LIMIT {
            let Some((project, id, _)) = self.preview_queue.pop_front() else {
                break;
            };
            self.send_preview_error(
                project,
                id,
                ErrorPayload::new(ErrorCode::Busy, "preview queue is full"),
            );
        }
    }

    /// Submit the queue's next request when no flight is out. A stale
    /// project answers itself and moves on to the next candidate; the
    /// display loop keeps calling until the queue empties.
    pub(super) fn start_preview(&mut self) {
        if self.preview_flight.is_some() {
            return;
        }
        while let Some((project, id, kind)) = self.preview_queue.pop_front() {
            if project != self.last_project["context"]["project"] {
                self.send_preview_error(
                    project,
                    id,
                    ErrorPayload::new(
                        ErrorCode::StaleProject,
                        "preview request belongs to a stale project",
                    ),
                );
                continue;
            }
            let request_id = uuid::Uuid::new_v4().to_string();
            let request = Request {
                protocol: protocol::PROTOCOL_VERSION.into(),
                request_id: request_id.clone(),
                method: kind.method(),
                params: preview_params(&project, &id, kind),
            };
            match self.core.submit(request) {
                Ok(receiver) => {
                    self.preview_flight = Some(PreviewFlight {
                        request_id,
                        project,
                        id,
                        kind,
                        receiver: Some(receiver),
                    });
                }
                Err(error) => {
                    self.send_preview_error(project, id, error);
                }
            }
            return;
        }
    }

    /// Poll the in-flight reply once, like `poll_thumbnail` does. The
    /// Update that carries the reply clears the slot immediately: no
    /// display-side ack is owed, since the session decodes on its own
    /// background executor.
    pub(super) fn poll_preview(&mut self) {
        let Some(flight) = self.preview_flight.as_mut() else {
            return;
        };
        let Some(receiver) = flight.receiver.as_ref() else {
            return;
        };
        let response = match receiver.try_recv() {
            Ok(response) => response,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                let Some(flight) = self.preview_flight.take() else {
                    return;
                };
                self.send_preview_error(
                    flight.project,
                    flight.id,
                    ErrorPayload::new(ErrorCode::IoError, "core stopped before preview response"),
                );
                return;
            }
        };
        let Some(flight) = self.preview_flight.take() else {
            return;
        };
        let result = preview_reply_result(response, &flight);
        let _ = self.tx.send(Update::Preview {
            project: flight.project,
            id: flight.id,
            result,
        });
    }

    /// Answer one preview request with an error (overflow, stale project,
    /// a refused submit, or a dead core).
    fn send_preview_error(&self, project: Value, id: String, error: ErrorPayload) {
        let _ = self.tx.send(Update::Preview {
            project,
            id,
            result: Err(error),
        });
    }
}

/// `project.preview_texture` / `_material` / `_asset` params for one
/// request, with the target in the kind's own id field.
fn preview_params(project: &Value, id: &str, kind: PreviewKind) -> Value {
    let mut params = json!({"project": project, "size": 128});
    params[kind.id_field()] = json!(id);
    params
}

/// A preview reply must answer its own request, or it lands as an error;
/// a match yields the cache file's path and the content key it was
/// written under.
// ErrorPayload is the wire's own shape.
#[allow(clippy::result_large_err)]
fn preview_reply_result(
    response: protocol::Response,
    flight: &PreviewFlight,
) -> Result<(String, String), ErrorPayload> {
    if response.request_id != flight.request_id {
        return Err(ErrorPayload::new(
            ErrorCode::InvalidRequest,
            "preview response request id did not match",
        ));
    }
    match (response.result, response.error) {
        (Some(result), None) => {
            let path = result["path"].as_str().unwrap_or_default();
            let key = result["key"].as_str().unwrap_or_default();
            let id = result[flight.kind.id_field()].as_str().unwrap_or_default();
            if result["project"] != flight.project
                || id != flight.id
                || path.is_empty()
                || key.is_empty()
            {
                Err(ErrorPayload::new(
                    ErrorCode::InvalidRequest,
                    "preview response did not match its request",
                ))
            } else {
                Ok((path.to_owned(), key.to_owned()))
            }
        }
        (_, Some(error)) => Err(error),
        _ => Err(ErrorPayload::new(
            ErrorCode::InvalidRequest,
            "empty preview response",
        )),
    }
}
