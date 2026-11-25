use project::*;
use sha2::{Digest, Sha256};
use std::fs;

fn asset(location: &str, bytes: &[u8]) -> Asset {
    Asset {
        id: AssetId::new(),
        location: location.into(),
        media_type: "image/png".into(),
        sha256: format!("{:x}", Sha256::digest(bytes)),
        size: bytes.len() as u64,
        origin: None,
    }
}

fn texture(id: u128, image: AssetId, scale: f64) -> Resource {
    Resource {
        id: ResourceId(uuid::Uuid::from_u128(id)),
        label: format!("old{id}"),
        location: format!("Textures/old{id}.texture"),
        edition: Revision::initial(),
        kind: ResourceKind::Texture,
        texture_value_type: Some(TextureValueType::Spectrum),
        implementation: "imagemap".into(),
        parameters: vec![
            Parameter {
                name: "filename".into(),
                value_type: "string".into(),
                color_space: ColorSpaceBinding::Inherit,
                values: ParameterValues::Assets(vec![image]),
            },
            Parameter::floats("scale", [scale]),
        ],
        definition_scope: None,
        declared_in: None,
        origin: None,
    }
}

#[test]
fn shared_images_migrate_once_with_every_id_location_and_reference_kept() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("source");
    let destination = temporary.path().join("converted");
    let bytes = b"original image";
    let shared = asset("Images/wood.png", bytes);
    let bare = asset("Images/bare.png", b"bare image");
    let first = texture(1, shared.id, 2.0);
    let second = texture(2, shared.id, 3.0);
    let mut document = ProjectDocument::new("Migration");
    document.assets.insert(shared.id, shared.clone());
    document.assets.insert(bare.id, bare.clone());
    document.resources.insert(first.id, first.clone());
    document.resources.insert(second.id, second.clone());
    let mut material = first.clone();
    material.id = ResourceId(uuid::Uuid::from_u128(3));
    material.kind = ResourceKind::Material;
    material.implementation = "diffuse".into();
    material.texture_value_type = None;
    material.location = "Materials/mat.material".into();
    material.parameters = vec![Parameter {
        name: "normalmap".into(),
        value_type: "string".into(),
        color_space: ColorSpaceBinding::Inherit,
        values: ParameterValues::Assets(vec![shared.id]),
    }];
    document.resources.insert(material.id, material.clone());
    document
        .folders
        .extend(["Images", "Textures", "Materials"].map(str::to_owned));
    let session = ProjectSession::new(document).unwrap();
    let mut folder = FolderStore::create(
        &source,
        session.document().id,
        "Migration",
        session.revision(),
    )
    .unwrap();
    folder.import_asset(&shared, &bytes[..], || false).unwrap();
    folder
        .import_asset(&bare, &b"bare image"[..], || false)
        .unwrap();
    folder.save(&session.snapshot()).unwrap();
    let original_meta = fs::read(source.join("Assets/Images/wood.png.meta")).unwrap();
    let original_manifest = fs::read(source.join("project.pbrtui.json")).unwrap();
    let store = ProjectStore::Folder(folder);
    let snapshot = session.snapshot();
    let plan = FormatThreePlan::new(&snapshot).unwrap();
    let migrated = plan.snapshot().document();
    assert_eq!(migrated, session.document(), "ids and locations are kept");
    let (copy, receipt) = store
        .publish_format_three(&destination, &snapshot, &plan, || false)
        .unwrap();
    let reopened = copy.read_document().unwrap();
    assert_eq!(ContentId::of(&reopened), *receipt.content_id());
    assert_eq!(ContentId::of(&reopened), ContentId::of(migrated));
    assert_eq!(reopened.assets.len(), 2, "shared images are not copied");
    assert_eq!(
        fs::read(copy.asset_path(shared.id).unwrap()).unwrap(),
        bytes
    );
    for id in [first.id, second.id] {
        assert_eq!(
            reopened.resources[&id].parameters[0].values,
            ParameterValues::Assets(vec![shared.id])
        );
    }
    assert!(destination.join("Assets/Textures/old1.texture").exists());
    assert!(destination.join("Assets/Textures/old2.texture").exists());
    assert!(destination.join("Assets/Images/wood.png").exists());
    assert_eq!(
        fs::read(source.join("Assets/Images/wood.png.meta")).unwrap(),
        original_meta
    );
    assert_eq!(
        fs::read(source.join("project.pbrtui.json")).unwrap(),
        original_manifest
    );
}

#[test]
fn a_migrated_sidecar_keeps_the_import_provenance_of_its_source() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("source");
    let destination = temporary.path().join("converted");
    let bytes = b"provenance bytes";
    let import = ImportId::new();
    let root = OccurrenceId::new();
    let source_id = SourceId::new();
    let mut document = ProjectDocument::new("Provenance");
    document.imports.insert(
        import,
        ImportRecord {
            id: import,
            importer_version: "test".into(),
            dialect: ImportDialect::PbrtR4,
            renderer_revision: None,
            diagnostics: vec![],
            root,
            sources: [(
                source_id,
                ImportedSource {
                    original_path: "/scene.pbrt".into(),
                    original: ObjectRef {
                        sha256: "0".repeat(64),
                        size: 4,
                    },
                    expanded: None,
                },
            )]
            .into(),
            occurrences: [(
                root,
                SourceOccurrence {
                    source: source_id,
                    parent: None,
                    callsite: None,
                },
            )]
            .into(),
        },
    );
    let asset = Asset {
        id: AssetId::new(),
        location: "Images/kept.png".into(),
        media_type: "image/png".into(),
        sha256: format!("{:x}", Sha256::digest(bytes)),
        size: bytes.len() as u64,
        origin: Some(AssetOrigin {
            logical_path: "textures/kept.png".into(),
            origin: Some(Origin {
                import,
                occurrence: root,
                span: SourceSpan {
                    offset: 0,
                    length: 4,
                    line: 3,
                    column: 1,
                },
            }),
        }),
    };
    document.assets.insert(asset.id, asset.clone());
    document.folders.insert("Images".into());
    let session = ProjectSession::new(document).unwrap();
    let mut folder = FolderStore::create(
        &source,
        session.document().id,
        "Provenance",
        session.revision(),
    )
    .unwrap();
    folder
        .import_asset(&session.document().assets[&asset.id], &bytes[..], || false)
        .unwrap();
    let store = ProjectStore::Folder(folder);
    let snapshot = session.snapshot();
    let plan = FormatThreePlan::new(&snapshot).unwrap();
    let (copy, _) = store
        .publish_format_three(&destination, &snapshot, &plan, || false)
        .unwrap();
    let sidecar: serde_json::Value =
        serde_json::from_slice(&fs::read(destination.join("Assets/Images/kept.png.meta")).unwrap())
            .unwrap();
    assert_eq!(
        sidecar["origin"]["import"].as_str().map(str::to_owned),
        Some(import.to_string()),
        "the migrated sidecar names the import its source came from"
    );
    assert_eq!(sidecar["origin"]["logical_path"], "textures/kept.png");
    let reopened = copy.read_document().unwrap();
    assert_eq!(
        reopened.assets[&asset.id]
            .origin
            .as_ref()
            .map(|origin| origin.logical_path.clone()),
        Some("textures/kept.png".to_owned())
    );
}

#[test]
fn cancellation_does_not_publish_and_the_same_plan_can_be_retried() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("source");
    let destination = temporary.path().join("converted");
    let document = ProjectDocument::new("Cancel");
    let snapshot = ProjectSession::new(document.clone()).unwrap().snapshot();
    let mut folder =
        FolderStore::create(&source, document.id, "Cancel", Revision::initial()).unwrap();
    folder.save(&snapshot).unwrap();
    let store = ProjectStore::Folder(folder);
    let plan = FormatThreePlan::new(&snapshot).unwrap();
    assert!(
        store
            .publish_format_three(&destination, &snapshot, &plan, || true)
            .is_err()
    );
    assert!(!destination.exists());
    let (_, receipt) = store
        .publish_format_three(&destination, &snapshot, &plan, || false)
        .unwrap();
    assert_eq!(
        *receipt.content_id(),
        ContentId::of(plan.snapshot().document())
    );
}

#[test]
fn migration_at_the_assets_root_does_not_create_absolute_paths() {
    let mut namer = project::layout::LocationNamer::new();
    namer.reserve("wood.png");
    assert_eq!(namer.take("", "wood", "png"), "wood_1.png");
}
