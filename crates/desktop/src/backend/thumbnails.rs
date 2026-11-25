use super::*;
use protocol::{ErrorCode, ErrorPayload, Request};
use serde_json::json;
use std::sync::mpsc::TryRecvError;

impl Display {
    pub(super) fn poll_thumbnail(&mut self) {
        let Some(flight) = self.thumbnail_flight.as_mut() else {
            return;
        };
        let Some(receiver) = flight.receiver.as_ref() else {
            return;
        };
        let response = match receiver.try_recv() {
            Ok(response) => response,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                let flight = self.thumbnail_flight.as_mut().unwrap();
                flight.receiver = None;
                self.send_thumbnail_error(ErrorPayload::new(
                    ErrorCode::IoError,
                    "core stopped before preview response",
                ));
                return;
            }
        };
        let flight = self.thumbnail_flight.as_mut().unwrap();
        flight.receiver = None;
        if response.request_id != flight.request_id {
            self.send_thumbnail_error(ErrorPayload::new(
                ErrorCode::InvalidRequest,
                "thumbnail response request id did not match",
            ));
            return;
        }
        let result = match (response.result, response.error) {
            (Some(result), None) => {
                let project = result["project"].clone();
                let id = result[flight.kind.id_field()].as_str().unwrap_or_default();
                let path = result["path"].as_str().unwrap_or_default();
                let key = result["key"].as_str().unwrap_or_default();
                if project != flight.project || id != flight.id || path.is_empty() || key.is_empty()
                {
                    Err(ErrorPayload::new(
                        ErrorCode::InvalidRequest,
                        "thumbnail response did not match its request",
                    ))
                } else {
                    Ok((path.to_owned(), key.to_owned()))
                }
            }
            (_, Some(error)) => Err(error),
            _ => Err(ErrorPayload::new(
                ErrorCode::InvalidRequest,
                "empty thumbnail response",
            )),
        };
        let _ = self.tx.send(Update::Thumbnail {
            project: flight.project.clone(),
            id: flight.id.clone(),
            kind: flight.kind,
            generation: flight.generation,
            request_id: flight.request_id.clone(),
            result,
        });
    }

    fn send_thumbnail_error(&self, error: ErrorPayload) {
        if let Some(flight) = self.thumbnail_flight.as_ref() {
            let _ = self.tx.send(Update::Thumbnail {
                project: flight.project.clone(),
                id: flight.id.clone(),
                kind: flight.kind,
                generation: flight.generation,
                request_id: flight.request_id.clone(),
                result: Err(error),
            });
        }
    }

    pub(super) fn start_thumbnail(&mut self) {
        if self.thumbnail_flight.is_some() {
            return;
        }
        let Some((id, kind)) = self.thumbnail_candidates.pop_front() else {
            return;
        };
        let request_id = uuid::Uuid::new_v4().to_string();
        let mut flight = ThumbnailFlight {
            request_id: request_id.clone(),
            project: self.thumbnail_project.clone(),
            id,
            kind,
            generation: self.thumbnail_generation,
            receiver: None,
        };
        if self.thumbnail_project != self.last_project["context"]["project"] {
            let _ = self.tx.send(Update::Thumbnail {
                project: flight.project.clone(),
                id: flight.id.clone(),
                kind: flight.kind,
                generation: flight.generation,
                request_id: request_id.clone(),
                result: Err(ErrorPayload::new(
                    ErrorCode::StaleProject,
                    "thumbnail request belongs to a stale project",
                )),
            });
            self.thumbnail_flight = Some(flight);
            return;
        }
        let mailbox_arc = self.thumbnail_mailbox.clone();
        let mailbox = mailbox_arc.lock().ok();
        let visible_now = mailbox.as_ref().is_some_and(|mailbox| {
            mailbox.project == flight.project
                && mailbox.generation == flight.generation
                && mailbox
                    .visible
                    .iter()
                    .any(|(id, kind)| id == &flight.id && *kind == flight.kind)
        });
        if !visible_now {
            let _ = self.tx.send(Update::Thumbnail {
                project: flight.project.clone(),
                id: flight.id.clone(),
                kind: flight.kind,
                generation: flight.generation,
                request_id: flight.request_id.clone(),
                result: Err(ErrorPayload::new(
                    ErrorCode::Cancelled,
                    "thumbnail left the visible Contents range before it started",
                )),
            });
            self.thumbnail_flight = Some(flight);
            return;
        }
        let request = Request {
            protocol: protocol::PROTOCOL_VERSION.into(),
            request_id: request_id.clone(),
            method: kind.method(),
            params: json!({"project":self.thumbnail_project.clone(),(kind.id_field()):flight.id.clone(),"size":64}),
        };
        match self.core.submit(request) {
            Ok(receiver) => flight.receiver = Some(receiver),
            Err(error) => {
                let _ = self.tx.send(Update::Thumbnail {
                    project: flight.project.clone(),
                    id: flight.id.clone(),
                    kind: flight.kind,
                    generation: flight.generation,
                    request_id: flight.request_id.clone(),
                    result: Err(error),
                });
            }
        }
        drop(mailbox);
        self.thumbnail_flight = Some(flight);
    }
}
