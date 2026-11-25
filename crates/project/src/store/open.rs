use super::*;
impl ObjectStore {
    /// Create a private content-addressed spool for an import operation.
    pub fn create(path: &Path) -> Result<Self> {
        fs::create_dir(path)?;
        let directory = protocol::path::canonicalize(path)?;
        let lock = lock(&directory)?;
        fs::create_dir(directory.join("objects"))?;
        fs::create_dir(directory.join("generations"))?;
        crate::sync_path(&directory)?;
        if let Some(parent) = directory.parent() {
            crate::sync_path(parent)?;
        }
        Ok(Self {
            directory,
            _lock: lock,
        })
    }

    /// Open a format-1 project read-only. Format 1 is converted to the new
    /// model in memory; saving it creates a new project folder.
    pub fn open_legacy(path: &Path) -> Result<(Self, ProjectSession)> {
        let directory = protocol::path::canonicalize(path)?;
        regular_directory(&directory)?;
        let lock = lock(&directory)?;
        regular_directory(&directory.join("objects"))?;
        regular_directory(&directory.join("generations"))?;
        let bytes = read_bounded(&directory.join(MANIFEST), MAX_RECORD_BYTES)?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        let version = value["format_version"]
            .as_u64()
            .ok_or_else(|| invalid("missing project format version"))?;
        if version != PROJECT_FORMAT_VERSION as u64 {
            return Err(ProjectError::UnsupportedVersion(version));
        }
        let manifest: Manifest = serde_json::from_slice(&bytes)?;
        if manifest.format != FORMAT {
            return Err(invalid("unrecognized project format"));
        }
        let revision = crate::validation::revision(&manifest.saved_revision)?;
        if manifest
            .files
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>()
            != ["assets.json", "imports.json", "scene.jsonl"]
        {
            return Err(invalid(
                "project generation must contain scene, assets and imports",
            ));
        }
        let generation = directory
            .join("generations")
            .join(manifest.head.to_string());
        regular_directory(&generation)?;
        let document = legacy_v1::read(&generation, &manifest)?;
        document.validate()?;
        let store = Self {
            directory,
            _lock: lock,
        };
        store.verify_objects(&document)?;
        Ok((store, ProjectSession::loaded(document, revision)?))
    }
}

impl ProjectStore {
    pub fn open(path: &Path) -> Result<(Self, ProjectSession)> {
        let manifest_path = path.join(MANIFEST);
        let manifest_bytes = match read_bounded(&manifest_path, MAX_RECORD_BYTES) {
            Ok(bytes) => bytes,
            Err(ProjectError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(invalid("project manifest is missing"));
            }
            Err(error) => return Err(error),
        };
        let header: serde_json::Value = serde_json::from_slice(&manifest_bytes)?;
        match header.get("format").and_then(serde_json::Value::as_str) {
            Some("pbrt-ui.project/2" | "pbrt-ui.project/3") => {
                let (store, session) = FolderStore::open_project(path)?;
                return Ok((Self::Folder(store), session));
            }
            Some("pbrt-ui-project") => {}
            Some(format) if format.starts_with("pbrt-ui.project/") => {
                let version = format
                    .rsplit('/')
                    .next()
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(0);
                return Err(ProjectError::UnsupportedVersion(version));
            }
            _ => return Err(invalid("unrecognized project format")),
        }
        let (objects, session) = ObjectStore::open_legacy(path)?;
        let assets = session
            .document()
            .assets
            .iter()
            .map(|(id, asset)| (*id, asset.object()))
            .collect();
        Ok((Self::Legacy(LegacyStore { objects, assets }), session))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;
    use std::collections::BTreeMap;

    #[derive(Serialize)]
    struct LegacyDocument {
        id: ProjectId,
        title: String,
        scene: LegacyScene,
        assets: BTreeMap<AssetId, serde_json::Value>,
        imports: BTreeMap<ImportId, serde_json::Value>,
    }
    #[derive(Serialize)]
    struct LegacyScene {
        options: Vec<NodeId>,
        world: NodeId,
        nodes: BTreeMap<NodeId, LegacyNode>,
        resources: BTreeMap<ResourceId, serde_json::Value>,
    }
    #[derive(Serialize)]
    struct LegacyNode {
        id: NodeId,
        label: String,
        edition: Revision,
        origin: Option<Origin>,
        data: LegacyNodeKind,
    }
    #[derive(Serialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    enum LegacyNodeKind {
        Scope {
            scope: ScopeKind,
            children: Vec<NodeId>,
        },
    }

    /// A legacy format-1 project's `.nvdb` asset converts to the media
    /// folder with the nanovdb media type, the same mapping fresh imports
    /// and batch placements use (folder doc: `Media/`, original
    /// basename).
    #[test]
    fn legacy_nvdb_assets_convert_to_the_media_folder() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("legacy");
        let id = ProjectId::new();
        let world = NodeId::new();
        let generation_id = uuid::Uuid::new_v4();
        let generation = path.join("generations").join(generation_id.to_string());
        let asset_id = AssetId::new();
        let payload = b"legacy nvdb payload";
        let mut objects = ObjectStore::create(&path).unwrap();
        let object = objects.put_object(&payload[..]).unwrap();
        drop(objects);
        fs::create_dir(&generation).unwrap();
        let document = LegacyDocument {
            id,
            title: "Legacy".into(),
            scene: LegacyScene {
                options: vec![],
                world,
                nodes: BTreeMap::from([(
                    world,
                    LegacyNode {
                        id: world,
                        label: "World".into(),
                        edition: Revision::initial(),
                        origin: None,
                        data: LegacyNodeKind::Scope {
                            scope: ScopeKind::World,
                            children: vec![],
                        },
                    },
                )]),
                resources: BTreeMap::new(),
            },
            assets: BTreeMap::from([(
                asset_id,
                serde_json::json!({
                    "id": asset_id,
                    "logical_path": "volumes/grid.nvdb",
                    "media_type": "application/octet-stream",
                    "object": {
                        "sha256": object.sha256,
                        "size": object.size,
                    },
                    "origin": null,
                }),
            )]),
            imports: BTreeMap::new(),
        };
        let scene = serde_json::json!({
            "record":"scene", "project_id":id, "title":"Legacy", "options":[], "world":world
        });
        let node = serde_json::json!({
            "record":"node",
            "node": {
                "id":world,
                "label":"World",
                "edition":"0",
                "origin":null,
                "data":{"kind":"scope","scope":"world","children":[]}
            }
        });
        let mut scene_bytes = serde_json::to_vec(&scene).unwrap();
        scene_bytes.push(b'\n');
        scene_bytes.extend(serde_json::to_vec(&node).unwrap());
        scene_bytes.push(b'\n');
        let assets_bytes = serde_json::to_vec(&document.assets).unwrap();
        let imports_bytes = b"{}".to_vec();
        fs::write(generation.join("scene.jsonl"), &scene_bytes).unwrap();
        fs::write(generation.join("assets.json"), &assets_bytes).unwrap();
        fs::write(generation.join("imports.json"), &imports_bytes).unwrap();
        let files = BTreeMap::from([
            ("assets.json".into(), object_ref(&assets_bytes)),
            ("imports.json".into(), object_ref(&imports_bytes)),
            ("scene.jsonl".into(), object_ref(&scene_bytes)),
        ]);
        let manifest = Manifest {
            format: FORMAT.into(),
            format_version: PROJECT_FORMAT_VERSION,
            project_id: id,
            saved_revision: Revision::initial(),
            head: generation_id,
            model_sha256: format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&document).unwrap())
            ),
            files,
        };
        fs::write(path.join(MANIFEST), serde_json::to_vec(&manifest).unwrap()).unwrap();

        let (_legacy, session) = ProjectStore::open(&path).unwrap();
        let converted = session
            .document()
            .assets
            .values()
            .find(|asset| asset.location.ends_with("grid.nvdb"))
            .unwrap_or_else(|| panic!("assets: {:?}", session.document().assets));
        assert_eq!(converted.media_type, "application/x-nanovdb");
        assert_eq!(converted.location, "Media/grid.nvdb");
    }

    #[test]
    fn legacy_project_opens_read_only_and_can_be_saved_as_folder() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("legacy");
        let id = ProjectId::new();
        let world = NodeId::new();
        let objects = ObjectStore::create(&path).unwrap();
        drop(objects);
        let generation_id = uuid::Uuid::new_v4();
        let generation = path.join("generations").join(generation_id.to_string());
        fs::create_dir(&generation).unwrap();
        let document = LegacyDocument {
            id,
            title: "Legacy".into(),
            scene: LegacyScene {
                options: vec![],
                world,
                nodes: BTreeMap::from([(
                    world,
                    LegacyNode {
                        id: world,
                        label: "World".into(),
                        edition: Revision::initial(),
                        origin: None,
                        data: LegacyNodeKind::Scope {
                            scope: ScopeKind::World,
                            children: vec![],
                        },
                    },
                )]),
                resources: BTreeMap::new(),
            },
            assets: BTreeMap::new(),
            imports: BTreeMap::new(),
        };
        let scene = serde_json::json!({
            "record":"scene", "project_id":id, "title":"Legacy", "options":[], "world":world
        });
        let node = serde_json::json!({
            "record":"node",
            "node": {
                "id":world,
                "label":"World",
                "edition":"0",
                "origin":null,
                "data":{"kind":"scope","scope":"world","children":[]}
            }
        });
        let mut scene_bytes = serde_json::to_vec(&scene).unwrap();
        scene_bytes.push(b'\n');
        scene_bytes.extend(serde_json::to_vec(&node).unwrap());
        scene_bytes.push(b'\n');
        let assets_bytes = b"{}".to_vec();
        let imports_bytes = b"{}".to_vec();
        fs::write(generation.join("scene.jsonl"), &scene_bytes).unwrap();
        fs::write(generation.join("assets.json"), &assets_bytes).unwrap();
        fs::write(generation.join("imports.json"), &imports_bytes).unwrap();
        let files = BTreeMap::from([
            ("assets.json".into(), object_ref(&assets_bytes)),
            ("imports.json".into(), object_ref(&imports_bytes)),
            ("scene.jsonl".into(), object_ref(&scene_bytes)),
        ]);
        let manifest = Manifest {
            format: FORMAT.into(),
            format_version: PROJECT_FORMAT_VERSION,
            project_id: id,
            saved_revision: Revision::initial(),
            head: generation_id,
            model_sha256: format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&document).unwrap())
            ),
            files,
        };
        fs::write(path.join(MANIFEST), serde_json::to_vec(&manifest).unwrap()).unwrap();

        let (mut legacy, session) = ProjectStore::open(&path).unwrap();
        assert!(legacy.is_read_only());
        assert!(matches!(
            legacy.save_as(&temporary.path().join("folder"), &session.snapshot()),
            Ok(ProjectStore::Folder(_))
        ));
        assert!(matches!(
            legacy.save(&session.snapshot()),
            Err(ProjectError::SaveUnavailable)
        ));
    }

    fn object_ref(bytes: &[u8]) -> ObjectRef {
        ObjectRef {
            sha256: format!("{:x}", Sha256::digest(bytes)),
            size: bytes.len() as u64,
        }
    }
}
