use crate::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

pub const PROJECT_FORMAT_VERSION: u32 = 1;
const FORMAT: &str = "pbrt-ui-project";
const MANIFEST: &str = "project.pbrtui.json";
const MAX_RECORD_BYTES: u64 = 4 * 1024 * 1024;
const MAX_METADATA_BYTES: u64 = 128 * 1024 * 1024;
const MAX_OBJECT_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: String,
    format_version: u32,
    project_id: ProjectId,
    saved_revision: Revision,
    head: uuid::Uuid,
    model_sha256: String,
    files: BTreeMap<String, ObjectRef>,
}

#[derive(Clone, Debug)]
pub struct SaveReceipt {
    project_id: ProjectId,
    revision: Revision,
    generation: uuid::Uuid,
    /// which content this save published. Dirty
    /// compares content, so the receipt is the save's baseline anchor.
    content_id: ContentId,
}
impl SaveReceipt {
    pub fn project_id(&self) -> ProjectId {
        self.project_id
    }
    pub fn revision(&self) -> &Revision {
        &self.revision
    }
    pub fn generation(&self) -> uuid::Uuid {
        self.generation
    }
    pub fn content_id(&self) -> &ContentId {
        &self.content_id
    }
}
/// Temporary object spool used during import and read-only format-1 storage.
/// Its lock remains held for the lifetime of the handle.
pub struct ObjectStore {
    directory: PathBuf,
    _lock: File,
}
/// Read-only adapter for a format-1 project. Asset IDs are resolved to the
/// content-addressed files in the legacy object store for export and preview.
pub struct LegacyStore {
    pub(crate) objects: ObjectStore,
    assets: BTreeMap<AssetId, ObjectRef>,
}
/// Store routing for editable format-3 projects and compatibility readers.
pub enum ProjectStore {
    Folder(FolderStore),
    Legacy(LegacyStore),
}
mod folder;
pub mod meta_v2;
pub use folder::{AssetMeta, FolderScan, FolderStore};
pub(crate) use folder::{AssetMetaOrigin, asset_file_by_id, asset_files_by_id};
mod cache;
pub use cache::{CacheHit, DEFAULT_CACHE_LIMIT, cache_get, cache_put, ensure_cache};
mod files;
mod format;
mod legacy_v1;
mod migration;
mod object_files;
mod objects;
mod open;
use files::*;
use format::*;
pub use migration::FormatThreePlan;
use object_files::*;

impl ProjectStore {
    pub fn directory(&self) -> &Path {
        match self {
            Self::Folder(store) => store.directory(),
            Self::Legacy(store) => &store.objects.directory,
        }
    }

    pub fn create_folder(
        path: &Path,
        project_id: ProjectId,
        title: &str,
        revision: Revision,
    ) -> Result<Self> {
        FolderStore::create_v3(path, project_id, title, revision).map(Self::Folder)
    }

    pub fn open_folder(path: &Path) -> Result<Self> {
        FolderStore::open(path).map(Self::Folder)
    }

    pub fn asset_path(&self, id: AssetId) -> Result<PathBuf> {
        match self {
            Self::Folder(store) => store.asset_path(id),
            Self::Legacy(store) => {
                let object = store
                    .assets
                    .get(&id)
                    .ok_or_else(|| invalid("asset id is not present in the project"))?;
                store.objects.object_path(object)
            }
        }
    }

    pub fn is_read_only(&self) -> bool {
        match self {
            Self::Folder(store) => !store.is_v3(),
            Self::Legacy(_) => true,
        }
    }

    pub fn read_document(&self) -> Result<ProjectDocument> {
        match self {
            Self::Folder(store) => store.read_document(),
            Self::Legacy(_) => Err(ProjectError::SaveUnavailable),
        }
    }

    pub fn save(&mut self, snapshot: &ProjectSnapshot) -> Result<SaveReceipt> {
        if self.is_read_only() {
            return Err(ProjectError::SaveUnavailable);
        }
        match self {
            Self::Folder(store) => store.save(snapshot),
            Self::Legacy(_) => {
                let _ = snapshot;
                Err(ProjectError::SaveUnavailable)
            }
        }
    }

    pub fn save_cancellable(
        &mut self,
        snapshot: &ProjectSnapshot,
        cancelled: impl Fn() -> bool,
    ) -> Result<SaveReceipt> {
        if self.is_read_only() {
            return Err(ProjectError::SaveUnavailable);
        }
        match self {
            Self::Folder(store) => store.save_cancellable(snapshot, cancelled),
            Self::Legacy(_) => {
                let _ = (snapshot, cancelled);
                Err(ProjectError::SaveUnavailable)
            }
        }
    }

    /// Persist the active scene without changing the project edit revision.
    /// Returns false for legacy stores or scenes not published by a save yet.
    pub fn set_default_scene(&mut self, scene: impl Into<Option<SceneId>>) -> Result<bool> {
        if self.is_read_only() {
            return Ok(false);
        }
        match self {
            Self::Folder(store) => store.set_default_scene(scene),
            Self::Legacy(_) => Ok(false),
        }
    }

    pub fn save_as(&self, destination: &Path, snapshot: &ProjectSnapshot) -> Result<Self> {
        self.save_as_cancellable(destination, snapshot, || false)
            .map(|(store, _)| store)
    }

    pub fn save_as_cancellable(
        &self,
        destination: &Path,
        snapshot: &ProjectSnapshot,
        cancelled: impl Fn() -> bool,
    ) -> Result<(Self, SaveReceipt)> {
        if self.is_read_only() {
            return self.save_as_format_three(destination, snapshot, cancelled);
        }
        match self {
            Self::Folder(store) => store
                .save_as_snapshot_cancellable(destination, snapshot, cancelled)
                .map(|(store, receipt)| (Self::Folder(store), receipt)),
            Self::Legacy(store) => FolderStore::save_as_from_objects_cancellable(
                destination,
                snapshot,
                &store.objects,
                cancelled,
            )
            .map(|(store, receipt)| (Self::Folder(store), receipt)),
        }
    }

    pub fn unregistered_files(&self) -> Result<Vec<PathBuf>> {
        match self {
            Self::Folder(store) => store.unregistered_files(),
            Self::Legacy(_) => Ok(Vec::new()),
        }
    }
}
