mod common;
use common::*;
use project::*;

#[test]
fn node_active_defaults_without_changing_json_and_survives_undo() {
    let (document, _, shape) = native_scene();
    let node = &scene(&document).nodes[&shape];
    let value = serde_json::to_value(node).unwrap();
    assert!(node.active);
    assert!(value.get("active").is_none());

    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::SetNodeActive {
                node: shape,
                active: false,
            }],
        )
        .unwrap();
    assert!(!scene(session.document()).nodes[&shape].active);
    session.undo(&session.revision()).unwrap();
    assert!(scene(session.document()).nodes[&shape].active);
}

#[test]
fn inactive_flag_survives_folder_save_and_reopen() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("project");
    let (document, _, shape) = native_scene();
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::SetNodeActive {
                node: shape,
                active: false,
            }],
        )
        .unwrap();
    let snapshot = session.snapshot();
    let mut store = FolderStore::create(
        &path,
        snapshot.document().id,
        &snapshot.document().title,
        snapshot.revision().clone(),
    )
    .unwrap();
    store.save(&snapshot).unwrap();
    drop(store);
    let (_, reopened) = ProjectStore::open(&path).unwrap();
    assert!(!scene(reopened.document()).nodes[&shape].active);
}

#[test]
fn set_node_active_refuses_state_and_camera_nodes() {
    let (mut document, state, _) = native_scene();
    let camera = Node::new(
        "Camera",
        NodeKind::Entity {
            entity: EntityKind::Camera,
            implementation: "perspective".into(),
            parameters: Vec::new(),
            pose: None,
        },
    );
    let camera_id = camera.id;
    let scene_id = document.default_scene.unwrap();
    std::sync::Arc::make_mut(document.scenes.get_mut(&scene_id).unwrap())
        .nodes
        .insert(camera_id, camera);
    std::sync::Arc::make_mut(document.scenes.get_mut(&scene_id).unwrap())
        .options
        .push(camera_id);
    let mut session = ProjectSession::new(document).unwrap();
    for node in [state, camera_id] {
        assert!(
            session
                .apply(
                    &session.revision(),
                    &[Edit::SetNodeActive {
                        node,
                        active: false,
                    }],
                )
                .is_err()
        );
    }
}

fn set_active(session: &mut ProjectSession, node: NodeId, active: bool) {
    session
        .apply(&session.revision(), &[Edit::SetNodeActive { node, active }])
        .unwrap();
}

#[test]
fn structural_conversions_keep_an_off_object_off() {
    let (document, _, shape) = native_scene();
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    let group_preset = presets::BUILTIN_PRESETS
        .iter()
        .find(|preset| preset.name == "group.empty")
        .unwrap()
        .id;
    let group = session
        .apply(
            &session.revision(),
            &[Edit::CreatePreset {
                preset: group_preset,
                parent: world,
                after: None,
                definition: None,
                view: None,
            }],
        )
        .unwrap()
        .created[0];
    session
        .apply(
            &session.revision(),
            &[Edit::MoveNode {
                node: shape,
                parent: group,
                after: None,
            }],
        )
        .unwrap();
    set_active(&mut session, group, false);

    // An Off group made into a definition leaves an Off instance.
    session
        .apply(
            &session.revision(),
            &[Edit::MakeObjectDefinition {
                node: group,
                label: None,
            }],
        )
        .unwrap();
    let instance = *scene(session.document())
        .nodes
        .iter()
        .find(|(_, node)| matches!(node.data, NodeKind::Instance { .. }))
        .unwrap()
        .0;
    assert!(!scene(session.document()).nodes[&instance].active);

    // Unpacking that Off instance leaves an Off group.
    let change = session
        .apply(
            &session.revision(),
            &[Edit::UnpackObjectInstance { node: instance }],
        )
        .unwrap();
    assert!(!scene(session.document()).nodes[&change.created[0]].active);
}
