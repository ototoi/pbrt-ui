//! File and source records shared by the preview geometry manifest and the
//! PBRT parser.

use serde::{Deserialize, Serialize};

/// Source location of a directive. Absent locations are `None`, never `0:0`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceLocation {
    pub path: String,
    pub line: u32,
    pub column: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageFile {
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependency {
    pub path: String,
    pub role: String,
    pub size: u64,
    pub sha256: String,
}
