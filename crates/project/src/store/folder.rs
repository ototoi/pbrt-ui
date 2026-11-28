use super::*;
mod meta_codec;

const FOLDER_FORMAT_V3: &str = "pbrt-ui.project/3";
const FOLDER_FORMAT: &str = "pbrt-ui.project/2";
const FOLDER_MANIFEST: &str = "project.pbrtui.json";
const FOLDER_META_FORMAT: &str = "pbrt-ui.meta/1";
/// The scene-file format this store writes. `owned_object` needs /3+,
/// `coordinate_system` needs /4; older files stay readable and infer it.
const FOLDER_SCENE_FORMAT: &str = "pbrt-ui.scene/4";
const LEGACY_SCENE_FORMAT: &str = "pbrt-ui.scene/3";
const FOLDER_RESOURCE_FORMAT: &str = "pbrt-ui.resource/2";
const MAX_FOLDER_JSON_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FolderManifest {
    format: String,
    project_id: ProjectId,
    title: String,
    default_scene: Option<SceneId>,
    importer: Option<FolderImporter>,
    saved_revision: Revision,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FolderImporter {
    version: String,
    r4_revision: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct AssetMeta {
    format: String,
    pub id: uuid::Uuid,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    origin: Option<AssetMetaOrigin>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AssetMetaOrigin {
    pub(crate) import: ImportId,
    pub(crate) logical_path: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneFile {
    format: String,
    label: String,
    options: Vec<NodeId>,
    #[serde(default, deserialize_with = "deserialize_camera_field")]
    render_camera: Option<Option<NodeId>>,
    /// The scene's up/handedness metadata. Format 4 always
    /// writes it; older formats infer it from their cameras on read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    coordinate_system: Option<CoordinateSystem>,
    world: NodeId,
    nodes: Vec<Node>,
}

fn deserialize_camera_field<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<Option<NodeId>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<NodeId>::deserialize(deserializer).map(Some)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceFile {
    format: String,
    kind: ResourceKind,
    label: String,
    edition: Revision,
    texture_value_type: Option<TextureValueType>,
    implementation: String,
    parameters: Vec<Parameter>,
    declared_in: Option<DeclarationState>,
    origin: Option<Origin>,
    nodes: Option<Vec<Node>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportFile {
    format: String,
    id: ImportId,
    importer_version: String,
    dialect: ImportDialect,
    renderer_revision: Option<String>,
    diagnostics: Vec<ImportDiagnostic>,
    root: OccurrenceId,
    sources: BTreeMap<SourceId, ImportedSource>,
    occurrences: BTreeMap<OccurrenceId, SourceOccurrence>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveJournal {
    files: Vec<JournalFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    asset_folders: Option<std::collections::BTreeSet<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalFile {
    path: PathBuf,
    temp: Option<PathBuf>,
    backup: Option<PathBuf>,
    delete: bool,
    sha256: Option<String>,
}

/// Handle for the format-2 project directory. The lock is held for its life.
pub struct FolderStore {
    directory: PathBuf,
    _lock: File,
    manifest: FolderManifest,
    baseline: BTreeMap<PathBuf, FileStamp>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileStamp {
    size: u64,
    modified_ns: u128,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderScan {
    pub files: Vec<PathBuf>,
    pub missing_meta: Vec<PathBuf>,
    pub orphan_meta: Vec<PathBuf>,
    pub duplicate_ids: Vec<(uuid::Uuid, PathBuf)>,
}

/// Test-only counters for the asset-folder work an import performs: folder
/// scans, sidecar reads and baseline walks. Regression tests assert these
/// stay linear so an N-asset import never degrades to N² file reads again.
#[cfg(test)]
mod import_metrics {
    use std::cell::Cell;
    thread_local! {
        static ASSET_SCANS: Cell<usize> = const { Cell::new(0) };
        static META_READS: Cell<usize> = const { Cell::new(0) };
        static FOLDER_WALKS: Cell<usize> = const { Cell::new(0) };
    }
    pub(super) fn reset() {
        ASSET_SCANS.with(|count| count.set(0));
        META_READS.with(|count| count.set(0));
        FOLDER_WALKS.with(|count| count.set(0));
    }
    pub(super) fn asset_scans() -> usize {
        ASSET_SCANS.with(Cell::get)
    }
    pub(super) fn meta_reads() -> usize {
        META_READS.with(Cell::get)
    }
    pub(super) fn folder_walks() -> usize {
        FOLDER_WALKS.with(Cell::get)
    }
    pub(super) fn note_asset_scan() {
        ASSET_SCANS.with(|count| count.set(count.get() + 1));
    }
    pub(super) fn note_meta_read() {
        META_READS.with(|count| count.set(count.get() + 1));
    }
    pub(super) fn note_folder_walk() {
        FOLDER_WALKS.with(|count| count.set(count.get() + 1));
    }
}

impl FolderStore {
    pub fn create(
        path: &Path,
        project_id: ProjectId,
        title: &str,
        revision: Revision,
    ) -> Result<Self> {
        // A destination may already exist as an empty folder — it is as
        // treat it as absent; `project.create` accepts an empty store.
        match fs::create_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = fs::metadata(path)?;
                if !metadata.is_dir() || fs::read_dir(path)?.next().is_some() {
                    return Err(invalid("project folders must be new or empty"));
                }
            }
            Err(error) => return Err(error.into()),
        }
        let directory = protocol::path::canonicalize(path)?;
        fs::create_dir(directory.join("Assets"))?;
        fs::create_dir(directory.join("Imports"))?;
        fs::create_dir(directory.join("Library"))?;
        cache::ensure_cache(&directory.join("Library/Cache"))?;
        fs::create_dir(directory.join(".pbrtui"))?;
        let lock = lock_folder(&directory)?;
        let manifest = FolderManifest {
            format: FOLDER_FORMAT.to_owned(),
            project_id,
            title: title.to_owned(),
            default_scene: None,
            importer: None,
            saved_revision: revision,
        };
        write_json_atomic(&directory.join(FOLDER_MANIFEST), &manifest)?;
        write_gitignore(&directory)?;
        let baseline = collect_file_stamps(&directory)?;
        Ok(Self {
            directory,
            _lock: lock,
            manifest,
            baseline,
        })
    }

    /// Create a format-3 folder. The lower-level create API remains format 2
    /// for the legacy import adapter and compatibility fixtures.
    pub fn create_v3(
        path: &Path,
        project_id: ProjectId,
        title: &str,
        revision: Revision,
    ) -> Result<Self> {
        let mut store = Self::create(path, project_id, title, revision)?;
        store.manifest.format = FOLDER_FORMAT_V3.into();
        write_json_atomic(&store.directory.join(FOLDER_MANIFEST), &store.manifest)?;
        store.baseline = collect_file_stamps(&store.directory)?;
        Ok(store)
    }

    pub fn is_v3(&self) -> bool {
        self.manifest.format == FOLDER_FORMAT_V3
    }

    pub fn open(path: &Path) -> Result<Self> {
        let directory = protocol::path::canonicalize(path)?;
        regular_directory(&directory)?;
        let lock = lock_folder(&directory)?;
        for name in ["Assets", "Imports", ".pbrtui"] {
            regular_directory(&directory.join(name))?;
        }
        // `Library/` is a regenerable cache: recreate
        // it when the user deleted it instead of refusing to open.
        cache::ensure_cache(&directory.join("Library/Cache"))?;
        recover_journal(&directory)?;
        let bytes = read_bounded(&directory.join(FOLDER_MANIFEST), MAX_RECORD_BYTES)?;
        let manifest: FolderManifest = serde_json::from_slice(&bytes)?;
        if manifest.format != FOLDER_FORMAT && manifest.format != FOLDER_FORMAT_V3 {
            let version = manifest
                .format
                .strip_prefix("pbrt-ui.project/")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            return Err(ProjectError::UnsupportedVersion(version));
        }
        crate::validation::revision(&manifest.saved_revision)?;
        let baseline = collect_file_stamps(&directory)?;
        Ok(Self {
            directory,
            _lock: lock,
            manifest,
            baseline,
        })
    }

    pub fn open_project(path: &Path) -> Result<(Self, ProjectSession)> {
        let store = Self::open(path)?;
        let mut document = store.read_document()?;
        // legacy spd assets become SpectrumData resources while the
        // project is being read; a save persists that, its file pruning
        // removing what no longer has a reference. The migration is not a
        // format change, so a converted project reads one revision ahead of
        // what the manifest says: the next save publishes it, an unsaved
        // close leaves the disk as it was and converts the same way again.
        let directory = store.directory.clone();
        let converted =
            crate::spectrum_data::convert_legacy_spd_references(&mut document, |asset| {
                let path = safe_asset_location(&asset.location)
                    .map(|relative| directory.join("Assets").join(relative))?;
                if std::fs::metadata(&path)?.len() > crate::MAX_SPD_FILE_BYTES {
                    return Err(invalid(format!(
                        "{} bytes exceeds the spectrum file limit",
                        asset.size
                    )));
                }
                Ok(std::fs::read(&path)?)
            });
        document.validate()?;
        let revision = crate::validation::revision(&store.manifest.saved_revision)?;
        let revision = revision + u64::from(converted);
        Ok((store, ProjectSession::loaded(document, revision)?))
    }

    pub fn read_document(&self) -> Result<ProjectDocument> {
        let scan = self.scan_assets()?;
        if self.is_v3() && !scan.duplicate_ids.is_empty() {
            return Err(invalid("duplicate format-3 sidecar identity"));
        }
        let mut declared_ids = std::collections::BTreeSet::new();
        for relative in scan
            .files
            .iter()
            .map(|path| meta_path(path))
            .chain(scan.orphan_meta.iter().cloned())
        {
            let path = self.directory.join("Assets").join(relative);
            if path.is_file() {
                let meta: AssetMeta = serde_json::from_slice(&read_bounded(&path, 1024 * 1024)?)?;
                let expected = if self.is_v3() {
                    "pbrt-ui.meta/2"
                } else {
                    FOLDER_META_FORMAT
                };
                if meta.format != expected {
                    return Err(invalid("project and sidecar versions do not match"));
                }
                if self.is_v3() {
                    if !declared_ids.insert(meta.id) {
                        return Err(invalid("duplicate format-3 sidecar identity"));
                    }
                }
            }
        }
        let mut scenes = BTreeMap::new();
        let mut resources = BTreeMap::new();
        let mut definition_nodes = BTreeMap::new();
        let mut assets = BTreeMap::new();
        let mut imports = BTreeMap::new();
        let mut ids = std::collections::BTreeSet::new();
        for relative in scan.files {
            let path = self.directory.join("Assets").join(&relative);
            let meta_path = PathBuf::from(format!("{}.meta", path.display()));
            if !meta_path.is_file() {
                continue;
            }
            let meta: AssetMeta = serde_json::from_slice(&read_bounded(&meta_path, 1024 * 1024)?)?;
            if !ids.insert(meta.id) {
                continue;
            }
            match meta.kind.as_str() {
                "scene" => {
                    let bytes = read_bounded(&path, MAX_FOLDER_JSON_BYTES)?;
                    let raw: serde_json::Value = serde_json::from_slice(&bytes)?;
                    if !matches!(
                        raw["format"].as_str(),
                        Some(FOLDER_SCENE_FORMAT) | Some(LEGACY_SCENE_FORMAT)
                    ) && contains_owned_object_field(&raw)
                    {
                        return Err(invalid("owned_object requires pbrt-ui.scene/3"));
                    }
                    let file: SceneFile = serde_json::from_slice(&bytes)?;
                    if !matches!(
                        file.format.as_str(),
                        FOLDER_SCENE_FORMAT
                            | LEGACY_SCENE_FORMAT
                            | "pbrt-ui.scene/2"
                            | "pbrt-ui.scene/1"
                    ) {
                        return Err(invalid("unrecognized scene file format"));
                    }
                    if !matches!(
                        file.format.as_str(),
                        FOLDER_SCENE_FORMAT | LEGACY_SCENE_FORMAT
                    ) && file.nodes.iter().any(node_has_owned_object)
                    {
                        return Err(invalid("owned_object requires pbrt-ui.scene/3"));
                    }
                    if path.extension().and_then(|ext| ext.to_str()) != Some("scene") {
                        return Err(invalid("scene file must use the .scene extension"));
                    }
                    let nodes = nodes_map(file.nodes)?;
                    let id = SceneId(meta.id);
                    let render_camera = match file.render_camera {
                        Some(camera) => camera,
                        None if file.format == "pbrt-ui.scene/1" => {
                            file.options.iter().rev().copied().find(|id| {
                                matches!(
                                    nodes.get(id).map(|node| &node.data),
                                    Some(NodeKind::Entity {
                                        entity: EntityKind::Camera,
                                        ..
                                    })
                                )
                            })
                        }
                        None if file.format == "pbrt-ui.scene/2"
                            || file.format == LEGACY_SCENE_FORMAT
                            || file.format == FOLDER_SCENE_FORMAT =>
                        {
                            return Err(invalid(format!(
                                "{} is missing its render_camera field",
                                file.format
                            )));
                        }
                        None => return Err(invalid("unrecognized scene file format")),
                    };
                    let coordinate_system = match file.coordinate_system {
                        Some(system) => system,
                        None if file.format == FOLDER_SCENE_FORMAT => {
                            return Err(invalid(format!(
                                "{} is missing its coordinate_system field",
                                file.format
                            )));
                        }
                        // Formats 1–3 predate the field: infer it from the
                        // cameras as a fresh PBRT import would.
                        None => {
                            coordinate::infer_cameras(&file.options, &nodes, render_camera)
                                .coordinate_system
                        }
                    };
                    scenes.insert(
                        id,
                        std::sync::Arc::new(SceneDocument {
                            id,
                            label: file.label,
                            location: relative.to_string_lossy().replace('\\', "/"),
                            options: file.options,
                            render_camera,
                            coordinate_system,
                            world: file.world,
                            nodes,
                        }),
                    );
                }
                "resource" => {
                    let bytes = read_bounded(&path, MAX_FOLDER_JSON_BYTES)?;
                    let raw: serde_json::Value = serde_json::from_slice(&bytes)?;
                    if raw["format"].as_str() != Some(FOLDER_RESOURCE_FORMAT)
                        && contains_owned_object_field(&raw)
                    {
                        return Err(invalid("owned_object requires pbrt-ui.resource/2"));
                    }
                    let file: ResourceFile = serde_json::from_slice(&bytes)?;
                    if file.format != FOLDER_RESOURCE_FORMAT && file.format != "pbrt-ui.resource/1"
                    {
                        return Err(invalid("unrecognized resource file format"));
                    }
                    if file.format != FOLDER_RESOURCE_FORMAT
                        && file
                            .nodes
                            .as_deref()
                            .is_some_and(|nodes| nodes.iter().any(node_has_owned_object))
                    {
                        return Err(invalid("owned_object requires pbrt-ui.resource/2"));
                    }
                    let expected_extension = match file.kind {
                        ResourceKind::Material => "material",
                        ResourceKind::Texture => "texture",
                        ResourceKind::Medium => "medium",
                        ResourceKind::ObjectDefinition => "object",
                        ResourceKind::SpectrumData => "spectrum",
                    };
                    if path.extension().and_then(|ext| ext.to_str()) != Some(expected_extension) {
                        return Err(invalid("resource extension does not match its kind"));
                    }
                    let id = ResourceId(meta.id);
                    let root = match (&file.kind, &file.nodes) {
                        (ResourceKind::ObjectDefinition, Some(nodes)) if !nodes.is_empty() => {
                            Some(nodes[0].id)
                        }
                        (ResourceKind::ObjectDefinition, _) => {
                            return Err(invalid("object resource is missing definition nodes"));
                        }
                        (_, Some(_)) => {
                            return Err(invalid("non-object resource contains definition nodes"));
                        }
                        (_, None) => None,
                    };
                    if let Some(nodes) = file.nodes {
                        for node in nodes {
                            if definition_nodes.insert(node.id, node).is_some() {
                                return Err(invalid("duplicate object-definition node id"));
                            }
                        }
                    }
                    resources.insert(
                        id,
                        Resource {
                            id,
                            label: file.label,
                            location: relative.to_string_lossy().replace('\\', "/"),
                            edition: file.edition,
                            kind: file.kind,
                            texture_value_type: file.texture_value_type,
                            implementation: file.implementation,
                            parameters: file.parameters,
                            definition_scope: root,
                            declared_in: file.declared_in,
                            origin: file.origin,
                        },
                    );
                }
                "asset" => {
                    if meta.sha256.is_none() || meta.size.is_none() {
                        return Err(invalid("asset meta is missing its content hash or size"));
                    }
                    let mut file = File::open(&path)?;
                    let mut hasher = Sha256::new();
                    let mut size = 0u64;
                    let mut buffer = [0u8; 64 * 1024];
                    loop {
                        let count = file.read(&mut buffer)?;
                        if count == 0 {
                            break;
                        }
                        size = size
                            .checked_add(count as u64)
                            .ok_or_else(|| invalid("asset size overflow"))?;
                        if size > MAX_OBJECT_BYTES {
                            return Err(ProjectError::ResourceLimit("asset exceeds 4 GiB".into()));
                        }
                        hasher.update(&buffer[..count]);
                    }
                    // Bytes unlike the sidecar were edited outside: they are the
                    // asset's content now, and the next save rewrites the sidecar.
                    let sha256 = format!("{:x}", hasher.finalize());
                    let id = AssetId(meta.id);
                    let origin = meta.origin.map(|origin| AssetOrigin {
                        logical_path: origin.logical_path,
                        origin: None,
                    });
                    assets.insert(
                        id,
                        Asset {
                            id,
                            location: relative.to_string_lossy().replace('\\', "/"),
                            media_type: meta
                                .media_type
                                .unwrap_or_else(|| "application/octet-stream".into()),
                            sha256,
                            size,
                            origin,
                        },
                    );
                }
                _ => return Err(invalid("unknown asset meta kind")),
            }
        }
        for relative in &scan.orphan_meta {
            let meta_path = self.directory.join("Assets").join(relative);
            let meta: AssetMeta = serde_json::from_slice(&read_bounded(&meta_path, 1024 * 1024)?)?;
            if meta.kind != "asset" || ids.contains(&meta.id) {
                continue;
            }
            let (Some(sha256), Some(size)) = (meta.sha256, meta.size) else {
                return Err(invalid("missing asset meta has no content hash or size"));
            };
            let relative_text = relative.to_string_lossy();
            let body = relative_text
                .strip_suffix(".meta")
                .ok_or_else(|| invalid("orphan meta has an invalid suffix"))?;
            let id = AssetId(meta.id);
            ids.insert(meta.id);
            assets.insert(
                id,
                Asset {
                    id,
                    location: body.replace('\\', "/"),
                    media_type: meta
                        .media_type
                        .unwrap_or_else(|| "application/octet-stream".into()),
                    sha256,
                    size,
                    origin: meta.origin.map(|origin| AssetOrigin {
                        logical_path: origin.logical_path,
                        origin: None,
                    }),
                },
            );
        }
        let imports_root = self.directory.join("Imports");
        let mut import_entries =
            fs::read_dir(&imports_root)?.collect::<std::io::Result<Vec<_>>>()?;
        import_entries.sort_by_key(|entry| entry.file_name());
        for entry in import_entries {
            if !entry.file_type()?.is_dir() || entry.file_type()?.is_symlink() {
                continue;
            }
            let import_path = entry.path().join("import.json");
            let file: ImportFile =
                serde_json::from_slice(&read_bounded(&import_path, MAX_FOLDER_JSON_BYTES)?)?;
            if file.format != "pbrt-ui.import/1" {
                return Err(invalid("unrecognized import record format"));
            }
            let id = file.id;
            if entry.file_name().to_string_lossy() != id.to_string() {
                return Err(invalid("import directory does not match its record id"));
            }
            imports.insert(
                id,
                ImportRecord {
                    id,
                    importer_version: file.importer_version,
                    dialect: file.dialect,
                    renderer_revision: file.renderer_revision,
                    diagnostics: file.diagnostics,
                    root: file.root,
                    sources: file.sources,
                    occurrences: file.occurrences,
                },
            );
        }
        let title = self.manifest.title.clone();
        let mut document = ProjectDocument::new_empty(title.clone());
        document.id = self.manifest.project_id;
        document.title = title;
        document.default_scene = self
            .manifest
            .default_scene
            .filter(|id| scenes.contains_key(id));
        document.scenes = scenes;
        document.resources = resources;
        document.definition_nodes = definition_nodes;
        document.assets = assets;
        document.imports = imports;
        collect_asset_directories(
            &self.directory.join("Assets"),
            &self.directory.join("Assets"),
            &mut document.folders,
        )?;
        Ok(document)
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
    /// The regenerable cache root under `Library/Cache/`.
    pub fn cache_root(&self) -> PathBuf {
        self.directory.join("Library/Cache")
    }
    pub fn project_id(&self) -> ProjectId {
        self.manifest.project_id
    }
    pub fn default_scene(&self) -> Option<SceneId> {
        self.manifest.default_scene
    }

    pub fn is_empty_for_import(&self) -> Result<bool> {
        if self.manifest.default_scene.is_some() || self.scan_assets()?.files.len() > 0 {
            return Ok(false);
        }
        for name in ["Assets", "Imports"] {
            if fs::read_dir(self.directory.join(name))?.next().is_some() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn asset_path(&self, id: AssetId) -> Result<PathBuf> {
        asset_file_by_id(&self.directory.join("Assets"), id)?
            .map(|(path, _)| path)
            .ok_or(ProjectError::AssetFileMissing(id))
    }

    /// Add many imported assets in one pass: the duplicate-id scan runs once
    /// up front (and is extended as assets import), the external-change
    /// baseline is refreshed once at the end, and the contents of each
    /// asset are verified against the model before the file and its
    /// sidecar become visible to subsequent saves. Each asset's reader is
    /// opened by `open` just before that asset is written; `cancelled` is
    /// honoured before every asset.
    pub fn import_assets<'a, R, I, F, C>(&mut self, assets: I, open: F, cancelled: C) -> Result<()>
    where
        R: Read,
        I: IntoIterator<Item = &'a Asset>,
        F: FnMut(&'a Asset) -> Result<R>,
        C: Fn() -> bool,
    {
        self.import_assets_with_origins(
            assets.into_iter().map(|asset| (asset, None)),
            open,
            cancelled,
        )
    }

    /// `import_assets` with per-asset provenance: `origin` names the import
    /// record the migrated sidecar came from, taking precedence over the
    /// model's own origin. Crate-visible because `AssetMetaOrigin` is a
    /// persistence detail.
    pub(crate) fn import_assets_with_origins<'a, R, I, F, C>(
        &mut self,
        entries: I,
        mut open: F,
        cancelled: C,
    ) -> Result<()>
    where
        R: Read,
        I: IntoIterator<Item = (&'a Asset, Option<AssetMetaOrigin>)>,
        F: FnMut(&'a Asset) -> Result<R>,
        C: Fn() -> bool,
    {
        self.require_recovered()?;
        let (_, ids) = self.scan_assets_with_ids()?;
        let mut batch = ImportBatchState {
            ids,
            created_dirs: Vec::new(),
            written: Vec::new(),
        };
        let mut outcome = Ok(());
        for (asset, origin) in entries {
            if cancelled() {
                outcome = Err(ProjectError::Cancelled);
                break;
            }
            let result = match open(asset) {
                Ok(mut input) => self.import_one(asset, origin, &mut input, &mut batch, &cancelled),
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                outcome = Err(error);
                break;
            }
        }
        if outcome.is_err() {
            rollback_import(&self.directory, &batch);
            return outcome;
        }
        self.baseline = collect_file_stamps(&self.directory)?;
        Ok(())
    }

    /// Add one imported asset to an unpublished folder project. The contents
    /// are verified against the model before the file and its metadata become
    /// visible to subsequent saves.
    pub fn import_asset(
        &mut self,
        asset: &Asset,
        mut input: impl Read,
        cancelled: impl Fn() -> bool,
    ) -> Result<()> {
        self.require_recovered()?;
        let (_, ids) = self.scan_assets_with_ids()?;
        let mut batch = ImportBatchState {
            ids,
            created_dirs: Vec::new(),
            written: Vec::new(),
        };
        if let Err(error) = self.import_one(asset, None, &mut input, &mut batch, &cancelled) {
            rollback_import(&self.directory, &batch);
            return Err(error);
        }
        self.baseline = collect_file_stamps(&self.directory)?;
        Ok(())
    }

    /// Write one imported asset: body first, then its sidecar naming it.
    /// Everything the write touches lands in `batch.written` or
    /// `batch.created_dirs` so a later batch failure can remove it again.
    fn import_one(
        &mut self,
        asset: &Asset,
        origin: Option<AssetMetaOrigin>,
        input: &mut impl Read,
        batch: &mut ImportBatchState,
        cancelled: &impl Fn() -> bool,
    ) -> Result<()> {
        if !batch.ids.insert(asset.id.0) {
            return Err(invalid("asset id already exists in the project"));
        }
        let relative = safe_asset_location(&asset.location)?;
        let body = PathBuf::from("Assets").join(relative);
        let metadata = meta_path(&body);
        validate_project_relative(&body)?;
        validate_project_relative(&metadata)?;
        validate_existing_parent_chain(&self.directory, &body)?;
        let target = self.directory.join(&body);
        let target_meta = self.directory.join(&metadata);
        if target.exists() || target_meta.exists() {
            return Err(invalid("asset location already exists in the project"));
        }
        let parent_relative = body
            .parent()
            .ok_or_else(|| invalid("asset path has no parent"))?;
        create_directory_chain(&self.directory, parent_relative, &mut batch.created_dirs)?;
        let temporary = sibling_name(&body, "import");
        batch.written.push(temporary.clone());
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.directory.join(&temporary))?;
        let mut hash = Sha256::new();
        let mut size = 0u64;
        let mut buffer = [0; 65536];
        loop {
            if cancelled() {
                return Err(ProjectError::Cancelled);
            }
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            size = size
                .checked_add(count as u64)
                .ok_or_else(|| invalid("asset size overflow"))?;
            if size > MAX_OBJECT_BYTES {
                return Err(ProjectError::ResourceLimit("asset exceeds 4 GiB".into()));
            }
            hash.update(&buffer[..count]);
            output.write_all(&buffer[..count])?;
        }
        if size != asset.size || format!("{:x}", hash.finalize()) != asset.sha256 {
            return Err(invalid("imported asset does not match its project model"));
        }
        output.sync_all()?;
        rename_noreplace(&self.directory.join(&temporary), &target)?;
        // Registered as soon as it exists, so a failure below still rolls
        // it back.
        batch.written.push(body.clone());
        // The body must be durable — content and directory entry — before a
        // sidecar names it, so recovery never sees an asset without bytes.
        crate::sync_path(target.parent().unwrap())?;
        let meta = AssetMeta {
            format: FOLDER_META_FORMAT.to_owned(),
            id: asset.id.0,
            kind: "asset".into(),
            media_type: Some(asset.media_type.clone()),
            sha256: Some(asset.sha256.clone()),
            size: Some(asset.size),
            origin: origin.or_else(|| model_asset_origin(asset)),
        };
        // `write_json_atomic` publishes the sidecar before syncing its
        // directory: register it first (unless something else already sat
        // there) so a failed sync still rolls it back.
        if !target_meta.exists() {
            batch.written.push(metadata.clone());
        }
        if self.is_v3() {
            write_json_atomic(&target_meta, &meta.into_v2()?)?;
        } else {
            write_json_atomic(&target_meta, &meta)?;
        }
        Ok(())
    }

    /// Persist the selected scene immediately without advancing the edit revision.
    pub fn set_default_scene(&mut self, scene: impl Into<Option<SceneId>>) -> Result<bool> {
        self.require_recovered()?;
        let scene = scene.into();
        if let Some(id) = scene {
            if !self.read_document()?.scenes.contains_key(&id) {
                return Ok(false);
            }
        }
        let manifest_path = PathBuf::from(FOLDER_MANIFEST);
        let current = file_stamp(&self.directory.join(FOLDER_MANIFEST))?;
        if current != self.baseline.get(&manifest_path).copied() {
            return Err(ProjectError::SourceChanged(FOLDER_MANIFEST.into()));
        }
        let previous = self.manifest.default_scene;
        self.manifest.default_scene = scene;
        if let Err(error) = write_json_atomic(&self.directory.join(FOLDER_MANIFEST), &self.manifest)
        {
            self.manifest.default_scene = previous;
            return Err(error);
        }
        if let Some(stamp) = file_stamp(&self.directory.join(FOLDER_MANIFEST))? {
            self.baseline.insert(manifest_path, stamp);
        }
        Ok(true)
    }

    pub fn scan_assets(&self) -> Result<FolderScan> {
        Ok(self.scan_assets_with_ids()?.0)
    }

    /// `scan_assets` plus the set of sidecar ids the scan saw, so a batch
    /// import can run its duplicate check against one pass over the folder
    /// instead of one pass per imported asset.
    fn scan_assets_with_ids(&self) -> Result<(FolderScan, std::collections::BTreeSet<uuid::Uuid>)> {
        #[cfg(test)]
        import_metrics::note_asset_scan();
        let root = self.directory.join("Assets");
        let mut scan = FolderScan::default();
        let mut owners = BTreeMap::<uuid::Uuid, PathBuf>::new();
        scan_directory(&root, &root, &mut scan, &mut owners)?;
        Ok((scan, owners.into_keys().collect()))
    }

    pub fn unregistered_files(&self) -> Result<Vec<PathBuf>> {
        Ok(self.scan_assets()?.missing_meta)
    }

    /// Publish a copy beside the destination. Library and the active lock are
    /// regenerated; project assets and import provenance are preserved.
    pub fn save_as(&self, destination: &Path) -> Result<Self> {
        self.require_recovered()?;
        let parent = destination
            .parent()
            .ok_or_else(|| invalid("Save As destination has no parent"))?;
        if destination.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "Save As destination already exists",
            )
            .into());
        }
        fs::create_dir_all(parent)?;
        let temporary = tempfile::tempdir_in(&parent)?;
        let staged = temporary.path();
        for name in ["Assets", "Imports"] {
            let source = self.directory.join(name);
            if source.exists() {
                copy_tree(&source, &staged.join(name))?;
            }
        }
        fs::create_dir(staged.join(".pbrtui"))?;
        copy_ui_sidecars(&self.directory, &staged)?;
        fs::create_dir(staged.join("Library"))?;
        cache::ensure_cache(&staged.join("Library/Cache"))?;
        write_gitignore(staged)?;
        write_json_atomic(&staged.join(FOLDER_MANIFEST), &self.manifest)?;
        crate::sync_path(staged)?;
        let staged_path = temporary.keep();
        if let Err(error) = rename_noreplace(&staged_path, destination) {
            let _ = fs::remove_dir_all(&staged_path);
            return Err(error.into());
        }
        crate::sync_path(parent)?;
        Self::open(destination)
    }

    pub fn save_as_snapshot(&self, destination: &Path, snapshot: &ProjectSnapshot) -> Result<Self> {
        self.save_as_snapshot_cancellable(destination, snapshot, || false)
            .map(|(store, _)| store)
    }

    pub fn save_as_snapshot_cancellable(
        &self,
        destination: &Path,
        snapshot: &ProjectSnapshot,
        cancelled: impl Fn() -> bool,
    ) -> Result<(Self, SaveReceipt)> {
        self.require_recovered()?;
        let (parent, destination) = prepare_save_as_destination(destination)?;
        if cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let temporary = tempfile::tempdir_in(&parent)?;
        let staged = temporary.path().join("project");
        fs::create_dir(&staged)?;
        for name in ["Assets", "Imports"] {
            if cancelled() {
                return Err(ProjectError::Cancelled);
            }
            let source = self.directory.join(name);
            if source.exists() {
                copy_tree_cancellable(&source, &staged.join(name), &cancelled)?;
            }
        }
        fs::create_dir(staged.join(".pbrtui"))?;
        copy_ui_sidecars(&self.directory, &staged)?;
        fs::create_dir(staged.join("Library"))?;
        cache::ensure_cache(&staged.join("Library/Cache"))?;
        write_gitignore(&staged)?;
        write_json_atomic(&staged.join(FOLDER_MANIFEST), &self.manifest)?;
        let mut copy = Self::open(&staged)?;
        let receipt = copy.save_cancellable(snapshot, &cancelled)?;
        drop(copy);
        // Publish without the staging lock file, like save_as_from_objects:
        // a child process forked while `copy` was open can still hold that
        // file's flock until it execs, so reopening the same inode at the
        // destination would fail with `Locked`. A fresh lock file cannot be
        // shared that way.
        fs::remove_file(staged.join(".pbrtui/lock"))?;
        crate::sync_path(&staged)?;
        if cancelled() {
            return Err(ProjectError::Cancelled);
        }
        remove_empty_save_as_destination(&destination)?;
        if let Err(error) = rename_noreplace(&staged, &destination) {
            return Err(error.into());
        }
        crate::sync_path(&parent)?;
        Ok((Self::open(&destination)?, receipt))
    }

    pub fn save_as_from_objects(
        destination: &Path,
        snapshot: &ProjectSnapshot,
        source: &ObjectStore,
    ) -> Result<Self> {
        Self::save_as_from_objects_cancellable(destination, snapshot, source, || false)
            .map(|(store, _)| store)
    }

    pub fn save_as_from_objects_cancellable(
        destination: &Path,
        snapshot: &ProjectSnapshot,
        source: &ObjectStore,
        cancelled: impl Fn() -> bool,
    ) -> Result<(Self, SaveReceipt)> {
        let (parent, destination) = prepare_save_as_destination(destination)?;
        if cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let temporary = tempfile::tempdir_in(&parent)?;
        let staged = temporary.path().join("project");
        let document = snapshot.document();
        let mut store = Self::create(
            &staged,
            document.id,
            &document.title,
            snapshot.revision().clone(),
        )?;
        for asset in document.assets.values() {
            if cancelled() {
                return Err(ProjectError::Cancelled);
            }
            let relative = safe_asset_location(&asset.location)?;
            let target = staged.join("Assets").join(&relative);
            fs::create_dir_all(
                target
                    .parent()
                    .ok_or_else(|| invalid("asset path has no parent"))?,
            )?;
            let mut input = source.open_object(&asset.object())?;
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
            copy_cancellable(&mut input, &mut output, &cancelled)?;
            output.sync_all()?;
            let origin = asset.origin.as_ref().and_then(|origin| {
                origin.origin.as_ref().map(|source| AssetMetaOrigin {
                    import: source.import,
                    logical_path: origin.logical_path.clone(),
                })
            });
            write_json_atomic(
                &meta_path(&target),
                &AssetMeta {
                    format: FOLDER_META_FORMAT.to_owned(),
                    id: asset.id.0,
                    kind: "asset".into(),
                    media_type: Some(asset.media_type.clone()),
                    sha256: Some(asset.sha256.clone()),
                    size: Some(asset.size),
                    origin,
                },
            )?;
        }
        store.baseline = collect_file_stamps(&staged)?;
        let receipt = store.save_cancellable(snapshot, &cancelled)?;
        drop(store);
        fs::remove_file(staged.join(".pbrtui/lock"))?;
        crate::sync_path(&staged)?;
        if cancelled() {
            return Err(ProjectError::Cancelled);
        }
        remove_empty_save_as_destination(&destination)?;
        rename_noreplace(&staged, &destination)?;
        crate::sync_path(&parent)?;
        Ok((Self::open(&destination)?, receipt))
    }

    pub(super) fn require_recovered(&self) -> Result<()> {
        if self.directory.join(".pbrtui/save-journal.json").exists() {
            return Err(invalid(
                "save recovery is pending; reopen the project before writing",
            ));
        }
        Ok(())
    }

    pub fn save(&mut self, snapshot: &ProjectSnapshot) -> Result<SaveReceipt> {
        self.save_cancellable(snapshot, || false)
    }

    pub fn save_cancellable(
        &mut self,
        snapshot: &ProjectSnapshot,
        cancelled: impl Fn() -> bool,
    ) -> Result<SaveReceipt> {
        self.require_recovered()?;
        let current_revision = crate::validation::revision(&self.manifest.saved_revision)?;
        let incoming_revision = crate::validation::revision(snapshot.revision())?;
        let mut persisted_snapshot = snapshot.document().clone();
        for asset in persisted_snapshot.assets.values_mut() {
            if let Some(origin) = &mut asset.origin {
                origin.origin = None;
            }
        }
        // Asset bodies are checked against the model's hash instead
        // (`snapshot_plan`): a body rewritten outside was adopted by the session.
        let bodies = snapshot
            .document()
            .assets
            .values()
            .map(|asset| PathBuf::from("Assets").join(&asset.location))
            .collect::<std::collections::BTreeSet<_>>();
        for (relative, expected) in &self.baseline {
            if bodies.contains(relative) {
                continue;
            }
            if file_stamp(&self.directory.join(relative))? != Some(*expected) {
                return Err(ProjectError::SourceChanged(
                    relative.to_string_lossy().into_owned(),
                ));
            }
        }
        let same_revision_conflict = incoming_revision == current_revision
            && self.manifest.default_scene.is_some()
            && self.read_document()? != persisted_snapshot;
        if incoming_revision < current_revision || same_revision_conflict {
            return Err(ProjectError::RevisionConflict {
                expected: snapshot.revision().0.clone(),
                current: self.manifest.saved_revision.0.clone(),
            });
        }
        let (mut writes, deletes, directories) = self.snapshot_plan(snapshot)?;
        if cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let mut next_manifest = self.manifest.clone();
        next_manifest.title = snapshot.document().title.clone();
        next_manifest.default_scene = snapshot.document().default_scene;
        next_manifest.saved_revision = snapshot.revision().clone();
        writes.insert(PathBuf::from(FOLDER_MANIFEST), json_bytes(&next_manifest)?);
        self.commit_plan(
            writes,
            deletes,
            directories,
            snapshot.document().folders.clone(),
            &cancelled,
        )?;
        self.manifest = next_manifest;
        let mut baseline = collect_file_stamps(&self.directory)?;
        baseline.remove(Path::new(".pbrtui/save-journal.json"));
        // the receipt reports the content this save published — read back
        // from the store so items registered during the save (unregistered
        // files that received a `.meta`) are part of the baseline the
        // session compares against.
        let published = self.read_document()?;
        finish_journal(&self.directory)?;
        self.baseline = baseline;
        Ok(SaveReceipt {
            project_id: self.manifest.project_id,
            revision: self.manifest.saved_revision.clone(),
            generation: uuid::Uuid::new_v4(),
            content_id: ContentId::of(&published),
        })
    }

    fn commit_plan(
        &mut self,
        writes: BTreeMap<PathBuf, Vec<u8>>,
        deletes: std::collections::BTreeSet<PathBuf>,
        directories: std::collections::BTreeSet<PathBuf>,
        asset_folders: std::collections::BTreeSet<String>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<()> {
        let mut created_dirs = Vec::new();
        for directory in directories {
            validate_project_relative(&directory)?;
            create_directory_chain(&self.directory, &directory, &mut created_dirs)?;
        }
        let mut plan = Vec::new();
        for (relative, bytes) in writes {
            validate_project_relative(&relative)?;
            validate_existing_parent_chain(&self.directory, &relative)?;
            let path = self.directory.join(&relative);
            if file_equals(&path, &bytes)? {
                continue;
            }
            let current = file_stamp(&path)?;
            let expected = self.baseline.get(&relative).copied();
            if current != expected {
                cleanup_plan(&self.directory, &plan);
                cleanup_created_dirs(&created_dirs);
                return Err(ProjectError::SourceChanged(relative.display().to_string()));
            }
            let parent = path
                .parent()
                .ok_or_else(|| invalid("save path has no parent"))?;
            let parent_relative = parent
                .strip_prefix(&self.directory)
                .map_err(|_| invalid("save parent escaped project"))?;
            create_directory_chain(&self.directory, parent_relative, &mut created_dirs)?;
            let temp = sibling_name(&path, "tmp");
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            let backup = if current.is_some() {
                let backup = sibling_name(&path, "bak");
                copy_file_synced(&path, &backup)?;
                Some(
                    backup
                        .strip_prefix(&self.directory)
                        .map_err(|_| invalid("backup escaped project"))?
                        .to_path_buf(),
                )
            } else {
                None
            };
            plan.push(JournalFile {
                path: relative,
                temp: Some(
                    temp.strip_prefix(&self.directory)
                        .map_err(|_| invalid("temporary escaped project"))?
                        .to_path_buf(),
                ),
                backup,
                delete: false,
                sha256: Some(format!("{:x}", Sha256::digest(&bytes))),
            });
            crate::sync_path(parent)?;
        }
        for relative in deletes {
            validate_project_relative(&relative)?;
            validate_existing_parent_chain(&self.directory, &relative)?;
            let path = self.directory.join(&relative);
            let current = file_stamp(&path)?;
            let expected = self.baseline.get(&relative).copied();
            if current != expected {
                cleanup_plan(&self.directory, &plan);
                cleanup_created_dirs(&created_dirs);
                return Err(ProjectError::SourceChanged(relative.display().to_string()));
            }
            if current.is_none() {
                continue;
            }
            let backup = sibling_name(&path, "bak");
            copy_file_synced(&path, &backup)?;
            plan.push(JournalFile {
                path: relative,
                temp: None,
                backup: Some(
                    backup
                        .strip_prefix(&self.directory)
                        .map_err(|_| invalid("backup escaped project"))?
                        .to_path_buf(),
                ),
                delete: true,
                sha256: None,
            });
            crate::sync_path(
                path.parent()
                    .ok_or_else(|| invalid("delete path has no parent"))?,
            )?;
        }
        if cancelled() {
            cleanup_plan(&self.directory, &plan);
            cleanup_created_dirs(&created_dirs);
            return Err(ProjectError::Cancelled);
        }
        plan.sort_by_key(|entry| (entry.path == Path::new(FOLDER_MANIFEST), entry.path.clone()));
        let journal = SaveJournal {
            files: plan,
            asset_folders: Some(asset_folders),
        };
        let journal_path = self.directory.join(".pbrtui/save-journal.json");
        if let Err(error) = write_json_atomic(&journal_path, &journal) {
            cleanup_plan(&self.directory, &journal.files);
            cleanup_created_dirs(&created_dirs);
            return Err(error);
        }
        apply_journal(&self.directory, &journal)?;
        Ok(())
    }

    fn asset_meta_origin_for_save(
        &self,
        current: &Path,
        asset: &Asset,
    ) -> Result<Option<AssetMetaOrigin>> {
        if let Some(origin) = asset.origin.as_ref()
            && let Some(source) = &origin.origin
        {
            return Ok(Some(AssetMetaOrigin {
                import: source.import,
                logical_path: origin.logical_path.clone(),
            }));
        }
        let path = self.directory.join(meta_path(current));
        match read_bounded(&path, 1024 * 1024) {
            Ok(bytes) => Ok(serde_json::from_slice::<AssetMeta>(&bytes)?.origin),
            Err(ProjectError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    fn snapshot_plan(
        &self,
        snapshot: &ProjectSnapshot,
    ) -> Result<(
        BTreeMap<PathBuf, Vec<u8>>,
        std::collections::BTreeSet<PathBuf>,
        std::collections::BTreeSet<PathBuf>,
    )> {
        let document = snapshot.document();
        document.validate()?;
        if document.scenes.is_empty() && !self.is_v3() {
            return Err(invalid(
                "saving a project without scenes requires project format 3",
            ));
        }
        if document.id != self.manifest.project_id {
            return Err(invalid("snapshot belongs to a different project"));
        }
        let mut writes = BTreeMap::new();
        let mut expected = std::collections::BTreeSet::new();
        let mut directories = std::collections::BTreeSet::new();
        let scan = self.scan_assets()?;
        if self.is_v3() && !scan.duplicate_ids.is_empty() {
            return Err(invalid("duplicate format-3 sidecar identity"));
        }
        let mut current_by_id = BTreeMap::new();
        for relative in &scan.files {
            let path = PathBuf::from("Assets").join(relative);
            if let Some(id) = self.meta_id(&path)? {
                current_by_id.entry(id).or_insert(path);
            }
        }
        for scene in document.scenes.values() {
            let relative = safe_asset_location(&scene.location)?;
            let scene_file = SceneFile {
                format: FOLDER_SCENE_FORMAT.to_owned(),
                label: scene.label.clone(),
                options: scene.options.clone(),
                render_camera: Some(scene.render_camera),
                coordinate_system: Some(scene.coordinate_system),
                world: scene.world,
                nodes: nodes_in_order(
                    &scene.nodes,
                    scene.options.iter().copied().chain([scene.world]),
                )?,
            };
            let path = PathBuf::from("Assets").join(relative);
            expected.insert(path.clone());
            writes.insert(path.clone(), json_bytes(&scene_file)?);
            let meta_path = meta_path(&path);
            expected.insert(meta_path.clone());
            writes.insert(
                meta_path,
                json_bytes(&AssetMeta {
                    format: FOLDER_META_FORMAT.to_owned(),
                    id: scene.id.0,
                    kind: "scene".into(),
                    media_type: None,
                    sha256: None,
                    size: None,
                    origin: None,
                })?,
            );
        }
        for import in document.imports.values() {
            let path = PathBuf::from("Imports")
                .join(import.id.to_string())
                .join("import.json");
            expected.insert(path.clone());
            writes.insert(
                path,
                json_bytes(&ImportFile {
                    format: "pbrt-ui.import/1".into(),
                    id: import.id,
                    importer_version: import.importer_version.clone(),
                    dialect: import.dialect,
                    renderer_revision: import.renderer_revision.clone(),
                    diagnostics: import.diagnostics.clone(),
                    root: import.root,
                    sources: import.sources.clone(),
                    occurrences: import.occurrences.clone(),
                })?,
            );
        }
        for resource in document.resources.values() {
            let relative = safe_asset_location(&resource.location)?;
            let suffix = match resource.kind {
                ResourceKind::Material => ".material",
                ResourceKind::Texture => ".texture",
                ResourceKind::Medium => ".medium",
                ResourceKind::SpectrumData => ".spectrum",
                ResourceKind::ObjectDefinition => ".object",
            };
            if !relative.to_string_lossy().ends_with(suffix) {
                return Err(invalid(
                    "resource location extension does not match its kind",
                ));
            }
            let nodes = if resource.kind == ResourceKind::ObjectDefinition {
                let root = resource
                    .definition_scope
                    .ok_or_else(|| invalid("object resource has no definition root"))?;
                Some(nodes_in_order(&document.definition_nodes, [root])?)
            } else {
                None
            };
            let file = ResourceFile {
                format: FOLDER_RESOURCE_FORMAT.to_owned(),
                kind: resource.kind,
                label: resource.label.clone(),
                edition: resource.edition.clone(),
                texture_value_type: resource.texture_value_type,
                implementation: resource.implementation.clone(),
                parameters: resource.parameters.clone(),
                declared_in: resource.declared_in.clone(),
                origin: resource.origin.clone(),
                nodes,
            };
            let path = PathBuf::from("Assets").join(relative);
            expected.insert(path.clone());
            writes.insert(path.clone(), json_bytes(&file)?);
            let meta_path = meta_path(&path);
            expected.insert(meta_path.clone());
            writes.insert(
                meta_path,
                json_bytes(&AssetMeta {
                    format: FOLDER_META_FORMAT.to_owned(),
                    id: resource.id.0,
                    kind: "resource".into(),
                    media_type: None,
                    sha256: None,
                    size: None,
                    origin: None,
                })?,
            );
        }
        for asset in document.assets.values() {
            let relative = safe_asset_location(&asset.location)?;
            let target = PathBuf::from("Assets").join(&relative);
            let current = current_by_id
                .get(&asset.id.0)
                .cloned()
                .unwrap_or_else(|| target.clone());
            let saved_origin = self
                .asset_meta_origin_for_save(&current, asset)?
                .or_else(|| {
                    snapshot
                        .payload_by(asset.id)
                        .and_then(|lease| lease.origin().cloned())
                });
            self.check_unchanged(&meta_path(&current))?;
            let source = self.directory.join(&current);
            if !source.is_file() {
                self.check_unchanged(&current)?;
                if let Some(lease) = snapshot.payload_by(asset.id) {
                    // a previous save already
                    // removed the file — the history's staged bytes
                    // re-materialize it, verified against the model like any
                    // save input. A stale orphan meta elsewhere keeps refusing.
                    if self
                        .find_orphan_meta(asset.id)?
                        .is_some_and(|sidecar| sidecar != meta_path(&target))
                    {
                        return Err(invalid("missing asset location changed before repair"));
                    }
                    let mut bytes = Vec::new();
                    File::open(lease.path())?
                        .take(MAX_OBJECT_BYTES + 1)
                        .read_to_end(&mut bytes)?;
                    if bytes.len() as u64 > MAX_OBJECT_BYTES {
                        return Err(ProjectError::ResourceLimit("asset exceeds 4 GiB".into()));
                    }
                    let hash = format!("{:x}", Sha256::digest(&bytes));
                    if hash != asset.sha256 || bytes.len() as u64 != asset.size {
                        return Err(invalid("staged asset bytes do not match the project model"));
                    }
                    expected.insert(target.clone());
                    writes.insert(target.clone(), bytes);
                    let target_meta = meta_path(&target);
                    expected.insert(target_meta.clone());
                    writes.insert(
                        target_meta,
                        json_bytes(&AssetMeta {
                            format: FOLDER_META_FORMAT.to_owned(),
                            id: asset.id.0,
                            kind: "asset".into(),
                            media_type: Some(asset.media_type.clone()),
                            sha256: Some(hash),
                            size: Some(asset.size),
                            origin: saved_origin.clone(),
                        })?,
                    );
                    continue;
                }
                let orphan_meta = self.find_orphan_meta(asset.id)?;
                if let Some(sidecar) = orphan_meta {
                    let target_meta = meta_path(&target);
                    if sidecar != target_meta {
                        return Err(invalid("missing asset location changed before repair"));
                    }
                    expected.insert(target_meta.clone());
                    writes.insert(
                        target_meta,
                        json_bytes(&AssetMeta {
                            format: FOLDER_META_FORMAT.to_owned(),
                            id: asset.id.0,
                            kind: "asset".into(),
                            media_type: Some(asset.media_type.clone()),
                            sha256: Some(asset.sha256.clone()),
                            size: Some(asset.size),
                            origin: saved_origin.clone(),
                        })?,
                    );
                    continue;
                }
                return Err(invalid("asset file is missing"));
            }
            let input = File::open(&source)?;
            let mut bytes = Vec::new();
            input.take(MAX_OBJECT_BYTES + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_OBJECT_BYTES {
                return Err(ProjectError::ResourceLimit("asset exceeds 4 GiB".into()));
            }
            let hash = format!("{:x}", Sha256::digest(&bytes));
            if hash != asset.sha256 || bytes.len() as u64 != asset.size {
                // Rewritten after the session adopted it; the next snapshot adopts again.
                return Err(ProjectError::SourceChanged(current.display().to_string()));
            }
            expected.insert(target.clone());
            writes.insert(target.clone(), bytes);
            let target_meta = meta_path(&target);
            expected.insert(target_meta.clone());
            writes.insert(
                target_meta,
                json_bytes(&AssetMeta {
                    format: FOLDER_META_FORMAT.to_owned(),
                    id: asset.id.0,
                    kind: "asset".into(),
                    media_type: Some(asset.media_type.clone()),
                    sha256: Some(hash),
                    size: Some(asset.size),
                    origin: saved_origin,
                })?,
            );
        }
        for relative in &scan.missing_meta {
            let body = PathBuf::from("Assets").join(relative);
            let sidecar = meta_path(&body);
            if expected.contains(&body) || expected.contains(&sidecar) {
                continue;
            }
            let (kind, media_type, sha256, size) =
                match relative.extension().and_then(|ext| ext.to_str()) {
                    Some("scene") => ("scene", None, None, None),
                    Some("material" | "texture" | "medium" | "object") => {
                        ("resource", None, None, None)
                    }
                    _ => {
                        let (sha, size) = hash_file(&self.directory.join(&body))?;
                        (
                            "asset",
                            Some("application/octet-stream".to_owned()),
                            Some(sha),
                            Some(size),
                        )
                    }
                };
            expected.insert(body);
            expected.insert(sidecar.clone());
            writes.insert(
                sidecar,
                json_bytes(&AssetMeta {
                    format: FOLDER_META_FORMAT.to_owned(),
                    id: uuid::Uuid::new_v4(),
                    kind: kind.to_owned(),
                    media_type,
                    sha256,
                    size,
                    origin: None,
                })?,
            );
        }
        for (_, relative) in &scan.duplicate_ids {
            let body = PathBuf::from("Assets").join(relative);
            let sidecar = meta_path(&body);
            let mut meta: AssetMeta = serde_json::from_slice(&read_bounded(
                &self.directory.join(&sidecar),
                1024 * 1024,
            )?)?;
            meta.id = uuid::Uuid::new_v4();
            expected.insert(body);
            expected.insert(sidecar.clone());
            writes.insert(sidecar, json_bytes(&meta)?);
        }
        for folder in &document.folders {
            let relative = safe_relative_path(folder)?;
            directories.insert(PathBuf::from("Assets").join(relative));
        }
        let mut next_manifest = self.manifest.clone();
        next_manifest.title = document.title.clone();
        next_manifest.default_scene = document.default_scene;
        next_manifest.saved_revision = snapshot.revision().clone();
        next_manifest.importer = document
            .imports
            .values()
            .next()
            .map(|import| FolderImporter {
                version: import.importer_version.clone(),
                r4_revision: import.renderer_revision.clone().unwrap_or_default(),
            });
        let manifest_path = PathBuf::from(FOLDER_MANIFEST);
        expected.insert(manifest_path.clone());
        writes.insert(manifest_path, json_bytes(&next_manifest)?);

        let mut deletes = std::collections::BTreeSet::new();
        for relative in scan.files {
            let path = PathBuf::from("Assets").join(relative);
            let meta = meta_path(&path);
            if self.meta_id(&path)?.is_some() && !expected.contains(&path) {
                deletes.insert(path);
                deletes.insert(meta);
            }
        }
        for relative in scan.orphan_meta {
            let sidecar = PathBuf::from("Assets").join(relative);
            if !expected.contains(&sidecar) {
                deletes.insert(sidecar);
            }
        }
        let mut import_entries =
            fs::read_dir(self.directory.join("Imports"))?.collect::<std::io::Result<Vec<_>>>()?;
        import_entries.sort_by_key(|entry| entry.file_name());
        for entry in import_entries {
            if !entry.file_type()?.is_dir() || entry.file_type()?.is_symlink() {
                continue;
            }
            let record = PathBuf::from("Imports")
                .join(entry.file_name())
                .join("import.json");
            if !expected.contains(&record) && self.directory.join(&record).is_file() {
                deletes.insert(record);
            }
        }
        if self.is_v3() {
            for (path, bytes) in &mut writes {
                if path.extension().and_then(|v| v.to_str()) == Some("meta") {
                    let meta: AssetMeta = serde_json::from_slice(bytes)?;
                    *bytes = json_bytes(&meta.into_v2()?)?;
                }
            }
        }
        Ok((writes, deletes, directories))
    }

    fn find_orphan_meta(&self, id: AssetId) -> Result<Option<PathBuf>> {
        for relative in self.scan_assets()?.orphan_meta {
            let sidecar = PathBuf::from("Assets").join(relative);
            let meta: AssetMeta = serde_json::from_slice(&read_bounded(
                &self.directory.join(&sidecar),
                1024 * 1024,
            )?)?;
            if meta.id == id.0 && meta.kind == "asset" {
                return Ok(Some(sidecar));
            }
        }
        Ok(None)
    }

    fn meta_id(&self, relative: &Path) -> Result<Option<uuid::Uuid>> {
        #[cfg(test)]
        import_metrics::note_meta_read();
        let path = self.directory.join(meta_path(relative));
        if !path.exists() {
            return Ok(None);
        }
        let meta: AssetMeta = serde_json::from_slice(&read_bounded(&path, 1024 * 1024)?)?;
        Ok(Some(meta.id))
    }

    fn check_unchanged(&self, relative: &Path) -> Result<()> {
        let current = file_stamp(&self.directory.join(relative))?;
        if current != self.baseline.get(relative).copied() {
            return Err(ProjectError::SourceChanged(relative.display().to_string()));
        }
        Ok(())
    }
}

pub(crate) fn asset_file_by_id(
    assets_root: &Path,
    id: AssetId,
) -> Result<Option<(PathBuf, Option<AssetMetaOrigin>)>> {
    Ok(asset_files_by_id(assets_root)?.remove(&id))
}

/// Mutable state shared by one import batch: the sidecar ids seen up front
/// (extended by each asset as it imports), plus every path and directory the
/// batch has written, so a later failure can remove them all again.
struct ImportBatchState {
    ids: std::collections::BTreeSet<uuid::Uuid>,
    created_dirs: Vec<PathBuf>,
    written: Vec<PathBuf>,
}

/// Best-effort removal of everything an import batch wrote so the project
/// is left as the batch found it. The baseline is untouched, so the store
/// stays usable without a reopen; callers discard their own destinations.
fn rollback_import(root: &Path, batch: &ImportBatchState) {
    for path in batch.written.iter().rev() {
        let _ = fs::remove_file(root.join(path));
    }
    cleanup_created_dirs(&batch.created_dirs);
}

/// The provenance a fresh import derives from the model's origin record.
fn model_asset_origin(asset: &Asset) -> Option<AssetMetaOrigin> {
    asset.origin.as_ref().and_then(|origin| {
        origin.origin.as_ref().map(|source| AssetMetaOrigin {
            import: source.import,
            logical_path: origin.logical_path.clone(),
        })
    })
}

pub(crate) fn asset_files_by_id(
    assets_root: &Path,
) -> Result<BTreeMap<AssetId, (PathBuf, Option<AssetMetaOrigin>)>> {
    #[cfg(test)]
    import_metrics::note_asset_scan();
    if !assets_root.is_dir() {
        return Ok(BTreeMap::new());
    }
    let mut scan = FolderScan::default();
    let mut ids = BTreeMap::new();
    scan_directory(assets_root, assets_root, &mut scan, &mut ids)?;
    let mut files = BTreeMap::new();
    for relative in scan.files {
        let path = assets_root.join(&relative);
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(".meta");
        let bytes = match read_bounded(Path::new(&sidecar), 1024 * 1024) {
            Ok(bytes) => bytes,
            Err(ProjectError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                continue;
            }
            Err(error) => return Err(error),
        };
        let meta: AssetMeta = serde_json::from_slice(&bytes)?;
        if meta.kind == "asset" {
            files.entry(AssetId(meta.id)).or_insert((path, meta.origin));
        }
    }
    Ok(files)
}

fn nodes_in_order(
    nodes: &BTreeMap<NodeId, Node>,
    roots: impl IntoIterator<Item = NodeId>,
) -> Result<Vec<Node>> {
    fn visit(
        id: NodeId,
        nodes: &BTreeMap<NodeId, Node>,
        seen: &mut std::collections::BTreeSet<NodeId>,
        output: &mut Vec<Node>,
    ) -> Result<()> {
        if !seen.insert(id) {
            return Ok(());
        }
        let node = nodes
            .get(&id)
            .ok_or_else(|| invalid("scene references a missing node"))?;
        output.push(node.clone());
        if let NodeKind::Scope { children, .. } = &node.data {
            for child in children {
                visit(*child, nodes, seen, output)?;
            }
        }
        Ok(())
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut output = Vec::new();
    for root in roots {
        visit(root, nodes, &mut seen, &mut output)?;
    }
    Ok(output)
}

fn nodes_map(nodes: Vec<Node>) -> Result<BTreeMap<NodeId, Node>> {
    let mut result = BTreeMap::new();
    for node in nodes {
        if result.insert(node.id, node).is_some() {
            return Err(invalid("duplicate node id in scene file"));
        }
    }
    Ok(result)
}

fn node_has_owned_object(node: &Node) -> bool {
    matches!(
        node.data,
        NodeKind::Scope {
            owned_object: Some(_),
            ..
        }
    )
}

fn contains_owned_object_field(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(object) => {
            object.contains_key("owned_object") || object.values().any(contains_owned_object_field)
        }
        serde_json::Value::Array(values) => values.iter().any(contains_owned_object_field),
        _ => false,
    }
}

fn safe_asset_location(location: &str) -> Result<PathBuf> {
    let path = safe_relative_path(location)?;
    if path.components().count() == 0 {
        return Err(invalid("empty asset location"));
    }
    Ok(path)
}

fn safe_relative_path(path: &str) -> Result<PathBuf> {
    let path = Path::new(path);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(invalid("project path must be a safe relative path"));
    }
    Ok(path.to_path_buf())
}

fn meta_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.meta", path.to_string_lossy()))
}

fn json_bytes(value: &impl Serialize) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    if bytes.len() as u64 > MAX_FOLDER_JSON_BYTES {
        return Err(ProjectError::ResourceLimit(
            "project JSON exceeds 64 MiB".into(),
        ));
    }
    bytes.push(b'\n');
    Ok(bytes)
}

fn write_gitignore(directory: &Path) -> Result<()> {
    let path = directory.join(".gitignore");
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(b"Library/\n.pbrtui/\n")?;
    file.sync_all()?;
    crate::sync_path(directory)?;
    Ok(())
}

fn validate_project_relative(path: &Path) -> Result<()> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(invalid("save journal contains an unsafe path"));
    }
    Ok(())
}

fn validate_existing_parent_chain(root: &Path, relative: &Path) -> Result<()> {
    let mut current = root.to_path_buf();
    let components = relative.components().collect::<Vec<_>>();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        let std::path::Component::Normal(name) = component else {
            return Err(invalid("project path contains a non-normal component"));
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err(invalid("project path traverses a non-directory or symlink")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn create_directory_chain(root: &Path, relative: &Path, created: &mut Vec<PathBuf>) -> Result<()> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(invalid("directory path contains a non-normal component"));
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(invalid(
                    "directory path traverses a non-directory or symlink",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current)?;
                created.push(current.clone());
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn cleanup_created_dirs(created: &[PathBuf]) {
    for path in created.iter().rev() {
        let _ = fs::remove_dir(path);
    }
}

fn file_stamp(path: &Path) -> Result<Option<FileStamp>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            let modified_ns = metadata
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            Ok(Some(FileStamp {
                size: metadata.len(),
                modified_ns,
            }))
        }
        Ok(_) => Err(invalid("save destination is not a regular file")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn file_equals(path: &Path, expected: &[u8]) -> Result<bool> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(invalid("save destination is not a regular file"));
        }
    }
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if file.metadata()?.len() != expected.len() as u64 {
        return Ok(false);
    }
    let mut actual = Vec::with_capacity(expected.len());
    file.read_to_end(&mut actual)?;
    Ok(actual == expected)
}

fn sibling_name(path: &Path, suffix: &str) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(".{name}.{}.pbrtui-{suffix}", uuid::Uuid::new_v4()))
}

fn copy_file_synced(source: &Path, destination: &Path) -> Result<()> {
    match fs::hard_link(source, destination) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::copy(source, destination)?;
            crate::sync_path(destination)?;
            Ok(())
        }
    }
}

fn cleanup_plan(root: &Path, files: &[JournalFile]) {
    for entry in files {
        for relative in [entry.temp.as_ref(), entry.backup.as_ref()]
            .into_iter()
            .flatten()
        {
            let _ = fs::remove_file(root.join(relative));
        }
    }
}

fn digest_matches(path: &Path, expected: &str) -> Result<bool> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(invalid("journal path is not a regular file"));
        }
    }
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()) == expected)
}

pub(crate) fn hash_file(path: &Path) -> Result<(String, u64)> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size = size
            .checked_add(count as u64)
            .ok_or_else(|| invalid("asset size overflow"))?;
        if size > MAX_OBJECT_BYTES {
            return Err(ProjectError::ResourceLimit("asset exceeds 4 GiB".into()));
        }
        hash.update(&buffer[..count]);
    }
    Ok((format!("{:x}", hash.finalize()), size))
}

fn apply_journal(root: &Path, journal: &SaveJournal) -> Result<()> {
    for entry in &journal.files {
        validate_project_relative(&entry.path)?;
        validate_existing_parent_chain(root, &entry.path)?;
        if let Some(backup) = &entry.backup {
            validate_project_relative(backup)?;
            validate_existing_parent_chain(root, backup)?;
        }
        let destination = root.join(&entry.path);
        if entry.delete {
            match fs::remove_file(&destination) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        } else if let Some(temp) = &entry.temp {
            validate_project_relative(temp)?;
            validate_existing_parent_chain(root, temp)?;
            let temporary = root.join(temp);
            if temporary.exists() {
                fs::rename(&temporary, &destination)?;
            } else if !digest_matches(&destination, entry.sha256.as_deref().unwrap_or_default())? {
                return Err(invalid("save journal is missing a staged file"));
            }
        }
        if let Some(mut parent) = destination.parent() {
            loop {
                match crate::sync_path(parent) {
                    Ok(()) => break,
                    Err(error)
                        if entry.delete
                            && error.kind() == std::io::ErrorKind::NotFound
                            && parent != root =>
                    {
                        parent = parent
                            .parent()
                            .ok_or_else(|| invalid("journal parent escaped project"))?;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
    }
    // Retain the recovery record until the caller has validated readback.
    // Published files are sufficient for roll-forward after backups are removed.
    cleanup_plan(root, &journal.files);
    if let Some(folders) = &journal.asset_folders {
        prune_empty_asset_directories(&root.join("Assets"), folders)?;
    }
    Ok(())
}

fn finish_journal(root: &Path) -> Result<()> {
    let journal_path = root.join(".pbrtui/save-journal.json");
    match fs::remove_file(&journal_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    crate::sync_path(root.join(".pbrtui"))?;
    Ok(())
}

fn recover_journal(root: &Path) -> Result<()> {
    let path = root.join(".pbrtui/save-journal.json");
    let bytes = match read_bounded(&path, MAX_FOLDER_JSON_BYTES) {
        Ok(bytes) => bytes,
        Err(ProjectError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let journal: SaveJournal = serde_json::from_slice(&bytes)?;
    for entry in &journal.files {
        validate_project_relative(&entry.path)?;
        validate_existing_parent_chain(root, &entry.path)?;
        if let Some(temp) = &entry.temp {
            validate_project_relative(temp)?;
            validate_existing_parent_chain(root, temp)?;
        }
        if let Some(backup) = &entry.backup {
            validate_project_relative(backup)?;
            validate_existing_parent_chain(root, backup)?;
        }
    }
    let complete = journal.files.iter().all(|entry| {
        let destination = root.join(&entry.path);
        if entry.delete {
            !destination.exists()
                || entry
                    .backup
                    .as_ref()
                    .is_some_and(|backup| root.join(backup).exists())
        } else {
            entry
                .temp
                .as_ref()
                .is_some_and(|temp| root.join(temp).exists())
                || entry
                    .sha256
                    .as_deref()
                    .is_some_and(|sha| digest_matches(&destination, sha).unwrap_or(false))
        }
    });
    if complete {
        apply_journal(root, &journal)?;
        return finish_journal(root);
    }
    for entry in &journal.files {
        let destination = root.join(&entry.path);
        if let Some(backup) = &entry.backup {
            let backup = root.join(backup);
            if backup.exists() {
                fs::rename(&backup, &destination)?;
            }
        } else if !entry.delete
            && entry
                .sha256
                .as_deref()
                .is_some_and(|sha| digest_matches(&destination, sha).unwrap_or(false))
        {
            fs::remove_file(&destination)?;
        }
        if let Some(temp) = &entry.temp {
            let _ = fs::remove_file(root.join(temp));
        }
    }
    fs::remove_file(path)?;
    crate::sync_path(root.join(".pbrtui"))?;
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn rename_noreplace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let source_c = std::ffi::CString::new(source.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let destination_c = std::ffi::CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: both strings are NUL terminated and remain live for the syscall.
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source_c.as_ptr(),
            libc::AT_FDCWD,
            destination_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    // SAFETY: as above.
    #[cfg(target_os = "macos")]
    let result =
        unsafe { libc::renamex_np(source_c.as_ptr(), destination_c.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    // Volumes without exclusive rename (exFAT, some network shares) on macOS.
    #[cfg(target_os = "macos")]
    if error.raw_os_error() == Some(libc::ENOTSUP) {
        return rename_checked(source, destination);
    }
    Err(error)
}

fn lock_folder(directory: &Path) -> Result<File> {
    let path = directory.join(".pbrtui/lock");
    if fs::symlink_metadata(&path)
        .is_ok_and(|metadata| !metadata.is_file() || metadata.file_type().is_symlink())
    {
        return Err(invalid("folder project lock is not a regular file"));
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    super::files::try_lock_briefly(&file)?;
    Ok(file)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn rename_noreplace(source: &Path, destination: &Path) -> std::io::Result<()> {
    rename_checked(source, destination)
}

/// Not atomic: another process can create `destination` between the check and
/// the rename.
#[cfg(not(target_os = "linux"))]
fn rename_checked(source: &Path, destination: &Path) -> std::io::Result<()> {
    if destination.exists() {
        return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
    }
    fs::rename(source, destination)
}

/// The desktop's per-project layout (node positions) belongs to the project,
/// so Save As carries it to the new folder.
pub(super) fn copy_ui_sidecars(source: &Path, staged: &Path) -> Result<()> {
    let name = Path::new(".pbrtui").join("node-positions.json");
    match fs::copy(source.join(&name), staged.join(&name)) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}

pub(super) fn prepare_save_as_destination(destination: &Path) -> Result<(PathBuf, PathBuf)> {
    if !destination.is_absolute() {
        return Err(invalid("Save As destination must be absolute"));
    }
    let name = destination
        .file_name()
        .ok_or_else(|| invalid("Save As destination requires a folder name"))?;
    let parent = protocol::path::canonicalize(
        destination
            .parent()
            .ok_or_else(|| invalid("Save As destination has no parent"))?,
    )?;
    regular_directory(&parent)?;
    let destination = parent.join(name);
    match fs::symlink_metadata(&destination) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(invalid(
                "Save As destination must be a new or empty directory",
            ));
        }
        Ok(_) if fs::read_dir(&destination)?.next().is_some() => {
            return Err(invalid("Save As destination directory is not empty"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok((parent, destination))
}

pub(super) fn remove_empty_save_as_destination(destination: &Path) -> Result<()> {
    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(invalid("Save As destination is not an empty directory"))
        }
        Ok(_) => {
            fs::remove_dir(destination)?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn copy_cancellable(
    input: &mut impl Read,
    output: &mut impl Write,
    cancelled: &impl Fn() -> bool,
) -> Result<()> {
    let mut buffer = [0u8; 64 * 1024];
    loop {
        if cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let count = input.read(&mut buffer)?;
        if count == 0 {
            return Ok(());
        }
        output.write_all(&buffer[..count])?;
    }
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    copy_tree_cancellable(source, destination, &|| false)
}

fn copy_tree_cancellable(
    source: &Path,
    destination: &Path,
    cancelled: &impl Fn() -> bool,
) -> Result<()> {
    if cancelled() {
        return Err(ProjectError::Cancelled);
    }
    let metadata = source.symlink_metadata()?;
    if metadata.file_type().is_symlink() {
        return Err(invalid("Save As does not copy symlinks"));
    }
    if metadata.is_dir() {
        fs::create_dir_all(destination)?;
        let mut entries = fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            copy_tree_cancellable(
                &entry.path(),
                &destination.join(entry.file_name()),
                cancelled,
            )?;
        }
    } else if metadata.is_file() {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut input = File::open(source)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            if cancelled() {
                return Err(ProjectError::Cancelled);
            }
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            output.write_all(&buffer[..count])?;
        }
        output.sync_all()?;
    } else {
        return Err(invalid("Save As encountered a non-file asset"));
    }
    Ok(())
}

fn collect_file_stamps(root: &Path) -> Result<BTreeMap<PathBuf, FileStamp>> {
    #[cfg(test)]
    import_metrics::note_folder_walk();
    fn walk(root: &Path, current: &Path, output: &mut BTreeMap<PathBuf, FileStamp>) -> Result<()> {
        let mut entries = fs::read_dir(current)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                // The regenerable cache is not project content; core writes
                // it between saves and it must never trip external-change
                // detection.
                if path == root.join("Library") {
                    continue;
                }
                walk(root, &path, output)?;
            } else if kind.is_file() {
                let metadata = entry.metadata()?;
                let modified_ns = metadata
                    .modified()?
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                output.insert(
                    path.strip_prefix(root)
                        .map_err(|_| invalid("file escaped project root"))?
                        .to_path_buf(),
                    FileStamp {
                        size: metadata.len(),
                        modified_ns,
                    },
                );
            }
        }
        Ok(())
    }
    let mut output = BTreeMap::new();
    walk(root, root, &mut output)?;
    Ok(output)
}

fn collect_asset_directories(
    root: &Path,
    current: &Path,
    output: &mut std::collections::BTreeSet<String>,
) -> Result<()> {
    let mut entries = fs::read_dir(current)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_symlink() || !kind.is_dir() {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| invalid("folder escaped Assets"))?;
        output.insert(relative.to_string_lossy().replace('\\', "/"));
        collect_asset_directories(root, &path, output)?;
    }
    Ok(())
}

fn prune_empty_asset_directories(
    root: &Path,
    desired: &std::collections::BTreeSet<String>,
) -> Result<()> {
    let mut existing = std::collections::BTreeSet::new();
    collect_asset_directories(root, root, &mut existing)?;
    let mut obsolete = existing
        .into_iter()
        .filter(|folder| !desired.contains(folder))
        .collect::<Vec<_>>();
    obsolete.sort_by_key(|folder| std::cmp::Reverse(folder.matches('/').count()));
    for folder in obsolete {
        let path = root.join(&folder);
        match fs::remove_dir(path) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn scan_directory(
    root: &Path,
    current: &Path,
    scan: &mut FolderScan,
    ids: &mut BTreeMap<uuid::Uuid, PathBuf>,
) -> Result<()> {
    let mut entries = fs::read_dir(current)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = entry.file_type()?;
        if metadata.is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            scan_directory(root, &path, scan, ids)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| invalid("asset escaped Assets directory"))?
                .to_path_buf();
            if path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".meta"))
            {
                let body =
                    PathBuf::from(path.to_string_lossy().strip_suffix(".meta").unwrap_or(""));
                let body_exists = fs::symlink_metadata(&body)
                    .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink());
                if !body_exists {
                    scan.orphan_meta.push(relative);
                }
            } else {
                scan.files.push(relative.clone());
                let meta_path = PathBuf::from(format!("{}.meta", path.display()));
                let meta_metadata = fs::symlink_metadata(&meta_path);
                if meta_metadata.is_err() {
                    scan.missing_meta
                        .push(path.strip_prefix(root).unwrap().to_path_buf());
                    continue;
                }
                let meta_metadata = meta_metadata?;
                if !meta_metadata.is_file() || meta_metadata.file_type().is_symlink() {
                    return Err(invalid("asset meta is not a regular file"));
                }
                let bytes = read_bounded(&meta_path, 1024 * 1024)?;
                let meta: AssetMeta = serde_json::from_slice(&bytes)?;
                #[cfg(test)]
                import_metrics::note_meta_read();
                if meta.format != FOLDER_META_FORMAT && meta.format != "pbrt-ui.meta/2" {
                    return Err(invalid("unrecognized asset meta format"));
                }
                if let Some(first) = ids.get(&meta.id) {
                    scan.duplicate_ids
                        .push((meta.id, path.strip_prefix(root).unwrap().to_path_buf()));
                    let _ = first;
                } else {
                    ids.insert(meta.id, relative);
                }
            }
        }
    }
    Ok(())
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    let parent = path
        .parent()
        .ok_or_else(|| invalid("manifest has no parent"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    crate::sync_path(parent)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn rename_noreplace_moves_files_and_directories_but_never_overwrites() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("file");
        let directory = temp.path().join("directory");
        fs::write(&file, b"source").unwrap();
        fs::create_dir(&directory).unwrap();
        rename_noreplace(&file, &temp.path().join("moved-file")).unwrap();
        rename_noreplace(&directory, &temp.path().join("moved-directory")).unwrap();
        let taken = temp.path().join("taken");
        fs::write(&taken, b"keep").unwrap();
        fs::write(&file, b"source").unwrap();
        let error = rename_noreplace(&file, &taken).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&taken).unwrap(), b"keep");
        assert!(file.exists());
    }

    #[test]
    fn scene_format_distinguishes_missing_camera_from_explicit_null() {
        let base = serde_json::json!({
            "format":"pbrt-ui.scene/1","label":"Scene","options":[],
            "world":NodeId::new(),"nodes":[]
        });
        let legacy: SceneFile = serde_json::from_value(base.clone()).unwrap();
        assert_eq!(legacy.render_camera, None);
        let mut explicit = base;
        explicit["render_camera"] = serde_json::Value::Null;
        let explicit: SceneFile = serde_json::from_value(explicit).unwrap();
        assert_eq!(explicit.render_camera, Some(None));
    }

    #[test]
    fn folder_store_creates_layout_and_updates_default_scene() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("Sample");
        let project_id = ProjectId::new();
        let mut store =
            FolderStore::create(&path, project_id, "Sample", Revision("0".into())).unwrap();
        let mut document = ProjectDocument::new("Sample");
        document.id = project_id;
        let scene = document.default_scene.unwrap();
        let session = ProjectSession::new(document).unwrap();
        store.save(&session.snapshot()).unwrap();
        assert!(store.set_default_scene(scene).unwrap());
        drop(store);

        let reopened = FolderStore::open(&path).unwrap();
        assert_eq!(reopened.project_id(), project_id);
        assert_eq!(reopened.default_scene(), Some(scene));
        assert!(path.join("Assets").is_dir());
        assert!(path.join("Imports").is_dir());
        assert!(path.join("Library").is_dir());
        assert!(path.join(".pbrtui").is_dir());
    }

    #[test]
    fn explicit_null_persists_and_scene_v1_missing_camera_uses_legacy_last_camera() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("Camera scene");
        let project_id = ProjectId::new();
        let mut store =
            FolderStore::create(&path, project_id, "Camera scene", Revision::initial()).unwrap();
        let mut document = ProjectDocument::new("Camera scene");
        document.id = project_id;
        let scene_id = document.default_scene.unwrap();
        let camera = Node::new(
            "Camera",
            NodeKind::Entity {
                entity: EntityKind::Camera,
                implementation: "perspective".into(),
                parameters: Vec::new(),
                pose: None,
            },
        );
        let scene = Arc::make_mut(document.scenes.get_mut(&scene_id).unwrap());
        scene.options.push(camera.id);
        scene.nodes.insert(camera.id, camera.clone());
        assert_eq!(scene.render_camera, None);
        store
            .save(&ProjectSession::new(document).unwrap().snapshot())
            .unwrap();
        drop(store);

        let (store, reopened) = FolderStore::open_project(&path).unwrap();
        assert_eq!(
            reopened.document().scene(scene_id).unwrap().render_camera,
            None
        );
        drop(store);

        let scene_path = path.join("Assets/Scenes/Camera scene.scene");
        let mut file: serde_json::Value =
            serde_json::from_slice(&fs::read(&scene_path).unwrap()).unwrap();
        file["format"] = serde_json::json!("pbrt-ui.scene/2");
        file.as_object_mut().unwrap().remove("render_camera");
        fs::write(&scene_path, serde_json::to_vec(&file).unwrap()).unwrap();
        assert!(FolderStore::open_project(&path).is_err());

        file["format"] = serde_json::json!("pbrt-ui.scene/1");
        fs::write(&scene_path, serde_json::to_vec(&file).unwrap()).unwrap();
        let (store, legacy) = FolderStore::open_project(&path).unwrap();
        assert_eq!(
            legacy.document().scene(scene_id).unwrap().render_camera,
            Some(camera.id)
        );
        drop(store);
    }

    #[test]
    fn latest_bodies_roundtrip_owned_scopes_and_legacy_bodies_do_not_infer_them() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("Owned scopes");
        let project_id = ProjectId::new();
        let mut store =
            FolderStore::create(&path, project_id, "Owned scopes", Revision::initial()).unwrap();

        let mut document = ProjectDocument::new("Owned scopes");
        document.id = project_id;
        let scene_id = document.default_scene.unwrap();
        let world = document.scene(scene_id).unwrap().world;
        let scene_shape = Node::new(
            "Scene shape",
            NodeKind::Entity {
                entity: EntityKind::Shape,
                implementation: "sphere".into(),
                parameters: Vec::new(),
                pose: None,
            },
        );
        let scene_wrapper = Node::new(
            "Scene implementation scope",
            NodeKind::Scope {
                scope: ScopeKind::Attribute,
                children: vec![scene_shape.id],
                owned_object: Some(scene_shape.id),
                parent_camera: None,
            },
        );
        {
            let scene = std::sync::Arc::make_mut(document.scenes.get_mut(&scene_id).unwrap());
            let NodeKind::Scope { children, .. } = &mut scene.nodes.get_mut(&world).unwrap().data
            else {
                unreachable!()
            };
            children.push(scene_wrapper.id);
            scene.nodes.insert(scene_wrapper.id, scene_wrapper.clone());
            scene.nodes.insert(scene_shape.id, scene_shape.clone());
        }

        let definition_shape = Node::new(
            "Definition shape",
            NodeKind::Entity {
                entity: EntityKind::Shape,
                implementation: "sphere".into(),
                parameters: Vec::new(),
                pose: None,
            },
        );
        let definition_shape_id = definition_shape.id;
        let definition_wrapper = Node::new(
            "Definition implementation scope",
            NodeKind::Scope {
                scope: ScopeKind::Attribute,
                children: vec![definition_shape.id],
                owned_object: Some(definition_shape.id),
                parent_camera: None,
            },
        );
        let definition_root = Node::new(
            "ObjectDefinition",
            NodeKind::Scope {
                scope: ScopeKind::ObjectDefinition,
                children: vec![definition_wrapper.id],
                owned_object: None,
                parent_camera: None,
            },
        );
        let resource_id = ResourceId::new();
        document
            .definition_nodes
            .insert(definition_shape.id, definition_shape);
        document
            .definition_nodes
            .insert(definition_wrapper.id, definition_wrapper.clone());
        document
            .definition_nodes
            .insert(definition_root.id, definition_root.clone());
        document.resources.insert(
            resource_id,
            Resource {
                id: resource_id,
                label: "Tree".into(),
                location: "Objects/Tree.object".into(),
                edition: Revision::initial(),
                kind: ResourceKind::ObjectDefinition,
                texture_value_type: None,
                implementation: String::new(),
                parameters: Vec::new(),
                definition_scope: Some(definition_root.id),
                declared_in: None,
                origin: None,
            },
        );
        store
            .save(&ProjectSession::new(document).unwrap().snapshot())
            .unwrap();
        drop(store);

        let scene_path = path.join("Assets/Scenes/Owned scopes.scene");
        let resource_path = path.join("Assets/Objects/Tree.object");
        let mut scene_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&scene_path).unwrap()).unwrap();
        let mut resource_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&resource_path).unwrap()).unwrap();
        assert_eq!(scene_json["format"], FOLDER_SCENE_FORMAT);
        assert_eq!(resource_json["format"], FOLDER_RESOURCE_FORMAT);
        assert_eq!(
            scene_json["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|node| node["id"] == scene_wrapper.id.to_string())
                .unwrap()["data"]["owned_object"],
            scene_shape.id.to_string()
        );
        assert_eq!(
            resource_json["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|node| node["id"] == definition_wrapper.id.to_string())
                .unwrap()["data"]["owned_object"],
            definition_shape_id.to_string()
        );
        let (store, reopened) = FolderStore::open_project(&path).unwrap();
        let loaded_scene = reopened.document().scene(scene_id).unwrap();
        assert!(matches!(
            loaded_scene.nodes[&scene_wrapper.id].data,
            NodeKind::Scope {
                owned_object: Some(id),
                ..
            } if id == scene_shape.id
        ));
        assert!(matches!(
            reopened.document().definition_nodes[&definition_wrapper.id].data,
            NodeKind::Scope {
                owned_object: Some(id),
                ..
            } if id == definition_shape_id
        ));
        drop(store);

        // Downgraded body versions have no owner field. The reader preserves
        // that lack of information rather than inferring from one child.
        scene_json["format"] = serde_json::json!("pbrt-ui.scene/2");
        scene_json["nodes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|node| node["id"] == scene_wrapper.id.to_string())
            .unwrap()["data"]["owned_object"] = serde_json::Value::Null;
        fs::write(&scene_path, serde_json::to_vec(&scene_json).unwrap()).unwrap();
        assert!(FolderStore::open_project(&path).is_err());
        for node in scene_json["nodes"].as_array_mut().unwrap() {
            node["data"].as_object_mut().unwrap().remove("owned_object");
        }
        resource_json["format"] = serde_json::json!("pbrt-ui.resource/1");
        resource_json["nodes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|node| node["id"] == definition_wrapper.id.to_string())
            .unwrap()["data"]["owned_object"] = serde_json::Value::Null;
        fs::write(&resource_path, serde_json::to_vec(&resource_json).unwrap()).unwrap();
        assert!(FolderStore::open_project(&path).is_err());
        for node in resource_json["nodes"].as_array_mut().unwrap() {
            node["data"].as_object_mut().unwrap().remove("owned_object");
        }
        fs::write(&scene_path, serde_json::to_vec(&scene_json).unwrap()).unwrap();
        fs::write(&resource_path, serde_json::to_vec(&resource_json).unwrap()).unwrap();
        let (store, legacy) = FolderStore::open_project(&path).unwrap();
        assert!(matches!(
            legacy.document().scene(scene_id).unwrap().nodes[&scene_wrapper.id].data,
            NodeKind::Scope {
                owned_object: None,
                ..
            }
        ));
        assert!(matches!(
            legacy.document().definition_nodes[&definition_wrapper.id].data,
            NodeKind::Scope {
                owned_object: None,
                ..
            }
        ));
        drop(store);
    }

    #[test]
    fn test_scan_reports_duplicate_ids_unregistered_and_missing() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("Sample");
        let store =
            FolderStore::create(&path, ProjectId::new(), "Sample", Revision("0".into())).unwrap();
        let asset = path.join("Assets/file.bin");
        fs::write(&asset, b"bytes").unwrap();
        let id = uuid::Uuid::new_v4();
        let meta = AssetMeta {
            format: FOLDER_META_FORMAT.to_owned(),
            id,
            kind: "asset".into(),
            media_type: Some("application/octet-stream".into()),
            sha256: Some("0".repeat(64)),
            size: Some(5),
            origin: None,
        };
        write_json_atomic(&PathBuf::from(format!("{}.meta", asset.display())), &meta).unwrap();
        let second = path.join("Assets/other.bin");
        fs::write(&second, b"other").unwrap();
        write_json_atomic(&PathBuf::from(format!("{}.meta", second.display())), &meta).unwrap();
        fs::write(path.join("Assets/unregistered.bin"), b"none").unwrap();
        fs::write(path.join("Assets/orphan.bin.meta"), b"{}").unwrap();

        let scan = store.scan_assets().unwrap();
        assert_eq!(scan.missing_meta, vec![PathBuf::from("unregistered.bin")]);
        assert_eq!(scan.orphan_meta, vec![PathBuf::from("orphan.bin.meta")]);
        assert_eq!(scan.duplicate_ids.len(), 1);
    }

    #[test]
    fn test_save_as_copies_assets_and_skips_library() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("Source");
        let source = FolderStore::create(
            &source_path,
            ProjectId::new(),
            "Source",
            Revision("0".into()),
        )
        .unwrap();
        fs::write(source_path.join("Assets/item.bin"), b"asset").unwrap();
        fs::write(source_path.join("Library/cache.bin"), b"cache").unwrap();
        fs::write(source_path.join(".pbrtui/lock-state"), b"runtime").unwrap();

        let destination = temp.path().join("Copy");
        let copy = source.save_as(&destination).unwrap();
        assert_eq!(
            fs::read(destination.join("Assets/item.bin")).unwrap(),
            b"asset"
        );
        fs::write(source_path.join("Assets/item.bin"), b"source changed").unwrap();
        assert_eq!(
            fs::read(destination.join("Assets/item.bin")).unwrap(),
            b"asset"
        );
        assert!(!destination.join("Library/cache.bin").exists());
        assert!(!destination.join(".pbrtui/lock-state").exists());
        assert_eq!(copy.project_id(), source.project_id());
    }

    #[test]
    fn save_as_snapshot_publishes_a_saved_format_two_folder() {
        let temp = tempfile::tempdir().unwrap();
        let document = ProjectDocument::new("SaveAs");
        let session = ProjectSession::new(document.clone()).unwrap();
        let source_path = temp.path().join("Source");
        let mut source =
            FolderStore::create(&source_path, document.id, "SaveAs", Revision("0".into())).unwrap();
        source.save(&session.snapshot()).unwrap();
        let destination = temp.path().join("Destination");
        let copy = source
            .save_as_snapshot(&destination, &session.snapshot())
            .unwrap();
        assert_eq!(copy.project_id(), document.id);
        assert!(
            destination
                .join("Assets/Scenes/SaveAs.scene.meta")
                .is_file()
        );
        assert!(!destination.join(".pbrtui/save-journal.json").exists());
    }

    #[test]
    fn save_as_from_staging_materializes_content_addressed_assets() {
        let temp = tempfile::tempdir().unwrap();
        let bytes = b"asset bytes";
        let hash = format!("{:x}", Sha256::digest(bytes));
        let mut staging = ObjectStore::create(&temp.path().join("staging")).unwrap();
        staging.put_object(&bytes[..]).unwrap();
        let mut document = ProjectDocument::new("Staged");
        let id = AssetId::new();
        document.assets.insert(
            id,
            Asset {
                id,
                location: "Meshes/mesh.ply".into(),
                media_type: "application/x-ply".into(),
                sha256: hash,
                size: bytes.len() as u64,
                origin: None,
            },
        );
        let session = ProjectSession::new(document.clone()).unwrap();
        let destination = temp.path().join("Published");
        let published =
            FolderStore::save_as_from_objects(&destination, &session.snapshot(), &staging).unwrap();
        let path = published.asset_path(id).unwrap();
        assert_eq!(fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn test_folder_save_is_byte_stable() {
        let temp = tempfile::tempdir().unwrap();
        let document = ProjectDocument::new("Stable");
        let project_id = document.id;
        let session = ProjectSession::new(document).unwrap();
        let path = temp.path().join("Stable");
        let mut store =
            FolderStore::create(&path, project_id, "Stable", Revision("0".into())).unwrap();
        store.save(&session.snapshot()).unwrap();
        let scene_file = path.join("Assets/Scenes/Stable.scene");
        let first = fs::read(&scene_file).unwrap();
        let stamp_before = file_stamp(&scene_file).unwrap();
        store.save(&session.snapshot()).unwrap();
        assert_eq!(fs::read(&scene_file).unwrap(), first);
        assert_eq!(file_stamp(&scene_file).unwrap(), stamp_before);
        drop(store);

        let (_, reopened) = ProjectStore::open(&path).unwrap();
        assert_eq!(reopened.document().title, "Stable");
        assert_eq!(reopened.document().scenes.len(), 1);
    }

    #[test]
    fn test_journal_rolls_forward_when_temporary_files_are_present() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join(".pbrtui")).unwrap();
        let target = temp.path().join("Assets/scene.scene");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target.parent().unwrap().join(".scene.tmp"), b"new").unwrap();
        let journal = SaveJournal {
            asset_folders: None,
            files: vec![JournalFile {
                path: PathBuf::from("Assets/scene.scene"),
                temp: Some(PathBuf::from("Assets/.scene.tmp")),
                backup: None,
                delete: false,
                sha256: Some(format!("{:x}", Sha256::digest(b"new"))),
            }],
        };
        write_json_atomic(&temp.path().join(".pbrtui/save-journal.json"), &journal).unwrap();
        recover_journal(temp.path()).unwrap();
        assert_eq!(fs::read(target).unwrap(), b"new");
        assert!(!temp.path().join(".pbrtui/save-journal.json").exists());
    }

    #[test]
    fn test_journal_rolls_back_when_temporary_files_are_missing() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".pbrtui")).unwrap();
        fs::create_dir_all(temp.path().join("Assets")).unwrap();
        fs::write(temp.path().join("Assets/scene.scene"), b"partially written").unwrap();
        fs::write(temp.path().join("Assets/.scene.backup"), b"previous").unwrap();
        let journal = SaveJournal {
            asset_folders: None,
            files: vec![JournalFile {
                path: PathBuf::from("Assets/scene.scene"),
                temp: Some(PathBuf::from("Assets/.scene.missing")),
                backup: Some(PathBuf::from("Assets/.scene.backup")),
                delete: false,
                sha256: Some(format!("{:x}", Sha256::digest(b"next"))),
            }],
        };
        write_json_atomic(&temp.path().join(".pbrtui/save-journal.json"), &journal).unwrap();
        recover_journal(temp.path()).unwrap();
        assert_eq!(
            fs::read(temp.path().join("Assets/scene.scene")).unwrap(),
            b"previous"
        );
        assert!(!temp.path().join(".pbrtui/save-journal.json").exists());
    }

    #[test]
    fn test_published_save_retains_recovery_until_readback_finishes() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".pbrtui")).unwrap();
        fs::create_dir_all(temp.path().join("Assets/obsolete/nested")).unwrap();
        let journal = SaveJournal {
            files: vec![],
            asset_folders: Some(Default::default()),
        };
        let path = temp.path().join(".pbrtui/save-journal.json");
        write_json_atomic(&path, &journal).unwrap();
        apply_journal(temp.path(), &journal).unwrap();
        assert!(path.exists(), "readback failure must retain recovery state");
        assert!(!temp.path().join("Assets/obsolete").exists());
        recover_journal(temp.path()).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn test_folder_cleanup_failure_can_be_recovered_on_reopen() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".pbrtui")).unwrap();
        fs::write(temp.path().join("Assets"), b"blocked").unwrap();
        let journal = SaveJournal {
            files: vec![],
            asset_folders: Some(Default::default()),
        };
        let path = temp.path().join(".pbrtui/save-journal.json");
        write_json_atomic(&path, &journal).unwrap();
        assert!(apply_journal(temp.path(), &journal).is_err());
        assert!(path.exists());
        fs::remove_file(temp.path().join("Assets")).unwrap();
        fs::create_dir(temp.path().join("Assets")).unwrap();
        recover_journal(temp.path()).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn test_save_requires_reopen_when_recovery_is_pending() {
        let temp = tempfile::tempdir().unwrap();
        let document = ProjectDocument::new("Pending");
        let session = ProjectSession::new(document.clone()).unwrap();
        let path = temp.path().join("Pending");
        let mut store =
            FolderStore::create(&path, document.id, "Pending", Revision("0".into())).unwrap();
        fs::write(path.join(".pbrtui/save-journal.json"), b"{}").unwrap();
        let error = store.save(&session.snapshot()).unwrap_err();
        assert!(error.to_string().contains("reopen"));
        assert_eq!(store.manifest.saved_revision, Revision("0".into()));
    }

    #[test]
    fn recovery_replays_deletions_after_their_parent_was_pruned() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        fs::create_dir_all(root.join(".pbrtui")).unwrap();
        fs::create_dir_all(root.join("Assets/old/nested")).unwrap();
        fs::write(root.join("Assets/old/nested/data.bin"), b"old").unwrap();
        let journal = SaveJournal {
            files: vec![JournalFile {
                path: PathBuf::from("Assets/old/nested/data.bin"),
                temp: None,
                backup: None,
                delete: true,
                sha256: None,
            }],
            asset_folders: Some(Default::default()),
        };
        write_json_atomic(&root.join(".pbrtui/save-journal.json"), &journal).unwrap();
        apply_journal(root, &journal).unwrap();
        assert!(!root.join("Assets/old").exists());
        assert!(root.join(".pbrtui/save-journal.json").exists());
        recover_journal(root).unwrap();
        assert!(!root.join(".pbrtui/save-journal.json").exists());
    }

    #[test]
    fn test_external_change_rejects_save() {
        let temp = tempfile::tempdir().unwrap();
        let document = ProjectDocument::new("External");
        let session = ProjectSession::new(document.clone()).unwrap();
        let path = temp.path().join("External");
        let mut store =
            FolderStore::create(&path, document.id, "External", Revision("0".into())).unwrap();
        store.save(&session.snapshot()).unwrap();
        let scene = path.join("Assets/Scenes/External.scene");
        fs::write(&scene, b"external edit").unwrap();
        assert!(matches!(
            store.save(&session.snapshot()),
            Err(ProjectError::SourceChanged(_))
        ));
    }

    #[test]
    fn missing_asset_body_remains_openable_and_keeps_its_meta_on_save() {
        let temp = tempfile::tempdir().unwrap();
        let bytes = b"mesh";
        let id = AssetId::new();
        let mut document = ProjectDocument::new("MissingAsset");
        document.assets.insert(
            id,
            Asset {
                id,
                location: "Meshes/mesh.ply".into(),
                media_type: "application/x-ply".into(),
                sha256: format!("{:x}", Sha256::digest(bytes)),
                size: bytes.len() as u64,
                origin: None,
            },
        );
        let session = ProjectSession::new(document.clone()).unwrap();
        let path = temp.path().join("MissingAsset");
        let store =
            FolderStore::create(&path, document.id, "MissingAsset", Revision("0".into())).unwrap();
        let asset_path = path.join("Assets/Meshes/mesh.ply");
        fs::create_dir_all(asset_path.parent().unwrap()).unwrap();
        fs::write(&asset_path, bytes).unwrap();
        drop(store);
        let mut store = FolderStore::open(&path).unwrap();
        store.save(&session.snapshot()).unwrap();
        drop(store);
        fs::remove_file(&asset_path).unwrap();

        let scan_store = FolderStore::open(&path).unwrap();
        assert_eq!(
            scan_store.scan_assets().unwrap().orphan_meta,
            vec![PathBuf::from("Meshes/mesh.ply.meta")]
        );
        let meta: AssetMeta =
            serde_json::from_slice(&fs::read(path.join("Assets/Meshes/mesh.ply.meta")).unwrap())
                .unwrap();
        assert_eq!(meta.id, id.0);
        assert_eq!(meta.kind, "asset");
        drop(scan_store);
        let (mut reopened_store, reopened) = FolderStore::open_project(&path).unwrap();
        assert!(reopened.document().assets.contains_key(&id));
        reopened_store.save(&reopened.snapshot()).unwrap();
        assert!(!asset_path.exists());
        assert!(PathBuf::from(format!("{}.meta", asset_path.display())).is_file());
    }

    #[test]
    fn cancellation_before_journal_removes_staged_files_and_new_folders() {
        let temp = tempfile::tempdir().unwrap();
        let document = ProjectDocument::new("Cancelled");
        let session = ProjectSession::new(document.clone()).unwrap();
        let path = temp.path().join("Cancelled");
        let mut store =
            FolderStore::create(&path, document.id, "Cancelled", Revision("0".into())).unwrap();
        let calls = std::cell::Cell::new(0);
        let result = store.save_cancellable(&session.snapshot(), || {
            let call = calls.get();
            calls.set(call + 1);
            call == 1
        });
        assert!(matches!(result, Err(ProjectError::Cancelled)));
        assert!(!path.join("Assets/Scenes").exists());
        assert!(!path.join(".pbrtui/save-journal.json").exists());
    }

    #[test]
    fn format_three_publish_cost_stays_linear_in_the_asset_count() {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("source");
        let destination = temporary.path().join("converted");
        let count = 100usize;
        let mut model = Vec::with_capacity(count);
        for index in 0..count {
            let bytes = format!("image bytes {index}");
            model.push((
                bytes.clone(),
                Asset {
                    id: AssetId::new(),
                    location: format!("Images/image{index:03}.png"),
                    media_type: "image/png".into(),
                    sha256: format!("{:x}", Sha256::digest(bytes.as_bytes())),
                    size: bytes.len() as u64,
                    origin: None,
                },
            ));
        }
        let mut document = ProjectDocument::new("Publish cost");
        for (_, asset) in &model {
            document.assets.insert(asset.id, asset.clone());
        }
        document.folders.insert("Images".into());
        let session = ProjectSession::new(document).unwrap();
        let mut folder = FolderStore::create(
            &source,
            session.document().id,
            "Publish cost",
            session.revision(),
        )
        .unwrap();
        for (bytes, asset) in &model {
            folder
                .import_asset(asset, bytes.as_bytes(), || false)
                .unwrap();
        }
        folder.save(&session.snapshot()).unwrap();
        let store = ProjectStore::Folder(folder);
        let snapshot = session.snapshot();
        let plan = FormatThreePlan::new(&snapshot).unwrap();
        import_metrics::reset();
        store
            .publish_format_three(&destination, &snapshot, &plan, || false)
            .unwrap();
        let (scans, reads, walks) = (
            import_metrics::asset_scans(),
            import_metrics::meta_reads(),
            import_metrics::folder_walks(),
        );
        assert!(
            scans <= 10,
            "format-3 publish ran {scans} asset-folder scans for {count} assets"
        );
        assert!(
            reads <= 10 * count,
            "format-3 publish read {reads} sidecars for {count} assets"
        );
        assert!(
            walks <= 6,
            "format-3 publish ran {walks} baseline walks for {count} assets"
        );
    }

    /// Assets distinguished by id and location only: every entry may share
    /// one payload, like assets deduplicated into one spool object do.
    const BATCH_PAYLOAD: &[u8] = b"shared batch payload";

    fn batch_model(count: usize, prefix: &str) -> Vec<Asset> {
        (0..count)
            .map(|index| Asset {
                id: AssetId::new(),
                location: format!("Images/{prefix}{index:03}.png"),
                media_type: "image/png".into(),
                sha256: format!("{:x}", Sha256::digest(BATCH_PAYLOAD)),
                size: BATCH_PAYLOAD.len() as u64,
                origin: None,
            })
            .collect()
    }

    #[test]
    fn batch_import_scans_the_asset_folder_once() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("batch");
        let mut store =
            FolderStore::create_v3(&path, ProjectId::new(), "batch", Revision::initial()).unwrap();
        let model = batch_model(200, "fresh");
        import_metrics::reset();
        store
            .import_assets(
                &model,
                |_: &Asset| Ok(std::io::Cursor::new(BATCH_PAYLOAD)),
                || false,
            )
            .unwrap();
        let (scans, reads, walks) = (
            import_metrics::asset_scans(),
            import_metrics::meta_reads(),
            import_metrics::folder_walks(),
        );
        assert_eq!(scans, 1, "the duplicate scan runs once for the whole batch");
        assert_eq!(reads, 0, "an empty store imports without sidecar reads");
        assert_eq!(walks, 1, "the baseline walk runs once after the batch");
        assert_eq!(store.read_document().unwrap().assets.len(), 200);
    }

    #[test]
    fn batch_import_reads_every_existing_sidecar_once() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("batch");
        let mut store =
            FolderStore::create_v3(&path, ProjectId::new(), "batch", Revision::initial()).unwrap();
        let seeded = batch_model(10, "seed");
        for asset in &seeded {
            store.import_asset(asset, BATCH_PAYLOAD, || false).unwrap();
        }
        let additions = batch_model(5, "more");
        import_metrics::reset();
        store
            .import_assets(
                &additions,
                |_: &Asset| Ok(std::io::Cursor::new(BATCH_PAYLOAD)),
                || false,
            )
            .unwrap();
        let (scans, reads, walks) = (
            import_metrics::asset_scans(),
            import_metrics::meta_reads(),
            import_metrics::folder_walks(),
        );
        assert_eq!(scans, 1);
        assert_eq!(reads, 10, "each pre-existing sidecar is read once");
        assert_eq!(walks, 1);
        assert_eq!(store.read_document().unwrap().assets.len(), 15);
    }

    #[test]
    fn batch_import_rejects_an_id_that_already_exists_in_the_project() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("batch");
        let mut store =
            FolderStore::create_v3(&path, ProjectId::new(), "batch", Revision::initial()).unwrap();
        let existing = batch_model(1, "kept");
        store
            .import_asset(&existing[0], BATCH_PAYLOAD, || false)
            .unwrap();
        let mut duplicate = batch_model(1, "dup");
        duplicate[0].id = existing[0].id;
        let trailing = batch_model(1, "tail");
        let batch = vec![duplicate[0].clone(), trailing[0].clone()];
        let error = store
            .import_assets(
                &batch,
                |_: &Asset| Ok(std::io::Cursor::new(BATCH_PAYLOAD)),
                || false,
            )
            .unwrap_err();
        assert!(error.to_string().contains("already exists"), "{error:?}");
        let document = store.read_document().unwrap();
        assert_eq!(document.assets.len(), 1, "the batch wrote nothing");
        assert!(!path.join("Assets/Images/dup000.png").exists());
        assert!(!path.join("Assets/Images/tail000.png").exists());
    }

    #[test]
    fn batch_import_rejects_a_duplicate_id_within_the_batch() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("batch");
        let mut store =
            FolderStore::create_v3(&path, ProjectId::new(), "batch", Revision::initial()).unwrap();
        let first = batch_model(1, "first");
        let mut second = batch_model(1, "second");
        second[0].id = first[0].id;
        let batch = vec![first[0].clone(), second[0].clone()];
        let error = store
            .import_assets(
                &batch,
                |_: &Asset| Ok(std::io::Cursor::new(BATCH_PAYLOAD)),
                || false,
            )
            .unwrap_err();
        assert!(error.to_string().contains("already exists"), "{error:?}");
        assert_eq!(store.read_document().unwrap().assets.len(), 0);
        assert!(
            fs::read_dir(path.join("Assets")).unwrap().next().is_none(),
            "the failed batch leaves no files or folders behind"
        );
    }

    #[test]
    fn a_mid_batch_failure_cleans_every_file_the_batch_wrote() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("batch");
        let mut store =
            FolderStore::create_v3(&path, ProjectId::new(), "batch", Revision::initial()).unwrap();
        let mut model = batch_model(3, "flaky");
        model[1].sha256 = "0".repeat(64);
        let error = store
            .import_assets(
                &model,
                |_: &Asset| Ok(std::io::Cursor::new(BATCH_PAYLOAD)),
                || false,
            )
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not match its project model"),
            "{error:?}"
        );
        assert_eq!(
            store.read_document().unwrap().assets.len(),
            0,
            "the assets imported before the failure are rolled back"
        );
        assert!(
            fs::read_dir(path.join("Assets")).unwrap().next().is_none(),
            "the failed batch leaves no files or folders behind"
        );
        // The external-change baseline survived the rollback, so the store
        // stays usable without a reopen.
        let retry = batch_model(1, "retry");
        store
            .import_assets(
                &retry,
                |_: &Asset| Ok(std::io::Cursor::new(BATCH_PAYLOAD)),
                || false,
            )
            .unwrap();
        assert!(path.join("Assets/Images/retry000.png").is_file());
    }

    #[test]
    fn a_cancelled_batch_leaves_nothing_behind() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("batch");
        let mut store =
            FolderStore::create_v3(&path, ProjectId::new(), "batch", Revision::initial()).unwrap();
        let model = batch_model(3, "stopped");
        let first_body = path.join("Assets/Images/stopped000.png");
        let load = |_: &Asset| Ok(std::io::Cursor::new(BATCH_PAYLOAD));
        let error = {
            let cancelled = || first_body.exists();
            store.import_assets(&model, load, cancelled).unwrap_err()
        };
        assert!(matches!(error, ProjectError::Cancelled));
        assert!(!first_body.exists(), "the first asset is rolled back");
        assert!(
            fs::read_dir(path.join("Assets")).unwrap().next().is_none(),
            "a cancelled batch leaves no files or folders behind"
        );
        let after = batch_model(1, "after");
        store.import_assets(&after, load, || false).unwrap();
        assert!(path.join("Assets/Images/after000.png").is_file());
    }
}
