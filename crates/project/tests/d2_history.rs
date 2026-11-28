//! dirty measured against the saved *content*,
//! not the communication revision — and the content identity itself is a
//! canonical hash that excludes view state and editions.
mod common;
use common::*;
use project::*;
use std::fs;

fn content_document() -> ProjectDocument {
    let (mut document, _, _) = native_scene();
    let material = Resource {
        id: ResourceId::new(),
        label: "Mat".into(),
        location: "Materials/Mat.material".into(),
        edition: Revision::initial(),
        kind: ResourceKind::Material,
        texture_value_type: None,
        implementation: "diffuse".into(),
        parameters: vec![Parameter::floats("roughness", [0.5])],
        definition_scope: None,
        declared_in: None,
        origin: None,
    };
    document.resources.insert(material.id, material);
    let mut mesh = asset("Meshes/thing.ply");
    mesh.media_type = "application/x-ply".into();
    document.assets.insert(mesh.id, mesh);
    document.folders.insert("Materials".into());
    document.folders.insert("Meshes".into());
    document.validate().unwrap();
    document
}

fn asset(location: &str) -> Asset {
    Asset {
        id: AssetId::new(),
        location: location.into(),
        media_type: "application/octet-stream".into(),
        sha256: "1".repeat(64),
        size: 12,
        origin: None,
    }
}

#[test]
fn content_id_excludes_view_state_and_editions() {
    let document = content_document();
    let id = ContentId::of(&document);
    // Editions move on every edit and undo; they are not content.
    let mut bumped = document.clone();
    for node in scene_mut(&mut bumped).nodes.values_mut() {
        node.edition = Revision::from_u64(9);
    }
    for resource in bumped.resources.values_mut() {
        resource.edition = Revision::from_u64(9);
    }
    // The default scene is persisted view state, not content.
    bumped.default_scene = None;
    assert_eq!(
        ContentId::of(&bumped),
        id,
        "editions and default are not content"
    );
}

#[test]
fn content_id_tracks_content() {
    let document = content_document();
    let id = ContentId::of(&document);
    let mut renamed = document.clone();
    let node = scene_mut(&mut renamed).nodes.values_mut().next().unwrap();
    node.label = "Renamed".into();
    assert_ne!(ContentId::of(&renamed), id, "a node label is content");
    let mut retitled = document.clone();
    retitled.title = "Other title".into();
    assert_ne!(ContentId::of(&retitled), id, "the title is content");
    let mut moved_asset = document.clone();
    let asset = moved_asset.assets.values_mut().next().unwrap();
    asset.location = "Meshes/moved.ply".into();
    assert_ne!(
        ContentId::of(&moved_asset),
        id,
        "an asset location is content"
    );
    let mut refoldered = document.clone();
    refoldered.folders.insert("Extras".into());
    assert_ne!(ContentId::of(&refoldered), id, "folders are content");
    let mut retuned = document.clone();
    let resource = retuned.resources.values_mut().next().unwrap();
    resource.implementation = "conductor".into();
    assert_ne!(ContentId::of(&retuned), id, "the implementation is content");
}

#[test]
fn content_id_is_stable_and_hashids_identity() {
    // Repeated hashing is pure, and identity is content: two documents
    // built the same way differ in their generated UUIDs and so in id.
    let document = content_document();
    assert_eq!(ContentId::of(&document), ContentId::of(&document.clone()));
    assert_ne!(
        ContentId::of(&content_document()),
        ContentId::of(&document),
        "freshly minted UUIDs differ, so their documents are not the same content"
    );
    assert_eq!(ContentId::of(&document).as_str().len(), 64);
}

/// A dirt baseline anchored by an actual store receipt: undo/redo move the
/// revision; dirty state follows the content.
fn anchored_session(temporary: &std::path::Path) -> (ProjectSession, NodeId, std::path::PathBuf) {
    let (document, _, shape) = native_scene();
    let mut session = ProjectSession::new(document).unwrap();
    assert!(session.is_dirty(), "a session that never saved is dirty");
    let path = temporary.join("project");
    let snapshot = session.snapshot();
    let mut store = FolderStore::create(
        &path,
        snapshot.document().id,
        &snapshot.document().title,
        snapshot.revision().clone(),
    )
    .unwrap();
    let receipt = store.save(&snapshot).unwrap();
    session.mark_saved(&receipt).unwrap();
    assert!(!session.is_dirty());
    drop(store);
    (session, shape, path)
}

#[test]
fn undo_back_to_the_saved_content_is_clean() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut session, shape, path) = anchored_session(temporary.path());
    session
        .apply(
            &session.revision(),
            &[Edit::RenameNode {
                node: shape,
                label: "Edited sphere".into(),
            }],
        )
        .unwrap();
    assert!(session.is_dirty());
    session.undo(&session.revision()).unwrap();
    assert!(
        !session.is_dirty(),
        "undo returned the document to the saved content"
    );
    session.redo(&session.revision()).unwrap();
    assert!(session.is_dirty(), "redo reapplies the unsaved change");
    session.undo(&session.revision()).unwrap();
    assert!(!session.is_dirty());
    // Rewinding this single edit any further is simply out of history.
    session.undo(&session.revision()).unwrap_err();
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn save_anchors_the_dirty_baseline_at_the_saved_content() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut session, shape, path) = anchored_session(temporary.path());
    session
        .apply(
            &session.revision(),
            &[Edit::RenameNode {
                node: shape,
                label: "Edited sphere".into(),
            }],
        )
        .unwrap();
    let snapshot = session.snapshot();
    let mut store = FolderStore::open(&path).unwrap();
    let receipt = store.save(&snapshot).unwrap();
    // The receipt reports which content the save published.
    assert_eq!(
        receipt.content_id(),
        &ContentId::of(snapshot.document()),
        "the receipt's content id is the saved snapshot's"
    );
    session.mark_saved(&receipt).unwrap();
    assert!(!session.is_dirty());
    session
        .apply(
            &session.revision(),
            &[Edit::RenameNode {
                node: shape,
                label: "Twice edited".into(),
            }],
        )
        .unwrap();
    session.undo(&session.revision()).unwrap();
    assert!(
        !session.is_dirty(),
        "content back at the saved baseline, at a revision the save never saw"
    );
    session.undo(&session.revision()).unwrap();
    assert!(
        session.is_dirty(),
        "undo past the save returns to content the save never saw"
    );
    drop(store);
    let _ = std::fs::remove_dir_all(&path);
}

/// A project on disk with one asset whose bytes live beside it, plus a
/// session resumed from that store with payload staging installed (the
/// grounds the store owner knows; core installs them in the app).
fn asset_project(
    temporary: &std::path::Path,
    bytes: &[u8],
) -> (ProjectSession, FolderStore, AssetId, std::path::PathBuf) {
    let (mut document, _, _) = native_scene();
    let asset = Asset {
        id: AssetId::new(),
        location: "Meshes/box.ply".into(),
        media_type: "application/octet-stream".into(),
        sha256: sha256hex(bytes),
        size: bytes.len() as u64,
        origin: None,
    };
    let id = asset.id;
    document.assets.insert(id, asset.clone());
    document.folders.insert("Meshes".into());
    let mut session = ProjectSession::new(document).unwrap();
    let path = temporary.join("project");
    let mut store = FolderStore::create(
        &path,
        session.document().id,
        &session.document().title.clone(),
        session.revision().clone(),
    )
    .unwrap();
    // Place the asset inside the store the way imports do, so its bytes and
    // baseline are visible to saves.
    store
        .import_asset(&asset, std::io::Cursor::new(bytes), || false)
        .unwrap();
    let receipt = store.save(&session.snapshot()).unwrap();
    session.mark_saved(&receipt).unwrap();
    assert!(path.join("Assets/Meshes/box.ply").is_file());
    (session, store, id, path)
}

#[test]
fn deleted_asset_bytes_survive_the_save_that_removed_them() {
    let temporary = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    let bytes = b"ply mesh bytes";
    let (mut session, mut store, asset, path) = asset_project(temporary.path(), bytes);
    session.enable_payload_staging(staging.path(), path.join("Assets"));

    // Delete the asset: the edit stages its bytes into history.
    session
        .apply(
            &session.revision(),
            &[Edit::DeleteItem {
                item: ItemId::Asset(asset),
            }],
        )
        .unwrap();
    assert!(session.is_dirty());
    let receipt = store.save(&session.snapshot()).unwrap();
    session.mark_saved(&receipt).unwrap();
    // The save published the deletion: the disk file is gone.
    assert!(!path.join("Assets/Meshes/box.ply").is_file());

    // Undo brings the record back while the bytes live only in the
    // history's lease; deleted bytes stay referenced by history.
    session.undo(&session.revision()).unwrap();
    assert!(session.document().assets.contains_key(&asset));
    assert!(
        session.snapshot().payload_by(asset).is_some(),
        "the resumed snapshot carries the staged bytes"
    );
    // the save published the deletion, so the restored record is
    // unsaved content - dirty until the next save re-publishes it.
    assert!(session.is_dirty());

    // The next save re-materializes the bytes on disk from the lease.
    let receipt = store.save(&session.snapshot()).unwrap();
    session.mark_saved(&receipt).unwrap();
    let restored = std::fs::read(path.join("Assets/Meshes/box.ply")).unwrap();
    assert_eq!(restored, bytes, "the re-materialized file holds its bytes");
    assert!(!session.is_dirty());
    drop(store);
    drop(session);
    assert!(
        staging.path().read_dir().unwrap().next().is_none(),
        "closing the session released every staged lease"
    );
}

#[test]
fn format_three_publication_reads_bytes_only_a_payload_lease_retains() {
    let temporary = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    let bytes = b"ply mesh bytes";
    let (mut session, mut store, asset, path) = asset_project(temporary.path(), bytes);
    session.enable_payload_staging(staging.path(), path.join("Assets"));
    session
        .apply(
            &session.revision(),
            &[Edit::DeleteItem {
                item: ItemId::Asset(asset),
            }],
        )
        .unwrap();
    let receipt = store.save(&session.snapshot()).unwrap();
    session.mark_saved(&receipt).unwrap();
    session.undo(&session.revision()).unwrap();
    // The folder no longer has the file; only the lease holds the bytes.
    assert!(!path.join("Assets/Meshes/box.ply").is_file());

    let snapshot = session.snapshot();
    let plan = FormatThreePlan::new(&snapshot).unwrap();
    let store = ProjectStore::Folder(store);
    let (copy, _) = store
        .publish_format_three(&temporary.path().join("migrated"), &snapshot, &plan, || {
            false
        })
        .unwrap();
    let migrated = plan
        .snapshot()
        .document()
        .assets
        .keys()
        .next()
        .copied()
        .unwrap();
    assert_eq!(fs::read(copy.asset_path(migrated).unwrap()).unwrap(), bytes);
}

#[test]
fn a_delete_whose_bytes_are_already_gone_stages_nothing() {
    let temporary = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    let bytes = b"ply mesh bytes";
    let (mut session, mut store, asset, path) = asset_project(temporary.path(), bytes);
    session.enable_payload_staging(staging.path(), path.join("Assets"));
    // The file vanished behind the project's back (before any save
    // recorded it): the delete has no bytes to pin and still applies.
    std::fs::remove_file(path.join("Assets/Meshes/box.ply")).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::DeleteItem {
                item: ItemId::Asset(asset),
            }],
        )
        .unwrap();
    assert!(session.snapshot().payload_by(asset).is_none());
    assert!(staging.path().read_dir().unwrap().next().is_none());
    drop(store);
    drop(session);
}

fn sha256hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// History keeps a copy, not a hard link: rewriting the user's file in place
/// after the delete must not change the bytes Undo brings back.
#[test]
fn deleted_asset_history_is_not_changed_by_an_in_place_edit() {
    let temporary = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    let bytes = b"ply mesh bytes";
    let (mut session, store, asset, path) = asset_project(temporary.path(), bytes);
    session.enable_payload_staging(staging.path(), path.join("Assets"));
    // An I/O snapshot first, as every save does, pinning the file.
    session.snapshot_for_io().unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::DeleteItem {
                item: ItemId::Asset(asset),
            }],
        )
        .unwrap();
    let file = path.join("Assets/Meshes/box.ply");
    std::fs::OpenOptions::new()
        .write(true)
        .open(&file)
        .and_then(|mut file| std::io::Write::write_all(&mut file, b"PLY"))
        .unwrap();
    session.undo(&session.revision()).unwrap();
    let snapshot = session.snapshot();
    let lease = snapshot.payload_by(asset).unwrap();
    assert_eq!(std::fs::read(lease.path()).unwrap(), bytes);
    drop(store);
}

/// A file rewritten outside the application is adopted as the asset's new
/// content by the next I/O snapshot; the save rewrites its sidecar, the
/// session stays clean, and the project reopens with the new hash.
#[test]
fn an_asset_rewritten_outside_is_adopted_by_the_next_save() {
    let temporary = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    let (mut session, mut store, asset, path) = asset_project(temporary.path(), b"ply mesh bytes");
    session.enable_payload_staging(staging.path(), path.join("Assets"));
    session.snapshot_for_io().unwrap();
    let file = path.join("Assets/Meshes/box.ply");
    let edited = b"ply mesh bytes, edited elsewhere";
    std::fs::write(&file, edited).unwrap();

    let snapshot = session.snapshot_for_io().unwrap();
    assert_eq!(session.document().assets[&asset].sha256, sha256hex(edited));
    assert!(!session.is_dirty());
    let receipt = store.save(&snapshot).unwrap();
    session.mark_saved(&receipt).unwrap();
    assert!(!session.is_dirty());
    let meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.join("Assets/Meshes/box.ply.meta")).unwrap())
            .unwrap();
    assert_eq!(meta["sha256"], sha256hex(edited));
    drop(store);

    // Rewritten again while closed: the open takes the bytes on disk.
    let again = b"third version";
    std::fs::write(&file, again).unwrap();
    let (_, reopened) = FolderStore::open_project(&path).unwrap();
    assert_eq!(reopened.document().assets[&asset].sha256, sha256hex(again));
    assert_eq!(reopened.document().assets[&asset].size, again.len() as u64);
}
