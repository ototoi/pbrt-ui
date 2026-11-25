mod common;
use common::*;
use project::*;
use std::fs;

#[test]
fn edited_folder_project_survives_save_move_and_reopen_with_stable_ids() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("project");
    let (document, transform, shape) = native_scene();
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[
                Edit::SetTransform {
                    node: transform,
                    value: TransformOperation::Translate([7., 8., 9.]),
                },
                set_parameter(shape, "radius", Some(Parameter::floats("radius", [3.]))),
            ],
        )
        .unwrap();
    let snapshot = session.snapshot();
    let mut store = FolderStore::create_v3(
        &path,
        snapshot.document().id,
        &snapshot.document().title,
        snapshot.revision().clone(),
    )
    .unwrap();
    let saved = store.save(&snapshot).unwrap();
    session.mark_saved(&saved).unwrap();
    assert!(!session.is_dirty());
    drop(store);
    drop(session);

    let moved = temporary.path().join("moved-project");
    fs::rename(path, &moved).unwrap();
    let (store, session) = ProjectStore::open(&moved).unwrap();
    assert!(!store.is_read_only());
    assert_eq!(session.document(), snapshot.document());
    assert_eq!(session.revision(), *snapshot.revision());
    assert_eq!(scene(session.document()).nodes[&shape].id, shape);
    assert_eq!(scene(session.document()).nodes[&transform].id, transform);
    assert!(!session.is_dirty());
    assert!(!session.can_undo());
    assert!(!session.can_redo());
}

#[test]
fn project_store_save_as_publishes_a_folder_project() {
    let temporary = tempfile::tempdir().unwrap();
    let source_path = temporary.path().join("source");
    let destination = temporary.path().join("copy");
    let document = ProjectDocument::new("Save As");
    let session = ProjectSession::new(document.clone()).unwrap();
    let store = ProjectStore::Folder(
        FolderStore::create(
            &source_path,
            document.id,
            &document.title,
            Revision::initial(),
        )
        .unwrap(),
    );
    let copy = store.save_as(&destination, &session.snapshot()).unwrap();
    assert!(!copy.is_read_only());
    drop(copy);
    let (_, reopened) = ProjectStore::open(&destination).unwrap();
    assert_eq!(reopened.document(), &document);
}

#[test]
fn older_snapshots_and_conflicting_same_revision_snapshots_cannot_replace_saved_state() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("project");
    let (document, _, shape) = native_scene();
    let mut session = ProjectSession::new(document).unwrap();
    let older = session.snapshot();
    let mut store = FolderStore::create(
        &path,
        older.document().id,
        &older.document().title,
        older.revision().clone(),
    )
    .unwrap();
    store.save(&older).unwrap();
    let mut different = older.document().clone();
    scene_mut(&mut different)
        .nodes
        .get_mut(&shape)
        .unwrap()
        .label = "same revision".into();
    let conflicting = ProjectSession::new(different).unwrap();
    assert!(matches!(
        store.save(&conflicting.snapshot()),
        Err(ProjectError::RevisionConflict { .. })
    ));
    session
        .apply(
            &session.revision(),
            &[Edit::RenameNode {
                node: shape,
                label: "newer".into(),
            }],
        )
        .unwrap();
    let newer = session.snapshot();
    store.save(&newer).unwrap();
    assert!(matches!(
        store.save(&older),
        Err(ProjectError::RevisionConflict { .. })
    ));
    drop(store);
    let (_, reopened) = ProjectStore::open(&path).unwrap();
    assert_eq!(reopened.document(), newer.document());
}

#[test]
fn folder_store_lock_excludes_second_writer_until_close() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("project");
    let document = ProjectDocument::new("Locked");
    let store =
        FolderStore::create(&path, document.id, &document.title, Revision::initial()).unwrap();
    assert!(matches!(
        ProjectStore::open(&path),
        Err(ProjectError::Locked)
    ));
    drop(store);
    assert!(ProjectStore::open(&path).is_ok());
}

#[test]
fn closed_scene_roundtrips_without_implicitly_opening_another_scene() {
    let directory = tempfile::tempdir().unwrap();
    let document = ProjectDocument::new("Closed scene");
    let scene = document.default_scene.unwrap();
    let mut store = FolderStore::create(
        &directory.path().join("project"),
        document.id,
        "Closed scene",
        Revision::initial(),
    )
    .unwrap();
    let mut session = ProjectSession::new(document).unwrap();
    let receipt = store.save(&session.snapshot()).unwrap();
    session.mark_saved(&receipt).unwrap();
    session.close_scene();
    store.set_default_scene(None).unwrap();
    store.save(&session.snapshot()).unwrap();
    let path = store.directory().to_owned();
    drop(store);
    let (_, reopened) = FolderStore::open_project(&path).unwrap();
    assert_eq!(reopened.active_scene(), None);
    assert_eq!(reopened.document().default_scene, None);
    assert!(reopened.document().scenes.contains_key(&scene));
}

#[test]
fn format_two_does_not_publish_a_zero_scene_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    let document = ProjectDocument::new("Format boundary");
    let mut store = FolderStore::create(
        &directory.path().join("project"),
        document.id,
        "Format boundary",
        Revision::initial(),
    )
    .unwrap();
    let mut session = ProjectSession::new(document).unwrap();
    store.save(&session.snapshot()).unwrap();
    let before = fs::read(store.directory().join("project.pbrtui.json")).unwrap();
    let scene = session.active_scene().unwrap();
    session
        .apply(&session.revision(), &[Edit::DeleteScene { scene }])
        .unwrap();
    let error = store.save(&session.snapshot()).unwrap_err();
    assert!(error.to_string().contains("format 3"));
    assert_eq!(
        before,
        fs::read(store.directory().join("project.pbrtui.json")).unwrap()
    );
    assert_eq!(store.read_document().unwrap().scenes.len(), 1);
}
