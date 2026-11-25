use pbrt_io::stage_snapshot_assets;
use project::*;
use sha2::{Digest, Sha256};
use std::fs;

#[test]
fn staging_reads_retained_snapshot_bytes_after_delete_save_and_session_close() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("project");
    let mut document = ProjectDocument::new("Payload");
    let bytes = b"retained bytes";
    let asset = Asset {
        id: AssetId::new(),
        location: "Data/file.bin".into(),
        media_type: "application/octet-stream".into(),
        sha256: format!("{:x}", Sha256::digest(bytes)),
        size: bytes.len() as u64,
        origin: None,
    };
    document.assets.insert(asset.id, asset.clone());
    document.folders.insert("Data".into());
    let mut folder =
        FolderStore::create(&path, document.id, &document.title, Revision::initial()).unwrap();
    folder.import_asset(&asset, &bytes[..], || false).unwrap();
    let mut session = ProjectSession::new(document).unwrap();
    session
        .enable_temporary_payload_staging(&path.join("Assets"))
        .unwrap();
    folder.save(&session.snapshot()).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::DeleteItem {
                item: ItemId::Asset(asset.id),
            }],
        )
        .unwrap();
    folder.save(&session.snapshot()).unwrap();
    assert!(!path.join("Assets/Data/file.bin").exists());
    session.undo(&session.revision()).unwrap();
    let snapshot = session.snapshot();
    drop(session);
    let store = ProjectStore::Folder(folder);
    let staged = temporary.path().join("bundle");
    fs::create_dir(&staged).unwrap();
    let names = stage_snapshot_assets(&snapshot, &store, &staged, &|| false).unwrap();
    assert_eq!(fs::read(staged.join(&names[&asset.id])).unwrap(), bytes);
}
