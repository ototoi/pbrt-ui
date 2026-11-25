//! Session-bound AF_UNIX socket client. Never reconnect or replay automatically.
pub mod view_argument;
pub use protocol as proto;
use protocol::commands::{Job, JobState};
use protocol::{ErrorPayload, Method, Request, Response, SessionId};
use serde_json::{Value, json};
use std::io::BufReader;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};
#[cfg(windows)]
use uds_windows::UnixStream;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Wire(#[from] protocol::ndjson::WireError),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("{}", .0.message)]
    Remote(ErrorPayload),
    #[error("{0}")]
    Invalid(String),
}
pub struct Client {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    pub session_id: SessionId,
}
impl Client {
    pub fn connect(endpoint: &Path) -> Result<Self, ClientError> {
        let writer = UnixStream::connect(endpoint)?;
        writer.set_read_timeout(Some(Duration::from_secs(35)))?;
        writer.set_write_timeout(Some(Duration::from_secs(5)))?;
        let reader = BufReader::new(writer.try_clone()?);
        let mut client = Self {
            writer,
            reader,
            session_id: uuid::Uuid::nil(),
        };
        let hello = client.call(
            Method::AppHello,
            json!({"client_name":"pbrt-ui-ctl","supported_protocols":[protocol::PROTOCOL_VERSION]}),
        )?;
        client.session_id = serde_json::from_value(hello["session_id"].clone())?;
        Ok(client)
    }
    pub fn send(&mut self, request: Request) -> Result<Response, ClientError> {
        protocol::ndjson::write_message(&mut self.writer, &request)?;
        let response: Response = protocol::ndjson::read_message(&mut self.reader)?
            .ok_or_else(|| ClientError::Invalid("host closed connection".into()))?;
        if response.request_id != request.request_id
            || response.protocol != protocol::PROTOCOL_VERSION
        {
            return Err(ClientError::Invalid("response id/protocol mismatch".into()));
        }
        Ok(response)
    }
    pub fn call(&mut self, method: Method, params: Value) -> Result<Value, ClientError> {
        self.call_with_id(method, params, uuid::Uuid::new_v4().to_string())
    }
    pub fn call_with_id(
        &mut self,
        method: Method,
        params: Value,
        request_id: String,
    ) -> Result<Value, ClientError> {
        let response = self.send(Request {
            protocol: protocol::PROTOCOL_VERSION.into(),
            request_id,
            method,
            params,
        })?;
        match (response.result, response.error) {
            (Some(result), None) => Ok(result),
            (None, Some(error)) => Err(ClientError::Remote(error)),
            _ => Err(ClientError::Invalid("invalid response".into())),
        }
    }
    pub fn wait_job(
        &mut self,
        job_id: protocol::JobId,
        timeout: Option<Duration>,
    ) -> Result<Job, ClientError> {
        let started = Instant::now();
        loop {
            let value = self.call(Method::JobGet, json!({"job_id":job_id}))?;
            let job: Job = serde_json::from_value(value)?;
            if job.state.is_terminal() {
                return match job.state {
                    JobState::Succeeded => Ok(job),
                    JobState::Failed => Err(ClientError::Remote(job.error.unwrap_or_else(|| {
                        ErrorPayload::new(
                            protocol::ErrorCode::OutputInvalid,
                            "job failed without error",
                        )
                    }))),
                    _ => Err(ClientError::Invalid("job was cancelled".into())),
                };
            }
            if timeout.is_some_and(|limit| started.elapsed() >= limit) {
                return Err(ClientError::Invalid(
                    "client wait timed out; the job continues until explicitly cancelled".into(),
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
