//! Prepared additions own their immutable bytes before entering a session.
use crate::*;
use std::{collections::BTreeMap, path::Path, sync::Arc};

#[derive(Clone, Debug)]
pub struct PreparedImport {
    pub(crate) document: ProjectDocument,
    pub(crate) payloads: Vec<Arc<PayloadLease>>,
}
impl PreparedImport {
    pub fn new(document: ProjectDocument, payloads: Vec<Arc<PayloadLease>>) -> Result<Self> {
        document.validate()?;
        if document.default_scene.is_some() {
            return Err(ProjectError::Invalid(
                "an import batch must not select a default scene".into(),
            ));
        }
        if document.scenes.is_empty() && document.resources.is_empty() && document.assets.is_empty()
        {
            return Err(ProjectError::Invalid("empty import batch".into()));
        }
        for scene in document.scenes.keys() {
            evaluate(&document.scene_view(*scene)?)?;
        }
        let mut leases = BTreeMap::new();
        let mut total = 0u64;
        for lease in &payloads {
            total = total
                .checked_add(lease.size())
                .ok_or_else(|| ProjectError::ResourceLimit("import byte count overflow".into()))?;
            if total > 8 * 1024 * 1024 * 1024 {
                return Err(ProjectError::ResourceLimit(
                    "import payloads exceed 8 GiB".into(),
                ));
            }
            if leases.insert(lease.asset(), lease).is_some() {
                return Err(ProjectError::Invalid("duplicate import payload".into()));
            }
        }
        if leases.len() != document.assets.len() {
            return Err(ProjectError::Invalid(
                "import payload count does not match assets".into(),
            ));
        }
        for asset in document.assets.values() {
            let lease = leases
                .get(&asset.id)
                .ok_or_else(|| ProjectError::Invalid("missing import payload".into()))?;
            if lease.sha256() != asset.sha256 || lease.size() != asset.size {
                return Err(ProjectError::Invalid(
                    "import payload does not match asset".into(),
                ));
            }
        }
        Ok(Self { document, payloads })
    }
    pub fn payload(&self, id: AssetId) -> Option<&Arc<PayloadLease>> {
        self.payloads.iter().find(|lease| lease.asset() == id)
    }
    pub fn document(&self) -> &ProjectDocument {
        &self.document
    }
}

/// Decoder-independent ownership boundary. Importers perform format validation
/// against the returned immutable lease, never the mutable source path.
pub struct ImportStaging {
    owner: Arc<tempfile::TempDir>,
    copied: u64,
}
impl ImportStaging {
    pub fn new() -> Result<Self> {
        Self::with_copied_bytes(0)
    }
    /// Continue a worker's budget after source/archive preparation.
    pub fn with_copied_bytes(copied: u64) -> Result<Self> {
        let base = crate::cache_dir::cache_directory()?;
        Self::with_copied_bytes_in(&base, copied)
    }
    /// Create import staging below an explicit project or cache base.
    pub fn new_in(base: &Path) -> Result<Self> {
        Self::with_copied_bytes_in(base, 0)
    }
    /// Continue a worker's budget after source/archive preparation, below `base`.
    pub fn with_copied_bytes_in(base: &Path, copied: u64) -> Result<Self> {
        if copied > 8 * 1024 * 1024 * 1024 {
            return Err(ProjectError::ResourceLimit("import exceeds 8 GiB".into()));
        }
        Ok(Self {
            owner: Arc::new(crate::temp_stage::tempdir_in(base, "pbrt-ui-import-")?),
            copied,
        })
    }
    pub fn stage_file(&mut self, asset: &Asset, source: &Path) -> Result<Arc<PayloadLease>> {
        self.stage_file_cancellable(asset, source, || false)
    }

    /// An array edit's copy: `source` with `patches` applied, verified
    /// against the recorded size and hash in the same pass (see
    /// `PayloadLease::stage_patched_cancellable`).
    pub fn stage_patched_cancellable(
        &mut self,
        asset: AssetId,
        source: &Path,
        size: u64,
        expected_sha256: &str,
        patches: &[(u64, Vec<u8>)],
        cancelled: impl Fn() -> bool,
    ) -> Result<Arc<PayloadLease>> {
        let total = self
            .copied
            .checked_add(size)
            .ok_or_else(|| ProjectError::ResourceLimit("import byte count overflow".into()))?;
        if total > 8 * 1024 * 1024 * 1024 {
            return Err(ProjectError::ResourceLimit("import exceeds 8 GiB".into()));
        }
        let lease = PayloadLease::stage_patched_cancellable(
            self.owner.path(),
            asset,
            source,
            size,
            expected_sha256,
            patches,
            Some(self.owner.clone()),
            cancelled,
        )?;
        self.copied = total;
        Ok(lease)
    }

    pub fn stage_file_cancellable(
        &mut self,
        asset: &Asset,
        source: &Path,
        cancelled: impl Fn() -> bool,
    ) -> Result<Arc<PayloadLease>> {
        if cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let total = self
            .copied
            .checked_add(asset.size)
            .ok_or_else(|| ProjectError::ResourceLimit("import byte count overflow".into()))?;
        if total > 8 * 1024 * 1024 * 1024 {
            return Err(ProjectError::ResourceLimit("import exceeds 8 GiB".into()));
        }
        let origin = asset.origin.as_ref().and_then(|origin| {
            origin
                .origin
                .as_ref()
                .map(|source| crate::store::AssetMetaOrigin {
                    import: source.import,
                    logical_path: origin.logical_path.clone(),
                })
        });
        let lease = PayloadLease::stage_cancellable(
            self.owner.path(),
            asset.id,
            &asset.sha256,
            asset.size,
            source,
            origin,
            Some(self.owner.clone()),
            // Import sources are the user's files and may be rewritten in place.
            false,
            cancelled,
        )?;
        self.copied = total;
        Ok(lease)
    }
}

impl PreparedImport {
    /// Resolve placement against the current document without touching bytes.
    /// A supplied folder controls scene placement for PBRT and is required
    /// for a single resource import; PBRT dependencies retain kind folders.
    pub fn placed_for(&self, target: &ProjectDocument, folder: Option<&str>) -> Result<Self> {
        if folder.is_none() && self.document.scenes.is_empty() {
            return Err(ProjectError::Invalid(
                "a single resource import requires a destination folder".into(),
            ));
        }
        if let Some(folder) = folder {
            if !folder.is_empty() && !target.folders.contains(folder) {
                return Err(ProjectError::Invalid(
                    "import destination folder does not exist".into(),
                ));
            }
        }
        let mut namer = layout::LocationNamer::new();
        for location in target
            .scenes
            .values()
            .map(|scene| &scene.location)
            .chain(target.resources.values().map(|resource| &resource.location))
            .chain(target.assets.values().map(|asset| &asset.location))
            .chain(target.folders.iter())
        {
            namer.reserve(location);
        }
        // An explicit folder already passed the exact existence check
        // above, and a document cannot hold two folders differing only in
        // case (validation normalizes locations), so it binds to the exact
        // string as written. Defaults keep reusing the stored case.
        let mut document = self.document.clone();
        let single = document.scenes.is_empty();
        document.folders.clear();
        for scene in document.scenes.values_mut() {
            let scene = Arc::make_mut(scene);
            scene.location = namer.take(
                match folder {
                    Some(explicit) => explicit,
                    None => canonical_folder(target, "Scenes"),
                },
                &scene.label,
                "scene",
            );
            scene.label = stem(&scene.location)?;
        }
        let ids = document.resources.keys().copied().collect::<Vec<_>>();
        for id in ids {
            let resource = &document.resources[&id];
            let default_folder = layout::kind_folder(resource.kind);
            let extension = layout::kind_extension(resource.kind);
            let destination = if single {
                folder.unwrap_or(default_folder)
            } else {
                default_folder
            };
            let location = namer.take(
                match (single, folder) {
                    (true, Some(explicit)) => explicit,
                    _ => canonical_folder(target, destination),
                },
                &resource.label,
                extension,
            );
            let resource = document.resources.get_mut(&id).unwrap();
            resource.label = stem(&location)?;
            resource.location = location;
        }
        for asset in document.assets.values_mut() {
            let default_folder = match asset.media_type.as_str() {
                media if media.starts_with("image/") => "Images",
                "application/x-ply" => "Meshes",
                "application/x-pbrt-ui-array" => "Data",
                "application/x-nanovdb" => "Media",
                "text/plain" => "Spectra",
                _ => "Other",
            };
            let destination = if single {
                folder.unwrap_or(default_folder)
            } else {
                default_folder
            };
            let extension = extension(&asset.location)?.to_owned();
            asset.location = namer.take(
                match (single, folder) {
                    (true, Some(explicit)) => explicit,
                    _ => canonical_folder(target, destination),
                },
                &stem(&asset.location)?,
                &extension,
            );
        }
        for location in document
            .scenes
            .values()
            .map(|scene| &scene.location)
            .chain(
                document
                    .resources
                    .values()
                    .map(|resource| &resource.location),
            )
            .chain(document.assets.values().map(|asset| &asset.location))
        {
            let mut parent = Path::new(location).parent();
            while let Some(path) = parent {
                if path.as_os_str().is_empty() {
                    break;
                }
                document.folders.insert(path.to_string_lossy().into_owned());
                parent = path.parent();
            }
        }
        // The leases stay tied to their assets; the combined-document
        // validation (graph, references, file-vs-folder collisions) runs in
        // the publisher's candidate session before any mutation, so it is
        // not repeated here.
        Self::new(document, self.payloads.clone())
    }
}

fn stem(location: &str) -> Result<String> {
    Path::new(location)
        .file_stem()
        .and_then(|value| value.to_str())
        .map(str::to_owned)
        .ok_or_else(|| ProjectError::Invalid("invalid import item name".into()))
}
fn extension(location: &str) -> Result<&str> {
    // Unknown dependencies still need a safe basename, even without a suffix.
    Ok(Path::new(location)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("bin"))
}

fn canonical_folder<'a>(target: &'a ProjectDocument, requested: &'a str) -> &'a str {
    target
        .folders
        .iter()
        .find(|folder| folder.to_lowercase() == requested.to_lowercase())
        .map(String::as_str)
        .unwrap_or(requested)
}
