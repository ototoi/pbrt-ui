//! `protocol` is the shared, dependency-light contract used by core, cli, mcp,
//! and desktop. It must not depend on r4, GPUI, or wgpu. Everything here is
//! serde DTOs, identifiers, and error codes.

pub mod commands;
pub mod error;
pub mod geometry;
pub mod ids;
pub mod jobs;
pub mod ndjson;
pub mod package;
pub mod path;
pub mod project;
pub mod view;
pub mod wire;

pub use error::{ErrorCode, ErrorPayload};
pub use geometry::{
    AreaLightRecord, ArrayRef, Bounds, CameraRecord, DrawRecord, GEOMETRY_BLOB_MAGIC,
    GEOMETRY_BLOB_VERSION, GEOMETRY_MANIFEST_VERSION, GeometryManifest, LightRecord,
    MAX_PREVIEW_PARAMETER_TEXT_CHARS, MAX_PREVIEW_PARAMETER_VALUES, MAX_PREVIEW_PARAMETERS,
    MaterialRecord, MeshRecord, PreviewParameter, TessellationSettings, UnsupportedRecord,
    blob_header, check_blob_header,
};
pub use ids::{ArtifactId, EntityId, JobId, Revision, SceneId, SceneRef, SessionId};
pub use package::SourceLocation;
pub use wire::{Event, MAX_LINE_BYTES, Method, PROTOCOL_VERSION, Request, Response};
