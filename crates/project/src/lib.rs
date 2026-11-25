//! Native editable project data, independent of PBRT syntax and GPU/UI types.
//! Model validation, editing and storage use the same public interface in
//! headless tests and the future core-owned ProjectSession.
mod cache_dir;
mod content_id;
pub mod coordinate;
mod evaluator;
mod history;
mod import_batch;
pub use cache_dir::cache_directory;
pub use import_batch::{ImportStaging, PreparedImport};
pub mod layout;
/// Import-time folding of a light's `from` parameter into its transform.
pub mod light_from_transforms;
pub mod math;
mod model;
mod payload;
pub mod presets;
mod references;
mod session;
mod spectrum_data;
mod store;
mod temp_stage;
pub use temp_stage::{jobs_tempdir_in, viewer_tempdir_in};
pub mod transform_parents;
pub mod transform_trs;
pub use store::meta_v2;
mod validation;

/// Flush `path` to disk: a file's data, or a directory's entries after a
/// create or rename. Windows cannot open a directory as a `File` (and NTFS
/// journals its metadata), so a directory there is a no-op.
pub fn sync_path(path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
    let path = path.as_ref();
    if cfg!(windows) && path.is_dir() {
        return Ok(());
    }
    std::fs::File::open(path)?.sync_all()
}

pub use content_id::ContentId;
pub use coordinate::{Axis, CoordinateSystem, Handedness};
pub use evaluator::{EvaluatedScene, GraphicsState, evaluate};
pub use history::{History, HistoryEntry, OperationLabel};
pub use light_from_transforms::fold_light_from_transforms;
pub use model::*;
pub use payload::PayloadLease;
pub use protocol::Revision;
pub use references::{ReferenceIndex, ReferenceLocation, ReferenceVia};
pub use session::{
    AreaLightContent, ChangeSet, Edit, LocalTransform, MediumInterface, ParameterTarget,
    ProjectSession, ProjectSnapshot, RelocatedPayloads,
};
pub use spectrum_data::{MAX_SPD_FILE_BYTES, SpdSamples, parse_spd};
pub use store::{
    AssetMeta, CacheHit, DEFAULT_CACHE_LIMIT, FolderScan, FolderStore, FormatThreePlan,
    LegacyStore, ObjectStore, PROJECT_FORMAT_VERSION, ProjectStore, SaveReceipt, cache_get,
    cache_put, ensure_cache,
};
pub use transform_parents::resolve_transform_parents;

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("invalid project: {0}")]
    Invalid(String),
    /// Resources reference themselves through their parameters; PBRT
    /// declares resources in order, so no export can hold this.
    #[error("invalid project: cyclic resource references")]
    ResourceCycle,
    #[error("project revision changed (expected {expected}, current {current})")]
    RevisionConflict { expected: String, current: String },
    #[error("project is already open for writing")]
    Locked,
    #[error("unsupported project format version {0}")]
    UnsupportedVersion(u64),
    #[error("project operation cancelled before publication")]
    Cancelled,
    #[error("project file changed outside the application: {0}")]
    SourceChanged(String),
    /// The asset is in the document, but its file is not in the project
    /// folder (moved or deleted outside the application).
    #[error("asset file is missing from the project folder")]
    AssetFileMissing(AssetId),
    #[error("this legacy project is read-only; use Save As to create a format-2 project")]
    SaveUnavailable,
    #[error("project resource limit exceeded: {0}")]
    ResourceLimit(String),
    #[error("project revision {revision} was published, but directory sync failed: {source}")]
    Durability {
        revision: String,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ProjectError>;
pub(crate) fn invalid(message: impl Into<String>) -> ProjectError {
    ProjectError::Invalid(message.into())
}
