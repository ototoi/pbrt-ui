use crate::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// of the transform parent: a run still carrying a re-parenting
/// reset (`Identity`, `Transform`, `CoordSysTransform`) — or a camera
/// without a pose — comes from an older import; its local transform is
/// refused until the scene is re-imported under the new rules.
pub(crate) const OLDER_IMPORT_RESET: &str =
    "the transform has a re-parenting reset (an older import)";

/// Immutable, Send snapshot used by saves and future render/import workers.
#[derive(Clone, Debug)]
pub struct ProjectSnapshot {
    pub(crate) document: Arc<ProjectDocument>,
    pub(crate) revision: Revision,
    /// Scene this snapshot renders/exports. Defaults to the session's active
    /// scene; `with_scene` pins another one explicitly.
    pub(crate) scene: Option<SceneId>,
    /// staged bytes this snapshot references —
    /// assets whose records an undo brought back while their file left
    /// the disk with a save. The store re-materializes them on the next
    /// save; render/export workers keep the leases alive by holding the
    /// snapshot.
    pub(crate) payloads: Vec<Arc<PayloadLease>>,
    /// The document's reference index, built on first use and shared by
    /// clones: a snapshot never changes.
    pub(crate) references: Arc<std::sync::OnceLock<Arc<ReferenceIndex>>>,
}
impl ProjectSnapshot {
    /// The document's reference index, built once per snapshot.
    pub fn reference_index(&self) -> Arc<ReferenceIndex> {
        self.references
            .get_or_init(|| Arc::new(self.document.reference_index()))
            .clone()
    }
    pub fn document(&self) -> &ProjectDocument {
        &self.document
    }
    pub fn revision(&self) -> &Revision {
        &self.revision
    }
    pub fn scene(&self) -> Option<SceneId> {
        self.scene
    }
    pub fn scene_view(&self) -> Result<SceneView<'_>> {
        self.document.scene_view(self.scene)
    }
    /// Same document and revision, evaluating another scene.
    pub fn with_scene(&self, scene: SceneId) -> Result<ProjectSnapshot> {
        self.document.scene_view(scene)?;
        Ok(ProjectSnapshot {
            document: self.document.clone(),
            revision: self.revision.clone(),
            scene: Some(scene),
            payloads: self.payloads.clone(),
            references: self.references.clone(),
        })
    }

    /// Resolve by identity, preferring the bytes retained by this snapshot.
    pub fn asset_path(&self, store: &ProjectStore, asset: AssetId) -> Result<std::path::PathBuf> {
        if !self.document.assets.contains_key(&asset) {
            return Err(invalid("asset is absent from snapshot"));
        }
        if let Some(lease) = self.payload_by(asset) {
            return Ok(lease.path().to_path_buf());
        }
        store.asset_path(asset)
    }

    /// Folder counterpart used by queries that have a root but do not own
    /// the I/O worker's store handle. Model placement may be unsaved.
    pub fn folder_asset_path(
        &self,
        assets_root: &std::path::Path,
        asset: AssetId,
    ) -> Result<std::path::PathBuf> {
        if !self.document.assets.contains_key(&asset) {
            return Err(invalid("asset is absent from snapshot"));
        }
        if let Some(lease) = self.payload_by(asset) {
            return Ok(lease.path().to_path_buf());
        }
        crate::store::asset_file_by_id(assets_root, asset)?
            .map(|(path, _)| path)
            .ok_or_else(|| invalid("asset bytes are missing"))
    }

    /// The staged bytes for one asset, if this snapshot
    /// carries them — the store's save asks when a record's file is gone.
    pub fn payload_by(&self, asset: AssetId) -> Option<&Arc<PayloadLease>> {
        self.payloads
            .iter()
            .rev()
            .find(|lease| lease.asset() == asset)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterTarget {
    Node(NodeId),
    Resource(ResourceId),
}

/// The content of an added area light component contains the
/// implementation plus the declared parameters. An empty implementation is
/// the explicit "no area light" that cancels a light shared from an outer
/// scope.
#[derive(Clone, Debug, PartialEq)]
pub struct AreaLightContent {
    pub implementation: String,
    pub parameters: Vec<Parameter>,
}

/// A node's new local transform (`set_local_matrix` / `set_local_trs`).
#[derive(Clone, Debug)]
pub enum LocalTransform {
    /// Row-major.
    Matrix([f64; 16]),
    Trs(crate::transform_trs::Trs),
}

#[derive(Clone, Debug)]
pub enum Edit {
    CreateCamera {
        scene: SceneId,
    },
    /// add one render configuration entity to the Scene's Options,
    /// in PBRT file order. `implementation` defaults per kind (rgb film,
    /// zsobol sampler, volpath integrator, gaussian filter, bvh
    /// accelerator); `parameters` declares what the creator chose — the
    /// rest follows r4's own defaults.
    CreateConfiguration {
        scene: SceneId,
        configuration: ConfigurationKind,
        implementation: Option<String>,
        parameters: Vec<Parameter>,
    },
    SetRenderCamera {
        scene: SceneId,
        camera: Option<NodeId>,
    },
    /// set a scene's up axis and handedness. Metadata only — node
    /// matrices never move, and exports write nothing.
    SetSceneCoordinateSystem {
        scene: SceneId,
        up: Axis,
        handedness: Handedness,
    },
    ImportBatch {
        batch: PreparedImport,
    },
    CreateScene {
        location: String,
    },
    DuplicateScene {
        scene: SceneId,
        location: String,
    },
    DeleteScene {
        scene: SceneId,
    },
    MoveItem {
        item: ItemId,
        location: String,
    },
    CreateFolder {
        path: String,
    },
    MoveFolder {
        from: String,
        to: String,
    },
    DeleteFolder {
        path: String,
    },
    DeleteItem {
        item: ItemId,
    },
    SetAssetReference {
        target: ParameterTarget,
        parameter: String,
        asset: AssetId,
    },
    CreatePreset {
        preset: ResourceId,
        parent: NodeId,
        after: Option<NodeId>,
        definition: Option<ResourceId>,
        /// the viewport view the new object takes:
        /// `camera.perspective` poses with it, and a shape, light or area
        /// preset is placed at its look-at point (a shape also sized to a
        /// tenth of the eye's distance). The group and instance presets
        /// take no view.
        view: Option<crate::presets::PresetView>,
    },
    /// create one resource. The location comes
    /// from the shared rules; object definitions come with an empty
    /// ObjectDefinition scope in `definition_nodes`, everything else needs
    /// an implementation (textures additionally a value type).
    CreateResource {
        /// The new resource's id; None picks a fresh one. Core passes one
        /// when a later edit of the same transaction refers to it ('s
        /// create-and-connect), so validating and applying agree.
        id: Option<ResourceId>,
        resource_kind: ResourceKind,
        implementation: String,
        label: String,
        /// None lands in the kind's default folder.
        folder: Option<String>,
        texture_value_type: Option<TextureValueType>,
        parameters: Vec<Parameter>,
    },
    /// Turn a scene Attribute/Transform group into an ObjectDefinition and
    /// leave an instance in its place, as one undoable edit.
    MakeObjectDefinition {
        node: NodeId,
        label: Option<String>,
    },
    UnpackObjectInstance {
        node: NodeId,
    },
    /// the area light component of one object — an entity at the head of
    /// the binding run in the object's dedicated group. `None` removes it;
    /// a light shared from an outer scope is cancelled with an explicit
    /// empty-implementation entity so siblings keep theirs.
    SetAreaLight {
        node: NodeId,
        light: Option<AreaLightContent>,
    },
    /// remove the object's local material and/or medium binding states
    ///; `set_local_binding`'s clearing values
    /// go through the same removal.
    ClearLocalBinding {
        node: NodeId,
        material: bool,
        medium: bool,
    },
    /// make reverse orientation take `enabled` effect for the object.
    /// The states toggle, so the dedicated group holds a cancelling toggle
    /// when the outer scope's parity asks for one.
    SetReverseOrientation {
        node: NodeId,
        enabled: bool,
    },
    /// Replace the transform states of a group's leading run or
    /// an object's dedicated group run (created on demand) with ones that
    /// make `value` the node's local transform. A run holding PBRT `Identity`
    /// or `Transform` has the root as its transform parent and keeps it.
    SetLocalTransform {
        node: NodeId,
        value: LocalTransform,
    },
    /// Object-local bindings. `None` leaves that
    /// binding unchanged; `MaterialBinding::Default` and an empty
    /// `MediumInterface` clear an existing local binding without adding one.
    SetLocalBinding {
        node: NodeId,
        material: Option<MaterialBinding>,
        medium: Option<MediumInterface>,
    },
    RenameNode {
        node: NodeId,
        label: String,
    },
    SetNodeActive {
        node: NodeId,
        active: bool,
    },
    RenameResource {
        resource: ResourceId,
        label: String,
    },
    SetTextureValueType {
        resource: ResourceId,
        value: TextureValueType,
    },
    SetTransform {
        node: NodeId,
        value: TransformOperation,
    },
    SetMaterialBinding {
        node: NodeId,
        value: MaterialBinding,
    },
    SetInstanceDefinition {
        node: NodeId,
        definition: ResourceId,
    },
    /// Remove an object (with its dedicated group, if it has one).
    DeleteNode {
        node: NodeId,
    },
    /// Copy an object (with its dedicated group) right after itself.
    DuplicateNode {
        node: NodeId,
    },
    /// Put an object (with its dedicated group) into another group, after
    /// `after` or at the end. It takes the new place's state.
    MoveNode {
        node: NodeId,
        parent: NodeId,
        after: Option<NodeId>,
    },
    /// None restores an omitted declaration, rather than a renderer default.
    SetParameter {
        target: ParameterTarget,
        name: String,
        value_type: String,
        value: Option<Parameter>,
    },
    SetParameterValues {
        target: ParameterTarget,
        name: String,
        offset: usize,
        values: ParameterValues,
    },
    /// rename an entity's, configuration's or
    /// resource's implementation, keeping every parameter. Core resolves
    /// the name through the parameter table and refuses what the new
    /// implementation could not export; the session refuses only the
    /// targets without a changeable implementation ('s cancelling area
    /// light, object definitions, other node kinds) and an empty name.
    SetImplementation {
        target: ParameterTarget,
        implementation: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeSet {
    pub revision: Revision,
    pub nodes: Vec<NodeId>,
    pub resources: Vec<ResourceId>,
    pub created: Vec<NodeId>,
    pub created_items: Vec<ItemId>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediumInterface {
    pub inside: Option<ResourceId>,
    pub outside: Option<ResourceId>,
}

/// Which node table owns an edited node. Crate-visible for the
/// import-time `from` fold, which shares the session's
/// dedicated-group machinery).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum NodeLocation {
    Scene(SceneId),
    Definition,
}
impl From<SceneId> for NodeLocation {
    fn from(scene: SceneId) -> Self {
        Self::Scene(scene)
    }
}
impl NodeLocation {
    /// The scene for `node_mut`; `None` looks in the definition nodes.
    fn scene(self) -> Option<SceneId> {
        match self {
            Self::Scene(scene) => Some(scene),
            Self::Definition => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Delta {
    nodes: Vec<(NodeLocation, NodeId, Node, Node)>,
    resources: Vec<(ResourceId, Option<Resource>, Option<Resource>)>,
    scenes: Vec<(
        SceneId,
        Option<Arc<SceneDocument>>,
        Option<Arc<SceneDocument>>,
    )>,
    assets: Vec<(AssetId, Option<Asset>, Option<Asset>)>,
    folders: Option<(BTreeSet<String>, BTreeSet<String>)>,
    created_nodes: Vec<(NodeLocation, Node)>,
    deleted_nodes: Vec<(NodeLocation, Node)>,
    /// Import records: no edit moves them yet; future import
    /// transactions will — but restore already carries them so the
    /// history's entry type does not change shape again.
    imports: Vec<(ImportId, Option<ImportRecord>, Option<ImportRecord>)>,
}

/// Single editing owner. History stores only the touched records, not one
/// full scene per edit. Render/save snapshots remain immutable during edits.
pub struct ProjectSession {
    document: Arc<ProjectDocument>,
    /// Cached at document changes so an idle dirty check stays O(1).
    current_content: ContentId,
    active_scene: Option<SceneId>,
    revision: u64,
    saved_revision: Option<u64>,
    /// the content the last accepted save
    /// published. Dirty compares content; `saved_revision` stays as the
    /// receipt-order guard and the wire-visible view of the same baseline.
    saved_content: Option<ContentId>,
    /// Where deleted assets stage their bytes. `None` until the
    /// store owner installs the grounds; deletions then simply pin nothing.
    payloads: Option<PayloadStaging>,
    /// Leases needed by assets currently in the document, independent of
    /// redo entries that may be discarded by a later edit.
    live_payloads: BTreeMap<AssetId, Arc<PayloadLease>>,
    /// Staged copies `snapshot_for_io` made of persisted assets. A copy never
    /// changes, so it is reused while its asset's hash and size still match.
    io_leases: std::cell::RefCell<BTreeMap<AssetId, StagedAsset>>,
    history: History,
    /// one reference index per revision, shared
    /// by the read-side queries (Contents counts, list_references). Edits
    /// check the working document through `ReferenceIndex::build` directly
    /// and never read this.
    reference_cache: std::cell::RefCell<Option<(u64, Arc<crate::ReferenceIndex>)>>,
}

type StagedAsset = (Arc<PayloadLease>, Option<SourceStamp>);

/// The source file a staged copy came from, as last seen.
struct SourceStamp {
    path: std::path::PathBuf,
    len: u64,
    modified: Option<std::time::SystemTime>,
}
impl SourceStamp {
    fn of(path: &std::path::Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        Some(Self {
            path: path.to_owned(),
            len: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }
    fn unchanged(&self) -> bool {
        Self::of(&self.path).is_some_and(|now| now.len == self.len && now.modified == self.modified)
    }
}

/// A payload stage in `<project>/Library/Staging`, beside the project's `Assets/`.
fn project_payload_stage(assets_root: &std::path::Path) -> std::io::Result<tempfile::TempDir> {
    let project_root = assets_root.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "project Assets directory must have a project folder parent",
        )
    })?;
    crate::temp_stage::tempdir_in(
        &project_root.join("Library").join("Staging"),
        "pbrt-ui-payloads-",
    )
}

/// Retained payloads linked (or copied) into a new project's stage, made
/// on the Save As worker so the copy never blocks the session's thread.
pub struct RelocatedPayloads {
    owner: Arc<tempfile::TempDir>,
    assets_root: std::path::PathBuf,
    /// Old staged file → its lease in the new stage.
    moved: BTreeMap<std::path::PathBuf, Arc<PayloadLease>>,
}
impl RelocatedPayloads {
    pub fn prepare(leases: &[Arc<PayloadLease>], assets_root: &std::path::Path) -> Result<Self> {
        let mut relocated = Self {
            owner: Arc::new(project_payload_stage(assets_root)?),
            assets_root: assets_root.to_path_buf(),
            moved: BTreeMap::new(),
        };
        for lease in leases {
            relocated.include(lease)?;
        }
        Ok(relocated)
    }

    fn include(&mut self, lease: &PayloadLease) -> Result<()> {
        if !self.moved.contains_key(lease.path()) {
            let new = lease.relink(self.owner.path(), Some(self.owner.clone()))?;
            self.moved.insert(lease.path().to_path_buf(), new);
        }
        Ok(())
    }
}

/// The grounds payload leases stage from: a session-private
/// staging area and the store's `Assets/` root on disk.
struct PayloadStaging {
    staging: std::path::PathBuf,
    assets_root: std::path::PathBuf,
    owner: Option<Arc<tempfile::TempDir>>,
}
impl ProjectSession {
    pub fn new(document: ProjectDocument) -> Result<Self> {
        Self::from_document(document, 0, None)
    }
    /// Resume a snapshot verified by a project store; edit history starts empty.
    pub fn from_saved_snapshot(snapshot: ProjectSnapshot) -> Result<Self> {
        let revision = crate::validation::revision(&snapshot.revision)?;
        snapshot.document.validate()?;
        if let Some(scene) = snapshot.scene {
            snapshot.document.scene_view(scene)?;
        }
        // a resumed snapshot is clean at exactly its read-back content.
        let current_content = ContentId::of(&snapshot.document);
        let saved_content = current_content.clone();
        Ok(Self {
            active_scene: snapshot.scene,
            document: snapshot.document,
            current_content,
            revision,
            saved_revision: Some(revision),
            saved_content: Some(saved_content),
            payloads: None,
            live_payloads: snapshot
                .payloads
                .into_iter()
                .map(|lease| (lease.asset(), lease))
                .collect(),
            io_leases: Default::default(),
            history: History::default(),
            reference_cache: Default::default(),
        })
    }
    pub(crate) fn loaded(document: ProjectDocument, revision: u64) -> Result<Self> {
        Self::from_document(document, revision, Some(revision))
    }
    fn from_document(
        document: ProjectDocument,
        revision: u64,
        saved_revision: Option<u64>,
    ) -> Result<Self> {
        document.validate()?;
        // clean exactly where a baseline was anchored — a loaded store
        // is clean at its read-back content; a brand-new session is dirty
        // until its first save.
        let current_content = ContentId::of(&document);
        let saved_content = saved_revision.is_some().then(|| current_content.clone());
        Ok(Self {
            active_scene: document.default_scene,
            document: Arc::new(document),
            current_content,
            revision,
            saved_revision,
            saved_content,
            payloads: None,
            live_payloads: BTreeMap::new(),
            io_leases: Default::default(),
            history: History::default(),
            reference_cache: Default::default(),
        })
    }
    /// install the grounds deleted assets stage
    /// their bytes from — a session-private staging area (its owner, who
    /// knows the store, provides it) and the store's `Assets/` root. Until
    /// installed, deletions pin nothing and keep today's behavior.
    pub fn enable_payload_staging(
        &mut self,
        staging: impl Into<std::path::PathBuf>,
        assets_root: impl Into<std::path::PathBuf>,
    ) {
        self.io_leases.get_mut().clear();
        self.payloads = Some(PayloadStaging {
            staging: staging.into(),
            assets_root: assets_root.into(),
            owner: None,
        });
    }

    /// Install a temporary, session-private payload stage for a live folder
    /// project. Leases retain the TempDir owner so snapshots remain readable
    /// after this session is closed.
    pub fn enable_temporary_payload_staging(
        &mut self,
        assets_root: &std::path::Path,
    ) -> Result<()> {
        let owner = Arc::new(project_payload_stage(assets_root)?);
        self.io_leases.get_mut().clear();
        self.payloads = Some(PayloadStaging {
            staging: owner.path().to_path_buf(),
            assets_root: assets_root.to_path_buf(),
            owner: Some(owner),
        });
        Ok(())
    }

    /// Allocate staging before accepting relocation; existing leases and
    /// their old root stay intact until the relocation is accepted.
    pub fn ensure_temporary_payload_staging(
        &mut self,
        assets_root: &std::path::Path,
    ) -> Result<()> {
        if self.payloads.is_none() {
            self.enable_temporary_payload_staging(assets_root)?;
        }
        Ok(())
    }

    /// Every payload the session keeps alive: live ones and the history's.
    /// Save As hands them to its worker (`RelocatedPayloads::prepare`).
    pub fn retained_payloads(&self) -> Vec<Arc<PayloadLease>> {
        self.live_payloads
            .values()
            .chain(self.history.entries().flat_map(|entry| &entry.payloads))
            .cloned()
            .collect()
    }

    /// Save As in one step: `RelocatedPayloads::prepare` then
    /// `adopt_relocated_payloads`, both on this thread.
    pub fn relocate_payload_staging(&mut self, assets_root: &std::path::Path) -> Result<()> {
        let relocated = RelocatedPayloads::prepare(&self.retained_payloads(), assets_root)?;
        self.adopt_relocated_payloads(relocated)
    }

    /// Save As: swap the retained payloads for their copies in the new
    /// project's stage, so deleting the old folder cannot take bytes an Undo
    /// still needs. A payload retained after `prepare` is relinked here. The
    /// asset root follows on acceptance (`set_payload_assets_root`). Nothing
    /// changes when a payload fails.
    pub fn adopt_relocated_payloads(&mut self, mut relocated: RelocatedPayloads) -> Result<()> {
        for lease in self.retained_payloads() {
            relocated.include(&lease)?;
        }
        let moved = &relocated.moved;
        let swap = |lease: &mut Arc<PayloadLease>| {
            if let Some(new) = moved.get(lease.path()) {
                *lease = new.clone();
            }
        };
        self.live_payloads.values_mut().for_each(swap);
        self.history
            .entries_mut()
            .flat_map(|entry| &mut entry.payloads)
            .for_each(swap);
        self.io_leases.get_mut().clear();
        let assets_root = match self.payloads.take() {
            Some(old) => old.assets_root,
            None => relocated.assets_root,
        };
        self.payloads = Some(PayloadStaging {
            staging: relocated.owner.path().to_path_buf(),
            assets_root,
            owner: Some(relocated.owner),
        });
        Ok(())
    }

    /// Update the disk root after Save As relocates the project.
    pub fn set_payload_assets_root(&mut self, assets_root: impl Into<std::path::PathBuf>) {
        // Copies staged from the old root must not stand in for the new one.
        self.io_leases.get_mut().clear();
        if let Some(payloads) = &mut self.payloads {
            payloads.assets_root = assets_root.into();
        }
    }

    /// Stage a deleted asset's bytes in history. Deletion pins the bytes
    /// when possible, but a later save cannot restore bytes that were not kept.
    /// `Ok(None)` when there is nothing to pin: no grounds installed, or
    /// the bytes already gone. A copy that exists but fails to stage is
    /// an error the edit refuses.
    fn stage_deleted_asset(&self, asset: &Asset) -> Result<Option<Arc<PayloadLease>>> {
        if let Some(lease) = self.live_payloads.get(&asset.id) {
            return Ok(Some(lease.clone()));
        }
        let Some(grounds) = &self.payloads else {
            return Ok(None);
        };
        let persisted = crate::store::asset_file_by_id(&grounds.assets_root, asset.id)?;
        // A copy: Undo must not see later in-place edits of the user's file.
        Ok(self
            .stage_asset_at(asset, persisted, false)?
            .map(|(lease, _)| lease))
    }

    fn stage_asset_at(
        &self,
        asset: &Asset,
        persisted: Option<(std::path::PathBuf, Option<crate::store::AssetMetaOrigin>)>,
        link: bool,
    ) -> Result<Option<StagedAsset>> {
        let Some(grounds) = &self.payloads else {
            return Ok(None);
        };
        let (source, origin) = if let Some((path, origin)) = persisted {
            (path, origin)
        } else {
            let candidate = grounds.assets_root.join(&asset.location);
            let mut sidecar = candidate.as_os_str().to_owned();
            sidecar.push(".meta");
            // Do not treat a different item's sidecar at the current location
            // as this asset's persisted bytes.
            if !candidate.is_file() || std::path::Path::new(&sidecar).exists() {
                return Ok(None);
            }
            (candidate, None)
        };
        if !source.is_file() {
            return Ok(None);
        }
        let stamp = SourceStamp::of(&source);
        let lease = PayloadLease::stage_cancellable(
            &grounds.staging,
            asset.id,
            &asset.sha256,
            asset.size,
            &source,
            origin,
            grounds.owner.clone(),
            link,
            || false,
        )
        // Bytes unlike the record changed outside the application.
        .map_err(|error| match error {
            ProjectError::Io(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                ProjectError::SourceChanged(format!("Assets/{}", asset.location))
            }
            error => error,
        })?;
        Ok(Some((lease, stamp)))
    }

    fn reconcile_live_payloads(&mut self, leases: &[Arc<PayloadLease>]) {
        let assets = &self.document.assets;
        self.live_payloads.retain(|id, _| assets.contains_key(id));
        for lease in leases {
            if assets.contains_key(&lease.asset()) {
                self.live_payloads.insert(lease.asset(), lease.clone());
            }
        }
    }

    /// Pin existing folder bytes before handing a snapshot to an I/O job.
    /// This can fail, unlike the cheap model snapshot used by read queries.
    /// An asset file rewritten outside the application is adopted first: its
    /// record takes the bytes on disk (project folder §5), without an Undo
    /// step and without changing whether the session is dirty.
    pub fn snapshot_for_io(&mut self) -> Result<ProjectSnapshot> {
        // History bytes stand in only while they still match the record; an
        // adopted edit leaves the history's copy stale.
        let mut held = self
            .snapshot()
            .payloads
            .iter()
            .filter(|lease| {
                self.document
                    .assets
                    .get(&lease.asset())
                    .is_some_and(|asset| {
                        asset.sha256 == lease.sha256() && asset.size == lease.size()
                    })
            })
            .map(|lease| lease.asset())
            .collect::<BTreeSet<_>>();
        let mut adopted = Vec::new();
        {
            // Borrowed in place: an early error must keep the pins, which may hold
            // the only bytes of a file deleted outside.
            let mut cache = self.io_leases.borrow_mut();
            // A file rewritten outside is staged again, which reports the change.
            cache.retain(|id, (lease, stamp)| {
                self.document
                .assets
                .get(id)
                .is_some_and(|asset| asset.sha256 == lease.sha256() && asset.size == lease.size())
                // A pin outlives its file deleted outside: the save restores it.
                && stamp
                    .as_ref()
                    .is_none_or(|stamp| stamp.unchanged() || !stamp.path.exists())
            });
            let unpinned = self
                .document
                .assets
                .keys()
                .any(|id| !held.contains(id) && !cache.contains_key(id));
            // Scanning `Assets/` costs as much as the copies; skip it when warm.
            let mut files = match &self.payloads {
                Some(grounds) if unpinned || !held.is_empty() => {
                    crate::store::asset_files_by_id(&grounds.assets_root)?
                }
                _ => BTreeMap::new(),
            };
            // History bytes stand in only for a file that is gone; a file on disk wins.
            held.retain(|id| !files.contains_key(id));
            let missing = self
                .document
                .assets
                .keys()
                .filter(|id| !held.contains(id) && !cache.contains_key(id))
                .copied()
                .collect::<Vec<_>>();
            for id in missing {
                let mut asset = self.document.assets[&id].clone();
                let persisted = files.remove(&id);
                let staged = match self.stage_asset_at(&asset, persisted.clone(), true) {
                    Err(ProjectError::SourceChanged(_)) if persisted.is_some() => {
                        let (path, _) = persisted.as_ref().unwrap();
                        (asset.sha256, asset.size) = crate::store::hash_file(path)?;
                        let staged = self.stage_asset_at(&asset, persisted, true)?;
                        adopted.push(asset);
                        staged
                    }
                    staged => staged?,
                };
                if let Some(staged) = staged {
                    cache.insert(id, staged);
                }
            }
        }
        if !adopted.is_empty() {
            let clean = self.saved_content.as_ref() == Some(&self.current_content);
            let mut document = (*self.document).clone();
            for asset in adopted {
                self.live_payloads.remove(&asset.id);
                document.assets.insert(asset.id, asset);
            }
            self.document = Arc::new(document);
            self.current_content = ContentId::of(&self.document);
            if clean {
                self.saved_content = Some(self.current_content.clone());
            }
        }
        let mut snapshot = self.snapshot();
        snapshot
            .payloads
            .retain(|lease| held.contains(&lease.asset()));
        let cache = self.io_leases.get_mut();
        for id in self.document.assets.keys() {
            if !held.contains(id)
                && let Some((lease, _)) = cache.get(id)
            {
                snapshot.payloads.push(lease.clone());
            }
        }
        Ok(snapshot)
    }

    pub fn snapshot(&self) -> ProjectSnapshot {
        // carry the history's staged bytes for assets the document
        // holds — a restored-by-undo record whose file a save removed.
        let mut payloads = self.live_payloads.clone();
        for entry in self.history.entries() {
            for lease in &entry.payloads {
                if self.document.assets.contains_key(&lease.asset()) {
                    payloads
                        .entry(lease.asset())
                        .or_insert_with(|| lease.clone());
                }
            }
        }
        // Seed the snapshot with this revision's index when one is built.
        let references = self
            .reference_cache
            .try_borrow()
            .ok()
            .and_then(|cache| {
                cache
                    .as_ref()
                    .filter(|(revision, _)| *revision == self.revision)
                    .map(|(_, index)| std::sync::OnceLock::from(index.clone()))
            })
            .unwrap_or_default();
        ProjectSnapshot {
            document: self.document.clone(),
            revision: Revision::from_u64(self.revision),
            scene: self.active_scene,
            payloads: payloads.into_values().collect(),
            references: Arc::new(references),
        }
    }
    pub fn document(&self) -> &ProjectDocument {
        &self.document
    }
    /// The document's reference index, built once per revision. The
    /// cache keys on the revision counter: reference-relevant fields only
    /// change through applies, which all bump it.
    pub fn reference_index(&self) -> Arc<crate::ReferenceIndex> {
        let mut cache = match self.reference_cache.try_borrow_mut() {
            Ok(cache) => cache,
            // The index build reads the document only; a re-entrant call
            // can only come from within this very function's build, which
            // cannot happen.
            Err(_) => return Arc::new(self.document.reference_index()),
        };
        if let Some((at, index)) = &*cache
            && *at == self.revision
        {
            return index.clone();
        }
        let index = Arc::new(self.document.reference_index());
        *cache = Some((self.revision, index.clone()));
        index
    }
    pub fn active_scene(&self) -> Option<SceneId> {
        self.active_scene
    }
    /// Switch the display target without changing project edit history.
    pub fn open_scene(&mut self, scene: SceneId) -> Result<()> {
        self.document.scene_view(scene)?;
        if self.active_scene != Some(scene) {
            self.active_scene = Some(scene);
            Arc::make_mut(&mut self.document).default_scene = Some(scene);
        }
        Ok(())
    }
    pub fn close_scene(&mut self) {
        self.active_scene = None;
        Arc::make_mut(&mut self.document).default_scene = None;
    }
    fn repair_active_scene(&mut self) {
        self.active_scene = self
            .active_scene
            .filter(|id| self.document.scenes.contains_key(id));
    }
    pub fn revision(&self) -> Revision {
        Revision::from_u64(self.revision)
    }
    pub fn saved_revision(&self) -> Option<Revision> {
        self.saved_revision.map(Revision::from_u64)
    }
    pub fn is_dirty(&self) -> bool {
        // dirty compares the current content with
        // the content the last accepted save published — not the revision,
        // which moves on undo/redo too. Unsavable sessions (no receipt yet)
        // are dirty.
        match &self.saved_content {
            None => true,
            Some(saved) => &self.current_content != saved,
        }
    }
    pub fn can_undo(&self) -> bool {
        self.history.undo_peek().is_some()
    }
    pub fn can_redo(&self) -> bool {
        self.history.redo_peek().is_some()
    }

    /// Replace a read-only compatibility document after explicit migration.
    /// Editable sessions must use ordinary Save As so their history survives.
    pub fn accept_saved_migration(
        &mut self,
        document: ProjectDocument,
        receipt: &SaveReceipt,
    ) -> Result<()> {
        if self.can_undo()
            || self.can_redo()
            || receipt.revision() != &self.revision()
            || document.id != self.document.id
            || receipt.project_id() != document.id
        {
            return Err(invalid(
                "migration cannot replace an edited or unrelated session",
            ));
        }
        document.validate()?;
        if ContentId::of(&document) != *receipt.content_id() {
            return Err(invalid("migration content does not match save receipt"));
        }
        let active = self
            .active_scene
            .filter(|id| document.scenes.contains_key(id));
        self.document = Arc::new(document);
        self.active_scene = active;
        self.current_content = receipt.content_id().clone();
        self.reference_cache = Default::default();
        self.mark_saved(receipt)
    }

    pub fn mark_saved(&mut self, receipt: &SaveReceipt) -> Result<()> {
        if receipt.project_id() != self.document.id {
            return Err(invalid("save receipt belongs to a different project"));
        }
        let revision = crate::validation::revision(receipt.revision())?;
        if revision > self.revision {
            return Err(invalid("save receipt is newer than the session"));
        }
        // Accept receipts in save order only. An older in-flight
        // save finishing after a newer one must not re-anchor the baseline to
        // content the newer save already superseded.
        let accept = self.saved_revision.is_none_or(|saved| revision >= saved);
        self.saved_revision = Some(
            self.saved_revision
                .map_or(revision, |old| old.max(revision)),
        );
        if accept {
            self.saved_content = Some(receipt.content_id().clone());
            if self.current_content == *receipt.content_id() {
                self.live_payloads.clear();
            }
        }
        Ok(())
    }

    /// Merge store-registered items created while saving unregistered files.
    /// These records belong to the saved snapshot and do not advance its edit
    /// revision; edits made while the save was in flight remain in place.
    pub fn merge_registered_items(
        &mut self,
        registered: &ProjectDocument,
        unregistered: &[String],
    ) -> bool {
        if registered.id != self.document.id {
            return false;
        }
        let mut document = (*self.document).clone();
        for (id, scene) in &registered.scenes {
            if unregistered.contains(&scene.location) {
                document.scenes.entry(*id).or_insert_with(|| scene.clone());
            }
        }
        for (id, resource) in &registered.resources {
            if unregistered.contains(&resource.location) {
                document
                    .resources
                    .entry(*id)
                    .or_insert_with(|| resource.clone());
            }
        }
        let mut definitions = registered
            .resources
            .values()
            .filter(|resource| unregistered.contains(&resource.location))
            .filter_map(|resource| resource.definition_scope)
            .collect::<Vec<_>>();
        while let Some(id) = definitions.pop() {
            let Some(node) = registered.definition_nodes.get(&id) else {
                continue;
            };
            if let NodeKind::Scope { children, .. } = &node.data {
                definitions.extend(children.iter().copied());
            }
            document
                .definition_nodes
                .entry(id)
                .or_insert_with(|| node.clone());
        }
        for (id, asset) in &registered.assets {
            if unregistered.contains(&asset.location) {
                document.assets.entry(*id).or_insert_with(|| asset.clone());
            }
        }
        document.folders.extend(
            registered
                .folders
                .iter()
                .filter(|folder| {
                    unregistered
                        .iter()
                        .any(|path| path.starts_with(&format!("{folder}/")))
                })
                .cloned(),
        );
        if document.validate().is_err() {
            return false;
        }
        self.document = Arc::new(document);
        self.repair_active_scene();
        self.current_content = ContentId::of(&self.document);
        self.reconcile_live_payloads(&[]);
        if self.saved_content.as_ref() == Some(&self.current_content) {
            self.live_payloads.clear();
        }
        true
    }

    fn check(&self, expected: &Revision) -> Result<()> {
        if expected.to_u64() != Some(self.revision) {
            return Err(ProjectError::RevisionConflict {
                expected: expected.as_str().into(),
                current: self.revision.to_string(),
            });
        }
        Ok(())
    }

    pub fn apply(&mut self, expected: &Revision, edits: &[Edit]) -> Result<ChangeSet> {
        self.check(expected)?;
        let mut document = (*self.document).clone();
        let mut node_ids = BTreeMap::<NodeId, NodeLocation>::new();
        let mut resource_ids = BTreeSet::new();
        let mut created_nodes = Vec::<(NodeLocation, Node)>::new();
        let mut deleted_nodes = Vec::<(NodeLocation, Node)>::new();
        let mut created_items = Vec::<ItemId>::new();
        let mut scene_records = BTreeSet::<SceneId>::new();
        let mut asset_records = BTreeSet::<AssetId>::new();
        let mut import_records = BTreeSet::<ImportId>::new();
        // leases the transaction's deletions staged for the history.
        let mut staged_payloads = Vec::<Arc<PayloadLease>>::new();
        let mut folder_records = false;
        let active = self.active_scene;
        for edit in edits {
            match edit {
                Edit::ImportBatch { batch } => {
                    let source = &batch.document;
                    for (id, scene) in &source.scenes {
                        if document.scenes.contains_key(id) {
                            return Err(invalid("import scene identity already exists"));
                        }
                        document.scenes.insert(*id, scene.clone());
                        scene_records.insert(*id);
                        created_items.push(ItemId::Scene(*id));
                    }
                    for (id, resource) in &source.resources {
                        if document.resources.contains_key(id) {
                            return Err(invalid("import resource identity already exists"));
                        }
                        document.resources.insert(*id, resource.clone());
                        resource_ids.insert(*id);
                        created_items.push(ItemId::Resource(*id));
                    }
                    for (id, asset) in &source.assets {
                        if document.assets.contains_key(id) {
                            return Err(invalid("import asset identity already exists"));
                        }
                        document.assets.insert(*id, asset.clone());
                        asset_records.insert(*id);
                        created_items.push(ItemId::Asset(*id));
                    }
                    for (id, node) in &source.definition_nodes {
                        if document.definition_nodes.contains_key(id) {
                            return Err(invalid("import definition node identity already exists"));
                        }
                        document.definition_nodes.insert(*id, node.clone());
                        created_nodes.push((NodeLocation::Definition, node.clone()));
                    }
                    for (id, record) in &source.imports {
                        if document.imports.contains_key(id) {
                            return Err(invalid("import record identity already exists"));
                        }
                        document.imports.insert(*id, record.clone());
                        import_records.insert(*id);
                    }
                    document.folders.extend(source.folders.iter().cloned());
                    folder_records = true;
                    staged_payloads.extend(batch.payloads.iter().cloned());
                }
                Edit::CreateScene { location } => {
                    validate_scene_location(location)?;
                    let label = scene_label(location)?;
                    let scene = empty_scene(label, location.clone());
                    let id = scene.id;
                    document.scenes.insert(id, Arc::new(scene));
                    scene_records.insert(id);
                    created_items.push(ItemId::Scene(id));
                }
                Edit::SetRenderCamera { scene, camera } => {
                    let current = document
                        .scenes
                        .get(scene)
                        .ok_or_else(|| invalid("missing scene for render camera"))?;
                    if let Some(camera) = camera {
                        if !matches!(
                            current.nodes.get(camera).map(|node| &node.data),
                            Some(NodeKind::Entity {
                                entity: EntityKind::Camera,
                                ..
                            })
                        ) || !current.options.contains(camera)
                        {
                            return Err(invalid(
                                "render camera must be a Camera in the same scene",
                            ));
                        }
                    }
                    if current.render_camera != *camera {
                        let mut updated = current.as_ref().clone();
                        updated.render_camera = *camera;
                        document.scenes.insert(*scene, Arc::new(updated));
                        scene_records.insert(*scene);
                    }
                }
                Edit::SetSceneCoordinateSystem {
                    scene,
                    up,
                    handedness,
                } => {
                    let current = document
                        .scenes
                        .get(scene)
                        .ok_or_else(|| invalid("missing scene for coordinate system"))?;
                    let requested = CoordinateSystem::new(*up, *handedness);
                    if current.coordinate_system != requested {
                        let mut updated = current.as_ref().clone();
                        updated.coordinate_system = requested;
                        document.scenes.insert(*scene, Arc::new(updated));
                        scene_records.insert(*scene);
                    }
                }
                Edit::CreateCamera { scene } => {
                    let target = document
                        .scenes
                        .get(scene)
                        .ok_or_else(|| invalid("missing scene for Camera creation"))?;
                    let mut updated = target.as_ref().clone();
                    let is_first_camera = !updated.options.iter().any(|id| {
                        matches!(
                            updated.nodes.get(id).map(|node| &node.data),
                            Some(NodeKind::Entity {
                                entity: EntityKind::Camera,
                                ..
                            })
                        )
                    });
                    let camera = Node::new(
                        "Camera",
                        NodeKind::Entity {
                            entity: EntityKind::Camera,
                            implementation: "perspective".into(),
                            parameters: Vec::new(),
                            // a created camera starts at the identity
                            // pose so its transform edits work immediately.
                            pose: Some(CameraPose {
                                start: crate::math::Mat4::identity().0,
                                end: crate::math::Mat4::identity().0,
                            }),
                        },
                    );
                    updated.options.push(camera.id);
                    updated.nodes.insert(camera.id, camera.clone());
                    if is_first_camera && updated.render_camera.is_none() {
                        updated.render_camera = Some(camera.id);
                    }
                    document.scenes.insert(*scene, Arc::new(updated));
                    scene_records.insert(*scene);
                    created_nodes.push((NodeLocation::Scene(*scene), camera));
                }
                Edit::CreateConfiguration {
                    scene,
                    configuration,
                    implementation,
                    parameters,
                } => {
                    let target = document
                        .scenes
                        .get(scene)
                        .ok_or_else(|| invalid("missing scene for render configuration"))?;
                    // One of each per scene, as Add Component offers them.
                    if target.nodes.values().any(|node| {
                        matches!(&node.data, NodeKind::Configuration { configuration: found, .. }
                            if found == configuration)
                    }) {
                        return Err(invalid("the scene already has this render configuration"));
                    }
                    let implementation = implementation.clone().unwrap_or_else(|| {
                        crate::presets::default_implementation(*configuration).to_owned()
                    });
                    if implementation.trim().is_empty() {
                        return Err(invalid("a render configuration needs an implementation"));
                    }
                    // The label matches what PBRT import names these nodes
                    // (the directive), so an added one and an imported one
                    // read the same.
                    let label = match configuration {
                        ConfigurationKind::Film => "Film",
                        ConfigurationKind::Sampler => "Sampler",
                        ConfigurationKind::Integrator => "Integrator",
                        ConfigurationKind::PixelFilter => "PixelFilter",
                        ConfigurationKind::Accelerator => "Accelerator",
                    };
                    let node = Node::new(
                        label,
                        NodeKind::Configuration {
                            configuration: *configuration,
                            implementation,
                            parameters: parameters.clone(),
                        },
                    );
                    let mut updated = target.as_ref().clone();
                    updated.options.push(node.id);
                    updated.nodes.insert(node.id, node.clone());
                    document.scenes.insert(*scene, Arc::new(updated));
                    scene_records.insert(*scene);
                    created_nodes.push((NodeLocation::Scene(*scene), node));
                }
                Edit::DuplicateScene { scene, location } => {
                    validate_scene_location(location)?;
                    let source = document
                        .scenes
                        .get(scene)
                        .ok_or_else(|| invalid("missing scene to duplicate"))?;
                    let mut copy = source.as_ref().clone();
                    let remap = copy
                        .nodes
                        .keys()
                        .map(|id| (*id, NodeId::new()))
                        .collect::<BTreeMap<_, _>>();
                    copy.id = SceneId::new();
                    copy.label = scene_label(location)?;
                    copy.location = location.clone();
                    copy.world = *remap
                        .get(&copy.world)
                        .ok_or_else(|| invalid("scene world node is missing"))?;
                    copy.options = copy
                        .options
                        .iter()
                        .map(|id| {
                            remap
                                .get(id)
                                .copied()
                                .ok_or_else(|| invalid("scene option node is missing"))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    copy.render_camera = copy.render_camera.map(|id| remap[&id]);
                    copy.nodes = copy
                        .nodes
                        .into_values()
                        .map(|mut node| {
                            node.id = remap[&node.id];
                            if let NodeKind::Scope {
                                children,
                                parent_camera,
                                ..
                            } = &mut node.data
                            {
                                *children = children.iter().map(|id| remap[id]).collect();
                                // an attached group's parent camera is a
                                // node id and remaps like any other link.
                                *parent_camera = parent_camera.map(|id| remap[&id]);
                            }
                            (node.id, node)
                        })
                        .collect();
                    let id = copy.id;
                    document.scenes.insert(id, Arc::new(copy));
                    scene_records.insert(id);
                    created_items.push(ItemId::Scene(id));
                }
                Edit::DeleteScene { scene } => {
                    document
                        .scenes
                        .remove(scene)
                        .ok_or_else(|| invalid("missing scene to delete"))?;
                    scene_records.insert(*scene);
                    if document.default_scene == Some(*scene) {
                        document.default_scene = None;
                    }
                }
                Edit::MoveItem { item, location } => match item {
                    ItemId::Scene(id) => {
                        validate_scene_location(location)?;
                        let current = document
                            .scenes
                            .get(id)
                            .cloned()
                            .ok_or_else(|| invalid("missing scene to move"))?;
                        let mut moved = current.as_ref().clone();
                        moved.location = location.clone();
                        document.scenes.insert(*id, Arc::new(moved));
                        scene_records.insert(*id);
                    }
                    ItemId::Resource(id) => {
                        let resource = document
                            .resources
                            .get_mut(id)
                            .ok_or_else(|| invalid("missing resource to move"))?;
                        resource.location = location.clone();
                        resource_ids.insert(*id);
                    }
                    ItemId::Asset(id) => {
                        let asset = document
                            .assets
                            .get_mut(id)
                            .ok_or_else(|| invalid("missing asset to move"))?;
                        asset.location = location.clone();
                        asset_records.insert(*id);
                    }
                },
                Edit::CreateFolder { path } => {
                    if !document.folders.insert(path.clone()) {
                        return Err(invalid("folder already exists"));
                    }
                    folder_records = true;
                }
                Edit::MoveFolder { from, to } => {
                    move_folder(&mut document, from, to)?;
                    folder_records = true;
                    scene_records.extend(document.scenes.keys().copied());
                    asset_records.extend(document.assets.keys().copied());
                    resource_ids.extend(document.resources.keys().copied());
                }
                Edit::DeleteFolder { path } => {
                    delete_folder(&mut document, path)?;
                    folder_records = true;
                }
                Edit::DeleteItem { item } => {
                    if document.reference_index().count(*item) > 0 {
                        return Err(invalid("cannot delete a referenced project item"));
                    }
                    match item {
                        ItemId::Scene(id) => {
                            document
                                .scenes
                                .remove(id)
                                .ok_or_else(|| invalid("missing scene"))?;
                            scene_records.insert(*id);
                        }
                        ItemId::Resource(id) => {
                            let resource = document
                                .resources
                                .remove(id)
                                .ok_or_else(|| invalid("missing resource"))?;
                            if let Some(root) = resource.definition_scope {
                                record_deleted_subtree(
                                    &self.document,
                                    NodeLocation::Definition,
                                    remove_definition_subtree(&mut document, root),
                                    &mut deleted_nodes,
                                );
                            }
                            resource_ids.insert(*id);
                        }
                        ItemId::Asset(id) => {
                            let asset = document
                                .assets
                                .remove(id)
                                .ok_or_else(|| invalid("missing asset"))?;
                            // The deletion keeps referencing
                            // its bytes from the history, so a save that has
                            // already removed the disk file cannot starve a
                            // later undo of them.
                            if let Some(lease) = self.stage_deleted_asset(&asset)? {
                                staged_payloads.push(lease);
                            }
                            asset_records.insert(*id);
                        }
                    }
                }
                Edit::SetAssetReference {
                    target,
                    parameter,
                    asset,
                } => {
                    if !document.assets.contains_key(asset) {
                        return Err(invalid("missing replacement asset"));
                    }
                    match target {
                        ParameterTarget::Node(id) => {
                            let (node_value, location) = node_mut(&mut document, active, *id)?;
                            let params = node_parameters_mut(node_value)?;
                            set_asset_param(params, parameter, *asset)?;
                            node_ids.insert(*id, location);
                        }
                        ParameterTarget::Resource(id) => {
                            let resource = document
                                .resources
                                .get_mut(id)
                                .ok_or_else(|| invalid("missing parameter resource"))?;
                            set_asset_param(&mut resource.parameters, parameter, *asset)?;
                            resource_ids.insert(*id);
                        }
                    }
                }
                Edit::CreatePreset {
                    preset,
                    parent,
                    after,
                    definition,
                    view,
                } => {
                    let active = active.ok_or_else(|| invalid("no active scene"))?;
                    let preset = crate::presets::builtin(*preset)
                        .ok_or_else(|| invalid("unknown built-in preset"))?;
                    if document.definition_nodes.contains_key(parent)
                        && matches!(preset.category, "light" | "area" | "camera" | "instance")
                    {
                        return Err(invalid(
                            "light, camera and instance presets are not allowed in object definitions",
                        ));
                    }
                    // a Camera belongs to the Scene's Options, not to a
                    // group; `parent` and `after` are not its to give.
                    if preset.name == "camera.perspective" {
                        let target = document
                            .scenes
                            .get(&active)
                            .ok_or_else(|| invalid("missing scene for Camera creation"))?;
                        // The same refusal the pose computation gives: a
                        // degenerate view or up cannot place a camera.
                        crate::presets::camera_pose(target.coordinate_system, *view).ok_or_else(
                            || invalid("the requested camera placement is degenerate"),
                        )?;
                        let mut members =
                            crate::presets::make_body(preset, target.coordinate_system, *view);
                        let camera = members.pop().expect("the camera preset builds a camera");
                        let mut updated = target.as_ref().clone();
                        if updated.render_camera.is_none() {
                            updated.render_camera = Some(camera.id);
                        }
                        updated.options.push(camera.id);
                        updated.nodes.insert(camera.id, camera.clone());
                        document.scenes.insert(active, Arc::new(updated));
                        scene_records.insert(active);
                        created_nodes.push((NodeLocation::Scene(active), camera));
                        continue;
                    }
                    // a shape, light or area preset takes the view as a
                    // placement — its body opens with a Translate to the
                    // look-at point (a shape also scales to stay in frame).
                    // The group and instance presets create nothing placeable.
                    let view = match view {
                        None => None,
                        Some(view) if matches!(preset.category, "shape" | "light" | "area") => {
                            if !crate::presets::placeable(*view) {
                                return Err(invalid("the requested placement is degenerate"));
                            }
                            Some(*view)
                        }
                        Some(_) => {
                            return Err(invalid(
                                "view is valid only for the camera, shape, light and area presets",
                            ));
                        }
                    };
                    // In the scene graph the Scene root is the virtual parent
                    // of World content; the PBRT model still inserts into the
                    // real World scope.
                    let parent = if *parent == NodeId(active.0) {
                        document.scene(active).unwrap().world
                    } else {
                        *parent
                    };
                    if preset.name == "instance.object" {
                        let definition = definition.ok_or_else(|| {
                            invalid("object instance preset requires a definition")
                        })?;
                        if document
                            .resources
                            .get(&definition)
                            .is_none_or(|resource| resource.kind != ResourceKind::ObjectDefinition)
                        {
                            return Err(invalid("missing object definition"));
                        }
                    } else if definition.is_some() {
                        return Err(invalid(
                            "definition is valid only for object instance preset",
                        ));
                    }
                    let parent_location = if document
                        .scene(active)
                        .is_some_and(|scene| scene.nodes.contains_key(&parent))
                    {
                        NodeLocation::Scene(active)
                    } else if document.definition_nodes.contains_key(&parent) {
                        NodeLocation::Definition
                    } else {
                        return Err(invalid("missing preset parent"));
                    };
                    let after = after
                        .map(|after| object_unit_at(&document, parent_location, after))
                        .transpose()?;
                    // preset bodies orient with the scene they land in.
                    let scene_coordinate_system = document
                        .scene(active)
                        .map(|scene| scene.coordinate_system)
                        .unwrap_or_default();
                    let label = unique_visible_child_label(
                        &document,
                        parent_location,
                        parent,
                        preset.label,
                    )?;
                    if matches!(
                        node(&document, parent_location, parent).map(|node| &node.data),
                        Some(NodeKind::Scope {
                            owned_object: Some(_),
                            ..
                        })
                    ) {
                        return Err(invalid("preset parent is an internal scope"));
                    }
                    // The view is in world space; the body inherits the CTM at
                    // its insertion point, so place it in that local space.
                    let view = match view {
                        Some(view) if parent_location == NodeLocation::Scene(active) => {
                            let evaluated = crate::evaluate(&document.scene_view(active)?)?;
                            let NodeKind::Scope { children, .. } =
                                &node(&document, parent_location, parent)
                                    .ok_or_else(|| invalid("missing preset parent"))?
                                    .data
                            else {
                                return Err(invalid("preset parent is not a group"));
                            };
                            let at = after.or(children.last().copied()).unwrap_or(parent);
                            let inverse = evaluated
                                .nodes
                                .get(&at)
                                .and_then(|state| crate::math::inverse(state.start))
                                .ok_or_else(|| invalid("the preset parent cannot be placed in"))?;
                            Some(crate::presets::PresetView {
                                position: inverse.transform_point(view.position),
                                look_at: inverse.transform_point(view.look_at),
                            })
                        }
                        Some(view) if parent_location == NodeLocation::Definition => {
                            let evaluated = crate::evaluate(&document.scene_view(active)?)?;
                            let NodeKind::Scope { children, .. } =
                                &node(&document, parent_location, parent)
                                    .ok_or_else(|| invalid("missing preset parent"))?
                                    .data
                            else {
                                return Err(invalid("preset parent is not a group"));
                            };
                            let at = after.or(children.last().copied()).unwrap_or(parent);
                            let parent_transform = evaluated
                                .nodes
                                .get(&at)
                                .ok_or_else(|| invalid("missing definition parent transform"))?
                                .start;
                            let definition = document
                                .resources
                                .values()
                                .find(|resource| {
                                    resource.definition_scope.is_some_and(|root| {
                                        definition_root(&document, parent) == root
                                    })
                                })
                                .map(|resource| resource.id)
                                .ok_or_else(|| invalid("missing parent object definition"))?;
                            let instance_transform = document
                                .scene(active)
                                .and_then(|scene| first_instance_of_definition(scene, definition))
                                .and_then(|instance| evaluated.nodes.get(&instance))
                                .map(|state| state.start)
                                .unwrap_or_else(crate::math::Mat4::identity);
                            let inverse =
                                crate::math::inverse(instance_transform.compose(parent_transform))
                                    .ok_or_else(|| {
                                        invalid("the definition parent cannot be placed in")
                                    })?;
                            Some(crate::presets::PresetView {
                                position: inverse.transform_point(view.position),
                                look_at: inverse.transform_point(view.look_at),
                            })
                        }
                        view => view,
                    };
                    let (parent_node, location) = node_mut(&mut document, active, parent)?;
                    let NodeKind::Scope { children, .. } = &mut parent_node.data else {
                        return Err(invalid("preset parent is not a group"));
                    };
                    let insert_at = insertion_index(children, after)?;
                    let group = Node::new(
                        label.clone(),
                        NodeKind::Scope {
                            scope: ScopeKind::Attribute,
                            children: Vec::new(),
                            owned_object: None,
                            parent_camera: None,
                        },
                    );
                    let group_id = group.id;
                    // No `Identity`: the preset inherits its parent's transform.
                    let mut members = Vec::new();
                    if preset.name == "instance.object" {
                        members.push(crate::presets::make_instance(definition.unwrap()));
                    } else {
                        members.extend(crate::presets::make_body(
                            preset,
                            scene_coordinate_system,
                            view,
                        ));
                    }
                    let member_ids = members.iter().map(|node| node.id).collect::<Vec<_>>();
                    let object_children = members
                        .iter()
                        .filter(|node| is_object_node(node))
                        .map(|node| node.id)
                        .collect::<Vec<_>>();
                    let owned_object = match object_children.as_slice() {
                        [object] => Some(*object),
                        _ => None,
                    };
                    if let Some(owner) = owned_object
                        && let Some(object) = members.iter_mut().find(|node| node.id == owner)
                    {
                        object.label = label.clone();
                    }
                    let mut group = group;
                    if let NodeKind::Scope {
                        children,
                        owned_object: owner,
                        ..
                    } = &mut group.data
                    {
                        *children = member_ids;
                        *owner = owned_object;
                    }
                    children.insert(insert_at, group_id);
                    // The preset lands in the same table as its parent, so a
                    // group inside an object definition stays in that definition.
                    insert_node(&mut document, location, group.clone())?;
                    for member in &members {
                        insert_node(&mut document, location, member.clone())?;
                    }
                    node_ids.insert(parent, location);
                    created_nodes.push((location, group));
                    created_nodes.extend(members.into_iter().map(|node| (location, node)));
                }
                Edit::CreateResource {
                    id: requested_id,
                    resource_kind,
                    implementation,
                    label,
                    folder,
                    texture_value_type,
                    parameters,
                } => {
                    let new_id = requested_id.unwrap_or_else(ResourceId::new);
                    if document.resources.contains_key(&new_id) {
                        return Err(invalid("resource id already exists"));
                    }
                    if label.trim().is_empty() {
                        return Err(invalid("resource label is empty"));
                    }
                    // A value type belongs to textures only, whatever the kind.
                    if (*resource_kind == ResourceKind::Texture) != texture_value_type.is_some() {
                        return Err(invalid("texture value type is required only for textures"));
                    }
                    if *resource_kind == ResourceKind::ObjectDefinition {
                        if !implementation.is_empty() || !parameters.is_empty() {
                            return Err(invalid(
                                "object definitions take no implementation or parameters",
                            ));
                        }
                    }
                    let folder = folder
                        .clone()
                        .unwrap_or_else(|| crate::layout::kind_folder(*resource_kind).to_owned());
                    if !document.folders.contains(&folder) {
                        return Err(invalid("resource folder does not exist"));
                    }
                    // Shared placement numbers around every existing
                    // item location, case-insensitively.
                    let mut namer = crate::layout::LocationNamer::new();
                    for location in document
                        .scenes
                        .values()
                        .map(|scene| scene.location.as_str())
                        .chain(
                            document
                                .resources
                                .values()
                                .map(|resource| resource.location.as_str()),
                        )
                        .chain(
                            document
                                .assets
                                .values()
                                .map(|asset| asset.location.as_str()),
                        )
                    {
                        namer.reserve(location);
                    }
                    if *resource_kind == ResourceKind::ObjectDefinition {
                        let scope = Node::new(
                            "ObjectDefinition",
                            NodeKind::Scope {
                                scope: ScopeKind::ObjectDefinition,
                                children: Vec::new(),
                                owned_object: None,
                                parent_camera: None,
                            },
                        );
                        let scope_id = scope.id;
                        document.definition_nodes.insert(scope_id, scope.clone());
                        let resource = Resource {
                            id: new_id,
                            label: label.clone(),
                            location: namer.take(&folder, label, "object"),
                            edition: Revision::initial(),
                            kind: *resource_kind,
                            texture_value_type: None,
                            implementation: String::new(),
                            parameters: Vec::new(),
                            definition_scope: Some(scope_id),
                            declared_in: None,
                            origin: None,
                        };
                        let id = resource.id;
                        document.resources.insert(id, resource);
                        resource_ids.insert(id);
                        created_items.push(ItemId::Resource(id));
                        created_nodes.push((NodeLocation::Definition, scope));
                    } else {
                        if implementation.is_empty() {
                            return Err(invalid("resource implementation is empty"));
                        }
                        let resource = Resource {
                            id: new_id,
                            label: label.clone(),
                            location: namer.take(
                                &folder,
                                label,
                                crate::layout::kind_extension(*resource_kind),
                            ),
                            edition: Revision::initial(),
                            kind: *resource_kind,
                            texture_value_type: *texture_value_type,
                            implementation: implementation.clone(),
                            parameters: parameters.clone(),
                            definition_scope: None,
                            declared_in: None,
                            origin: None,
                        };
                        let id = resource.id;
                        document.resources.insert(id, resource);
                        resource_ids.insert(id);
                        created_items.push(ItemId::Resource(id));
                    }
                }
                Edit::MakeObjectDefinition {
                    node: target,
                    label,
                } => {
                    let active = active.ok_or_else(|| invalid("no active scene"))?;
                    let scene = document.scene(active).unwrap().clone();
                    let group = node_value(&document, active, *target)?.clone();
                    let (scope_kind, children) = match &group.data {
                        NodeKind::Scope {
                            scope: scope_kind @ (ScopeKind::Attribute | ScopeKind::Transform),
                            children,
                            owned_object: None,
                            parent_camera: None,
                        } => (*scope_kind, children.clone()),
                        _ => return Err(invalid("object definition requires an unattached group")),
                    };
                    let mut descendants = Vec::new();
                    let mut stack = children.clone();
                    while let Some(id) = stack.pop() {
                        let child = node_value(&document, active, id)?.clone();
                        match child.data {
                            NodeKind::Entity {
                                entity: EntityKind::Camera | EntityKind::Light,
                                ..
                            }
                            | NodeKind::Instance { .. } => {
                                return Err(invalid(
                                    "object definitions cannot contain lights, cameras, or object instances",
                                ));
                            }
                            NodeKind::Scope { ref children, .. } => {
                                stack.extend(children.iter().copied())
                            }
                            // TransformEnd restores only the CTM, so these
                            // changes reach later siblings; a definition would
                            // isolate them.
                            NodeKind::State {
                                operation:
                                    StateOperation::Material(_)
                                    | StateOperation::Medium { .. }
                                    | StateOperation::ReverseOrientation
                                    | StateOperation::ColorSpace(_)
                                    | StateOperation::TransformTimes(_)
                                    | StateOperation::DefineCoordinates(_),
                            } if scope_kind == ScopeKind::Transform => {
                                return Err(invalid(
                                    "this Transform group changes state used after it; only Attribute groups can become object definitions",
                                ));
                            }
                            _ => {}
                        }
                        descendants.push(child);
                    }
                    // The body inherits the group's material, medium,
                    // orientation and color space; the instance keeps placement.
                    let entry = crate::evaluate(&document.scene_view(active)?)?
                        .nodes
                        .remove(target)
                        .ok_or_else(|| invalid("missing group state"))?;
                    let declared_in = DeclarationState {
                        transform: None,
                        graphics: Some(CapturedGraphics {
                            material: entry.material,
                            inside_medium: entry.inside_medium,
                            outside_medium: entry.outside_medium,
                            reverse_orientation: entry.reverse_orientation,
                            color_space: entry.color_space,
                        }),
                    };
                    let label = label.as_deref().unwrap_or(&group.label).trim();
                    if label.is_empty() || label.len() > 256 {
                        return Err(invalid("object definition label must be 1 to 256 bytes"));
                    }
                    let mut namer = crate::layout::LocationNamer::new();
                    for location in document
                        .scenes
                        .values()
                        .map(|scene| scene.location.as_str())
                        .chain(
                            document
                                .resources
                                .values()
                                .map(|resource| resource.location.as_str()),
                        )
                        .chain(
                            document
                                .assets
                                .values()
                                .map(|asset| asset.location.as_str()),
                        )
                    {
                        namer.reserve(location);
                    }
                    let definition_id = ResourceId::new();
                    let definition_scope = Node::new(
                        "ObjectDefinition",
                        NodeKind::Scope {
                            scope: ScopeKind::ObjectDefinition,
                            children: Vec::new(),
                            owned_object: None,
                            parent_camera: None,
                        },
                    );
                    let scope_id = definition_scope.id;
                    document
                        .definition_nodes
                        .insert(scope_id, definition_scope.clone());
                    let definition = Resource {
                        id: definition_id,
                        label: label.to_owned(),
                        location: namer.take("Objects", label, "object"),
                        edition: Revision::initial(),
                        kind: ResourceKind::ObjectDefinition,
                        texture_value_type: None,
                        implementation: String::new(),
                        parameters: Vec::new(),
                        definition_scope: Some(scope_id),
                        declared_in: Some(declared_in),
                        origin: None,
                    };
                    document.resources.insert(definition_id, definition);
                    resource_ids.insert(definition_id);
                    created_items.push(ItemId::Resource(definition_id));

                    // Keep the group's leading transform run on the instance.
                    // The remaining scope stream becomes the definition body.
                    let transform_count = children
                        .iter()
                        .take_while(|id| {
                            matches!(
                                node_value(&document, active, **id).map(|n| &n.data),
                                Ok(NodeKind::State {
                                    operation: StateOperation::Transform(_)
                                })
                            )
                        })
                        .count();
                    let (transforms, body) = children.split_at(transform_count);
                    let mut instance = Node::new(
                        group.label.clone(),
                        NodeKind::Instance {
                            definition: definition_id,
                        },
                    );
                    instance.active = group.active;
                    let wrapper = Node::new(
                        group.label.clone(),
                        NodeKind::Scope {
                            // Owned objects need an Attribute scope; nothing
                            // leaks from the body now, so this is equivalent.
                            scope: ScopeKind::Attribute,
                            children: transforms.iter().copied().chain([instance.id]).collect(),
                            owned_object: Some(instance.id),
                            parent_camera: None,
                        },
                    );
                    // Report the visible instance first (its wrapper is transparent
                    // in the hierarchy) so clients can select it after the edit.
                    created_nodes.push((NodeLocation::Scene(active), instance.clone()));
                    created_nodes.push((NodeLocation::Scene(active), wrapper.clone()));
                    let parent = find_parent(&document, active, *target)?.0;
                    let mut updated_scene = scene.clone();
                    let parent_node = updated_scene
                        .nodes
                        .get_mut(&parent)
                        .ok_or_else(|| invalid("missing group parent"))?;
                    let NodeKind::Scope { children, .. } = &mut parent_node.data else {
                        return Err(invalid("group parent is not a scope"));
                    };
                    let index = children
                        .iter()
                        .position(|id| *id == *target)
                        .ok_or_else(|| invalid("missing group parent slot"))?;
                    children[index] = wrapper.id;
                    updated_scene.nodes.remove(target);
                    updated_scene.nodes.insert(wrapper.id, wrapper.clone());
                    updated_scene.nodes.insert(instance.id, instance.clone());
                    for moved in transforms {
                        node_ids.insert(*moved, NodeLocation::Scene(active));
                    }
                    for moved in &descendants {
                        if transforms.contains(&moved.id) {
                            continue;
                        }
                        updated_scene.nodes.remove(&moved.id);
                        if body.contains(&moved.id) || descendants.iter().any(|ancestor| {
                            matches!(&ancestor.data, NodeKind::Scope { children, .. } if children.contains(&moved.id))
                        }) {
                            let moved = moved.clone();
                            if moved.id == scope_id { continue; }
                            let moved_id = moved.id;
                            document.definition_nodes.insert(moved.id, moved.clone());
                            created_nodes.push((NodeLocation::Definition, moved));
                            deleted_nodes.push((NodeLocation::Scene(active), descendants.iter().find(|old| old.id == moved_id).unwrap().clone()));
                        }
                    }
                    let def_root = document.definition_nodes.get_mut(&scope_id).unwrap();
                    let NodeKind::Scope { children, .. } = &mut def_root.data else {
                        unreachable!()
                    };
                    *children = body.to_vec();
                    document.scenes.insert(active, Arc::new(updated_scene));
                    scene_records.insert(active);
                    deleted_nodes.push((NodeLocation::Scene(active), group));
                    created_nodes.push((NodeLocation::Definition, definition_scope));
                }
                Edit::UnpackObjectInstance { node: target } => {
                    let active = active.ok_or_else(|| invalid("no active scene"))?;
                    let instance = node_value(&document, active, *target)?.clone();
                    let NodeKind::Instance { definition } = instance.data else {
                        return Err(invalid("unpack target is not an object instance"));
                    };
                    let resource = document
                        .resources
                        .get(&definition)
                        .filter(|resource| resource.kind == ResourceKind::ObjectDefinition)
                        .ok_or_else(|| invalid("instance definition is missing"))?
                        .clone();
                    let declared = resource.declared_in.clone().unwrap_or(DeclarationState {
                        transform: None,
                        graphics: None,
                    });
                    let root_id = resource
                        .definition_scope
                        .ok_or_else(|| invalid("instance definition has no body"))?;
                    let root = document
                        .definition_nodes
                        .get(&root_id)
                        .ok_or_else(|| invalid("instance definition body is missing"))?;
                    let NodeKind::Scope { children: body, .. } = &root.data else {
                        return Err(invalid("instance definition body is not a scope"));
                    };
                    let body = body.clone();
                    let evaluated = crate::evaluate(&document.scene_view(active)?)?;
                    let site = evaluated
                        .nodes
                        .get(target)
                        .ok_or_else(|| invalid("missing instance state"))?;
                    let mut prefix = Vec::new();
                    // TransformTimes is Options-only; the captured interval is
                    // the scene's, so the samples below keep their meaning.
                    if let Some(transform) = &declared.transform {
                        if transform.start == transform.end {
                            prefix.push(Node::new(
                                "Transform",
                                NodeKind::State {
                                    operation: StateOperation::Transform(
                                        TransformOperation::Concat(transform.start),
                                    ),
                                },
                            ));
                        } else {
                            for (sample, matrix) in [
                                (ActiveSamples::Start, transform.start),
                                (ActiveSamples::End, transform.end),
                            ] {
                                prefix.push(Node::new(
                                    "ActiveTransform",
                                    NodeKind::State {
                                        operation: StateOperation::ActiveTransform(sample),
                                    },
                                ));
                                prefix.push(Node::new(
                                    "Transform",
                                    NodeKind::State {
                                        operation: StateOperation::Transform(
                                            TransformOperation::Concat(matrix),
                                        ),
                                    },
                                ));
                            }
                            prefix.push(Node::new(
                                "ActiveTransform",
                                NodeKind::State {
                                    operation: StateOperation::ActiveTransform(ActiveSamples::All),
                                },
                            ));
                        }
                    }
                    if let Some(graphics) = &declared.graphics {
                        if graphics.material != site.material {
                            if graphics.material == MaterialBinding::Default {
                                let default_material = document
                                    .resources
                                    .values()
                                    .find(|resource| {
                                        resource.kind == ResourceKind::Material
                                            && resource.label == "Default Material"
                                            && resource.implementation == "diffuse"
                                            && resource.parameters.is_empty()
                                    })
                                    .map(|resource| resource.id)
                                    .unwrap_or_else(|| {
                                        let id = ResourceId::new();
                                        let folder =
                                            crate::layout::kind_folder(ResourceKind::Material);
                                        let mut namer = crate::layout::LocationNamer::new();
                                        for location in document
                                            .scenes
                                            .values()
                                            .map(|scene| scene.location.as_str())
                                            .chain(
                                                document
                                                    .resources
                                                    .values()
                                                    .map(|resource| resource.location.as_str()),
                                            )
                                            .chain(
                                                document
                                                    .assets
                                                    .values()
                                                    .map(|asset| asset.location.as_str()),
                                            )
                                        {
                                            namer.reserve(location);
                                        }
                                        let resource = Resource {
                                            id,
                                            label: "Default Material".into(),
                                            location: namer.take(
                                                folder,
                                                "Default Material",
                                                crate::layout::kind_extension(
                                                    ResourceKind::Material,
                                                ),
                                            ),
                                            edition: Revision::initial(),
                                            kind: ResourceKind::Material,
                                            texture_value_type: None,
                                            implementation: "diffuse".into(),
                                            parameters: Vec::new(),
                                            definition_scope: None,
                                            declared_in: None,
                                            origin: None,
                                        };
                                        document.resources.insert(id, resource);
                                        resource_ids.insert(id);
                                        created_items.push(ItemId::Resource(id));
                                        id
                                    });
                                prefix.push(Node::new(
                                    "Material",
                                    NodeKind::State {
                                        operation: StateOperation::Material(
                                            MaterialBinding::Resource(default_material),
                                        ),
                                    },
                                ));
                            } else {
                                prefix.push(Node::new(
                                    "Material",
                                    NodeKind::State {
                                        operation: StateOperation::Material(
                                            graphics.material.clone(),
                                        ),
                                    },
                                ));
                            }
                        }
                        if graphics.inside_medium != site.inside_medium
                            || graphics.outside_medium != site.outside_medium
                        {
                            prefix.push(Node::new(
                                "MediumInterface",
                                NodeKind::State {
                                    operation: StateOperation::Medium {
                                        inside: graphics.inside_medium,
                                        outside: graphics.outside_medium,
                                    },
                                },
                            ));
                        }
                        if graphics.reverse_orientation != site.reverse_orientation {
                            prefix.push(Node::new(
                                "ReverseOrientation",
                                NodeKind::State {
                                    operation: StateOperation::ReverseOrientation,
                                },
                            ));
                        }
                        if graphics.color_space != site.color_space {
                            prefix.push(Node::new(
                                "ColorSpace",
                                NodeKind::State {
                                    operation: StateOperation::ColorSpace(
                                        graphics.color_space.clone(),
                                    ),
                                },
                            ));
                        }
                    }
                    let mut copies = Vec::new();
                    let mut copied_roots = Vec::new();
                    for child in body {
                        let source_ids =
                            subtree_ids_at(&document, NodeLocation::Definition, child)?;
                        let mut subtree =
                            copy_subtree_at(&document, NodeLocation::Definition, child)?;
                        for (source_id, copy) in source_ids.iter().zip(&mut subtree) {
                            let operation = matches!(
                                node(&document, NodeLocation::Definition, *source_id)
                                    .map(|source| &source.data),
                                Some(NodeKind::State {
                                    operation: StateOperation::Transform(
                                        TransformOperation::Identity
                                            | TransformOperation::Replace(_),
                                    ) | StateOperation::UseCoordinates(_),
                                })
                            );
                            if !operation {
                                continue;
                            }
                            let before = evaluated
                                .transform_before
                                .get(source_id)
                                .ok_or_else(|| invalid("missing definition transform state"))?;
                            if before.start != before.end {
                                return Err(invalid(
                                    "cannot unpack: the definition's transform is animated",
                                ));
                            }
                            let inverse = crate::math::inverse(before.start).ok_or_else(|| {
                                invalid("cannot unpack: singular definition transform")
                            })?;
                            let rebased = match node(
                                &document,
                                NodeLocation::Definition,
                                *source_id,
                            )
                            .map(|source| &source.data)
                            {
                                Some(NodeKind::State {
                                    operation:
                                        StateOperation::Transform(TransformOperation::Identity),
                                }) => inverse,
                                Some(NodeKind::State {
                                    operation:
                                        StateOperation::Transform(TransformOperation::Replace(matrix)),
                                }) => inverse.compose(crate::math::Mat4(*matrix)),
                                Some(NodeKind::State {
                                    operation: StateOperation::UseCoordinates(_),
                                }) => {
                                    let after = &evaluated.nodes[source_id];
                                    let target = match before.active {
                                        ActiveSamples::Start => after.start,
                                        ActiveSamples::End => after.end,
                                        ActiveSamples::All if after.start == after.end => {
                                            after.start
                                        }
                                        ActiveSamples::All => {
                                            return Err(invalid(
                                                "cannot unpack: the definition's coordinates are animated",
                                            ));
                                        }
                                    };
                                    inverse.compose(target)
                                }
                                _ => unreachable!(),
                            };
                            let NodeKind::State { operation } = &mut copy.data else {
                                unreachable!()
                            };
                            *operation =
                                StateOperation::Transform(TransformOperation::Concat(rebased.0));
                        }
                        copied_roots.push(subtree[0].id);
                        copies.extend(subtree);
                    }
                    let (parent, _) = find_parent(&document, active, *target)?;
                    let wrapper_id = match &node_value(&document, active, parent)?.data {
                        NodeKind::Scope {
                            children,
                            owned_object: Some(owner),
                            scope: ScopeKind::Attribute,
                            ..
                        } if *owner == *target && children.contains(target) => Some(parent),
                        _ => None,
                    };
                    // The instance's slot (inside its wrapper, if it has one)
                    // takes the state prefix and the copies; the wrapper's other
                    // children (placement, bindings) stay. A fresh group replaces
                    // the wrapper so it is the first created node a client selects.
                    let unpacked = prefix
                        .iter()
                        .map(|node| node.id)
                        .chain(copied_roots.iter().copied())
                        .collect::<Vec<_>>();
                    let (container, group_children, parent_camera) = match wrapper_id {
                        Some(wrapper_id) => {
                            let NodeKind::Scope {
                                children,
                                parent_camera,
                                ..
                            } = &node_value(&document, active, wrapper_id)?.data
                            else {
                                unreachable!()
                            };
                            let children = children
                                .iter()
                                .flat_map(|id| {
                                    if id == target {
                                        unpacked.clone()
                                    } else {
                                        vec![*id]
                                    }
                                })
                                .collect();
                            (wrapper_id, children, *parent_camera)
                        }
                        None => (*target, unpacked, None),
                    };
                    let (container_parent, _) = find_parent(&document, active, container)?;
                    let mut group = Node::new(
                        instance.label.clone(),
                        NodeKind::Scope {
                            scope: ScopeKind::Attribute,
                            children: group_children,
                            owned_object: None,
                            parent_camera,
                        },
                    );
                    group.active = instance.active;
                    let mut updated = document.scene(active).unwrap().clone();
                    let NodeKind::Scope { children, .. } = &mut updated
                        .nodes
                        .get_mut(&container_parent)
                        .ok_or_else(|| invalid("missing instance parent"))?
                        .data
                    else {
                        unreachable!()
                    };
                    let index = children
                        .iter()
                        .position(|id| *id == container)
                        .ok_or_else(|| invalid("missing instance parent slot"))?;
                    children[index] = group.id;
                    updated.nodes.remove(target);
                    if let Some(wrapper_id) = wrapper_id {
                        let wrapper = updated.nodes.remove(&wrapper_id).unwrap();
                        deleted_nodes.push((NodeLocation::Scene(active), wrapper));
                    }
                    updated.nodes.insert(group.id, group.clone());
                    document.scenes.insert(active, Arc::new(updated));
                    node_ids.insert(container_parent, NodeLocation::Scene(active));
                    deleted_nodes.push((NodeLocation::Scene(active), instance));
                    created_nodes.push((NodeLocation::Scene(active), group));
                    let copied_nodes = copies;
                    for node in prefix {
                        insert_node(&mut document, NodeLocation::Scene(active), node.clone())?;
                        created_nodes.push((NodeLocation::Scene(active), node));
                    }
                    for node in copied_nodes {
                        insert_node(&mut document, NodeLocation::Scene(active), node.clone())?;
                        created_nodes.push((NodeLocation::Scene(active), node));
                    }
                    scene_records.insert(active);
                }
                Edit::SetLocalBinding {
                    node: object_id,
                    material,
                    medium,
                } => {
                    let active = active.ok_or_else(|| invalid("no active scene"))?;
                    if let Some(MaterialBinding::Resource(id)) = material
                        && document
                            .resources
                            .get(id)
                            .is_none_or(|resource| resource.kind != ResourceKind::Material)
                    {
                        return Err(invalid(
                            "local material binding has the wrong resource kind",
                        ));
                    }
                    for id in medium
                        .iter()
                        .flat_map(|medium| [medium.inside, medium.outside])
                        .flatten()
                    {
                        if document
                            .resources
                            .get(&id)
                            .is_none_or(|resource| resource.kind != ResourceKind::Medium)
                        {
                            return Err(invalid(
                                "local medium binding has the wrong resource kind",
                            ));
                        }
                    }
                    if !document
                        .scene(active)
                        .is_some_and(|scene| scene.nodes.contains_key(object_id))
                        || !matches!(
                            node(&document, NodeLocation::Scene(active), *object_id)
                                .map(is_object_node),
                            Some(true)
                        )
                    {
                        return Err(invalid("local binding target is not a scene object"));
                    }
                    if material.is_some()
                        && matches!(
                            node(&document, NodeLocation::Scene(active), *object_id)
                                .map(|node| &node.data),
                            Some(NodeKind::Entity {
                                entity: EntityKind::Light,
                                ..
                            })
                        )
                    {
                        return Err(invalid("lights take no material binding"));
                    }
                    let operations = material
                        .iter()
                        .map(|binding| StateOperation::Material(binding.clone()))
                        .chain(medium.iter().map(|medium| StateOperation::Medium {
                            inside: medium.inside,
                            outside: medium.outside,
                        }))
                        .collect::<Vec<_>>();
                    let clears = |operation: &StateOperation| {
                        matches!(
                            operation,
                            StateOperation::Material(MaterialBinding::Default)
                                | StateOperation::Medium {
                                    inside: None,
                                    outside: None,
                                }
                        )
                    };
                    // A group target keeps its components at the
                    // group's head — the same place the group's transform
                    // states sit. A camera takes its medium as a state in
                    // the options list, right before the camera.
                    let world = document.scene(active).unwrap().world;
                    if let Some(group) = attribute_group(&document, active, world, *object_id) {
                        for operation in operations {
                            if clears(&operation) {
                                remove_group_head_state(
                                    &mut document,
                                    active,
                                    group,
                                    &operation,
                                    &self.document,
                                    &mut node_ids,
                                    &mut deleted_nodes,
                                )?;
                            } else {
                                set_group_head_state(
                                    &mut document,
                                    active,
                                    group,
                                    operation,
                                    &mut created_nodes,
                                    &mut node_ids,
                                )?;
                            }
                        }
                        continue;
                    }
                    if matches!(
                        node(&document, NodeLocation::Scene(active), *object_id)
                            .map(|node| &node.data),
                        Some(NodeKind::Entity {
                            entity: EntityKind::Camera,
                            ..
                        })
                    ) {
                        if material.is_some() {
                            return Err(invalid("cameras take no material binding"));
                        }
                        // The options list is scene-level state; record the
                        // scene so the change commits and undoes as a unit.
                        scene_records.insert(active);
                        for operation in operations {
                            set_camera_medium(
                                &mut document,
                                active,
                                *object_id,
                                operation,
                                &self.document,
                                &mut created_nodes,
                                &mut node_ids,
                                &mut deleted_nodes,
                            )?;
                        }
                        continue;
                    }
                    let (parent, index) = find_parent(&document, active, *object_id)?;
                    let sole_child_group =
                        is_dedicated_group(&document, active, world, parent, *object_id);
                    if operations.iter().all(clears) {
                        // Clearing never creates a dedicated group or a
                        // binding; it deletes the states of the dedicated
                        // group and, once emptied, the group itself
                        //. Without a dedicated
                        // group there is nothing to clear.
                        if !sole_child_group {
                            continue;
                        }
                        for operation in operations {
                            remove_adjacent_binding(
                                &mut document,
                                active,
                                parent,
                                *object_id,
                                &operation,
                                &self.document,
                                &mut node_ids,
                                &mut deleted_nodes,
                            )?;
                        }
                        unwrap_emptied_group(
                            &mut document,
                            active,
                            parent,
                            *object_id,
                            &self.document,
                            &mut node_ids,
                            &mut deleted_nodes,
                        )?;
                        continue;
                    }
                    let binding_parent = if sole_child_group {
                        parent
                    } else {
                        let label = node(&document, NodeLocation::Scene(active), *object_id)
                            .map(|node| node.label.clone())
                            .unwrap_or_else(|| "Object".into());
                        let wrapper = Node::new(
                            label,
                            NodeKind::Scope {
                                scope: ScopeKind::Attribute,
                                children: vec![*object_id],
                                owned_object: Some(*object_id),
                                parent_camera: None,
                            },
                        );
                        let wrapper_id = wrapper.id;
                        let (parent_node, location) = node_mut(&mut document, active, parent)?;
                        let NodeKind::Scope { children, .. } = &mut parent_node.data else {
                            return Err(invalid("object parent is not a group"));
                        };
                        children[index] = wrapper_id;
                        node_ids.insert(parent, location);
                        insert_node(&mut document, location, wrapper.clone())?;
                        created_nodes.push((location, wrapper));
                        wrapper_id
                    };
                    for operation in operations {
                        let object_index =
                            child_index(&document, active, binding_parent, *object_id)?;
                        if clears(&operation) {
                            remove_adjacent_binding(
                                &mut document,
                                active,
                                binding_parent,
                                *object_id,
                                &operation,
                                &self.document,
                                &mut node_ids,
                                &mut deleted_nodes,
                            )?;
                        } else {
                            insert_binding(
                                &mut document,
                                active,
                                binding_parent,
                                object_index,
                                operation,
                                &mut created_nodes,
                                &mut node_ids,
                            )?;
                        }
                    }
                    node_ids.insert(binding_parent, NodeLocation::Scene(active));
                }
                Edit::SetAreaLight {
                    node: object_id,
                    light,
                } => {
                    let active = active.ok_or_else(|| invalid("no active scene"))?;
                    let world = document.scene(active).unwrap().world;
                    // A group target carries its area light at the
                    // group's head — the same place its transform states sit.
                    if let Some(group) = attribute_group(&document, active, world, *object_id) {
                        match light {
                            Some(content) => {
                                set_group_head_area_light(
                                    &mut document,
                                    active,
                                    group,
                                    &content,
                                    &mut created_nodes,
                                    &mut node_ids,
                                )?;
                            }
                            None => {
                                remove_group_head_area_light(
                                    &mut document,
                                    active,
                                    group,
                                    &self.document,
                                    &mut node_ids,
                                    &mut deleted_nodes,
                                    &mut created_nodes,
                                )?;
                            }
                        }
                        continue;
                    }
                    require_shape(
                        &document,
                        active,
                        *object_id,
                        "area light target is not a shape",
                    )?;
                    // The pre-edit effective light decides which case this is.
                    let effective = {
                        let view = document.scene_view(active)?;
                        crate::evaluate(&view)?
                            .nodes
                            .get(object_id)
                            .and_then(|state| state.area_light)
                    };
                    // The light is local when it sits in the object's
                    // dedicated group; anything else is shared with
                    // siblings and can only be cancelled, not rewritten.
                    let is_local = match (effective, find_parent(&document, active, *object_id)) {
                        (Some(light), Ok((object_parent, _))) if matches!(find_parent(&document, active, light), Ok((light_parent, _)) if light_parent == object_parent) => {
                            is_dedicated_group(&document, active, world, object_parent, *object_id)
                        }
                        _ => false,
                    };
                    match light {
                        // Add or change: the dedicated group's run may hold
                        // an entity already — including an explicit
                        // cancellation, which the re-add rewrites in place
                        // instead of leaving a cancel that still wins in
                        // graphics-state order. Without a dedicated group
                        // (or with a run that holds no entity), create one
                        // at the head of the run.
                        Some(content) => {
                            let entity_kind = |content: &AreaLightContent| NodeKind::Entity {
                                entity: EntityKind::AreaLight,
                                implementation: content.implementation.clone(),
                                parameters: content.parameters.clone(),
                                pose: None,
                            };
                            // The dedicated group's run may hold an entity
                            // already — including an explicit cancellation,
                            // which the re-add rewrites in place so that no
                            // cancel stays between the light and the shape.
                            // Without one, wrap and lead the run.
                            let mut rewrite = None;
                            if let Ok((parent, _)) = find_parent(&document, active, *object_id)
                                && is_dedicated_group(&document, active, world, parent, *object_id)
                            {
                                let index = child_index(&document, active, parent, *object_id)?;
                                rewrite = run_area_lights(&document, active, parent, index).pop();
                            }
                            match rewrite {
                                Some(id) => {
                                    let (entity, _) = node_mut(&mut document, active, id)?;
                                    entity.data = entity_kind(&content);
                                    node_ids.insert(id, NodeLocation::Scene(active));
                                }
                                None => {
                                    let group = ensure_dedicated_group(
                                        &mut document,
                                        active,
                                        world,
                                        *object_id,
                                        &mut node_ids,
                                        &mut created_nodes,
                                    )?;
                                    let index = child_index(&document, active, group, *object_id)?;
                                    let run_start =
                                        component_run_start(&document, active, group, index);
                                    let entity =
                                        Node::new("Local Area Light", entity_kind(&content));
                                    insert_child_node(
                                        &mut document,
                                        active,
                                        group,
                                        run_start,
                                        entity,
                                        &mut created_nodes,
                                        &mut node_ids,
                                    )?;
                                }
                            }
                        }
                        // Remove: delete a local entity, or cancel a shared
                        // one without touching the siblings.
                        None => {
                            let Some(light) = effective else {
                                continue;
                            };
                            if is_local {
                                let (group, _) = find_parent(&document, active, light)?;
                                remove_child_node(
                                    &mut document,
                                    active,
                                    group,
                                    light,
                                    &self.document,
                                    &mut node_ids,
                                    &mut deleted_nodes,
                                )?;
                                unwrap_emptied_group(
                                    &mut document,
                                    active,
                                    group,
                                    *object_id,
                                    &self.document,
                                    &mut node_ids,
                                    &mut deleted_nodes,
                                )?;
                            } else {
                                let group = ensure_dedicated_group(
                                    &mut document,
                                    active,
                                    world,
                                    *object_id,
                                    &mut node_ids,
                                    &mut created_nodes,
                                )?;
                                let index = child_index(&document, active, group, *object_id)?;
                                let run_start =
                                    component_run_start(&document, active, group, index);
                                let cancel = Node::new(
                                    "No Area Light",
                                    NodeKind::Entity {
                                        entity: EntityKind::AreaLight,
                                        implementation: String::new(),
                                        parameters: Vec::new(),
                                        pose: None,
                                    },
                                );
                                insert_child_node(
                                    &mut document,
                                    active,
                                    group,
                                    run_start,
                                    cancel,
                                    &mut created_nodes,
                                    &mut node_ids,
                                )?;
                            }
                        }
                    }
                }
                Edit::ClearLocalBinding {
                    node: object_id,
                    material,
                    medium,
                } => {
                    let active = active.ok_or_else(|| invalid("no active scene"))?;
                    if !document
                        .scene(active)
                        .is_some_and(|scene| scene.nodes.contains_key(object_id))
                        || !matches!(
                            node(&document, NodeLocation::Scene(active), *object_id)
                                .map(is_object_node),
                            Some(true)
                        )
                    {
                        return Err(invalid("local binding target is not a scene object"));
                    }
                    let world = document.scene(active).unwrap().world;
                    // A group's head states and a camera's options-list state
                    // are cleared where they sit; shapes keep the
                    // dedicated-group procedure.
                    if let Some(group) = attribute_group(&document, active, world, *object_id) {
                        for (wanted, operation) in [
                            (
                                *material,
                                StateOperation::Material(MaterialBinding::Default),
                            ),
                            (
                                *medium,
                                StateOperation::Medium {
                                    inside: None,
                                    outside: None,
                                },
                            ),
                        ] {
                            if wanted {
                                remove_group_head_state(
                                    &mut document,
                                    active,
                                    group,
                                    &operation,
                                    &self.document,
                                    &mut node_ids,
                                    &mut deleted_nodes,
                                )?;
                            }
                        }
                        continue;
                    }
                    if matches!(
                        node(&document, NodeLocation::Scene(active), *object_id)
                            .map(|node| &node.data),
                        Some(NodeKind::Entity {
                            entity: EntityKind::Camera,
                            ..
                        })
                    ) {
                        if *medium {
                            // Options-list state: record the scene so the
                            // removal commits (and undoes) with the document.
                            scene_records.insert(active);
                            set_camera_medium(
                                &mut document,
                                active,
                                *object_id,
                                StateOperation::Medium {
                                    inside: None,
                                    outside: None,
                                },
                                &self.document,
                                &mut created_nodes,
                                &mut node_ids,
                                &mut deleted_nodes,
                            )?;
                        }
                        continue;
                    }
                    let (parent, _) = find_parent(&document, active, *object_id)?;
                    if !is_dedicated_group(&document, active, world, parent, *object_id) {
                        // Nothing is bound locally; clearing stays a no-op.
                        continue;
                    }
                    let mut cleared = false;
                    if *material {
                        remove_adjacent_binding(
                            &mut document,
                            active,
                            parent,
                            *object_id,
                            &StateOperation::Material(MaterialBinding::Default),
                            &self.document,
                            &mut node_ids,
                            &mut deleted_nodes,
                        )?;
                        cleared = true;
                    }
                    if *medium {
                        remove_adjacent_binding(
                            &mut document,
                            active,
                            parent,
                            *object_id,
                            &StateOperation::Medium {
                                inside: None,
                                outside: None,
                            },
                            &self.document,
                            &mut node_ids,
                            &mut deleted_nodes,
                        )?;
                        cleared = true;
                    }
                    let _ = cleared;
                    unwrap_emptied_group(
                        &mut document,
                        active,
                        parent,
                        *object_id,
                        &self.document,
                        &mut node_ids,
                        &mut deleted_nodes,
                    )?;
                }
                Edit::SetReverseOrientation {
                    node: object_id,
                    enabled,
                } => {
                    let active = active.ok_or_else(|| invalid("no active scene"))?;
                    require_shape(
                        &document,
                        active,
                        *object_id,
                        "reverse orientation target is not a shape",
                    )?;
                    let world = document.scene(active).unwrap().world;
                    let (parent, _) = find_parent(&document, active, *object_id)?;
                    let dedicated =
                        is_dedicated_group(&document, active, world, parent, *object_id);
                    // Toggles in the adjacent run count local only when the
                    // parent is the object's dedicated group; anything else
                    // belongs to the outer parity the group must cancel.
                    let index = child_index(&document, active, parent, *object_id)?;
                    let toggles = if dedicated {
                        run_toggles(&document, active, parent, index)
                    } else {
                        Vec::new()
                    };
                    let effective = {
                        let view = document.scene_view(active)?;
                        crate::evaluate(&view)?
                            .nodes
                            .get(object_id)
                            .map(|state| state.reverse_orientation)
                            .unwrap_or(false)
                    };
                    let outer = effective != (toggles.len() % 2 == 1);
                    let want_local = *enabled != outer;
                    if (toggles.len() % 2 == 1) == want_local {
                        continue; // already in the requested shape
                    }
                    for toggle in &toggles {
                        remove_child_node(
                            &mut document,
                            active,
                            parent,
                            *toggle,
                            &self.document,
                            &mut node_ids,
                            &mut deleted_nodes,
                        )?;
                    }
                    if !toggles.is_empty() {
                        unwrap_emptied_group(
                            &mut document,
                            active,
                            parent,
                            *object_id,
                            &self.document,
                            &mut node_ids,
                            &mut deleted_nodes,
                        )?;
                    }
                    if want_local {
                        let group = ensure_dedicated_group(
                            &mut document,
                            active,
                            world,
                            *object_id,
                            &mut node_ids,
                            &mut created_nodes,
                        )?;
                        let index = child_index(&document, active, group, *object_id)?;
                        let run_start = component_run_start(&document, active, group, index);
                        let toggle = Node::new(
                            "Local Reverse Orientation",
                            NodeKind::State {
                                operation: StateOperation::ReverseOrientation,
                            },
                        );
                        insert_child_node(
                            &mut document,
                            active,
                            group,
                            run_start,
                            toggle,
                            &mut created_nodes,
                            &mut node_ids,
                        )?;
                    }
                }
                Edit::SetLocalTransform {
                    node: target,
                    value,
                } => {
                    let active = active.ok_or_else(|| invalid("no active scene"))?;
                    // Scene nodes, or nodes inside an object definition.
                    let location = if node(&document, active.into(), *target).is_some() {
                        NodeLocation::Scene(active)
                    } else {
                        NodeLocation::Definition
                    };
                    let world = document.scene(active).unwrap().world;
                    let camera = matches!(
                        node(&document, location, *target).map(|node| &node.data),
                        Some(NodeKind::Entity {
                            entity: EntityKind::Camera,
                            ..
                        })
                    );
                    if camera {
                        // A camera's local transform is its pose: a
                        // static edit rewrites the pose value and nothing
                        // else. Animated cameras keep refusing the static
                        // edit API; a pose-less camera still holds an older
                        // import's transform run and is refused with the
                        // same reason as a reset run.
                        let pose = match node(&document, location, *target).map(|node| &node.data) {
                            Some(NodeKind::Entity {
                                pose: Some(pose), ..
                            }) => *pose,
                            _ => return Err(invalid(OLDER_IMPORT_RESET)),
                        };
                        if pose.start != pose.end {
                            return Err(invalid("the camera pose is animated"));
                        }
                        let matrix = match value {
                            LocalTransform::Matrix(matrix) => crate::math::Mat4(*matrix),
                            LocalTransform::Trs(trs) => crate::transform_trs::compose(trs),
                        };
                        if matrix.0.iter().any(|value| !value.is_finite()) {
                            return Err(invalid("local matrix is not finite"));
                        }
                        // Export writes inverse(pose) before the Camera.
                        if crate::math::inverse(matrix).is_none() {
                            return Err(invalid("camera pose is singular"));
                        }
                        let (node_value, node_location) = node_mut(&mut document, active, *target)?;
                        let NodeKind::Entity { pose, .. } = &mut node_value.data else {
                            return Err(invalid("the edited node is not a Camera"));
                        };
                        *pose = Some(CameraPose {
                            start: matrix.0,
                            end: matrix.0,
                        });
                        node_ids.insert(*target, node_location);
                        scene_records.insert(active);
                        continue;
                    }
                    let group = match node(&document, location, *target).map(|node| &node.data) {
                        _ if *target == world => {
                            return Err(invalid("the world has no local transform"));
                        }
                        Some(NodeKind::Scope {
                            scope: ScopeKind::Attribute | ScopeKind::Transform,
                            ..
                        }) => true,
                        Some(NodeKind::Entity {
                            entity: EntityKind::Shape | EntityKind::Light,
                            ..
                        })
                        | Some(NodeKind::Instance { .. }) => false,
                        _ => {
                            return Err(invalid(
                                "local transform target is not a group or an object",
                            ));
                        }
                    };
                    // The run: a group's own head, or the object's dedicated
                    // group up to the object.
                    let (scope, end) = if group {
                        let children = scope_children(&document, location, *target);
                        let end = children
                            .iter()
                            .position(|child| {
                                node(&document, location, *child).is_some_and(is_object_node)
                            })
                            .unwrap_or(children.len());
                        (Some(*target), end)
                    } else {
                        let (parent, index) = find_parent(&document, location, *target)?;
                        if is_dedicated_group(&document, location, world, parent, *target) {
                            (Some(parent), index)
                        } else {
                            (None, 0)
                        }
                    };
                    let run = scope.map_or_else(Vec::new, |scope| {
                        scope_children(&document, location, scope)[..end].to_vec()
                    });
                    let mut transforms = Vec::new();
                    for child in &run {
                        match node(&document, location, *child).map(|node| &node.data) {
                            Some(NodeKind::State {
                                operation: StateOperation::Transform(operation),
                            }) => {
                                // a reset operation re-parents its run —
                                // newer imports resolve that away, so a run
                                // still carrying one is an older import and
                                // keeps its local as it is.
                                if matches!(
                                    operation,
                                    TransformOperation::Identity | TransformOperation::Replace(_)
                                ) {
                                    return Err(invalid(OLDER_IMPORT_RESET));
                                }
                                transforms.push(*child);
                            }
                            Some(NodeKind::State {
                                operation: StateOperation::DefineCoordinates(_),
                            }) => {
                                return Err(invalid(
                                    "the local transform declares a named coordinate system",
                                ));
                            }
                            Some(NodeKind::State {
                                operation: StateOperation::UseCoordinates(_),
                            }) => {
                                return Err(invalid(OLDER_IMPORT_RESET));
                            }
                            Some(NodeKind::State {
                                operation:
                                    StateOperation::ActiveTransform(_)
                                    | StateOperation::TransformTimes(_),
                            }) => return Err(invalid("the local transform is animated")),
                            _ => {}
                        }
                    }
                    {
                        let view = document.scene_view(active)?;
                        let evaluated = crate::evaluate(&view)?;
                        // Also refuses an inherited ActiveTransform Start/End:
                        // new states would then set only one sample.
                        let animated = |id: &NodeId| {
                            evaluated.nodes.get(id).is_some_and(|state| {
                                state.start != state.end || state.active != ActiveSamples::All
                            })
                        };
                        if animated(target) || transforms.iter().any(animated) {
                            return Err(invalid("the local transform is animated"));
                        }
                    }
                    let operations = local_operations(value)?;
                    let at = transforms
                        .first()
                        .map(|first| run.iter().position(|child| child == first).unwrap());
                    if let Some(scope) = scope {
                        for id in &transforms {
                            remove_child_node(
                                &mut document,
                                location,
                                scope,
                                *id,
                                &self.document,
                                &mut node_ids,
                                &mut deleted_nodes,
                            )?;
                        }
                        if !group && operations.is_empty() && !transforms.is_empty() {
                            unwrap_emptied_group(
                                &mut document,
                                location,
                                scope,
                                *target,
                                &self.document,
                                &mut node_ids,
                                &mut deleted_nodes,
                            )?;
                        }
                    }
                    if !operations.is_empty() {
                        let scope = match scope {
                            Some(scope) => scope,
                            None => ensure_dedicated_group(
                                &mut document,
                                location,
                                world,
                                *target,
                                &mut node_ids,
                                &mut created_nodes,
                            )?,
                        };
                        for (offset, (label, operation)) in operations.into_iter().enumerate() {
                            insert_child_node(
                                &mut document,
                                location,
                                scope,
                                at.unwrap_or(0) + offset,
                                Node::new(
                                    label,
                                    NodeKind::State {
                                        operation: StateOperation::Transform(operation),
                                    },
                                ),
                                &mut created_nodes,
                                &mut node_ids,
                            )?;
                        }
                    }
                }
                Edit::RenameNode {
                    node: target_node,
                    label,
                } => {
                    let active = active.ok_or_else(|| invalid("no active scene"))?;
                    let location = {
                        let (value, location) = node_mut(&mut document, active, *target_node)?;
                        value.label = label.clone();
                        location
                    };
                    if let Ok((parent, _)) = find_parent(&document, location, *target_node)
                        && let Some(parent_node) = node(&document, location, parent)
                        && matches!(
                            parent_node.data,
                            NodeKind::Scope {
                                owned_object: Some(owner),
                                ..
                            } if owner == *target_node
                        )
                    {
                        node_mut(&mut document, active, parent)?.0.label = label.clone();
                        node_ids.insert(parent, location);
                    }
                    node_ids.insert(*target_node, location);
                }
                Edit::SetNodeActive {
                    node: target_node,
                    active: value,
                } => {
                    let location = if let Some(scene) = active.filter(|scene| {
                        document
                            .scenes
                            .get(scene)
                            .is_some_and(|record| record.nodes.contains_key(target_node))
                    }) {
                        NodeLocation::Scene(scene)
                    } else if document.definition_nodes.contains_key(target_node) {
                        NodeLocation::Definition
                    } else {
                        return Err(invalid("missing edited node"));
                    };
                    let target = node(&document, location, *target_node)
                        .ok_or_else(|| invalid("missing edited node"))?;
                    let visible = matches!(
                        &target.data,
                        NodeKind::Entity {
                            entity: EntityKind::Shape | EntityKind::Light,
                            ..
                        } | NodeKind::Instance { .. }
                            | NodeKind::Scope {
                                scope: ScopeKind::Attribute | ScopeKind::Transform,
                                ..
                            }
                    );
                    if !visible {
                        return Err(invalid(
                            "only visible groups, shapes, lights and instances can be activated",
                        ));
                    }
                    let target = nodes_mut(&mut document, location)?
                        .get_mut(target_node)
                        .ok_or_else(|| invalid("missing edited node"))?;
                    if target.active != *value {
                        target.active = *value;
                        node_ids.insert(*target_node, location);
                    }
                }
                Edit::RenameResource { resource, label } => {
                    document
                        .resources
                        .get_mut(resource)
                        .ok_or_else(|| invalid("missing edited resource"))?
                        .label = label.clone();
                    resource_ids.insert(*resource);
                }
                Edit::SetTextureValueType { resource, value } => {
                    let target = document
                        .resources
                        .get_mut(resource)
                        .ok_or_else(|| invalid("missing edited texture"))?;
                    if target.kind != ResourceKind::Texture {
                        return Err(invalid("value type can only change on textures"));
                    }
                    if target.texture_value_type != Some(*value) {
                        target.texture_value_type = Some(*value);
                        resource_ids.insert(*resource);
                    }
                }
                Edit::SetTransform { node, value } => {
                    let active = active.ok_or_else(|| invalid("no active scene"))?;
                    let (node_value, location) = node_mut(&mut document, active, *node)?;
                    let NodeKind::State {
                        operation: StateOperation::Transform(transform),
                    } = &mut node_value.data
                    else {
                        return Err(invalid("edited node is not a transform operation"));
                    };
                    *transform = value.clone();
                    node_ids.insert(*node, location);
                }
                Edit::SetMaterialBinding { node, value } => {
                    let active = active.ok_or_else(|| invalid("no active scene"))?;
                    if matches!(value, MaterialBinding::Default) {
                        return Err(invalid(
                            "default material binding is not exportable; remove the binding",
                        ));
                    }
                    let (node_value, location) = node_mut(&mut document, active, *node)?;
                    let NodeKind::State {
                        operation: StateOperation::Material(binding),
                    } = &mut node_value.data
                    else {
                        return Err(invalid("edited node is not a material binding"));
                    };
                    *binding = value.clone();
                    node_ids.insert(*node, location);
                }
                Edit::DeleteNode { node } => {
                    let active = active.ok_or_else(|| invalid("no active scene"))?;
                    if document.definition_nodes.contains_key(node) {
                        let location = NodeLocation::Definition;
                        let unit = object_unit_at(&document, location, *node)?;
                        let (parent, index) = find_parent(&document, location, unit)?;
                        let (parent_node, _) = node_mut(&mut document, active, parent)?;
                        if let NodeKind::Scope { children, .. } = &mut parent_node.data {
                            children.remove(index);
                        }
                        node_ids.insert(parent, location);
                        let removed = remove_definition_subtree(&mut document, unit);
                        record_deleted_subtree(
                            &self.document,
                            location,
                            removed,
                            &mut deleted_nodes,
                        );
                        continue;
                    }
                    if document.scenes.get(&active).is_some_and(|scene| {
                        matches!(
                            scene.nodes.get(node).map(|node| &node.data),
                            Some(NodeKind::Configuration { .. })
                        ) && scene.options.contains(node)
                    }) {
                        // a render configuration is Options-only; it
                        // has no world subtree, just its own node.
                        let location = NodeLocation::Scene(active);
                        let configuration_node =
                            document.scenes[&active].as_ref().nodes[node].clone();
                        let mut updated = document.scenes[&active].as_ref().clone();
                        updated.nodes.remove(node);
                        updated.options.retain(|id| id != node);
                        document.scenes.insert(active, Arc::new(updated));
                        scene_records.insert(active);
                        record_deleted_subtree(
                            &self.document,
                            location,
                            vec![configuration_node],
                            &mut deleted_nodes,
                        );
                        continue;
                    }
                    if document.scenes.get(&active).is_some_and(|scene| {
                        matches!(
                            scene.nodes.get(node).map(|node| &node.data),
                            Some(NodeKind::Entity {
                                entity: EntityKind::Camera,
                                ..
                            })
                        ) && scene.options.contains(node)
                    }) {
                        // deleting a camera deletes every group attached
                        // to it — with their descendants, like any other tree
                        // parent — inside the same single undo step.
                        let location = NodeLocation::Scene(active);
                        let world = document.scene(active).unwrap().world;
                        let camera_node = document.scenes[&active].as_ref().nodes[node].clone();
                        let mut updated = document.scenes[&active].as_ref().clone();
                        let mut attached = Vec::new();
                        let mut world_tail = match &updated.nodes[&world].data {
                            NodeKind::Scope { children, .. } => children.clone(),
                            _ => unreachable!("the world is a scope"),
                        };
                        world_tail.retain(|child| {
                            let mine = matches!(
                                updated.nodes.get(child).map(|node| &node.data),
                                Some(NodeKind::Scope {
                                    parent_camera: Some(camera),
                                    ..
                                }) if *camera == *node
                            );
                            if mine {
                                attached.push(*child);
                            }
                            !mine
                        });
                        if let NodeKind::Scope { children, .. } =
                            &mut updated.nodes.get_mut(&world).unwrap().data
                        {
                            *children = world_tail;
                        }
                        updated.nodes.remove(node);
                        updated.options.retain(|id| id != node);
                        if updated.render_camera == Some(*node) {
                            updated.render_camera = None;
                        }
                        document.scenes.insert(active, Arc::new(updated));
                        scene_records.insert(active);
                        record_deleted_subtree(
                            &self.document,
                            location,
                            vec![camera_node],
                            &mut deleted_nodes,
                        );
                        for group in attached {
                            let removed = remove_scene_subtree(&mut document, active, group)?;
                            record_deleted_subtree(
                                &self.document,
                                location,
                                removed,
                                &mut deleted_nodes,
                            );
                        }
                        continue;
                    }
                    let unit = object_unit(&document, active, *node)?;
                    if document
                        .scenes
                        .get(&active)
                        .is_some_and(|scene| scene.render_camera == Some(unit))
                    {
                        let mut updated = document.scenes[&active].as_ref().clone();
                        updated.render_camera = None;
                        document.scenes.insert(active, Arc::new(updated));
                        scene_records.insert(active);
                    }
                    let (parent, index) = find_parent(&document, active, unit)?;
                    let (parent_node, location) = node_mut(&mut document, active, parent)?;
                    if let NodeKind::Scope { children, .. } = &mut parent_node.data {
                        children.remove(index);
                    }
                    node_ids.insert(parent, location);
                    record_deleted_subtree(
                        &self.document,
                        location,
                        remove_scene_subtree(&mut document, active, unit)?,
                        &mut deleted_nodes,
                    );
                }
                Edit::DuplicateNode { node } => {
                    let active = active.ok_or_else(|| invalid("no active scene"))?;
                    if document.definition_nodes.contains_key(node) {
                        let location = NodeLocation::Definition;
                        let unit = object_unit_at(&document, location, *node)?;
                        let (parent, index) = find_parent(&document, location, unit)?;
                        let source = visible_unit_label(&document, location, unit)?;
                        let label =
                            unique_visible_child_label(&document, location, parent, &source)?;
                        let mut copies = copy_subtree_at(&document, location, unit)?;
                        copies[0].label = label.clone();
                        let copied_owner = match &copies[0].data {
                            NodeKind::Scope {
                                owned_object: Some(owner),
                                ..
                            } => Some(*owner),
                            _ => None,
                        };
                        if let Some(owner) = copied_owner
                            && let Some(copy_owner) =
                                copies.iter_mut().find(|copy| copy.id == owner)
                        {
                            copy_owner.label = label;
                        }
                        let copy_id = copies[0].id;
                        let (parent_node, _) = node_mut(&mut document, active, parent)?;
                        if let NodeKind::Scope { children, .. } = &mut parent_node.data {
                            children.insert(index + 1, copy_id);
                        }
                        node_ids.insert(parent, location);
                        for copy in copies {
                            insert_node(&mut document, location, copy.clone())?;
                            created_nodes.push((location, copy));
                        }
                        continue;
                    }
                    if document.scenes.get(&active).is_some_and(|scene| {
                        matches!(
                            scene.nodes.get(node).map(|node| &node.data),
                            Some(NodeKind::Entity {
                                entity: EntityKind::Camera,
                                ..
                            })
                        ) && scene.options.contains(node)
                    }) {
                        let location = NodeLocation::Scene(active);
                        let scene = document.scenes[&active].as_ref();
                        let index = scene
                            .options
                            .iter()
                            .position(|id| id == node)
                            .ok_or_else(|| invalid("Camera is not in Options"))?;
                        let mut copy = scene.nodes[node].clone();
                        copy.id = NodeId::new();
                        copy.label = format!("{} Copy", copy.label);
                        copy.edition = Revision::initial();
                        let copy_id = copy.id;
                        let mut updated = scene.clone();
                        updated.options.insert(index + 1, copy_id);
                        updated.nodes.insert(copy_id, copy.clone());
                        created_nodes.push((location, copy));
                        // the duplicated camera owns copies of every
                        // attached group, right after the originals in the
                        // world, like copy_scene_subtree's own duplicates.
                        let world = updated.world;
                        let world_children = match &updated.nodes[&world].data {
                            NodeKind::Scope { children, .. } => children.clone(),
                            _ => unreachable!("the world is a scope"),
                        };
                        let mut created_groups = Vec::new();
                        for attached in world_children.iter().copied() {
                            let group =
                                match &document.scenes[&active].as_ref().nodes[&attached].data {
                                    NodeKind::Scope {
                                        parent_camera: Some(camera),
                                        ..
                                    } if *camera == *node => attached,
                                    _ => continue,
                                };
                            let source_label = visible_unit_label(&document, location, group)?;
                            let label = unique_visible_child_label(
                                &document,
                                location,
                                world,
                                &source_label,
                            )?;
                            let mut copies = copy_scene_subtree(&document, active, group)?;
                            copies[0].label = label.clone();
                            let copied_owner = match &copies[0].data {
                                NodeKind::Scope {
                                    owned_object: Some(owner),
                                    ..
                                } => Some(*owner),
                                _ => None,
                            };
                            if let Some(owner) = copied_owner
                                && let Some(copy_owner) =
                                    copies.iter_mut().find(|copy| copy.id == owner)
                            {
                                copy_owner.label = label;
                            }
                            if let NodeKind::Scope { parent_camera, .. } = &mut copies[0].data {
                                *parent_camera = Some(copy_id);
                            }
                            created_groups.push((group, copies[0].id));
                            for copy_node in copies {
                                updated.nodes.insert(copy_node.id, copy_node.clone());
                                created_nodes.push((location, copy_node));
                            }
                        }
                        if let NodeKind::Scope { children, .. } =
                            &mut updated.nodes.get_mut(&world).unwrap().data
                        {
                            // Each copy lands right after its original: the
                            // live `position` already counts the copies
                            // inserted by earlier turns of this loop.
                            for (original, group_copy) in &created_groups {
                                let at = children
                                    .iter()
                                    .position(|id| id == original)
                                    .expect("the attached group is a world child")
                                    + 1;
                                children.insert(at, *group_copy);
                            }
                        }
                        document.scenes.insert(active, Arc::new(updated));
                        scene_records.insert(active);
                        continue;
                    }
                    let unit = object_unit(&document, active, *node)?;
                    let (parent, index) = find_parent(&document, active, unit)?;
                    let location = NodeLocation::Scene(active);
                    let source = visible_unit_label(&document, location, unit)?;
                    let label = unique_visible_child_label(&document, location, parent, &source)?;
                    let mut copies = copy_scene_subtree(&document, active, unit)?;
                    copies[0].label = label.clone();
                    let copied_owner = match &copies[0].data {
                        NodeKind::Scope {
                            owned_object: Some(owner),
                            ..
                        } => Some(*owner),
                        _ => None,
                    };
                    if let Some(owner) = copied_owner
                        && let Some(copy_owner) = copies.iter_mut().find(|copy| copy.id == owner)
                    {
                        copy_owner.label = label;
                    }
                    let copy_id = copies[0].id;
                    let (parent_node, _) = node_mut(&mut document, active, parent)?;
                    if let NodeKind::Scope { children, .. } = &mut parent_node.data {
                        children.insert(index + 1, copy_id);
                    }
                    node_ids.insert(parent, location);
                    for copy in copies {
                        insert_node(&mut document, location, copy.clone())?;
                        created_nodes.push((location, copy));
                    }
                }
                Edit::MoveNode {
                    node,
                    parent: target,
                    after,
                } => {
                    let active = active.ok_or_else(|| invalid("no active scene"))?;
                    if document.definition_nodes.contains_key(node) {
                        if !document.definition_nodes.contains_key(target)
                            || after.is_some_and(|after| {
                                !document.definition_nodes.contains_key(&after)
                            })
                        {
                            return Err(invalid(
                                "nodes cannot move between a definition and the scene or another definition",
                            ));
                        }
                        let location = NodeLocation::Definition;
                        let unit = object_unit_at(&document, location, *node)?;
                        let after = after
                            .map(|after| object_unit_at(&document, location, after))
                            .transpose()?;
                        let target = *target;
                        if definition_root(&document, unit) != definition_root(&document, target) {
                            return Err(invalid(
                                "nodes cannot move between a definition and the scene or another definition",
                            ));
                        }
                        if subtree_ids_at(&document, location, unit)?.contains(&target) {
                            return Err(invalid("cannot move a group into itself"));
                        }
                        if !matches!(
                            document
                                .definition_nodes
                                .get(&target)
                                .map(|value| &value.data),
                            Some(NodeKind::Scope {
                                scope: ScopeKind::ObjectDefinition
                                    | ScopeKind::Attribute
                                    | ScopeKind::Transform,
                                owned_object: None,
                                ..
                            })
                        ) {
                            return Err(invalid("move target is not a group"));
                        }
                        let (old_parent, old_index) = find_parent(&document, location, unit)?;
                        let (old_parent_node, _) = node_mut(&mut document, active, old_parent)?;
                        if let NodeKind::Scope { children, .. } = &mut old_parent_node.data {
                            children.remove(old_index);
                        }
                        node_ids.insert(old_parent, location);
                        let (target_node, _) = node_mut(&mut document, active, target)?;
                        let NodeKind::Scope { children, .. } = &mut target_node.data else {
                            unreachable!()
                        };
                        let at = insertion_index(children, after)?;
                        children.insert(at, unit);
                        node_ids.insert(target, location);
                        continue;
                    } else if document.definition_nodes.contains_key(target)
                        || after.is_some_and(|after| document.definition_nodes.contains_key(&after))
                    {
                        return Err(invalid(
                            "nodes cannot move between a definition and the scene or another definition",
                        ));
                    }
                    let world = document.scene(active).unwrap().world;
                    let target = if *target == NodeId(active.0) {
                        world
                    } else {
                        *target
                    };
                    let unit = object_unit(&document, active, *node)?;
                    let after = after
                        .map(|after| object_unit(&document, active, after))
                        .transpose()?;
                    if after == Some(unit) {
                        continue;
                    }
                    // dropping a unit on a Camera row attaches it: the
                    // unit moves to World level and references the camera.
                    let target_is_camera = target != world
                        && matches!(
                            node_value(&document, active, target)?.data,
                            NodeKind::Entity {
                                entity: EntityKind::Camera,
                                ..
                            }
                        )
                        && document.scene(active).unwrap().options.contains(&target);
                    if !target_is_camera {
                        if !matches!(
                            node_value(&document, active, target)?.data,
                            NodeKind::Scope {
                                scope: ScopeKind::World
                                    | ScopeKind::Attribute
                                    | ScopeKind::Transform,
                                ..
                            }
                        ) {
                            return Err(invalid("move target is not a group"));
                        }
                        if matches!(
                            node_value(&document, active, target)?.data,
                            NodeKind::Scope {
                                owned_object: Some(_),
                                ..
                            }
                        ) {
                            return Err(invalid("move target is an internal scope"));
                        }
                    }
                    if subtree_ids(&document, active, unit)?.contains(&target) {
                        return Err(invalid("cannot move a group into itself"));
                    }
                    let (parent, index) = find_parent(&document, active, unit)?;
                    let (parent_node, location) = node_mut(&mut document, active, parent)?;
                    if let NodeKind::Scope { children, .. } = &mut parent_node.data {
                        children.remove(index);
                    }
                    node_ids.insert(parent, location);
                    if target_is_camera {
                        // A non-Attribute unit wraps in a fresh Attribute
                        // scope first, so the camera references a scope. The
                        // unit is already detached; only the wrapper's node
                        // registers here.
                        let unit = if matches!(
                            node_value(&document, active, unit)?.data,
                            NodeKind::Scope {
                                scope: ScopeKind::Attribute,
                                ..
                            }
                        ) {
                            unit
                        } else {
                            let scope = Node::new(
                                node_value(&document, active, unit)?.label.clone(),
                                NodeKind::Scope {
                                    scope: ScopeKind::Attribute,
                                    children: vec![unit],
                                    owned_object: Some(unit),
                                    parent_camera: None,
                                },
                            );
                            let location = NodeLocation::Scene(active);
                            insert_node(&mut document, location, scope.clone())?;
                            created_nodes.push((location, scope.clone()));
                            node_ids.insert(unit, location);
                            scope.id
                        };
                        let (unit_node, unit_location) = node_mut(&mut document, active, unit)?;
                        let NodeKind::Scope { parent_camera, .. } = &mut unit_node.data else {
                            unreachable!("the moved unit is a scope");
                        };
                        *parent_camera = Some(target);
                        node_ids.insert(unit, unit_location);
                        let (world_node, location) = node_mut(&mut document, active, world)?;
                        let NodeKind::Scope { children, .. } = &mut world_node.data else {
                            unreachable!("the world is a scope");
                        };
                        let at = match after {
                            // After another attached group: slot in next to it.
                            Some(after) => children
                                .iter()
                                .position(|child| *child == after)
                                .map(|at| at + 1)
                                .unwrap_or(children.len()),
                            None => children.len(),
                        };
                        children.insert(at, unit);
                        node_ids.insert(unit, location);
                        node_ids.insert(world, location);
                        continue;
                    }
                    // Any camera attachment clears when the unit moves to a
                    // group or the World.
                    if let Ok((unit_node, location)) = node_mut(&mut document, active, unit)
                        && let NodeKind::Scope { parent_camera, .. } = &mut unit_node.data
                        && parent_camera.is_some()
                    {
                        *parent_camera = None;
                        node_ids.insert(unit, location);
                    }
                    let (target_node, _) = node_mut(&mut document, active, target)?;
                    let NodeKind::Scope { children, .. } = &mut target_node.data else {
                        return Err(invalid("move target is not a group"));
                    };
                    let at = insertion_index(children, after)?;
                    children.insert(at, unit);
                    node_ids.insert(target, location);
                }
                Edit::SetInstanceDefinition { node, definition } => {
                    let active = active.ok_or_else(|| invalid("no active scene"))?;
                    if document
                        .resources
                        .get(definition)
                        .is_none_or(|resource| resource.kind != ResourceKind::ObjectDefinition)
                    {
                        return Err(invalid("missing object definition"));
                    }
                    let (node_value, location) = node_mut(&mut document, active, *node)?;
                    let NodeKind::Instance {
                        definition: current,
                    } = &mut node_value.data
                    else {
                        return Err(invalid("edited node is not an object instance"));
                    };
                    *current = *definition;
                    node_ids.insert(*node, location);
                }
                Edit::SetParameter {
                    target,
                    name,
                    value_type,
                    value,
                } => {
                    if let Some(value) = value
                        && (value.name != *name || value.value_type != *value_type)
                    {
                        return Err(invalid("parameter key does not match replacement"));
                    }
                    let parameters = match target {
                        ParameterTarget::Node(id) => {
                            let (node, location) = node_mut(&mut document, active, *id)?;
                            node_ids.insert(*id, location);
                            match &mut node.data {
                                NodeKind::Entity { parameters, .. }
                                | NodeKind::Configuration { parameters, .. } => parameters,
                                _ => return Err(invalid("node does not declare parameters")),
                            }
                        }
                        ParameterTarget::Resource(id) => {
                            resource_ids.insert(*id);
                            &mut document
                                .resources
                                .get_mut(id)
                                .ok_or_else(|| invalid("missing parameter resource"))?
                                .parameters
                        }
                    };
                    let matches = parameters
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| p.name == *name)
                        .map(|(i, _)| i)
                        .collect::<Vec<_>>();
                    if matches.len() > 1 {
                        return Err(invalid("ambiguous duplicate parameter declaration"));
                    }
                    if let Some(&index) = matches.first() {
                        if parameters[index].value_type != *value_type {
                            return Err(invalid(
                                "cannot silently change a parameter declaration type",
                            ));
                        }
                        if let Some(value) = value {
                            parameters[index] = value.clone();
                        } else {
                            parameters.remove(index);
                        }
                    } else if let Some(value) = value {
                        parameters.push(value.clone());
                    }
                }
                Edit::SetParameterValues {
                    target,
                    name,
                    offset,
                    values,
                } => {
                    let parameters = match target {
                        ParameterTarget::Node(id) => {
                            let (node, location) = node_mut(&mut document, active, *id)?;
                            node_ids.insert(*id, location);
                            node_parameters_mut(node)?
                        }
                        ParameterTarget::Resource(id) => {
                            resource_ids.insert(*id);
                            &mut document
                                .resources
                                .get_mut(id)
                                .ok_or_else(|| invalid("missing parameter resource"))?
                                .parameters
                        }
                    };
                    let matches = parameters
                        .iter_mut()
                        .filter(|parameter| parameter.name == *name)
                        .collect::<Vec<_>>();
                    if matches.len() != 1 {
                        return Err(invalid("parameter is missing or ambiguous"));
                    }
                    patch_parameter_window(
                        &mut matches.into_iter().next().unwrap().values,
                        *offset,
                        values,
                    )?;
                }
                Edit::SetImplementation {
                    target,
                    implementation,
                } => {
                    if implementation.is_empty() {
                        return Err(invalid("implementation must not be empty"));
                    }
                    let current = match target {
                        ParameterTarget::Node(id) => {
                            let (node, location) = node_mut(&mut document, active, *id)?;
                            node_ids.insert(*id, location);
                            match &mut node.data {
                                NodeKind::Entity { implementation, .. }
                                | NodeKind::Configuration { implementation, .. } => implementation,
                                _ => return Err(invalid("node has no implementation")),
                            }
                        }
                        ParameterTarget::Resource(id) => {
                            resource_ids.insert(*id);
                            let resource = document
                                .resources
                                .get_mut(id)
                                .ok_or_else(|| invalid("missing resource"))?;
                            if resource.kind == ResourceKind::ObjectDefinition {
                                return Err(invalid("object definitions have no implementation"));
                            }
                            &mut resource.implementation
                        }
                    };
                    if current.is_empty() {
                        return Err(invalid(
                            "the cancelling area light has no implementation to change",
                        ));
                    }
                    current.clone_from(implementation);
                }
            }
        }
        document.default_scene = document
            .default_scene
            .filter(|id| document.scenes.contains_key(id));
        node_ids.retain(|id, location| {
            node(&document, *location, *id).is_some_and(|current| {
                node(&self.document, *location, *id).is_some_and(|old| current != old)
            })
        });
        resource_ids.retain(|id| document.resources.get(id) != self.document.resources.get(id));
        let changed_scenes = scene_records
            .into_iter()
            .filter(|id| document.scenes.get(id) != self.document.scenes.get(id))
            .collect::<Vec<_>>();
        let changed_assets = asset_records
            .into_iter()
            .filter(|id| document.assets.get(id) != self.document.assets.get(id))
            .collect::<Vec<_>>();
        folder_records &= document.folders != self.document.folders;
        let default_scene_changed = document.default_scene != self.document.default_scene;
        if node_ids.is_empty()
            && resource_ids.is_empty()
            && changed_scenes.is_empty()
            && changed_assets.is_empty()
            && !folder_records
            && !default_scene_changed
            && created_items.is_empty()
        {
            return Ok(ChangeSet {
                revision: self.revision(),
                nodes: Vec::new(),
                resources: Vec::new(),
                created: Vec::new(),
                created_items: Vec::new(),
            });
        }
        let next = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("project revision overflow"))?;
        for (id, location) in &node_ids {
            let (edited, _) = node_mut(&mut document, active, *id)?;
            edited.edition = bump(&node(&self.document, *location, *id).unwrap().edition)?;
        }
        for id in &resource_ids {
            if let (Some(current), Some(old)) = (
                document.resources.get_mut(id),
                self.document.resources.get(id),
            ) {
                current.edition = bump(&old.edition)?;
            }
        }
        document.default_scene = document
            .default_scene
            .filter(|id| document.scenes.contains_key(id));
        document.validate()?;
        for scene in document.scenes.values() {
            crate::evaluate(&document.scene_view(scene.id)?)?;
        }
        let delta = Delta {
            nodes: node_ids
                .iter()
                .map(|(id, location)| {
                    (
                        *location,
                        *id,
                        node(&self.document, *location, *id).unwrap().clone(),
                        node(&document, *location, *id).unwrap().clone(),
                    )
                })
                .collect(),
            resources: resource_ids
                .iter()
                .map(|id| {
                    (
                        *id,
                        self.document.resources.get(id).cloned(),
                        document.resources.get(id).cloned(),
                    )
                })
                .collect(),
            scenes: changed_scenes
                .iter()
                .map(|id| {
                    (
                        *id,
                        self.document.scenes.get(id).cloned(),
                        document.scenes.get(id).cloned(),
                    )
                })
                .collect(),
            assets: changed_assets
                .iter()
                .map(|id| {
                    (
                        *id,
                        self.document.assets.get(id).cloned(),
                        document.assets.get(id).cloned(),
                    )
                })
                .collect(),
            folders: folder_records
                .then(|| (self.document.folders.clone(), document.folders.clone())),
            created_nodes: created_nodes
                .into_iter()
                .filter_map(|(location, new_node)| {
                    let updated = node(&document, location, new_node.id)?.clone();
                    Some((location, updated))
                })
                .collect(),
            deleted_nodes,
            imports: import_records
                .iter()
                .map(|id| {
                    (
                        *id,
                        self.document.imports.get(id).cloned(),
                        document.imports.get(id).cloned(),
                    )
                })
                .collect(),
        };
        let created = delta
            .created_nodes
            .iter()
            .map(|(_, node)| node.id)
            .collect::<Vec<_>>();
        let created_items = created_items.clone();
        self.document = Arc::new(document);
        self.repair_active_scene();
        self.revision = next;
        self.current_content = ContentId::of(&self.document);
        self.reconcile_live_payloads(&staged_payloads);
        self.history.record(HistoryEntry::new(
            OperationLabel::of_edits(edits),
            delta,
            staged_payloads,
        ));
        Ok(ChangeSet {
            revision: self.revision(),
            nodes: node_ids.keys().copied().collect(),
            resources: resource_ids.into_iter().collect(),
            created,
            created_items,
        })
    }

    fn restore(&self, delta: &Delta, forward: bool) -> Result<(ProjectDocument, u64)> {
        let mut document = (*self.document).clone();
        for (location, id, before, after) in &delta.nodes {
            let owner = match location {
                NodeLocation::Scene(scene) => Some(*scene),
                NodeLocation::Definition => None,
            };
            let (node, _) = node_mut(&mut document, owner, *id)?;
            let source = if forward { after } else { before };
            let edition = bump(&node.edition)?;
            let mut replacement = source.clone();
            replacement.edition = edition;
            *node = replacement;
        }
        for (id, before, after) in &delta.resources {
            match if forward { after } else { before } {
                Some(resource) => {
                    let mut resource = resource.clone();
                    if let Some(current) = document.resources.get(id) {
                        resource.edition = bump(&current.edition)?;
                    }
                    document.resources.insert(*id, resource);
                }
                None => {
                    document.resources.remove(id);
                }
            }
        }
        for (id, before, after) in &delta.scenes {
            match if forward { after } else { before } {
                Some(scene) => {
                    document.scenes.insert(*id, scene.clone());
                }
                None => {
                    document.scenes.remove(id);
                }
            }
        }
        for (id, before, after) in &delta.assets {
            match if forward { after } else { before } {
                Some(asset) => {
                    document.assets.insert(*id, asset.clone());
                }
                None => {
                    document.assets.remove(id);
                }
            }
        }
        for (id, before, after) in &delta.imports {
            match if forward { after } else { before } {
                Some(record) => {
                    document.imports.insert(*id, record.clone());
                }
                None => {
                    document.imports.remove(id);
                }
            }
        }
        if let Some((before, after)) = &delta.folders {
            document.folders = if forward {
                after.clone()
            } else {
                before.clone()
            };
        }
        for (location, created) in &delta.created_nodes {
            match location {
                NodeLocation::Scene(scene) => {
                    let Some(scene) = document.scenes.get_mut(scene) else {
                        if forward {
                            // The same transaction may also remove this entire scene.
                            continue;
                        }
                        return Err(invalid("missing scene for created node"));
                    };
                    let nodes = &mut Arc::make_mut(scene).nodes;
                    if forward {
                        nodes.insert(created.id, created.clone());
                    } else {
                        nodes.remove(&created.id);
                    }
                }
                NodeLocation::Definition => {
                    if forward {
                        document
                            .definition_nodes
                            .insert(created.id, created.clone());
                    } else {
                        document.definition_nodes.remove(&created.id);
                    }
                }
            }
        }
        for (location, deleted) in &delta.deleted_nodes {
            match location {
                NodeLocation::Scene(scene) => {
                    let Some(scene) = document.scenes.get_mut(scene) else {
                        if forward {
                            // Deleting the scene already removes all of its node records.
                            continue;
                        }
                        return Err(invalid("missing scene for deleted node"));
                    };
                    let nodes = &mut Arc::make_mut(scene).nodes;
                    if forward {
                        nodes.remove(&deleted.id);
                    } else {
                        nodes.insert(deleted.id, deleted.clone());
                    }
                }
                NodeLocation::Definition => {
                    if forward {
                        document.definition_nodes.remove(&deleted.id);
                    } else {
                        document
                            .definition_nodes
                            .insert(deleted.id, deleted.clone());
                    }
                }
            }
        }
        document.default_scene = document
            .default_scene
            .filter(|id| document.scenes.contains_key(id));
        document.validate()?;
        for scene in document.scenes.values() {
            crate::evaluate(&document.scene_view(scene.id)?)?;
        }
        Ok((
            document,
            self.revision
                .checked_add(1)
                .ok_or_else(|| invalid("project revision overflow"))?,
        ))
    }
    pub fn undo(&mut self, expected: &Revision) -> Result<ChangeSet> {
        self.check(expected)?;
        let entry = self
            .history
            .undo_peek()
            .cloned()
            .ok_or_else(|| invalid("nothing to undo"))?;
        let (document, next) = self.restore(&entry.delta, false)?;
        let result = changes(&entry.delta, next, false);
        self.history.shift_to_redo();
        self.document = Arc::new(document);
        self.repair_active_scene();
        self.revision = next;
        self.current_content = ContentId::of(&self.document);
        self.reconcile_live_payloads(&entry.payloads);
        Ok(result)
    }
    pub fn redo(&mut self, expected: &Revision) -> Result<ChangeSet> {
        self.check(expected)?;
        let entry = self
            .history
            .redo_peek()
            .cloned()
            .ok_or_else(|| invalid("nothing to redo"))?;
        let (document, next) = self.restore(&entry.delta, true)?;
        let result = changes(&entry.delta, next, true);
        self.history.shift_to_undo();
        self.document = Arc::new(document);
        self.repair_active_scene();
        self.revision = next;
        self.current_content = ContentId::of(&self.document);
        self.reconcile_live_payloads(&entry.payloads);
        Ok(result)
    }
}
fn node<'a>(document: &'a ProjectDocument, location: NodeLocation, id: NodeId) -> Option<&'a Node> {
    match location {
        NodeLocation::Scene(scene) => document.scenes.get(&scene)?.nodes.get(&id),
        NodeLocation::Definition => document.definition_nodes.get(&id),
    }
}
fn scene_label(location: &str) -> Result<String> {
    let filename = location
        .rsplit('/')
        .next()
        .ok_or_else(|| invalid("scene location has no filename"))?;
    let label = filename.rsplit_once('.').map_or(filename, |(stem, _)| stem);
    if label.trim().is_empty() {
        return Err(invalid("scene location has an empty filename"));
    }
    Ok(label.to_owned())
}
fn validate_scene_location(location: &str) -> Result<()> {
    if !location.ends_with(".scene") {
        return Err(invalid("scene location must use the .scene extension"));
    }
    Ok(())
}
fn empty_scene(label: String, location: String) -> SceneDocument {
    let world = Node::new(
        "World",
        NodeKind::Scope {
            scope: ScopeKind::World,
            children: Vec::new(),
            owned_object: None,
            parent_camera: None,
        },
    );
    SceneDocument {
        id: SceneId::new(),
        label,
        location,
        options: Vec::new(),
        render_camera: None,
        coordinate_system: CoordinateSystem::default(),
        world: world.id,
        nodes: BTreeMap::from([(world.id, world)]),
    }
}
fn node_parameters_mut(node: &mut Node) -> Result<&mut Vec<Parameter>> {
    match &mut node.data {
        NodeKind::Entity { parameters, .. } | NodeKind::Configuration { parameters, .. } => {
            Ok(parameters)
        }
        _ => Err(invalid("node does not declare parameters")),
    }
}
fn set_asset_param(parameters: &mut [Parameter], name: &str, asset: AssetId) -> Result<()> {
    let matches = parameters
        .iter_mut()
        .filter(|parameter| parameter.name == name)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(invalid("asset parameter is missing or ambiguous"));
    }
    match &mut matches.into_iter().next().unwrap().values {
        ParameterValues::Assets(ids) if ids.len() == 1 => ids[0] = asset,
        ParameterValues::Array { asset: current, .. } => *current = asset,
        _ => return Err(invalid("parameter is not a replaceable asset reference")),
    }
    Ok(())
}

fn patch_parameter_window(
    current: &mut ParameterValues,
    offset: usize,
    replacement: &ParameterValues,
) -> Result<()> {
    fn patch<T: Clone>(current: &mut [T], offset: usize, replacement: &[T]) -> Result<()> {
        let end = offset
            .checked_add(replacement.len())
            .ok_or_else(|| invalid("parameter value window overflows"))?;
        if replacement.is_empty() || replacement.len() > 4096 || end > current.len() {
            return Err(invalid(
                "parameter value window is outside the existing array",
            ));
        }
        current[offset..end].clone_from_slice(replacement);
        Ok(())
    }
    match (current, replacement) {
        (ParameterValues::Floats(current), ParameterValues::Floats(replacement)) => {
            patch(current, offset, replacement)
        }
        (ParameterValues::Integers(current), ParameterValues::Integers(replacement)) => {
            patch(current, offset, replacement)
        }
        (ParameterValues::Booleans(current), ParameterValues::Booleans(replacement)) => {
            patch(current, offset, replacement)
        }
        (ParameterValues::Strings(current), ParameterValues::Strings(replacement)) => {
            patch(current, offset, replacement)
        }
        (
            ParameterValues::BuiltinSpectra(current),
            ParameterValues::BuiltinSpectra(replacement),
        ) => patch(current, offset, replacement),
        (ParameterValues::Assets(current), ParameterValues::Assets(replacement)) => {
            patch(current, offset, replacement)
        }
        (ParameterValues::Resources { .. }, _) => {
            Err(invalid("resource lists are not editable as array values"))
        }
        (ParameterValues::Array { .. }, _) => {
            Err(invalid("data asset values require a staged replacement"))
        }
        _ => Err(invalid(
            "replacement values do not match the parameter array type",
        )),
    }
}

fn move_folder(document: &mut ProjectDocument, from: &str, to: &str) -> Result<()> {
    if !document.folders.contains(from) {
        return Err(invalid("source folder does not exist"));
    }
    if to == from || to.starts_with(&format!("{from}/")) {
        return Err(invalid("cannot move a folder into itself"));
    }
    let replace = |path: &str| {
        if path == from {
            to.to_owned()
        } else {
            format!("{to}{}", &path[from.len()..])
        }
    };
    let moved_folders = document
        .folders
        .iter()
        .filter(|path| *path == from || path.starts_with(&format!("{from}/")))
        .cloned()
        .collect::<Vec<_>>();
    if moved_folders.is_empty() {
        return Err(invalid("source folder does not exist"));
    }
    for folder in &moved_folders {
        document.folders.remove(folder);
    }
    for folder in moved_folders {
        document.folders.insert(replace(&folder));
    }
    for scene in document.scenes.values_mut() {
        if scene.location == from || scene.location.starts_with(&format!("{from}/")) {
            let mut moved = scene.as_ref().clone();
            moved.location = replace(&moved.location);
            *scene = Arc::new(moved);
        }
    }
    for resource in document.resources.values_mut() {
        if resource.location == from || resource.location.starts_with(&format!("{from}/")) {
            resource.location = replace(&resource.location);
        }
    }
    for asset in document.assets.values_mut() {
        if asset.location == from || asset.location.starts_with(&format!("{from}/")) {
            asset.location = replace(&asset.location);
        }
    }
    Ok(())
}
fn delete_folder(document: &mut ProjectDocument, path: &str) -> Result<()> {
    if !document.folders.contains(path) {
        return Err(invalid("folder does not exist"));
    }
    let prefix = format!("{path}/");
    if document
        .folders
        .iter()
        .any(|folder| folder.starts_with(&prefix))
        || document
            .scenes
            .values()
            .any(|scene| scene.location.starts_with(&prefix))
        || document
            .resources
            .values()
            .any(|resource| resource.location.starts_with(&prefix))
        || document
            .assets
            .values()
            .any(|asset| asset.location.starts_with(&prefix))
    {
        return Err(invalid("folder is not empty"));
    }
    document.folders.remove(path);
    Ok(())
}
fn remove_definition_subtree(document: &mut ProjectDocument, root: NodeId) -> Vec<Node> {
    let mut stack = vec![root];
    let mut removed = Vec::new();
    while let Some(id) = stack.pop() {
        if let Some(node) = document.definition_nodes.remove(&id) {
            if let NodeKind::Scope { children, .. } = &node.data {
                stack.extend(children);
            }
            removed.push(node);
        }
    }
    removed
}
fn node_mut<'a>(
    document: &'a mut ProjectDocument,
    active: impl Into<Option<SceneId>>,
    id: NodeId,
) -> Result<(&'a mut Node, NodeLocation)> {
    if let Some(active) = active.into()
        && document
            .scenes
            .get(&active)
            .is_some_and(|scene| scene.nodes.contains_key(&id))
    {
        let scene = Arc::make_mut(
            document
                .scenes
                .get_mut(&active)
                .ok_or_else(|| invalid("missing active scene"))?,
        );
        let node = scene
            .nodes
            .get_mut(&id)
            .ok_or_else(|| invalid("missing edited node"))?;
        return Ok((node, NodeLocation::Scene(active)));
    }
    let node = document
        .definition_nodes
        .get_mut(&id)
        .ok_or_else(|| invalid("missing edited node"))?;
    Ok((node, NodeLocation::Definition))
}
fn nodes_mut(
    document: &mut ProjectDocument,
    location: NodeLocation,
) -> Result<&mut BTreeMap<NodeId, Node>> {
    Ok(match location {
        NodeLocation::Scene(scene) => {
            &mut Arc::make_mut(
                document
                    .scenes
                    .get_mut(&scene)
                    .ok_or_else(|| invalid("missing node scene"))?,
            )
            .nodes
        }
        NodeLocation::Definition => &mut document.definition_nodes,
    })
}
fn insert_node(document: &mut ProjectDocument, location: NodeLocation, node: Node) -> Result<()> {
    nodes_mut(document, location)?.insert(node.id, node);
    Ok(())
}
fn insertion_index(children: &[NodeId], after: Option<NodeId>) -> Result<usize> {
    match after {
        Some(after) => children
            .iter()
            .position(|id| *id == after)
            .map(|index| index + 1)
            .ok_or_else(|| invalid("after node is not a child of the parent")),
        None => Ok(children.len()),
    }
}

/// First scene-order instance of a definition. Instances can be nested in
/// attribute groups, so walk the ordered World tree rather than the node map.
fn first_instance_of_definition(scene: &SceneDocument, definition: ResourceId) -> Option<NodeId> {
    fn visit(scene: &SceneDocument, id: NodeId, definition: ResourceId) -> Option<NodeId> {
        let node = scene.nodes.get(&id)?;
        match &node.data {
            NodeKind::Instance { definition: target } if *target == definition => Some(id),
            NodeKind::Scope { children, .. } => children
                .iter()
                .find_map(|child| visit(scene, *child, definition)),
            _ => None,
        }
    }
    visit(scene, scene.world, definition)
}

fn unique_visible_child_label(
    document: &ProjectDocument,
    location: NodeLocation,
    parent: NodeId,
    label: &str,
) -> Result<String> {
    let parent =
        node(document, location, parent).ok_or_else(|| invalid("missing preset parent"))?;
    let NodeKind::Scope { children, .. } = &parent.data else {
        return Err(invalid("preset parent is not a group"));
    };
    let used = children
        .iter()
        .filter_map(|id| node(document, location, *id))
        .map(|child| match &child.data {
            NodeKind::Scope {
                owned_object: Some(owner),
                ..
            } => node(document, location, *owner).unwrap_or(child),
            _ => child,
        })
        .map(|node| node.label.as_str())
        .collect::<BTreeSet<_>>();
    if !used.contains(label) {
        return Ok(label.to_owned());
    }
    for suffix in 1..=u32::MAX {
        let candidate = format!("{label} ({suffix})");
        if !used.contains(candidate.as_str()) {
            return Ok(candidate);
        }
    }
    Err(invalid("could not allocate a unique node label"))
}

fn visible_unit_label(
    document: &ProjectDocument,
    location: NodeLocation,
    unit: NodeId,
) -> Result<String> {
    let root = node(document, location, unit).ok_or_else(|| invalid("missing edited node"))?;
    if let NodeKind::Scope {
        owned_object: Some(owner),
        ..
    } = &root.data
    {
        return node(document, location, *owner)
            .map(|node| node.label.clone())
            .ok_or_else(|| invalid("missing owned object"));
    }
    Ok(root.label.clone())
}
fn node_value(document: &ProjectDocument, scene: SceneId, id: NodeId) -> Result<&Node> {
    node(document, NodeLocation::Scene(scene), id).ok_or_else(|| invalid("missing edited node"))
}

/// What delete / duplicate / move act on for an object: its dedicated group
/// (an Attribute scope other than World whose only object child it is, the
/// shape a preset or `set_local_binding` makes) or the object itself. Only
/// scene-graph objects under World qualify: groups, shapes, lights and
/// instances, never World, the camera or the Scene root.
fn object_unit(document: &ProjectDocument, scene: SceneId, object: NodeId) -> Result<NodeId> {
    object_unit_at(document, NodeLocation::Scene(scene), object)
}

/// The model node a visible object owns when it has an explicit internal
/// Attribute wrapper. Ordinary one-child Attribute groups remain independent.
fn object_unit_at(
    document: &ProjectDocument,
    location: NodeLocation,
    object: NodeId,
) -> Result<NodeId> {
    let value = node(document, location, object).ok_or_else(|| invalid("missing edited node"))?;
    let movable = matches!(
        &value.data,
        NodeKind::Entity {
            entity: EntityKind::Shape | EntityKind::Light,
            ..
        } | NodeKind::Instance { .. }
            | NodeKind::Scope {
                scope: ScopeKind::Attribute | ScopeKind::Transform,
                ..
            }
    );
    if !movable {
        return Err(invalid(
            "only groups, shapes, lights and instances can be edited",
        ));
    }
    if matches!(
        &value.data,
        NodeKind::Scope {
            owned_object: Some(_),
            ..
        }
    ) {
        return Err(invalid(
            "internal scopes are not visible scene-graph objects",
        ));
    }
    let (parent, _) = find_parent(document, location, object)?;
    let parent_value =
        node(document, location, parent).ok_or_else(|| invalid("missing edited node parent"))?;
    if matches!(
        &parent_value.data,
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            owned_object: Some(owner),
            ..
        } if *owner == object
    ) {
        return Ok(parent);
    }
    Ok(object)
}

/// Ids in one scene subtree, root first.
fn subtree_ids(document: &ProjectDocument, scene: SceneId, root: NodeId) -> Result<Vec<NodeId>> {
    let mut ids = Vec::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let value = node_value(document, scene, id)?;
        ids.push(id);
        if let NodeKind::Scope { children, .. } = &value.data {
            stack.extend(children.iter().rev());
        }
    }
    Ok(ids)
}

fn subtree_ids_at(
    document: &ProjectDocument,
    location: NodeLocation,
    root: NodeId,
) -> Result<Vec<NodeId>> {
    let mut ids = Vec::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let value = node(document, location, id).ok_or_else(|| invalid("missing edited node"))?;
        ids.push(id);
        if let NodeKind::Scope { children, .. } = &value.data {
            stack.extend(children.iter().rev());
        }
    }
    Ok(ids)
}

fn copy_subtree_at(
    document: &ProjectDocument,
    location: NodeLocation,
    root: NodeId,
) -> Result<Vec<Node>> {
    let ids = subtree_ids_at(document, location, root)?;
    let fresh = ids
        .iter()
        .map(|id| (*id, NodeId::new()))
        .collect::<BTreeMap<_, _>>();
    ids.iter()
        .map(|id| {
            let mut copy = node(document, location, *id)
                .ok_or_else(|| invalid("missing edited node"))?
                .clone();
            copy.id = fresh[id];
            copy.edition = Revision::initial();
            if let NodeKind::Scope { children, .. } = &mut copy.data {
                *children = children.iter().map(|child| fresh[child]).collect();
            }
            if let NodeKind::Scope {
                owned_object: Some(owner),
                ..
            } = &mut copy.data
            {
                *owner = fresh[owner];
            }
            Ok(copy)
        })
        .collect()
}

fn remove_scene_subtree(
    document: &mut ProjectDocument,
    scene: SceneId,
    root: NodeId,
) -> Result<Vec<Node>> {
    let ids = subtree_ids(document, scene, root)?;
    let nodes = &mut Arc::make_mut(
        document
            .scenes
            .get_mut(&scene)
            .ok_or_else(|| invalid("missing active scene"))?,
    )
    .nodes;
    Ok(ids.iter().filter_map(|id| nodes.remove(id)).collect())
}

/// A copy of one subtree with fresh node ids, root first.
fn copy_scene_subtree(
    document: &ProjectDocument,
    scene: SceneId,
    root: NodeId,
) -> Result<Vec<Node>> {
    let ids = subtree_ids(document, scene, root)?;
    let fresh = ids
        .iter()
        .map(|id| (*id, NodeId::new()))
        .collect::<BTreeMap<_, _>>();
    ids.iter()
        .map(|id| {
            let mut copy = node_value(document, scene, *id)?.clone();
            copy.id = fresh[id];
            copy.edition = Revision::initial();
            if let NodeKind::Scope { children, .. } = &mut copy.data {
                *children = children.iter().map(|child| fresh[child]).collect();
            }
            if let NodeKind::Scope {
                owned_object: Some(owner),
                ..
            } = &mut copy.data
            {
                *owner = fresh[owner];
            }
            Ok(copy)
        })
        .collect()
}

/// The ObjectDefinition scope that holds a definition node (itself for a root).
fn definition_root(document: &ProjectDocument, mut id: NodeId) -> NodeId {
    while let Ok((parent, _)) = find_parent(document, NodeLocation::Definition, id) {
        id = parent;
    }
    id
}
fn find_parent(
    document: &ProjectDocument,
    location: impl Into<NodeLocation>,
    target: NodeId,
) -> Result<(NodeId, usize)> {
    let nodes = match location.into() {
        NodeLocation::Scene(scene) => {
            &document
                .scene(scene)
                .ok_or_else(|| invalid("missing active scene"))?
                .nodes
        }
        NodeLocation::Definition => &document.definition_nodes,
    };
    for node in nodes.values() {
        if let NodeKind::Scope { children, .. } = &node.data
            && let Some(index) = children.iter().position(|id| *id == target)
        {
            return Ok((node.id, index));
        }
    }
    Err(invalid("node has no scene parent"))
}
/// Undo re-inserts a deleted subtree as it was at the start of this apply, so
/// edits earlier in the same transaction roll back with it (a rename, a
/// local binding, a parameter edit). Nodes created inside the transaction
/// never existed before it and are not recorded: their `created_nodes`
/// entries drop out at delta construction.
fn record_deleted_subtree(
    before: &ProjectDocument,
    location: NodeLocation,
    removed: Vec<Node>,
    deleted: &mut Vec<(NodeLocation, Node)>,
) {
    for removed in removed {
        if let Some(pre) = node(before, location, removed.id) {
            deleted.push((location, pre.clone()));
        }
    }
}

/// A scene-graph object: groups, shapes, lights, cameras and
/// instances. State nodes (bindings, transforms) are never objects. This is
/// the one predicate that counts object children.
fn is_object_node(node: &Node) -> bool {
    matches!(
        &node.data,
        NodeKind::Entity {
            entity: EntityKind::Shape | EntityKind::Light | EntityKind::Camera,
            ..
        } | NodeKind::Instance { .. }
            | NodeKind::Scope {
                scope: ScopeKind::Attribute | ScopeKind::Transform,
                ..
            }
    )
}
fn child_index(
    document: &ProjectDocument,
    scene: SceneId,
    parent: NodeId,
    target: NodeId,
) -> Result<usize> {
    let parent = node(document, NodeLocation::Scene(scene), parent)
        .ok_or_else(|| invalid("missing binding group"))?;
    let NodeKind::Scope { children, .. } = &parent.data else {
        return Err(invalid("binding parent is not a group"));
    };
    children
        .iter()
        .position(|id| *id == target)
        .ok_or_else(|| invalid("object is not in its binding group"))
}
fn insert_binding(
    document: &mut ProjectDocument,
    scene: SceneId,
    parent: NodeId,
    object_index: usize,
    operation: StateOperation,
    created: &mut Vec<(NodeLocation, Node)>,
    changed: &mut BTreeMap<NodeId, NodeLocation>,
) -> Result<()> {
    let location = NodeLocation::Scene(scene);
    let (run_start, run) = binding_run(document, scene, parent, object_index)?;
    let kind = operation_kind(&operation);
    let existing = run
        .iter()
        .find(|(_, current_kind)| *current_kind == kind)
        .map(|(id, _)| *id);
    if let Some(id) = existing {
        let (node_value, _) = node_mut(document, scene, id)?;
        let NodeKind::State { operation: current } = &mut node_value.data else {
            unreachable!();
        };
        *current = operation;
        changed.insert(id, location);
        return Ok(());
    }
    let insert_at = match kind {
        0 => run_start,
        1 => run
            .iter()
            .filter(|(_, current_kind)| *current_kind == 0)
            .map(|(id, _)| id)
            .filter_map(|id| {
                let parent_node = node(document, location, parent)?;
                let NodeKind::Scope { children, .. } = &parent_node.data else {
                    return None;
                };
                children
                    .iter()
                    .position(|child| child == id)
                    .map(|index| index + 1)
            })
            .max()
            .unwrap_or(object_index),
        _ => object_index,
    };
    let state = Node::new("Local Binding", NodeKind::State { operation });
    let id = state.id;
    let (group, _) = node_mut(document, scene, parent)?;
    let NodeKind::Scope { children, .. } = &mut group.data else {
        return Err(invalid("binding parent is not a group"));
    };
    children.insert(insert_at, id);
    Arc::make_mut(
        document
            .scenes
            .get_mut(&scene)
            .ok_or_else(|| invalid("missing active scene"))?,
    )
    .nodes
    .insert(id, state.clone());
    created.push((location, state));
    Ok(())
}
/// Delete the binding state of the cleared kind sitting directly before the
/// object in its dedicated group:
/// removing, not rewriting to `Material(Default)` / empty media, so the outer
/// binding applies again and export stays possible. A state of that kind that
/// is not there simply has nothing to do.
///
/// `before` is the document as it was at the start of this apply; Undo
/// re-inserts the state from it, so edits earlier in the same transaction
/// roll back with the removal. A state created earlier in the same apply
/// never existed before it and is not recorded (its `created_nodes` entry is
/// dropped at delta construction).
/// targets so far are shapes; groups take their
/// own "head of group" rules later.
fn require_shape(
    document: &ProjectDocument,
    scene: SceneId,
    object: NodeId,
    message: &'static str,
) -> Result<()> {
    let is_shape = node(&document, NodeLocation::Scene(scene), object)
        .map(|node| {
            matches!(
                &node.data,
                NodeKind::Entity {
                    entity: EntityKind::Shape,
                    ..
                }
            )
        })
        .unwrap_or(false)
        && document
            .scene(scene)
            .is_some_and(|scene| scene.nodes.contains_key(&object));
    if !is_shape {
        return Err(invalid(message));
    }
    Ok(())
}

/// Whether `parent` is the object's dedicated group.
fn is_dedicated_group(
    document: &ProjectDocument,
    location: impl Into<NodeLocation>,
    world: NodeId,
    parent: NodeId,
    object: NodeId,
) -> bool {
    let location = location.into();
    parent != world
        && matches!(
                   node(document, location, parent).map(|node| &node.data),
        // The explicit owner relationship distinguishes an
        // internal wrapper from an ordinary one-child Attribute Group.
                   Some(NodeKind::Scope {
                       scope: ScopeKind::Attribute,
                       owned_object: Some(owner),
                       ..
                   }) if *owner == object
               )
}

/// A scope's children, empty for anything else.
fn scope_children(document: &ProjectDocument, location: NodeLocation, id: NodeId) -> &[NodeId] {
    match node(document, location, id).map(|node| &node.data) {
        Some(NodeKind::Scope { children, .. }) => children,
        _ => &[],
    }
}

/// The transform states that make `value` a local transform, labelled like
/// imported directives. An identity local writes nothing — never a reset
/// operation. Re-parenting belongs to import, and older imports are refused
/// before this function runs.
fn local_operations(value: &LocalTransform) -> Result<Vec<(&'static str, TransformOperation)>> {
    let mut operations = Vec::new();
    match value {
        LocalTransform::Matrix(m) => {
            if m.iter().any(|v| !v.is_finite()) {
                return Err(invalid("local matrix is not finite"));
            }
            if *m != crate::math::Mat4::identity().0 {
                operations.push(("ConcatTransform", TransformOperation::Concat(*m)));
            }
        }
        LocalTransform::Trs(trs) => {
            let [x, y, z] = trs.rotation_euler_degrees;
            if trs
                .translation
                .iter()
                .chain([x, y, z].iter())
                .chain(trs.scale.iter())
                .any(|v| !v.is_finite())
            {
                return Err(invalid("local TRS is not finite"));
            }
            if trs.translation != [0.; 3] {
                operations.push(("Translate", TransformOperation::Translate(trs.translation)));
            }
            // `T · Ry · Rx · Rz · S` (transform_trs::compose).
            for (degrees, axis) in [(y, [0., 1., 0.]), (x, [1., 0., 0.]), (z, [0., 0., 1.])] {
                if degrees != 0. {
                    operations.push(("Rotate", TransformOperation::Rotate { degrees, axis }));
                }
            }
            if trs.scale != [1.; 3] {
                operations.push(("Scale", TransformOperation::Scale(trs.scale)));
            }
        }
    }
    Ok(operations)
}

/// The dedicated group, wrapping a lone object on
/// demand. Shared with the import-time `from` fold.
pub(crate) fn ensure_dedicated_group(
    document: &mut ProjectDocument,
    location: impl Into<NodeLocation>,
    world: NodeId,
    object: NodeId,
    changed: &mut BTreeMap<NodeId, NodeLocation>,
    created: &mut Vec<(NodeLocation, Node)>,
) -> Result<NodeId> {
    let location = location.into();
    let (parent, index) = find_parent(document, location, object)?;
    if is_dedicated_group(document, location, world, parent, object) {
        return Ok(parent);
    }
    let label = node(document, location, object)
        .map(|node| node.label.clone())
        .unwrap_or_else(|| "Object".into());
    let wrapper = Node::new(
        label,
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children: vec![object],
            owned_object: Some(object),
            parent_camera: None,
        },
    );
    let wrapper_id = wrapper.id;
    let (parent_node, location) = node_mut(document, location.scene(), parent)?;
    let NodeKind::Scope { children, .. } = &mut parent_node.data else {
        return Err(invalid("object parent is not a group"));
    };
    children[index] = wrapper_id;
    changed.insert(parent, location);
    insert_node(document, location, wrapper.clone())?;
    created.push((location, wrapper));
    Ok(wrapper_id)
}

/// Where a new component node goes: the head of the run of area light
/// entities and binding/toggle states immediately before the object.
fn component_run_start(
    document: &ProjectDocument,
    scene: SceneId,
    group: NodeId,
    object_index: usize,
) -> usize {
    let location = NodeLocation::Scene(scene);
    let mut start = object_index;
    while start > 0 {
        let keeps = node(document, location, group)
            .map(|group| &group.data)
            .and_then(|data| match data {
                NodeKind::Scope { children, .. } => Some(children[start - 1]),
                _ => None,
            })
            .and_then(|child| node(document, location, child))
            .map(|child| match &child.data {
                NodeKind::State { operation } => matches!(
                    operation,
                    StateOperation::Material(_)
                        | StateOperation::Medium { .. }
                        | StateOperation::ReverseOrientation
                ),
                NodeKind::Entity {
                    entity: EntityKind::AreaLight,
                    ..
                } => true,
                _ => false,
            })
            .unwrap_or(false);
        if keeps {
            start -= 1;
        } else {
            break;
        }
    }
    start
}

/// The reverse-orientation toggles in the run immediately before the
/// object: they belong to the object only via its dedicated group.
fn run_toggles(
    document: &ProjectDocument,
    scene: SceneId,
    group: NodeId,
    object_index: usize,
) -> Vec<NodeId> {
    let location = NodeLocation::Scene(scene);
    let Some(group) = node(document, location, group) else {
        return Vec::new();
    };
    let NodeKind::Scope { children, .. } = &group.data else {
        return Vec::new();
    };
    children[component_run_start(document, scene, group.id, object_index)..object_index]
        .iter()
        .filter(|child| {
            matches!(
                node(document, location, **child).map(|node| &node.data),
                Some(NodeKind::State {
                    operation: StateOperation::ReverseOrientation,
                    ..
                })
            )
        })
        .copied()
        .collect()
}

/// Put one freshly built node at `index` in a scene group's children,
/// recording it for undo and change reporting. Shared with the
/// import-time `from` fold.
pub(crate) fn insert_child_node(
    document: &mut ProjectDocument,
    location: impl Into<NodeLocation>,
    group: NodeId,
    index: usize,
    node: Node,
    created: &mut Vec<(NodeLocation, Node)>,
    changed: &mut BTreeMap<NodeId, NodeLocation>,
) -> Result<()> {
    let location = location.into();
    let id = node.id;
    let (group_node, _) = node_mut(document, location.scene(), group)?;
    let NodeKind::Scope { children, .. } = &mut group_node.data else {
        return Err(invalid("component parent is not a group"));
    };
    children.insert(index, id);
    insert_node(document, location, node.clone())?;
    created.push((location, node));
    changed.insert(group, location);
    Ok(())
}

/// Drop one node from a scene group's children and the node table,
/// recording the pre-edit content for undo.
fn remove_child_node(
    document: &mut ProjectDocument,
    location: impl Into<NodeLocation>,
    group: NodeId,
    id: NodeId,
    before: &ProjectDocument,
    changed: &mut BTreeMap<NodeId, NodeLocation>,
    deleted: &mut Vec<(NodeLocation, Node)>,
) -> Result<()> {
    let location = location.into();
    nodes_mut(document, location)?
        .remove(&id)
        .ok_or_else(|| invalid("missing removed component node"))?;
    let (group_node, _) = node_mut(document, location.scene(), group)?;
    let NodeKind::Scope { children, .. } = &mut group_node.data else {
        return Ok(());
    };
    children.retain(|child| *child != id);
    changed.insert(group, location);
    if let Some(removed) = node(before, location, id) {
        deleted.push((location, removed.clone()));
    }
    Ok(())
}

/// the node as a group component target — an Attribute scope except
/// the World. `None` for shapes, cameras, and everything else.
fn attribute_group(
    document: &ProjectDocument,
    scene: SceneId,
    world: NodeId,
    id: NodeId,
) -> Option<NodeId> {
    (id != world
        && matches!(
            node(document, NodeLocation::Scene(scene), id).map(|node| &node.data),
            Some(NodeKind::Scope {
                scope: ScopeKind::Attribute,
                ..
            })
        ))
    .then_some(id)
}

/// The component nodes leading a group's children: states and
/// area light entities before the group's first other content.
fn group_head_ids(
    document: &ProjectDocument,
    scene: SceneId,
    group: NodeId,
) -> (Vec<NodeId>, usize) {
    let location = NodeLocation::Scene(scene);
    let Some(group) = node(document, location, group) else {
        return (Vec::new(), 0);
    };
    let NodeKind::Scope { children, .. } = &group.data else {
        return (Vec::new(), 0);
    };
    let mut end = 0;
    while end < children.len() {
        let keeps = node(document, location, children[end])
            .map(|child| match &child.data {
                NodeKind::State { operation } => matches!(
                    operation,
                    StateOperation::Material(_)
                        | StateOperation::Medium { .. }
                        | StateOperation::ReverseOrientation
                ),
                NodeKind::Entity {
                    entity: EntityKind::AreaLight,
                    ..
                } => true,
                _ => false,
            })
            .unwrap_or(false);
        if keeps {
            end += 1;
        } else {
            break;
        }
    }
    (children[..end].to_vec(), end)
}

/// The area light entities in the leading run of a group's children.
fn group_head_area_lights(
    document: &ProjectDocument,
    scene: SceneId,
    group: NodeId,
) -> Vec<NodeId> {
    group_head_ids(document, scene, group)
        .0
        .iter()
        .filter(|child| {
            matches!(
                node(document, NodeLocation::Scene(scene), **child).map(|node| &node.data),
                Some(NodeKind::Entity {
                    entity: EntityKind::AreaLight,
                    ..
                })
            )
        })
        .copied()
        .collect()
}

/// Where in the leading run a binding state goes: after the area light
/// entity for materials, after it and any material state for media.
fn group_head_insert_at(
    document: &ProjectDocument,
    scene: SceneId,
    group: NodeId,
    kind: u8,
) -> usize {
    let (head, _) = group_head_ids(document, scene, group);
    let is_area_light = |id: &NodeId| {
        matches!(
            node(document, NodeLocation::Scene(scene), *id).map(|node| &node.data),
            Some(NodeKind::Entity {
                entity: EntityKind::AreaLight,
                ..
            })
        )
    };
    let is_material = |id: &NodeId| {
        matches!(
            node(document, NodeLocation::Scene(scene), *id).map(|node| &node.data),
            Some(NodeKind::State {
                operation: StateOperation::Material(_),
                ..
            })
        )
    };
    match kind {
        0 => head
            .iter()
            .rposition(|id| is_area_light(id))
            .map(|index| index + 1)
            .unwrap_or(0),
        1 => head
            .iter()
            .rposition(|id| is_area_light(id) || is_material(id))
            .map(|index| index + 1)
            .unwrap_or(0),
        _ => head.len(),
    }
}

/// A group's binding state: rewrite the existing one or insert one at the
/// group's head (never a wrapper for a group target).
fn set_group_head_state(
    document: &mut ProjectDocument,
    scene: SceneId,
    group: NodeId,
    operation: StateOperation,
    created: &mut Vec<(NodeLocation, Node)>,
    changed: &mut BTreeMap<NodeId, NodeLocation>,
) -> Result<()> {
    let location = NodeLocation::Scene(scene);
    let kind = operation_kind(&operation);
    if let Some(id) = group_head_ids(document, scene, group)
        .0
        .iter()
        .copied()
        .find(|child| {
            let _ = child;
            matches!(
                node(document, location, **&child).map(|node| &node.data),
                Some(NodeKind::State { operation: current, .. }) if operation_kind(current) == kind
            )
        })
    {
        let (state, _) = node_mut(document, scene, id)?;
        state.data = NodeKind::State { operation };
        changed.insert(id, location);
        return Ok(());
    }
    let insert_at = group_head_insert_at(document, scene, group, kind);
    let state = Node::new("Group Binding", NodeKind::State { operation });
    insert_child_node(document, scene, group, insert_at, state, created, changed)
}

/// Drop a binding state from a group's leading run (if the head holds one).
fn remove_group_head_state(
    document: &mut ProjectDocument,
    scene: SceneId,
    group: NodeId,
    operation: &StateOperation,
    before: &ProjectDocument,
    changed: &mut BTreeMap<NodeId, NodeLocation>,
    deleted: &mut Vec<(NodeLocation, Node)>,
) -> Result<()> {
    let location = NodeLocation::Scene(scene);
    let kind = operation_kind(operation);
    let Some(id) = group_head_ids(document, scene, group)
        .0
        .iter()
        .copied()
        .find(|child| {
            matches!(
                node(document, location, **&child).map(|node| &node.data),
                Some(NodeKind::State { operation: current, .. }) if operation_kind(current) == kind
            )
        })
    else {
        return Ok(());
    };
    remove_child_node(document, scene, group, id, before, changed, deleted)
}

/// A group's area light: rewrite the leading entity, or lead with a new one.
fn set_group_head_area_light(
    document: &mut ProjectDocument,
    scene: SceneId,
    group: NodeId,
    content: &AreaLightContent,
    created: &mut Vec<(NodeLocation, Node)>,
    changed: &mut BTreeMap<NodeId, NodeLocation>,
) -> Result<()> {
    if let Some(id) = group_head_area_lights(document, scene, group).pop() {
        let (entity, _) = node_mut(document, scene, id)?;
        entity.data = NodeKind::Entity {
            entity: EntityKind::AreaLight,
            implementation: content.implementation.clone(),
            parameters: content.parameters.clone(),
            pose: None,
        };
        changed.insert(id, NodeLocation::Scene(scene));
        return Ok(());
    }
    let entity = Node::new(
        "Group Area Light",
        NodeKind::Entity {
            entity: EntityKind::AreaLight,
            implementation: content.implementation.clone(),
            parameters: content.parameters.clone(),
            pose: None,
        },
    );
    insert_child_node(document, scene, group, 0, entity, created, changed)
}

/// Drop a group's leading area light; cancel an inherited one explicitly so
/// the group's descendants go light-less while the siblings outside keep
/// theirs.
fn remove_group_head_area_light(
    document: &mut ProjectDocument,
    scene: SceneId,
    group: NodeId,
    before: &ProjectDocument,
    changed: &mut BTreeMap<NodeId, NodeLocation>,
    deleted: &mut Vec<(NodeLocation, Node)>,
    created: &mut Vec<(NodeLocation, Node)>,
) -> Result<()> {
    if let Some(id) = group_head_area_lights(document, scene, group).pop() {
        return remove_child_node(document, scene, group, id, before, changed, deleted);
    }
    // Nothing of its own: an effective light from an outer scope takes an
    // explicit cancellation at the head.
    let effective = {
        let view = document.scene_view(scene)?;
        let evaluated = crate::evaluate(&view)?;
        let first =
            node(document, NodeLocation::Scene(scene), group).and_then(|group| match &group.data {
                NodeKind::Scope { children, .. } => children.first().copied(),
                _ => None,
            });
        first
            .and_then(|first| evaluated.nodes.get(&first))
            .and_then(|state| state.area_light)
    };
    if effective.is_none() {
        return Ok(()); // nothing to cancel either
    }
    let cancel = Node::new(
        "No Area Light",
        NodeKind::Entity {
            entity: EntityKind::AreaLight,
            implementation: String::new(),
            parameters: Vec::new(),
            pose: None,
        },
    );
    // The cancel node must land in the delta's created set, or undo leaves
    // it as an unreachable scene node and validation fails the session.
    insert_child_node(document, scene, group, 0, cancel, created, changed)
}

/// A camera's medium: the state sits in the options list right before the
/// camera. Clearing removes that state; setting rewrites it in place or
/// inserts it.
fn set_camera_medium(
    document: &mut ProjectDocument,
    scene: SceneId,
    camera: NodeId,
    operation: StateOperation,
    before: &ProjectDocument,
    created: &mut Vec<(NodeLocation, Node)>,
    changed: &mut BTreeMap<NodeId, NodeLocation>,
    deleted: &mut Vec<(NodeLocation, Node)>,
) -> Result<()> {
    let location = NodeLocation::Scene(scene);
    let is_clear = matches!(
        operation,
        StateOperation::Medium {
            inside: None,
            outside: None,
        }
    );
    let (existing, camera_index) = {
        let scene_document = document
            .scenes
            .get(&scene)
            .ok_or_else(|| invalid("missing active scene"))?;
        let options = &scene_document.options;
        let Some(index) = options.iter().position(|id| *id == camera) else {
            return Err(invalid("missing camera in the options list"));
        };
        let existing = index
            .checked_sub(1)
            .and_then(|previous| options.get(previous).copied())
            .filter(|id| {
                matches!(
                    node(document, location, *id).map(|node| &node.data),
                    Some(NodeKind::State {
                        operation: StateOperation::Medium { .. },
                    })
                )
            });
        (existing, index)
    };
    if let Some(id) = existing {
        if is_clear {
            let scene_document = Arc::make_mut(
                document
                    .scenes
                    .get_mut(&scene)
                    .ok_or_else(|| invalid("missing active scene"))?,
            );
            scene_document.options.remove(camera_index - 1);
            scene_document
                .nodes
                .remove(&id)
                .ok_or_else(|| invalid("missing cleared camera medium"))?;
            if let Some(state) = node(before, location, id) {
                deleted.push((location, state.clone()));
            }
            return Ok(());
        }
        let (state, _) = node_mut(document, scene, id)?;
        state.data = NodeKind::State { operation };
        changed.insert(id, location);
        return Ok(());
    }
    if is_clear {
        return Ok(()); // nothing bound to clear
    }
    let state = Node::new("Camera Medium", NodeKind::State { operation });
    let id = state.id;
    Arc::make_mut(
        document
            .scenes
            .get_mut(&scene)
            .ok_or_else(|| invalid("missing active scene"))?,
    )
    .options
    .insert(camera_index, id);
    created.push((location, state.clone()));
    insert_node(document, location, state)?;
    Ok(())
}

/// The area light entities in the run immediately before the object (the
/// slot list; the last one sits closest to the object).
fn run_area_lights(
    document: &ProjectDocument,
    scene: SceneId,
    parent: NodeId,
    object_index: usize,
) -> Vec<NodeId> {
    let location = NodeLocation::Scene(scene);
    let run_start = component_run_start(document, scene, parent, object_index);
    node(document, location, parent)
        .and_then(|parent| match &parent.data {
            NodeKind::Scope { children, .. } => Some(children[run_start..object_index].to_vec()),
            _ => None,
        })
        .unwrap_or_default()
        .iter()
        .copied()
        .filter(|child| {
            matches!(
                node(document, location, *child).map(|node| &node.data),
                Some(NodeKind::Entity {
                    entity: EntityKind::AreaLight,
                    ..
                })
            )
        })
        .collect()
}

fn remove_adjacent_binding(
    document: &mut ProjectDocument,
    scene: SceneId,
    parent: NodeId,
    object: NodeId,
    operation: &StateOperation,
    before: &ProjectDocument,
    changed: &mut BTreeMap<NodeId, NodeLocation>,
    deleted: &mut Vec<(NodeLocation, Node)>,
) -> Result<()> {
    let location = NodeLocation::Scene(scene);
    let object_index = child_index(document, scene, parent, object)?;
    let Some(id) = binding_run(document, scene, parent, object_index)
        .ok()
        .and_then(|(_, run)| {
            run.into_iter()
                .find(|(_, kind)| *kind == operation_kind(operation))
                .map(|(id, _)| id)
        })
    else {
        return Ok(());
    };
    Arc::make_mut(
        document
            .scenes
            .get_mut(&scene)
            .ok_or_else(|| invalid("missing active scene"))?,
    )
    .nodes
    .remove(&id)
    .ok_or_else(|| invalid("missing cleared binding state"))?;
    let (group, _) = node_mut(document, scene, parent)?;
    let NodeKind::Scope { children, .. } = &mut group.data else {
        return Ok(());
    };
    children.retain(|child| *child != id);
    changed.insert(parent, location);
    if let Some(state) = node(before, location, id) {
        deleted.push((location, state.clone()));
    }
    Ok(())
}

/// Move the object back where its dedicated group sits and delete the group
/// once clearing has left the group holding nothing but the object
///. Groups that keep transforms or other
/// content — transforms, an AreaLight entity, other states — still
/// hold more children and stay. The group is recorded as it was at the start
/// of this apply, so both the removals above and the unwrap restore together
/// as one undo step; a group created earlier in the same apply is not
/// recorded (undo reverts the whole transaction anyway).
fn unwrap_emptied_group(
    document: &mut ProjectDocument,
    location: impl Into<NodeLocation>,
    parent: NodeId,
    object: NodeId,
    before: &ProjectDocument,
    changed: &mut BTreeMap<NodeId, NodeLocation>,
    deleted: &mut Vec<(NodeLocation, Node)>,
) -> Result<()> {
    let location = location.into();
    let children_empty = node(document, location, parent).is_some_and(|group| {
        matches!(
                   &group.data,
                   NodeKind::Scope {
                       children,
                       owned_object: Some(owner),
        // The wrapper is the camera attachment; it stays.
                       parent_camera: None,
                       ..
                   } if children.len() == 1 && children[0] == object && *owner == object
               )
    });
    if !children_empty {
        return Ok(());
    }
    let (grandparent, index) = find_parent(document, location, parent)?;
    let (grandparent_node, _) = node_mut(document, location.scene(), grandparent)?;
    let NodeKind::Scope { children, .. } = &mut grandparent_node.data else {
        return Err(invalid("object parent is not a group"));
    };
    children[index] = object;
    changed.insert(grandparent, location);
    nodes_mut(document, location)?
        .remove(&parent)
        .ok_or_else(|| invalid("missing cleared binding group"))?;
    if let Some(group) = node(before, location, parent) {
        deleted.push((location, group.clone()));
    }
    Ok(())
}
fn binding_run(
    document: &ProjectDocument,
    scene: SceneId,
    parent: NodeId,
    object_index: usize,
) -> Result<(usize, Vec<(NodeId, u8)>)> {
    let location = NodeLocation::Scene(scene);
    let parent =
        node(document, location, parent).ok_or_else(|| invalid("missing binding group"))?;
    let NodeKind::Scope { children, .. } = &parent.data else {
        return Err(invalid("binding parent is not a group"));
    };
    let mut start = object_index;
    let mut run = Vec::new();
    while start > 0 {
        let id = children[start - 1];
        let kind = node(document, location, id).and_then(|node| match &node.data {
            NodeKind::State { operation } => Some(operation_kind(operation)),
            _ => None,
        });
        let Some(kind @ (0 | 1)) = kind else { break };
        start -= 1;
        run.push((id, kind));
    }
    Ok((start, run))
}
fn operation_kind(operation: &StateOperation) -> u8 {
    match operation {
        StateOperation::Material(_) => 0,
        StateOperation::Medium { .. } => 1,
        _ => 2,
    }
}
fn changes(delta: &Delta, revision: u64, forward: bool) -> ChangeSet {
    ChangeSet {
        revision: Revision::from_u64(revision),
        nodes: delta.nodes.iter().map(|(_, id, _, _)| *id).collect(),
        resources: delta.resources.iter().map(|(id, _, _)| *id).collect(),
        created: if forward {
            delta
                .created_nodes
                .iter()
                .map(|(_, node)| node.id)
                .collect()
        } else {
            delta
                .deleted_nodes
                .iter()
                .map(|(_, node)| node.id)
                .collect()
        },
        created_items: delta
            .scenes
            .iter()
            .filter_map(|(id, before, after)| {
                let (source, target) = if forward {
                    (before, after)
                } else {
                    (after, before)
                };
                (source.is_none() && target.is_some()).then_some(ItemId::Scene(*id))
            })
            .chain(delta.resources.iter().filter_map(|(id, before, after)| {
                let (source, target) = if forward {
                    (before, after)
                } else {
                    (after, before)
                };
                (source.is_none() && target.is_some()).then_some(ItemId::Resource(*id))
            }))
            .chain(delta.assets.iter().filter_map(|(id, before, after)| {
                let (source, target) = if forward {
                    (before, after)
                } else {
                    (after, before)
                };
                (source.is_none() && target.is_some()).then_some(ItemId::Asset(*id))
            }))
            .collect(),
    }
}
fn bump(revision: &Revision) -> Result<Revision> {
    crate::validation::revision(revision)?
        .checked_add(1)
        .map(Revision::from_u64)
        .ok_or_else(|| invalid("record edition overflow"))
}
