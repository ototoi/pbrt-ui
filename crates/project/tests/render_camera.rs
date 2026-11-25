use project::*;
use std::sync::Arc;

fn camera_pose(x: f64) -> CameraPose {
    CameraPose {
        start: math::Mat4::translation(x, 0., 0.).0,
        end: math::Mat4::translation(x, 0., 0.).0,
    }
}

/// Two independently posed cameras and the resolved-import shape.
fn scene_with_cameras() -> (ProjectDocument, SceneId, NodeId, NodeId, NodeId) {
    let mut document = ProjectDocument::new("Camera selection");
    let scene_id = document.default_scene.unwrap();
    let first = Node::new(
        "Left",
        NodeKind::Entity {
            entity: EntityKind::Camera,
            implementation: "perspective".into(),
            parameters: Vec::new(),
            pose: Some(camera_pose(-1.)),
        },
    );
    let second = Node::new(
        "Right",
        NodeKind::Entity {
            entity: EntityKind::Camera,
            implementation: "orthographic".into(),
            parameters: Vec::new(),
            pose: Some(camera_pose(1.)),
        },
    );
    let shape = Node::new(
        "Shape",
        NodeKind::Entity {
            entity: EntityKind::Shape,
            implementation: "sphere".into(),
            parameters: Vec::new(),
            pose: None,
        },
    );
    let scene = Arc::make_mut(document.scenes.get_mut(&scene_id).unwrap());
    scene.options.extend([first.id, second.id]);
    let NodeKind::Scope { children, .. } = &mut scene.nodes.get_mut(&scene.world).unwrap().data
    else {
        unreachable!()
    };
    children.push(shape.id);
    scene.nodes.extend([
        (first.id, first.clone()),
        (second.id, second.clone()),
        (shape.id, shape.clone()),
    ]);
    document.validate().unwrap();
    (document, scene_id, first.id, second.id, shape.id)
}

#[test]
fn explicit_render_camera_edit_undo_redo_and_scene_duplicate_remap() {
    let (document, scene_id, first, _second, _shape) = scene_with_cameras();
    let mut session = ProjectSession::new(document).unwrap();
    let initial = session.revision();
    session
        .apply(
            &initial,
            &[Edit::SetRenderCamera {
                scene: scene_id,
                camera: Some(first),
            }],
        )
        .unwrap();
    let selected_revision = session.revision();
    assert_eq!(
        session.document().scene(scene_id).unwrap().render_camera,
        Some(first)
    );
    session.undo(&selected_revision).unwrap();
    assert_eq!(
        session.document().scene(scene_id).unwrap().render_camera,
        None
    );
    let undone_revision = session.revision();
    session.redo(&undone_revision).unwrap();
    assert_eq!(
        session.document().scene(scene_id).unwrap().render_camera,
        Some(first)
    );
    let duplicate_revision = session.revision();
    let duplicated = session
        .apply(&duplicate_revision, &[Edit::DuplicateNode { node: first }])
        .unwrap();
    let camera_copy = duplicated.created[0];
    assert_eq!(
        session.document().scene(scene_id).unwrap().render_camera,
        Some(first)
    );
    assert!(
        session
            .document()
            .scene(scene_id)
            .unwrap()
            .options
            .contains(&camera_copy)
    );
    let selected_revision = session.revision();
    session
        .apply(&selected_revision, &[Edit::DeleteNode { node: first }])
        .unwrap();
    assert_eq!(
        session.document().scene(scene_id).unwrap().render_camera,
        None
    );
    let deleted_revision = session.revision();
    session.undo(&deleted_revision).unwrap();
    assert_eq!(
        session.document().scene(scene_id).unwrap().render_camera,
        Some(first)
    );
    let duplicated_revision = session.revision();
    session
        .apply(
            &duplicated_revision,
            &[Edit::DuplicateScene {
                scene: scene_id,
                location: "Scenes/Copy.scene".into(),
            }],
        )
        .unwrap();
    let clone = session
        .document()
        .scenes
        .values()
        .find(|scene| scene.id != scene_id)
        .unwrap();
    let cloned_camera = clone.render_camera.unwrap();
    assert_ne!(cloned_camera, first);
    assert!(clone.options.contains(&cloned_camera));
    assert_eq!(clone.nodes[&cloned_camera].label, "Left");
    assert!(!clone.nodes.contains_key(&first));
    assert_eq!(
        clone.nodes.len(),
        session.document().scene(scene_id).unwrap().nodes.len()
    );
}

#[test]
fn explicit_render_camera_rejects_non_camera_ids() {
    let (document, scene_id, _, _, shape) = scene_with_cameras();
    let mut session = ProjectSession::new(document).unwrap();
    assert!(
        session
            .apply(
                &session.revision(),
                &[Edit::SetRenderCamera {
                    scene: scene_id,
                    camera: Some(shape)
                }]
            )
            .is_err()
    );
}

#[test]
fn selecting_a_render_camera_changes_saved_content_identity() {
    let (document, scene, first, second, _) = scene_with_cameras();
    let mut session = ProjectSession::new(document).unwrap();
    let unset = ContentId::of(session.document());
    session
        .apply(
            &session.revision(),
            &[Edit::SetRenderCamera {
                scene,
                camera: Some(first),
            }],
        )
        .unwrap();
    let selected = ContentId::of(session.document());
    assert_ne!(unset, selected);
    session
        .apply(
            &session.revision(),
            &[Edit::SetRenderCamera {
                scene,
                camera: Some(second),
            }],
        )
        .unwrap();
    assert_ne!(selected, ContentId::of(session.document()));
    session.undo(&session.revision()).unwrap();
    assert_eq!(selected, ContentId::of(session.document()));
    session.undo(&session.revision()).unwrap();
    assert_eq!(unset, ContentId::of(session.document()));
}

#[test]
fn editing_one_camera_pose_leaves_the_other_pose_alone() {
    let (document, scene, first, second, _) = scene_with_cameras();
    let mut session = ProjectSession::new(document).unwrap();
    let view = session.document().scene_view(scene).unwrap();
    let before = evaluate(&view).unwrap();
    let second_before = before.nodes[&second].clone();

    // the edit writes the first camera's own pose value; with nothing
    // shared between the two declarations, the second camera cannot move.
    session
        .apply(
            &session.revision(),
            &[Edit::SetLocalTransform {
                node: first,
                value: LocalTransform::Trs(project::transform_trs::Trs {
                    translation: [3., 0., 0.],
                    rotation_euler_degrees: [0.; 3],
                    scale: [1.; 3],
                }),
            }],
        )
        .unwrap();
    let after_edit = evaluate(&session.document().scene_view(scene).unwrap()).unwrap();
    assert_eq!(after_edit.nodes[&second].start, second_before.start);
    assert_eq!(after_edit.nodes[&second].end, second_before.end);
    let scene_document = session.document().scene(scene).unwrap();
    assert_eq!(
        scene_document.options.len(),
        2,
        "a pose edit inserts nothing into Options"
    );
    let NodeKind::Entity {
        pose: Some(first_pose),
        ..
    } = &scene_document.nodes[&first].data
    else {
        panic!("the edited camera keeps its pose");
    };
    assert_eq!(first_pose.start, math::Mat4::translation(3., 0., 0.).0);

    let edited_revision = session.revision();
    session.undo(&edited_revision).unwrap();
    let undone = evaluate(&session.document().scene_view(scene).unwrap()).unwrap();
    assert_eq!(undone.nodes[&second].start, second_before.start);
    assert_eq!(undone.nodes[&second].end, second_before.end);
    let NodeKind::Entity {
        pose: Some(first_pose),
        ..
    } = &session.document().scene(scene).unwrap().nodes[&first].data
    else {
        panic!("the edited camera keeps its pose");
    };
    assert_eq!(first_pose.start, math::Mat4::translation(-1., 0., 0.).0);
    let undo_revision = session.revision();
    session.redo(&undo_revision).unwrap();
    let redone = evaluate(&session.document().scene_view(scene).unwrap()).unwrap();
    assert_eq!(redone.nodes[&second].start, second_before.start);
    assert_eq!(redone.nodes[&second].end, second_before.end);
}

#[test]
fn first_created_camera_is_selected_but_later_cameras_do_not_replace_it() {
    let document = ProjectDocument::new("Empty scene");
    let scene_id = document.default_scene.unwrap();
    let mut session = ProjectSession::new(document).unwrap();
    let created = session
        .apply(
            &session.revision(),
            &[Edit::CreateCamera { scene: scene_id }],
        )
        .unwrap();
    let first = created.created[0];
    assert_eq!(
        session.document().scene(scene_id).unwrap().render_camera,
        Some(first)
    );
    let created_revision = session.revision();
    session.undo(&created_revision).unwrap();
    assert_eq!(
        session.document().scene(scene_id).unwrap().render_camera,
        None
    );
    let undone_revision = session.revision();
    session.redo(&undone_revision).unwrap();
    assert_eq!(
        session.document().scene(scene_id).unwrap().render_camera,
        Some(first)
    );

    let revision = session.revision();
    session
        .apply(&revision, &[Edit::CreateCamera { scene: scene_id }])
        .unwrap();
    assert_eq!(
        session.document().scene(scene_id).unwrap().render_camera,
        Some(first)
    );

    let revision = session.revision();
    session
        .apply(
            &revision,
            &[Edit::SetRenderCamera {
                scene: scene_id,
                camera: None,
            }],
        )
        .unwrap();
    let revision = session.revision();
    session
        .apply(&revision, &[Edit::CreateCamera { scene: scene_id }])
        .unwrap();
    assert_eq!(
        session.document().scene(scene_id).unwrap().render_camera,
        None
    );
}

#[test]
fn first_camera_is_selected_for_a_non_active_scene_and_same_batch_unset_is_respected() {
    let document = ProjectDocument::new("Scenes");
    let active = document.default_scene.unwrap();
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::CreateScene {
                location: "Scenes/Other.scene".into(),
            }],
        )
        .unwrap();
    let other = *session
        .document()
        .scenes
        .keys()
        .find(|scene| **scene != active)
        .unwrap();
    assert_eq!(session.active_scene(), Some(active));
    session
        .apply(&session.revision(), &[Edit::CreateCamera { scene: other }])
        .unwrap();
    assert!(
        session
            .document()
            .scene(other)
            .unwrap()
            .render_camera
            .is_some()
    );

    let empty = ProjectDocument::new("Explicit unset");
    let scene = empty.default_scene.unwrap();
    let mut session = ProjectSession::new(empty).unwrap();
    session
        .apply(
            &session.revision(),
            &[
                Edit::CreateCamera { scene },
                Edit::SetRenderCamera {
                    scene,
                    camera: None,
                },
            ],
        )
        .unwrap();
    assert_eq!(session.document().scene(scene).unwrap().render_camera, None);
}
