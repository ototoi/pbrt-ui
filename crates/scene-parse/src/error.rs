//! Parser diagnostics and errors.
//!
//! `Diagnostic` mirrors the record shape the worker writes into
//! `diagnostics.jsonl` (`severity` / `code` / target) and adds the source
//! location, which is how preview draws are joined back to r4 entities.

use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
    Info,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Warning => "warning",
            Severity::Error => "error",
            Severity::Info => "info",
        }
    }
}

/// One parser-level diagnostic. `path` / `line` locate the offending directive
/// in the scene file the directive was parsed from (after `Include`).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Diagnostic {
    pub severity: String,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape_type: Option<String>,
}

impl Diagnostic {
    pub fn warning(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning.as_str().to_string(),
            code: code.into(),
            message: message.into(),
            path: None,
            line: None,
            shape_type: None,
        }
    }

    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error.as_str().to_string(),
            code: code.into(),
            message: message.into(),
            path: None,
            line: None,
            shape_type: None,
        }
    }

    /// A note about something the preview does not draw but the project
    /// retains: the Console shows it, and it does not count as
    /// a dropped or approximated piece the way a warning does.
    pub fn info(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Info.as_str().to_string(),
            code: code.into(),
            message: message.into(),
            path: None,
            line: None,
            shape_type: None,
        }
    }

    /// Whether this record warns about something the preview dropped or
    /// approximated — the count callers report as unsupported.
    pub fn is_warning(&self) -> bool {
        self.severity == Severity::Warning.as_str()
    }

    /// Whether this record is an informational note (shown, never counted
    /// as unsupported).
    pub fn is_info(&self) -> bool {
        self.severity == Severity::Info.as_str()
    }

    pub fn at(mut self, path: &str, line: u32) -> Self {
        self.path = Some(path.to_string());
        self.line = Some(line);
        self
    }

    pub fn for_shape(mut self, shape_type: &str) -> Self {
        self.shape_type = Some(shape_type.to_string());
        self
    }
}

#[derive(Debug, Error)]
pub enum SceneParseError {
    #[error("cannot read {path}: {reason}")]
    Unreadable { path: String, reason: String },

    #[error("{path}:{line}:{column}: {message}")]
    Syntax {
        path: String,
        line: u32,
        column: u32,
        message: String,
    },

    #[error("include depth of {limit} exceeded at {path}:{line}")]
    IncludeDepth { path: String, line: u32, limit: u32 },

    #[error("{path}:{line}: include cycle through {cycle}")]
    IncludeCycle {
        path: String,
        line: u32,
        cycle: String,
    },

    #[error("ply file {path}: {message}")]
    Ply { path: String, message: String },

    /// A model asset's bytes no longer hash to the sha256 the project
    /// model records for it: the in-place read on a cache
    /// miss fails if the bytes do not match the recorded digest.
    #[error("asset changed during export")]
    AssetChanged,

    #[error("geometry resource exhausted: {message}")]
    ResourceExhausted { message: String },
}

impl SceneParseError {
    pub fn syntax(path: &str, line: u32, column: u32, message: impl Into<String>) -> Self {
        Self::Syntax {
            path: path.to_string(),
            line,
            column,
            message: message.into(),
        }
    }

    pub fn unreadable(path: &str, reason: impl Into<String>) -> Self {
        Self::Unreadable {
            path: path.to_string(),
            reason: reason.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_records_source_location_and_shape_type() {
        let record = Diagnostic::warning("unsupported_shape", "loopsubdiv is not tessellated")
            .at("scene.pbrt", 42)
            .for_shape("loopsubdiv");
        let json = serde_json::to_string(&record).expect("serialize");
        assert!(json.contains("\"severity\":\"warning\""));
        assert!(json.contains("\"code\":\"unsupported_shape\""));
        assert!(json.contains("\"path\":\"scene.pbrt\""));
        assert!(json.contains("\"line\":42"));
        assert!(json.contains("\"shape_type\":\"loopsubdiv\""));
    }

    #[test]
    fn diagnostic_omits_absent_source_location() {
        let record = Diagnostic::error("resource_exhausted", "mesh too large");
        let json = serde_json::to_string(&record).expect("serialize");
        assert!(!json.contains("path"));
        assert!(!json.contains("shape_type"));
    }
}
