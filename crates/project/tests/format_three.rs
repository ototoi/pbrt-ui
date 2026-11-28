use project::*;
use sha2::{Digest, Sha256};
use std::fs;

fn imagemap(label: &str, image: AssetId, scale: f64) -> Resource {
    Resource {
        id: ResourceId::new(),
        label: label.into(),
        location: format!("Textures/{label}.texture"),
        edition: Revision::from_u64(7),
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

/// One image at `Images/wood.png` and one imagemap Texture referencing it.
fn image_document() -> (ProjectDocument, AssetId, ResourceId) {
    let mut document = ProjectDocument::new_empty("Images");
    let asset = Asset {
        id: AssetId::new(),
        location: "Images/wood.png".into(),
        media_type: "image/png".into(),
        sha256: format!("{:x}", Sha256::digest(b"image bytes")),
        size: 11,
        origin: None,
    };
    let resource = imagemap("wood", asset.id, 2.0);
    let ids = (asset.id, resource.id);
    document.assets.insert(asset.id, asset);
    document.resources.insert(resource.id, resource);
    document
        .folders
        .extend(["Images", "Textures"].map(str::to_owned));
    (document, ids.0, ids.1)
}

#[test]
fn an_image_and_the_textures_sharing_it_save_as_separate_items() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("project");
    let (mut document, asset, first) = image_document();
    let second = imagemap("wood_rough", asset, 3.0);
    let second_id = second.id;
    document.resources.insert(second.id, second);
    let mut store =
        FolderStore::create_v3(&path, document.id, &document.title, Revision::initial()).unwrap();
    store
        .import_asset(&document.assets[&asset], &b"image bytes"[..], || false)
        .unwrap();
    let mut session = ProjectSession::new(document).unwrap();
    let receipt = store.save(&session.snapshot()).unwrap();
    session.mark_saved(&receipt).unwrap();
    assert!(!session.is_dirty());
    let sidecar: serde_json::Value =
        serde_json::from_slice(&fs::read(path.join("Assets/Images/wood.png.meta")).unwrap())
            .unwrap();
    assert_eq!(sidecar["format"], "pbrt-ui.meta/2");
    assert_eq!(sidecar["kind"], "asset");
    assert_eq!(sidecar["id"], asset.to_string());
    assert_eq!(fs::read_dir(path.join("Assets/Images")).unwrap().count(), 2);
    assert!(path.join("Assets/Textures/wood.texture").exists());
    assert!(path.join("Assets/Textures/wood_rough.texture").exists());
    let content = ContentId::of(session.document());
    drop(store);
    let (_, reopened) = ProjectStore::open(&path).unwrap();
    assert_eq!(ContentId::of(reopened.document()), content);
    for id in [first, second_id] {
        assert_eq!(
            reopened.document().resources[&id].parameters[0].values,
            ParameterValues::Assets(vec![asset])
        );
    }
    assert!(!reopened.is_dirty());
    assert!(reopened.active_scene().is_none());
}

#[test]
fn zero_scenes_and_null_default_roundtrip_in_format_three() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("empty");
    let document = ProjectDocument::new_empty("Empty");
    let session = ProjectSession::new(document.clone()).unwrap();
    let mut store =
        FolderStore::create_v3(&path, document.id, &document.title, Revision::initial()).unwrap();
    store.save(&session.snapshot()).unwrap();
    drop(store);
    let (_, reopened) = ProjectStore::open(&path).unwrap();
    assert_eq!(reopened.document(), &document);
}

#[test]
fn a_texture_cannot_share_an_image_location() {
    let (document, asset, resource) = image_document();
    document.validate().unwrap();
    let mut other = document.clone();
    other.resources.get_mut(&resource).unwrap().location = other.assets[&asset].location.clone();
    assert!(other.validate().is_err());
}

#[test]
fn an_imagemap_without_a_filename_is_valid_until_an_image_is_chosen() {
    let (mut document, _, resource) = image_document();
    document
        .resources
        .get_mut(&resource)
        .unwrap()
        .parameters
        .retain(|p| p.name != "filename");
    document.validate().unwrap();
}

#[test]
fn format_three_rejects_old_sidecar_versions() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("mixed");
    let document = ProjectDocument::new("Mixed");
    let mut store =
        FolderStore::create(&path, document.id, &document.title, Revision::initial()).unwrap();
    store
        .save(&ProjectSession::new(document).unwrap().snapshot())
        .unwrap();
    drop(store);
    let manifest = path.join("project.pbrtui.json");
    let mut json: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    json["format"] = "pbrt-ui.project/3".into();
    fs::write(manifest, serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(ProjectStore::open(&path).is_err());
}

#[test]
fn missing_image_keeps_its_identity_and_its_textures_on_save() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("missing");
    let (document, asset, resource) = image_document();
    let mut store =
        FolderStore::create_v3(&path, document.id, &document.title, Revision::initial()).unwrap();
    store
        .import_asset(&document.assets[&asset], &b"image bytes"[..], || false)
        .unwrap();
    store
        .save(&ProjectSession::new(document).unwrap().snapshot())
        .unwrap();
    drop(store);
    fs::remove_file(path.join("Assets/Images/wood.png")).unwrap();
    let (mut store, session) = ProjectStore::open(&path).unwrap();
    assert!(session.document().resources.contains_key(&resource));
    assert!(session.document().assets.contains_key(&asset));
    store.save(&session.snapshot()).unwrap();
    let meta: serde_json::Value =
        serde_json::from_slice(&fs::read(path.join("Assets/Images/wood.png.meta")).unwrap())
            .unwrap();
    assert_eq!(meta["kind"], "asset");
    assert!(path.join("Assets/Textures/wood.texture.meta").exists());
    assert!(!path.join("Assets/Images/wood.png").exists());
}

#[test]
fn format_three_save_as_keeps_image_references_and_source_unchanged() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("source");
    let destination = temporary.path().join("copy");
    let (document, asset, resource) = image_document();
    let mut store =
        FolderStore::create_v3(&path, document.id, &document.title, Revision::initial()).unwrap();
    store
        .import_asset(&document.assets[&asset], &b"image bytes"[..], || false)
        .unwrap();
    let session = ProjectSession::new(document).unwrap();
    store.save(&session.snapshot()).unwrap();
    let source_meta = fs::read(path.join("Assets/Images/wood.png.meta")).unwrap();
    fs::write(path.join(".pbrtui/node-positions.json"), b"{\"layout\":1}").unwrap();
    let copied = store
        .save_as_snapshot(&destination, &session.snapshot())
        .unwrap();
    assert_eq!(
        copied.read_document().unwrap().resources[&resource].parameters[0].values,
        ParameterValues::Assets(vec![asset])
    );
    assert_eq!(
        fs::read(path.join("Assets/Images/wood.png.meta")).unwrap(),
        source_meta
    );
    assert_eq!(
        fs::read(destination.join("Assets/Images/wood.png.meta")).unwrap(),
        source_meta
    );
    assert_eq!(
        fs::read(destination.join(".pbrtui/node-positions.json")).unwrap(),
        b"{\"layout\":1}",
        "Save As keeps the node editor layout"
    );
}

#[test]
/// Bytes edited outside keep the asset's id and become its content (§5).
fn image_bytes_edited_outside_keep_the_id_and_take_the_new_hash() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("corrupt");
    let (document, asset, _) = image_document();
    let original = document.assets[&asset].sha256.clone();
    let mut store =
        FolderStore::create_v3(&path, document.id, &document.title, Revision::initial()).unwrap();
    store
        .import_asset(&document.assets[&asset], &b"image bytes"[..], || false)
        .unwrap();
    store
        .save(&ProjectSession::new(document).unwrap().snapshot())
        .unwrap();
    drop(store);
    fs::write(path.join("Assets/Images/wood.png"), b"changed").unwrap();
    let (_, session) = ProjectStore::open(&path).unwrap();
    let opened = &session.document().assets[&asset];
    assert_eq!(opened.size, b"changed".len() as u64);
    assert_ne!(opened.sha256, original);
}

#[test]
fn moving_an_image_leaves_its_texture_in_place_and_referencing_it() {
    let (document, asset, resource) = image_document();
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::MoveItem {
                item: ItemId::Asset(asset),
                location: "Images/moved.png".into(),
            }],
        )
        .unwrap();
    let document = session.document();
    assert_eq!(document.assets[&asset].location, "Images/moved.png");
    assert_eq!(
        document.resources[&resource].location,
        "Textures/wood.texture"
    );
    assert_eq!(
        document.resources[&resource].parameters[0].values,
        ParameterValues::Assets(vec![asset])
    );
    session.undo(&session.revision()).unwrap();
    assert_eq!(
        session.document().assets[&asset].location,
        "Images/wood.png"
    );
}

#[test]
fn deleting_saving_and_undoing_an_image_restores_bytes_even_after_redo_is_cleared() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("history");
    let (mut document, asset, resource) = image_document();
    document.resources.remove(&resource);
    let mut store =
        FolderStore::create_v3(&path, document.id, &document.title, Revision::initial()).unwrap();
    store
        .import_asset(&document.assets[&asset], &b"image bytes"[..], || false)
        .unwrap();
    let mut session = ProjectSession::new(document).unwrap();
    session
        .enable_temporary_payload_staging(&path.join("Assets"))
        .unwrap();
    store.save(&session.snapshot()).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::DeleteItem {
                item: ItemId::Asset(asset),
            }],
        )
        .unwrap();
    store.save(&session.snapshot()).unwrap();
    assert!(!path.join("Assets/Images/wood.png").exists());
    session.undo(&session.revision()).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::MoveItem {
                item: ItemId::Asset(asset),
                location: "Images/restored.png".into(),
            }],
        )
        .unwrap();
    assert!(!session.can_redo());
    let receipt = store.save(&session.snapshot()).unwrap();
    session.mark_saved(&receipt).unwrap();
    assert!(!session.is_dirty());
    assert_eq!(
        fs::read(path.join("Assets/Images/restored.png")).unwrap(),
        b"image bytes"
    );
}

#[test]
fn undo_after_save_as_restores_a_deleted_image_once_the_old_folder_is_gone() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("original");
    let copy = temporary.path().join("copy");
    let (mut document, asset, resource) = image_document();
    document.resources.remove(&resource);
    let mut store =
        FolderStore::create_v3(&path, document.id, &document.title, Revision::initial()).unwrap();
    store
        .import_asset(&document.assets[&asset], &b"image bytes"[..], || false)
        .unwrap();
    let mut session = ProjectSession::new(document).unwrap();
    session
        .enable_temporary_payload_staging(&path.join("Assets"))
        .unwrap();
    store.save(&session.snapshot()).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::DeleteItem {
                item: ItemId::Asset(asset),
            }],
        )
        .unwrap();
    store.save(&session.snapshot()).unwrap();
    let mut moved = store.save_as_snapshot(&copy, &session.snapshot()).unwrap();
    // As core does: relink on the Save As worker, swap on the session's thread.
    let retained = session.retained_payloads();
    let assets = copy.join("Assets");
    let relocated =
        std::thread::spawn(move || project::RelocatedPayloads::prepare(&retained, &assets))
            .join()
            .unwrap()
            .unwrap();
    session.adopt_relocated_payloads(relocated).unwrap();
    session.set_payload_assets_root(copy.join("Assets"));
    drop(store);
    fs::remove_dir_all(&path).unwrap();

    session.undo(&session.revision()).unwrap();
    moved.save(&session.snapshot()).unwrap();

    assert_eq!(
        fs::read(copy.join("Assets/Images/wood.png")).unwrap(),
        b"image bytes"
    );
}

#[test]
fn a_referenced_image_cannot_be_deleted() {
    let (document, asset, _) = image_document();
    let mut session = ProjectSession::new(document).unwrap();
    let before = session.document().clone();
    assert!(
        session
            .apply(
                &session.revision(),
                &[Edit::DeleteItem {
                    item: ItemId::Asset(asset)
                }]
            )
            .is_err()
    );
    assert_eq!(session.document(), &before);
    assert!(!session.can_undo());
}

#[test]
fn io_snapshot_taken_before_deletion_keeps_its_image_bytes_after_save_and_close() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("running-job");
    let (document, asset, resource) = image_document();
    let mut folder =
        FolderStore::create_v3(&path, document.id, &document.title, Revision::initial()).unwrap();
    folder
        .import_asset(&document.assets[&asset], &b"image bytes"[..], || false)
        .unwrap();
    let mut session = ProjectSession::new(document).unwrap();
    session
        .enable_temporary_payload_staging(&path.join("Assets"))
        .unwrap();
    folder.save(&session.snapshot()).unwrap();
    let running = session.snapshot_for_io().unwrap();
    session
        .apply(
            &session.revision(),
            &[
                Edit::DeleteItem {
                    item: ItemId::Resource(resource),
                },
                Edit::DeleteItem {
                    item: ItemId::Asset(asset),
                },
            ],
        )
        .unwrap();
    folder.save(&session.snapshot()).unwrap();
    drop(session);
    assert!(!path.join("Assets/Images/wood.png").exists());
    let store = ProjectStore::Folder(folder);
    assert_eq!(
        fs::read(running.asset_path(&store, asset).unwrap()).unwrap(),
        b"image bytes"
    );
}

#[test]
fn io_snapshots_reuse_one_staged_copy_of_an_unchanged_asset() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("reuse");
    let (document, asset, _) = image_document();
    let mut folder =
        FolderStore::create_v3(&path, document.id, &document.title, Revision::initial()).unwrap();
    folder
        .import_asset(&document.assets[&asset], &b"image bytes"[..], || false)
        .unwrap();
    let mut session = ProjectSession::new(document).unwrap();
    session
        .enable_temporary_payload_staging(&path.join("Assets"))
        .unwrap();
    folder.save(&session.snapshot()).unwrap();
    let first = session.snapshot_for_io().unwrap();
    let second = session.snapshot_for_io().unwrap();
    let store = ProjectStore::Folder(folder);
    let staged = first.asset_path(&store, asset).unwrap();
    assert!(staged.starts_with(path.join("Library/Staging")));
    assert_ne!(
        staged,
        path.join("Assets/Images/wood.png"),
        "staged, not the store file"
    );
    // A render or preview no longer copies every asset again.
    assert_eq!(second.asset_path(&store, asset).unwrap(), staged);
}

#[test]
fn setting_the_existing_texture_value_type_is_a_no_op() {
    let (document, _, resource) = image_document();
    let mut session = ProjectSession::new(document).unwrap();
    let revision = session.revision();
    session
        .apply(
            &revision,
            &[Edit::SetTextureValueType {
                resource,
                value: TextureValueType::Spectrum,
            }],
        )
        .unwrap();
    assert_eq!(session.revision(), revision);
    assert!(!session.can_undo());
}
