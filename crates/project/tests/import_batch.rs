use project::*;
use sha2::{Digest, Sha256};

fn batch() -> (PreparedImport, AssetId) {
    named_batch("image.png")
}

/// A single image import: the image Asset alone, no Texture.
fn named_batch(file: &str) -> (PreparedImport, AssetId) {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join(file);
    std::fs::write(&source, b"immutable image bytes").unwrap();
    let asset = Asset {
        id: AssetId::new(),
        location: format!("Images/{file}"),
        media_type: "image/png".into(),
        sha256: format!("{:x}", Sha256::digest(b"immutable image bytes")),
        size: 21,
        origin: None,
    };
    let mut staging = ImportStaging::new().unwrap();
    let lease = staging.stage_file(&asset, &source).unwrap();
    let mut document = ProjectDocument::new_empty("Detached");
    document.folders.insert("Images".into());
    let id = asset.id;
    document.assets.insert(asset.id, asset);
    (PreparedImport::new(document, vec![lease]).unwrap(), id)
}

#[test]
fn import_is_one_undo_and_retains_bytes_without_the_original_source() {
    let (batch, asset) = batch();
    let mut session = ProjectSession::new(ProjectDocument::new_empty("Project")).unwrap();
    let initial = ContentId::of(session.document());
    let change = session
        .apply(&session.revision(), &[Edit::ImportBatch { batch }])
        .unwrap();
    assert_eq!(change.created_items, vec![ItemId::Asset(asset)]);
    assert_eq!(session.revision(), Revision::from_u64(1));
    assert!(session.active_scene().is_none());
    let snapshot = session.snapshot();
    assert_eq!(
        std::fs::read(snapshot.payload_by(asset).unwrap().path()).unwrap(),
        b"immutable image bytes"
    );
    session.undo(&session.revision()).unwrap();
    assert_eq!(ContentId::of(session.document()), initial);
    assert!(!session.can_undo());
    session.redo(&session.revision()).unwrap();
    assert!(session.document().assets.contains_key(&asset));
    assert_eq!(
        std::fs::read(session.snapshot().payload_by(asset).unwrap().path()).unwrap(),
        b"immutable image bytes"
    );
}

#[test]
fn imported_bytes_survive_save_undo_save_and_redo() {
    let temporary = tempfile::tempdir().unwrap();
    let (batch, asset) = batch();
    let mut session = ProjectSession::new(ProjectDocument::new_empty("Project")).unwrap();
    let mut store = FolderStore::create_v3(
        &temporary.path().join("project"),
        session.document().id,
        "Project",
        Revision::initial(),
    )
    .unwrap();
    let receipt = store.save(&session.snapshot()).unwrap();
    session.mark_saved(&receipt).unwrap();
    session
        .apply(&session.revision(), &[Edit::ImportBatch { batch }])
        .unwrap();
    let receipt = store.save(&session.snapshot()).unwrap();
    session.mark_saved(&receipt).unwrap();
    assert!(!session.is_dirty());
    session.undo(&session.revision()).unwrap();
    assert!(session.is_dirty());
    let receipt = store.save(&session.snapshot()).unwrap();
    session.mark_saved(&receipt).unwrap();
    assert!(!session.is_dirty());
    session.redo(&session.revision()).unwrap();
    assert!(session.is_dirty());
    let receipt = store.save(&session.snapshot()).unwrap();
    session.mark_saved(&receipt).unwrap();
    assert!(!session.is_dirty());
    let reopened = store.read_document().unwrap();
    assert!(reopened.assets.contains_key(&asset));
    assert!(reopened.resources.is_empty());
    assert_eq!(
        std::fs::read(store.asset_path(asset).unwrap()).unwrap(),
        b"immutable image bytes"
    );
}

#[test]
fn a_colliding_import_leaves_model_revision_and_history_unchanged() {
    let mut session = ProjectSession::new(ProjectDocument::new_empty("Project")).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::ImportBatch { batch: batch().0 }],
        )
        .unwrap();
    let before = session.document().clone();
    let revision = session.revision();
    assert!(
        session
            .apply(&revision, &[Edit::ImportBatch { batch: batch().0 }])
            .is_err()
    );
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), revision);
    session.undo(&session.revision()).unwrap();
    assert!(!session.can_undo());
    assert!(session.document().assets.is_empty());
}

#[test]
fn live_import_payload_survives_history_eviction_before_save() {
    let (batch, asset) = batch();
    let mut session = ProjectSession::new(ProjectDocument::new_empty("Project")).unwrap();
    session
        .apply(&session.revision(), &[Edit::ImportBatch { batch }])
        .unwrap();
    for number in 0..129 {
        session
            .apply(
                &session.revision(),
                &[Edit::CreateFolder {
                    path: format!("folder{number}"),
                }],
            )
            .unwrap();
    }
    assert_eq!(
        std::fs::read(session.snapshot().payload_by(asset).unwrap().path()).unwrap(),
        b"immutable image bytes"
    );
}

#[test]
fn imported_scene_and_import_record_undo_together_without_opening_the_scene() {
    let mut source = ProjectDocument::new("Imported");
    source.default_scene = None;
    let scene = *source.scenes.keys().next().unwrap();
    let import = ImportId::new();
    let occurrence = OccurrenceId::new();
    let file = SourceId::new();
    source.imports.insert(
        import,
        ImportRecord {
            id: import,
            importer_version: "D4".into(),
            dialect: ImportDialect::PbrtR4,
            renderer_revision: None,
            diagnostics: vec![],
            root: occurrence,
            sources: [(
                file,
                ImportedSource {
                    original_path: "/removed.pbrt".into(),
                    original: ObjectRef {
                        sha256: "0".repeat(64),
                        size: 0,
                    },
                    expanded: None,
                },
            )]
            .into(),
            occurrences: [(
                occurrence,
                SourceOccurrence {
                    source: file,
                    parent: None,
                    callsite: None,
                },
            )]
            .into(),
        },
    );
    let batch = PreparedImport::new(source, vec![]).unwrap();
    let mut session = ProjectSession::new(ProjectDocument::new("Existing")).unwrap();
    let active = session.active_scene();
    let default = session.document().default_scene;
    session
        .apply(&session.revision(), &[Edit::ImportBatch { batch }])
        .unwrap();
    assert_eq!(session.active_scene(), active);
    assert_eq!(session.document().default_scene, default);
    assert!(session.document().scenes.contains_key(&scene));
    assert!(session.document().imports.contains_key(&import));
    session.undo(&session.revision()).unwrap();
    assert!(!session.document().scenes.contains_key(&scene));
    assert!(!session.document().imports.contains_key(&import));
    session.redo(&session.revision()).unwrap();
    assert!(session.document().imports.contains_key(&import));
    assert_eq!(session.active_scene(), active);
}

#[test]
fn object_definition_nodes_are_restored_with_the_import_entry() {
    let mut source = ProjectDocument::new_empty("Definition");
    let node = Node::new(
        "Tree",
        NodeKind::Scope {
            scope: ScopeKind::ObjectDefinition,
            children: vec![],
            owned_object: None,
            parent_camera: None,
        },
    );
    let node_id = node.id;
    let resource = Resource {
        id: ResourceId::new(),
        label: "Tree".into(),
        location: "Objects/Tree.object".into(),
        edition: Revision::initial(),
        kind: ResourceKind::ObjectDefinition,
        texture_value_type: None,
        implementation: String::new(),
        parameters: vec![],
        definition_scope: Some(node.id),
        declared_in: None,
        origin: None,
    };
    source.definition_nodes.insert(node.id, node);
    source.resources.insert(resource.id, resource);
    source.folders.insert("Objects".into());
    let batch = PreparedImport::new(source, vec![]).unwrap();
    let mut session = ProjectSession::new(ProjectDocument::new_empty("Project")).unwrap();
    session
        .apply(&session.revision(), &[Edit::ImportBatch { batch }])
        .unwrap();
    assert!(session.document().definition_nodes.contains_key(&node_id));
    session.undo(&session.revision()).unwrap();
    assert!(session.document().definition_nodes.is_empty());
    session.redo(&session.revision()).unwrap();
    assert!(session.document().definition_nodes.contains_key(&node_id));
}

#[test]
fn staging_cancellation_does_not_publish_a_partial_payload() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("data");
    let bytes = vec![42; 128 * 1024];
    std::fs::write(&path, &bytes).unwrap();
    let asset = Asset {
        id: AssetId::new(),
        location: "Data/file.bin".into(),
        media_type: "application/octet-stream".into(),
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        size: bytes.len() as u64,
        origin: None,
    };
    let mut staging = ImportStaging::new().unwrap();
    let checks = std::cell::Cell::new(0);
    let error = staging
        .stage_file_cancellable(&asset, &path, || {
            checks.set(checks.get() + 1);
            checks.get() > 3
        })
        .unwrap_err();
    assert!(matches!(error, ProjectError::Cancelled));
    let lease = staging.stage_file(&asset, &path).unwrap();
    assert_eq!(std::fs::read(lease.path()).unwrap(), bytes);
}

#[test]
fn single_imports_number_conflicts_in_the_requested_folder_without_changing_identity() {
    let mut document = ProjectDocument::new_empty("Target");
    document.folders.remove("Textures");
    document.folders.insert("textures".into());
    let mut target = ProjectSession::new(document).unwrap();
    for (index, expected) in ["textures/image.png", "textures/image_1.png"]
        .into_iter()
        .enumerate()
    {
        let (batch, asset) = batch();
        let placed = batch
            .placed_for(target.document(), Some("textures"))
            .unwrap();
        assert_eq!(placed.document().assets[&asset].location, expected);
        assert!(placed.document().resources.is_empty());
        target
            .apply(&target.revision(), &[Edit::ImportBatch { batch: placed }])
            .unwrap();
        assert_eq!(target.document().assets.len(), index + 1);
    }
    let (batch, asset) = batch();
    assert_eq!(
        batch
            .placed_for(target.document(), Some(""))
            .unwrap()
            .document()
            .assets[&asset]
            .location,
        "image.png"
    );
    assert!(
        batch
            .placed_for(target.document(), Some("missing"))
            .is_err()
    );
}

/// Folders have no fixed role, so a single import has no default folder.
#[test]
fn single_imports_require_an_explicit_folder() {
    let (batch, _) = batch();
    assert!(
        batch
            .placed_for(&ProjectDocument::new_empty("Target"), None)
            .is_err()
    );
}

/// The job copy budget is 8 GiB, inclusive at the cap and
/// rejecting one byte over, both at seeding and as staged assets add up.
/// A stem that already ends in digits gets the conflict
/// suffix appended to the whole stem - importing `image_1.png` onto an
/// existing `image_1.png` lands at `image_1_1.png`, never at `image_2.png`
/// (which would rewrite the name instead of numbering it).
#[test]
fn placement_suffixes_trailing_digit_stems_as_a_whole() {
    let mut document = ProjectDocument::new_empty("Target");
    document.folders.remove("Textures");
    document.folders.insert("textures".into());
    let mut target = ProjectSession::new(document).unwrap();
    let (existing, _) = named_batch("image_1.png");
    let placed = existing
        .placed_for(target.document(), Some("textures"))
        .unwrap();
    target
        .apply(&target.revision(), &[Edit::ImportBatch { batch: placed }])
        .unwrap();
    for (index, (source, expected)) in [
        // Importing image_1.png over the existing image_1.png numbers the
        // whole stem; it must never rewrite the name to image_2.png.
        ("image_1.png", "textures/image_1_1.png"),
        // A stem that is itself a collision suffix extends again as a
        // whole: image_1_1.png collides to image_1_1_1.png.
        ("image_1_1.png", "textures/image_1_1_1.png"),
    ]
    .into_iter()
    .enumerate()
    {
        let (batch, asset) = named_batch(source);
        let placed = batch
            .placed_for(target.document(), Some("textures"))
            .unwrap();
        assert_eq!(
            placed.document().assets[&asset].location,
            expected,
            "collision numbering must extend the trailing-digit stem"
        );
        target
            .apply(&target.revision(), &[Edit::ImportBatch { batch: placed }])
            .unwrap();
        assert_eq!(target.document().assets.len(), index + 2);
    }
}

/// Explicit folders are case-strict. A variant of an
/// existing folder's name is not that folder and is rejected. (A document
/// cannot hold two folders differing only in case - validation normalizes
/// locations - so an accepted explicit folder always binds to the exact
/// requested string.)
#[test]
fn placement_keeps_explicit_folders_case_strict() {
    let mut document = ProjectDocument::new_empty("Target");
    document.folders.remove("Textures");
    document.folders.insert("textures".into());
    let target = ProjectSession::new(document).unwrap();
    let (matching, asset) = batch();
    let placed = matching
        .placed_for(target.document(), Some("textures"))
        .unwrap();
    assert_eq!(
        placed.document().assets[&asset].location,
        "textures/image.png"
    );
    let (mismatched, _) = batch();
    assert!(
        mismatched
            .placed_for(target.document(), Some("Textures"))
            .is_err(),
        "a case-mismatched explicit folder is not the stored folder"
    );
}

#[test]
fn staging_budget_is_inclusive_and_counts_stage_copies() {
    let cap = 8u64 * 1024 * 1024 * 1024;
    assert!(ImportStaging::with_copied_bytes(cap).is_ok());
    match ImportStaging::with_copied_bytes(cap + 1) {
        Err(ProjectError::ResourceLimit(_)) => {}
        Err(other) => panic!("expected budget rejection, got {other:?}"),
        Ok(_) => panic!("budget over the cap must be rejected"),
    }
    // A staged file lands exactly at the cap...
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("image.png");
    let bytes = b"twenty one image bytes";
    std::fs::write(&source, bytes).unwrap();
    let asset = Asset {
        id: AssetId::new(),
        location: "Textures/image.png".into(),
        media_type: "image/png".into(),
        sha256: format!("{:x}", Sha256::digest(bytes)),
        size: bytes.len() as u64,
        origin: None,
    };
    let mut at_cap = ImportStaging::with_copied_bytes(cap - bytes.len() as u64).unwrap();
    assert!(at_cap.stage_file(&asset, &source).is_ok());
    //... and one byte of preparation over the cap ends the job.
    let mut over = ImportStaging::with_copied_bytes(cap - bytes.len() as u64 + 1).unwrap();
    assert!(over.stage_file(&asset, &source).is_err());
}

/// An array edit stages a patched copy in one pass: windows land where they
/// belong (also across the copy's 64 KiB blocks), the result's hash is the
/// patched bytes', and a source that no longer holds the recorded bytes is
/// refused as changed.
#[test]
fn a_patched_copy_is_staged_in_one_pass_and_refuses_a_changed_source() {
    use sha2::{Digest, Sha256};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("array.bin");
    let original = (0..200_000u32).map(|i| (i % 251) as u8).collect::<Vec<_>>();
    std::fs::write(&source, &original).unwrap();
    let hash = |bytes: &[u8]| format!("{:x}", Sha256::digest(bytes));
    let patches = vec![
        (65_530u64, vec![1u8; 12]),
        (199_995, vec![2; 5]),
        (0, vec![3; 2]),
    ];
    let mut expected = original.clone();
    for (start, bytes) in &patches {
        expected[*start as usize..*start as usize + bytes.len()].copy_from_slice(bytes);
    }
    let mut staging = ImportStaging::new().unwrap();
    let id = AssetId::new();
    let lease = staging
        .stage_patched_cancellable(
            id,
            &source,
            original.len() as u64,
            &hash(&original),
            &patches,
            || false,
        )
        .unwrap();
    assert_eq!(lease.asset(), id);
    assert_eq!(std::fs::read(lease.path()).unwrap(), expected);
    assert_eq!(lease.sha256(), hash(&expected));
    assert_eq!(lease.size(), original.len() as u64);

    let changed = staging.stage_patched_cancellable(
        AssetId::new(),
        &source,
        original.len() as u64,
        &hash(b"something else"),
        &patches,
        || false,
    );
    assert!(
        matches!(changed, Err(ProjectError::Io(ref error)) if error.kind() == std::io::ErrorKind::InvalidData),
        "{changed:?}"
    );
}

#[test]
fn an_import_lease_keeps_its_bytes_when_the_source_is_rewritten_in_place() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("image.png");
    std::fs::write(&source, b"original image").unwrap();
    let asset = Asset {
        id: AssetId::new(),
        location: "Images/image.png".into(),
        media_type: "image/png".into(),
        sha256: format!("{:x}", Sha256::digest(b"original image")),
        size: 14,
        origin: None,
    };
    let mut staging = ImportStaging::new().unwrap();
    let lease = staging.stage_file(&asset, &source).unwrap();
    // Truncate-and-write keeps the inode, as most editors' in-place saves do.
    std::fs::write(&source, b"edited  image!").unwrap();
    assert_eq!(std::fs::read(lease.path()).unwrap(), b"original image");
}
