//! PBRT interchange for self-contained native projects. The renderer is external.
mod emit;
mod export;
pub mod implementation_change;
mod import;
pub mod parameter_schema;
mod schema;
mod source;
mod spectra;
mod texture_types;
pub use texture_types::texture_type_mismatches;

pub use emit::spectrum_samples;
pub use export::{
    ExportWarning, asset_store_paths, export_project, export_warnings, program_from_project,
    stage_assets, stage_snapshot_assets,
};
pub use import::{
    ImportOptions, ImportReport, import_into, import_project, import_project_with_report,
    parse_unresolved, prepare_scene, prepare_scene_in,
};
pub use schema::check_exportable;
pub use spectra::BUILTIN_SPECTRA;
use std::path::{Path, PathBuf};
/// Source revision used to check parser state and builtin spectrum behavior.
pub const R4_SEMANTIC_TARGET: &str = "16b0aab6b518e541082423c03d63174436cd01de";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Project(#[from] project::ProjectError),
    #[error(transparent)]
    Parse(#[from] scene_parse::SceneParseError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("published {directory}, but directory sync failed: {source}")]
    Published {
        directory: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("unsupported PBRT input: {0}")]
    Unsupported(String),
    #[error("invalid PBRT input: {0}")]
    Invalid(String),
    #[error("{message}")]
    MissingRequired {
        message: String,
        missing: Vec<MissingRequired>,
    },
    /// An asset's file left the project folder (moved or deleted outside
    /// the application).
    #[error("asset file is missing from the project folder: {location}")]
    AssetMissing {
        asset: project::AssetId,
        location: String,
    },
    /// A texture the export writes or refers to is an imagemap whose image
    /// was never chosen.
    #[error("imagemap texture \"{label}\" has no image")]
    ImagemapWithoutImage {
        resource: project::ResourceId,
        label: String,
    },
    /// An input file changed while it was being imported. Unlike unsupported
    /// or invalid input, this stops the import instead of being skipped.
    #[error("input changed during import: {0}")]
    SourceChanged(String),
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct MissingRequired {
    pub id: project::ResourceId,
    pub label: String,
    pub parameter: String,
}
pub type Result<T> = std::result::Result<T, Error>;
fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}
fn unsupported(message: impl Into<String>) -> Error {
    Error::Unsupported(message.into())
}
fn check(cancelled: &impl Fn() -> bool) -> Result<()> {
    if cancelled() {
        Err(project::ProjectError::Cancelled.into())
    } else {
        Ok(())
    }
}
/// Publish a staged bundle by renaming it into place (no replace). Shared
/// by the text export and the direct preview job.
pub fn publish(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(unix)]
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        source,
        rustix::fs::CWD,
        destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(std::io::Error::from)?;
    // shortcut: not atomic (another process can create `destination` between
    // the check and the rename), as project's Windows rename_noreplace; use
    // MoveFileExW without MOVEFILE_REPLACE_EXISTING if that race matters.
    #[cfg(not(unix))]
    {
        if destination.exists() {
            return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists).into());
        }
        std::fs::rename(source, destination)?;
    }
    if let Some(parent) = destination.parent() {
        project::sync_path(parent).map_err(|source| Error::Published {
            directory: destination.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}
