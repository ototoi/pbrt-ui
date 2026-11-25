//! Structured error codes and payloads.
//!
//! `code` is for branching, `message` is for display, `details` carries the
//! current revision and target ids an agent needs to re-query.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Machine-readable error codes. Kept flat; branching is done on these.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    UnsupportedProtocol,
    Unsupported,
    NotFound,
    RevisionConflict,
    StaleScene,
    StaleProject,
    ReadOnly,
    UnsavedChanges,
    ProjectLocked,
    Cancelled,
    StaleFrame,
    InvalidCursor,
    RequestIdConflict,
    Busy,
    InvalidCamera,
    ParseFailed,
    ReferenceUnresolved,
    SourceChanged,
    PreviewUnavailable,
    ResourceExhausted,
    WorkerCrashed,
    OutputInvalid,
    IoError,
    ResyncRequired,
    UnsupportedPathEncoding,
    UnsupportedTime,
    BoundsUnavailable,
    DisplayColorSpaceUnsupported,
}

/// A structured error returned in place of `result`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorPayload {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    /// Free-form, code-specific details (e.g. expected/current revision).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl ErrorPayload {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        ErrorPayload {
            code,
            message: message.into(),
            target_id: None,
            details: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn with_target(mut self, target_id: impl Into<String>) -> Self {
        self.target_id = Some(target_id.into());
        self
    }
}
