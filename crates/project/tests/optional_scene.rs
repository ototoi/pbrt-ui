use project::*;

#[test]
fn deleting_the_active_and_last_scene_keeps_a_valid_empty_project() {
    let document = ProjectDocument::new("Empty after deletion");
    let scene = document.default_scene.unwrap();
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(&session.revision(), &[Edit::DeleteScene { scene }])
        .unwrap();
    assert!(session.document().scenes.is_empty());
    session.document().validate().unwrap();
    assert!(session.snapshot().scene_view().is_err());
    session.undo(&session.revision()).unwrap();
    assert!(session.document().scenes.contains_key(&scene));
    assert!(session.snapshot().scene_view().is_err());
    session.redo(&session.revision()).unwrap();
    assert!(session.document().scenes.is_empty());
}

#[test]
fn closing_a_scene_preserves_edits_revision_and_history() {
    let document = ProjectDocument::new("Close");
    let scene = document.default_scene.unwrap();
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::CreateFolder {
                path: "Materials/Sub".into(),
            }],
        )
        .unwrap();
    let revision = session.revision();
    session.close_scene();
    assert_eq!(session.active_scene(), None);
    assert_eq!(session.document().default_scene, None);
    assert_eq!(session.revision(), revision);
    assert!(session.can_undo());
    session.undo(&revision).unwrap();
    assert!(!session.document().folders.contains("Materials/Sub"));
    session.open_scene(scene).unwrap();
    assert!(session.can_redo());
    session.redo(&session.revision()).unwrap();
    assert!(session.document().folders.contains("Materials/Sub"));
}

#[test]
fn resources_can_be_edited_without_an_active_scene() {
    let document = ProjectDocument::new_empty("Resources only");
    document.validate().unwrap();
    let mut session = ProjectSession::new(document).unwrap();
    let change = session
        .apply(
            &session.revision(),
            &[Edit::CreateResource {
                id: None,
                resource_kind: ResourceKind::Material,
                implementation: "diffuse".into(),
                label: "Matte".into(),
                folder: None,
                texture_value_type: None,
                parameters: vec![],
            }],
        )
        .unwrap();
    let ItemId::Resource(id) = change.created_items[0] else {
        panic!("expected resource");
    };
    session
        .apply(
            &session.revision(),
            &[Edit::RenameResource {
                resource: id,
                label: "Renamed".into(),
            }],
        )
        .unwrap();
    session.undo(&session.revision()).unwrap();
    assert_eq!(session.document().resources[&id].label, "Matte");
    assert_eq!(session.active_scene(), None);
    session.document().validate().unwrap();
}

#[test]
fn undo_of_a_newly_opened_scene_does_not_pick_a_replacement() {
    let mut session = ProjectSession::new(ProjectDocument::new("Initial")).unwrap();
    let change = session
        .apply(
            &session.revision(),
            &[Edit::CreateScene {
                location: "Scenes/Added.scene".into(),
            }],
        )
        .unwrap();
    let ItemId::Scene(scene) = change.created_items[0] else {
        panic!("expected scene");
    };
    session.open_scene(scene).unwrap();
    session.undo(&session.revision()).unwrap();
    assert_eq!(session.active_scene(), None);
    assert_eq!(session.document().default_scene, None);
    assert_eq!(session.document().scenes.len(), 1);
    session.redo(&session.revision()).unwrap();
    assert!(session.document().scenes.contains_key(&scene));
    assert_eq!(session.active_scene(), None);
}
