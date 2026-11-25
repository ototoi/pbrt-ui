//! Detached format-3 migration maps each output asset to the source
//! asset whose bytes it publishes.
use super::*;

pub struct FormatThreePlan {
    snapshot: ProjectSnapshot,
    sources: BTreeMap<AssetId, AssetId>,
    source_content: ContentId,
}

impl FormatThreePlan {
    pub fn snapshot(&self) -> &ProjectSnapshot {
        &self.snapshot
    }
    pub fn source_asset(&self, output: AssetId) -> Option<AssetId> {
        self.sources.get(&output).copied()
    }

    /// Format 3 keeps every UUID and location: images stay Assets and
    /// imagemap Textures keep referencing them, so nothing is copied.
    pub fn new(source: &ProjectSnapshot) -> Result<Self> {
        let document = source.document().clone();
        document.validate()?;
        let sources = document.assets.keys().map(|id| (*id, *id)).collect();
        Ok(Self {
            snapshot: ProjectSnapshot {
                document: std::sync::Arc::new(document),
                revision: source.revision().clone(),
                scene: source.scene(),
                payloads: vec![],
                references: Default::default(),
            },
            sources,
            source_content: ContentId::of(source.document()),
        })
    }
}

impl ProjectStore {
    /// Convert into a private sibling directory and publish only after all
    /// bytes, sidecars and references have been validated. Source is untouched.
    pub fn save_as_format_three(
        &self,
        destination: &Path,
        source: &ProjectSnapshot,
        cancelled: impl Fn() -> bool,
    ) -> Result<(Self, SaveReceipt)> {
        let plan = FormatThreePlan::new(source)?;
        self.publish_format_three(destination, source, &plan, cancelled)
    }

    /// Publish a detached format-three project; a cancelled publication can be retried as is.
    pub fn publish_format_three(
        &self,
        destination: &Path,
        source: &ProjectSnapshot,
        plan: &FormatThreePlan,
        cancelled: impl Fn() -> bool,
    ) -> Result<(Self, SaveReceipt)> {
        if plan.source_content != ContentId::of(source.document())
            || plan.snapshot.document().id != source.document().id
            || plan.snapshot.revision() != source.revision()
        {
            return Err(invalid("migration plan belongs to another snapshot"));
        }
        if let Self::Folder(store) = self {
            store.require_recovered()?;
        }
        if !self.unregistered_files()?.is_empty() {
            return Err(invalid(
                "migration requires registered source files; unregistered files would otherwise be omitted",
            ));
        }
        let (parent, destination) = folder::prepare_save_as_destination(destination)?;
        if cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let temporary = tempfile::tempdir_in(&parent)?;
        let staged = temporary.path().join("project");
        let snapshot = &plan.snapshot;
        let mut output = FolderStore::create_v3(
            &staged,
            snapshot.document().id,
            &snapshot.document().title,
            snapshot.revision().clone(),
        )?;
        // Each input location and its origin is looked up in one scan of the
        // source's Assets folder, made before any asset copies: resolving
        // them per asset re-walked the folder once per asset. Legacy stores
        // keep resolving through their in-memory index.
        let source_assets = if let Self::Folder(folder) = self {
            asset_files_by_id(&folder.directory().join("Assets"))?
        } else {
            BTreeMap::new()
        };
        let open_input = |asset: &Asset| -> Result<std::fs::File> {
            let old = plan.sources[&asset.id];
            // Bytes the snapshot retains (an undone, saved deletion keeps
            // them only in a payload lease) win over the folder, as
            // `ProjectSnapshot::asset_path` resolves them.
            if let Some(lease) = source.payload_by(old) {
                return Ok(std::fs::File::open(lease.path())?);
            }
            if let Self::Folder(_) = self {
                let (path, _) = source_assets
                    .get(&old)
                    .ok_or(ProjectError::AssetFileMissing(old))?;
                Ok(std::fs::File::open(path)?)
            } else {
                Ok(std::fs::File::open(source.asset_path(self, old)?)?)
            }
        };
        let origin_of = |asset: &Asset| -> Option<AssetMetaOrigin> {
            if let Self::Folder(_) = self {
                source_assets
                    .get(&plan.sources[&asset.id])
                    .and_then(|(_, origin)| origin.clone())
            } else {
                None
            }
        };
        output.import_assets_with_origins(
            snapshot
                .document()
                .assets
                .values()
                .map(|asset| (asset, origin_of(asset))),
            open_input,
            &cancelled,
        )?;
        let receipt = output.save_cancellable(snapshot, &cancelled)?;
        let published = output.read_document()?;
        published.validate()?;
        if ContentId::of(&published) != *receipt.content_id() {
            return Err(invalid("migration readback content mismatch"));
        }
        drop(output);
        if let Self::Folder(folder) = self {
            folder::copy_ui_sidecars(folder.directory(), &staged)?;
        }
        fs::remove_file(staged.join(".pbrtui/lock"))?;
        crate::sync_path(&staged)?;
        if cancelled() {
            return Err(ProjectError::Cancelled);
        }
        folder::remove_empty_save_as_destination(&destination)?;
        folder::rename_noreplace(&staged, &destination)?;
        crate::sync_path(&parent)?;
        FolderStore::open(&destination).map(|store| (Self::Folder(store), receipt))
    }
}
