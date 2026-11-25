//! Camera rows own their attached groups:
//! creating, editing, deleting, duplicating and moving those units keeps
//! the hierarchy parent and the transform parent the same node.
mod common;
use common::*;
use project::math::Mat4;
use project::transform_trs::approx_eq;
use project::*;

fn posed_camera(document: &mut ProjectDocument, pose: CameraPose) -> NodeId {
    let camera = Node::new(
        "Camera",
        NodeKind::Entity {
            entity: EntityKind::Camera,
            implementation: "perspective".into(),
            parameters: Vec::new(),
            pose: Some(pose),
        },
    );
    let id = camera.id;
    let target = scene_mut(document);
    target.options.push(id);
    target.render_camera = Some(id);
    target.nodes.insert(id, camera);
    id
}

fn shape_node(label: &str) -> Node {
    Node::new(
        label,
        NodeKind::Entity {
            entity: EntityKind::Shape,
            implementation: "sphere".into(),
            parameters: vec![Parameter::floats("radius", [1.])],
            pose: None,
        },
    )
}

fn light_node(label: &str) -> Node {
    Node::new(
        label,
        NodeKind::Entity {
            entity: EntityKind::Light,
            implementation: "point".into(),
            parameters: Vec::new(),
            pose: None,
        },
    )
}

fn group_node(label: &str, children: Vec<NodeId>, parent_camera: Option<NodeId>) -> Node {
    Node::new(
        label,
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children,
            owned_object: None,
            parent_camera,
        },
    )
}

fn pose_of(document: &ProjectDocument, camera: NodeId) -> Option<CameraPose> {
    match &scene(document).nodes[&camera].data {
        NodeKind::Entity { pose, .. } => *pose,
        other => panic!("not a camera: {other:?}"),
    }
}

fn parent_camera_of(document: &ProjectDocument, id: NodeId) -> Option<NodeId> {
    match &scene(document).nodes[&id].data {
        NodeKind::Scope { parent_camera, .. } => *parent_camera,
        other => panic!("not a scope: {other:?}"),
    }
}

fn world_children(session: &ProjectSession) -> Vec<NodeId> {
    let document = session.document();
    let world = scene(document).world;
    match &scene(document).nodes[&world].data {
        NodeKind::Scope { children, .. } => children.clone(),
        _ => unreachable!(),
    }
}

/// A world with a posed camera, a light group attached to it and a free
/// sphere: the hair-scene shape after import.
fn camera_scene() -> (ProjectSession, NodeId, NodeId, NodeId, NodeId) {
    let mut document = ProjectDocument::new("Camera scene");
    let camera = posed_camera(
        &mut document,
        CameraPose {
            start: Mat4::translation(1., 2., 8.).0,
            end: Mat4::translation(1., 2., 8.).0,
        },
    );
    let light = light_node("Fill light");
    let light_id = light.id;
    scene_mut(&mut document).nodes.insert(light_id, light);
    let group = group_node("Camera lights", vec![light_id], Some(camera));
    let sphere = shape_node("Sphere");
    let ids = (camera, group.id, light_id, sphere.id);
    append_world(&mut document, group);
    append_world(&mut document, sphere);
    (
        ProjectSession::new(document).unwrap(),
        ids.0,
        ids.1,
        ids.2,
        ids.3,
    )
}

#[test]
fn a_camera_local_edit_rewrites_the_pose_and_undo_restores_it() {
    let (mut session, camera, _, light, _) = camera_scene();
    let before = session.document().clone();
    let value = Mat4::translation(3., 0., -2.).compose(Mat4::rotation(30., 0., 1., 0.));
    session
        .apply(
            &session.revision(),
            &[Edit::SetLocalTransform {
                node: camera,
                value: LocalTransform::Matrix(value.0),
            }],
        )
        .unwrap();
    let document = session.document();
    assert_eq!(
        pose_of(document, camera).map(|pose| (pose.start, pose.end)),
        Some((value.0, value.0)),
        "the pose is the local edit directly"
    );
    // The evaluator follows: the attached light enters at the new pose.
    let evaluated = evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
    assert!(approx_eq(&evaluated.nodes[&light].start, &value));
    // One undo restores the original pose.
    session.undo(&session.revision()).unwrap();
    assert_eq!(
        project::ContentId::of(session.document()),
        project::ContentId::of(&before),
        "one undo restores the whole pose edit"
    );
    let document = session.document();
    assert_eq!(
        pose_of(document, camera).map(|pose| (pose.start, pose.end)),
        Some((
            Mat4::translation(1., 2., 8.).0,
            Mat4::translation(1., 2., 8.).0
        ))
    );
}

#[test]
fn a_camera_trs_edit_composes_into_the_pose() {
    let (mut session, camera, _, _, _) = camera_scene();
    let value = transform_trs::Trs {
        translation: [0., 1., 2.],
        rotation_euler_degrees: [0., 90., 0.],
        scale: [1., 1., 1.],
    };
    session
        .apply(
            &session.revision(),
            &[Edit::SetLocalTransform {
                node: camera,
                value: LocalTransform::Trs(value),
            }],
        )
        .unwrap();
    let composed = transform_trs::compose(&value).0;
    let document = session.document();
    assert_eq!(
        pose_of(document, camera).map(|pose| (pose.start, pose.end)),
        Some((composed, composed))
    );
}

#[test]
fn a_singular_camera_pose_edit_is_refused() {
    let mut document = ProjectDocument::new("Singular camera");
    let camera = posed_camera(
        &mut document,
        CameraPose {
            start: Mat4::identity().0,
            end: Mat4::identity().0,
        },
    );
    let mut session = ProjectSession::new(document).unwrap();
    let mut flat = Mat4::identity().0;
    flat[10] = 0.0;
    let error = session
        .apply(
            &session.revision(),
            &[Edit::SetLocalTransform {
                node: camera,
                value: LocalTransform::Matrix(flat),
            }],
        )
        .unwrap_err();
    assert!(error.to_string().contains("singular"), "{error}");
}

#[test]
fn an_animated_camera_pose_refuses_static_local_edits() {
    let mut document = ProjectDocument::new("Animated camera");
    let camera = posed_camera(
        &mut document,
        CameraPose {
            start: Mat4::identity().0,
            end: Mat4::translation(0., 4., 0.).0,
        },
    );
    let world = scene(&document).world;
    let sphere = shape_node("Sphere");
    append_world(&mut document, sphere);
    let _ = world;
    let mut session = ProjectSession::new(document).unwrap();
    let error = session
        .apply(
            &session.revision(),
            &[Edit::SetLocalTransform {
                node: camera,
                value: LocalTransform::Matrix(Mat4::identity().0),
            }],
        )
        .unwrap_err();
    assert!(error.to_string().contains("animated"), "{error}");
}

#[test]
fn a_poseless_camera_refuses_local_edits_like_an_older_import() {
    let mut document = ProjectDocument::new("Poseless camera");
    let camera = {
        let camera = Node::new(
            "Camera",
            NodeKind::Entity {
                entity: EntityKind::Camera,
                implementation: "perspective".into(),
                parameters: Vec::new(),
                pose: None,
            },
        );
        let id = camera.id;
        let target = scene_mut(&mut document);
        target.options.push(id);
        target.render_camera = Some(id);
        target.nodes.insert(id, camera);
        id
    };
    // The pose-less camera keeps its declaration transform: an older
    // import's residue, refused with the same reason as a reset run.
    append_world(
        &mut document,
        Node::new(
            "LookAt",
            NodeKind::State {
                operation: StateOperation::Transform(TransformOperation::Translate([0., 2., 8.])),
            },
        ),
    );
    let mut session = ProjectSession::new(document).unwrap();
    let error = session
        .apply(
            &session.revision(),
            &[Edit::SetLocalTransform {
                node: camera,
                value: LocalTransform::Matrix(Mat4::identity().0),
            }],
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("older import"),
        "the refusal names the older import: {error}"
    );
}

#[test]
fn deleting_a_camera_takes_its_attached_groups_and_undo_restores_them() {
    let (mut session, camera, group, light, sphere) = camera_scene();
    session
        .apply(&session.revision(), &[Edit::DeleteNode { node: camera }])
        .unwrap();
    let document = session.document();
    assert_eq!(scene(document).options, Vec::<NodeId>::new());
    assert_eq!(scene(document).render_camera, None);
    assert!(!scene(document).nodes.contains_key(&group));
    assert!(!scene(document).nodes.contains_key(&light));
    assert!(
        scene(document).nodes.contains_key(&sphere),
        "the free sphere stays"
    );
    assert_eq!(world_children(&session), vec![sphere]);
    // One undo returns the camera, its attached group, and the light.
    session.undo(&session.revision()).unwrap();
    let document = session.document();
    assert_eq!(scene(document).options, vec![camera]);
    assert_eq!(scene(document).render_camera, Some(camera));
    assert!(parent_camera_of(document, group).is_some());
    assert!(scene(document).nodes.contains_key(&light));
    assert_eq!(
        world_children(&session),
        vec![group, sphere],
        "undo puts the attached group back in order"
    );
    evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
}

#[test]
fn duplicating_a_camera_attaches_copies_of_its_children() {
    let (mut session, camera, group, _, sphere) = camera_scene();
    let change = session
        .apply(&session.revision(), &[Edit::DuplicateNode { node: camera }])
        .unwrap();
    let document = session.document();
    let copy = change.created[0];
    assert!(
        matches!(
            &scene(document).nodes[&copy].data,
            NodeKind::Entity {
                entity: EntityKind::Camera,
                ..
            }
        ),
        "the copy is a camera"
    );
    let options = &scene(document).options;
    let index = options.iter().position(|id| *id == camera).unwrap();
    assert_eq!(
        options[index + 1],
        copy,
        "the copy sits right after the original in Options"
    );
    let children = world_children(&session);
    let at = children
        .iter()
        .position(|id| *id == group)
        .expect("the original group");
    assert_eq!(
        at + 1,
        children
            .iter()
            .position(|id| {
                scene(document).nodes.get(id).is_some_and(|node| {
                    matches!(
                        &node.data,
                        NodeKind::Scope {
                            parent_camera: Some(attached),
                            ..
                        } if *attached == copy
                    )
                })
            })
            .expect("the copied group right after the original"),
        "the copied group follows the original in the world"
    );
    assert_eq!(children[0], group);
    assert_eq!(children[2], sphere);
    session.undo(&session.revision()).unwrap();
    assert_eq!(world_children(&session), vec![group, sphere]);
}

#[test]
fn duplicating_a_camera_copies_every_attached_group_in_order() {
    // Regression: with more than one attached group the old insertion index
    // double-counted earlier inserts and went out of bounds (a panic), and
    // with a trailing sibling the copies landed after the wrong node.
    let mut document = ProjectDocument::new("Two units follow the camera");
    let camera = posed_camera(
        &mut document,
        CameraPose {
            start: Mat4::identity().0,
            end: Mat4::identity().0,
        },
    );
    let follow = light_node("Follow light");
    let follow_id = follow.id;
    scene_mut(&mut document).nodes.insert(follow_id, follow);
    let first = group_node("Camera lights", vec![follow_id], Some(camera));
    let rig = group_node("Camera rig", Vec::new(), Some(camera));
    let free = shape_node("Free");
    let (first_id, rig_id, free_id) = (first.id, rig.id, free.id);
    append_world(&mut document, first);
    append_world(&mut document, rig);
    append_world(&mut document, free);
    let mut session = ProjectSession::new(document).unwrap();

    let change = session
        .apply(&session.revision(), &[Edit::DuplicateNode { node: camera }])
        .unwrap();
    let copy = change.created[0];
    let document = session.document();
    let attached_copies = |parent: NodeId| {
        world_children(&session)
            .into_iter()
            .filter(|id| {
                matches!(
                    scene(document).nodes.get(id).map(|node| &node.data),
                    Some(NodeKind::Scope { .. })
                )
            })
            .filter(|id| parent_camera_of(document, *id) == Some(parent))
            .collect::<Vec<_>>()
    };
    let copies = attached_copies(copy);
    assert_eq!(copies.len(), 2, "both groups attached to the copy");
    assert_eq!(
        world_children(&session),
        vec![first_id, copies[0], rig_id, copies[1], free_id],
        "each copy sits right after its original"
    );
    assert_eq!(scene(document).options, vec![camera, copy]);
    // One undo takes the camera and both copied groups back out.
    session.undo(&session.revision()).unwrap();
    assert_eq!(world_children(&session), vec![first_id, rig_id, free_id]);
    assert_eq!(scene(session.document()).options, vec![camera]);
}

#[test]
fn duplicating_a_camera_when_the_groups_end_the_world_stays_in_bounds() {
    // Regression: two attached groups ending the World used to compute an
    // insertion index past the list and panic inside the session.
    let mut document = ProjectDocument::new("Groups end the world");
    let camera = posed_camera(
        &mut document,
        CameraPose {
            start: Mat4::identity().0,
            end: Mat4::identity().0,
        },
    );
    let follow = light_node("Follow light");
    let follow_id = follow.id;
    scene_mut(&mut document).nodes.insert(follow_id, follow);
    let first = group_node("Camera lights", vec![follow_id], Some(camera));
    let rig = group_node("Camera rig", Vec::new(), Some(camera));
    let (first_id, rig_id) = (first.id, rig.id);
    append_world(&mut document, first);
    append_world(&mut document, rig);
    let mut session = ProjectSession::new(document).unwrap();

    let change = session
        .apply(&session.revision(), &[Edit::DuplicateNode { node: camera }])
        .unwrap();
    let copy = change.created[0];
    let document = session.document();
    let copies = world_children(&session)
        .into_iter()
        .filter(|id| parent_camera_of(document, *id) == Some(copy))
        .collect::<Vec<_>>();
    assert_eq!(copies.len(), 2);
    assert_eq!(
        world_children(&session),
        vec![first_id, copies[0], rig_id, copies[1]]
    );
}

#[test]
fn moving_a_unit_onto_a_camera_row_attaches_it_and_moving_off_clears_it() {
    let (session, camera, group, _, sphere) = camera_scene();
    let mut session = session;
    let before = session.document().clone();
    let world = scene(session.document()).world;
    // A bare shape wraps into a fresh Attribute scope holding only it.
    session
        .apply(
            &session.revision(),
            &[Edit::MoveNode {
                node: sphere,
                parent: camera,
                after: None,
            }],
        )
        .unwrap();
    let document = session.document();
    let children = world_children(&session);
    let wrapper = children[1];
    assert_ne!(wrapper, group);
    assert_eq!(children[0], group, "the camera's own group stays first");
    assert_eq!(parent_camera_of(document, wrapper), Some(camera));
    match &scene(document).nodes[&wrapper].data {
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children,
            owned_object: Some(owner),
            ..
        } => {
            assert_eq!(children, &vec![sphere]);
            assert_eq!(*owner, sphere);
        }
        other => panic!("a wrapper scope owns the moved shape: {other:?}"),
    }
    // Moving the wrapped unit off the camera clears the camera reference.
    session
        .apply(
            &session.revision(),
            &[Edit::MoveNode {
                node: sphere,
                parent: world,
                after: None,
            }],
        )
        .unwrap();
    assert_eq!(parent_camera_of(session.document(), wrapper), None);
    session.undo(&session.revision()).unwrap();
    session.undo(&session.revision()).unwrap();
    assert_eq!(
        project::ContentId::of(session.document()),
        project::ContentId::of(&before)
    );
    assert_eq!(world_children(&session), vec![group, sphere]);
    assert!(
        !scene(session.document()).nodes.contains_key(&wrapper),
        "undo takes the wrapper down"
    );
}

#[test]
fn clearing_the_local_transform_keeps_a_camera_wrapper() {
    let (mut session, camera, _, _, sphere) = camera_scene();
    session
        .apply(
            &session.revision(),
            &[Edit::MoveNode {
                node: sphere,
                parent: camera,
                after: None,
            }],
        )
        .unwrap();
    let wrapper = world_children(&session)[1];
    for value in [Mat4::translation(0., 1., 0.), Mat4::identity()] {
        session
            .apply(
                &session.revision(),
                &[Edit::SetLocalTransform {
                    node: sphere,
                    value: LocalTransform::Matrix(value.0),
                }],
            )
            .unwrap();
    }
    assert_eq!(world_children(&session)[1], wrapper, "the wrapper stays");
    assert_eq!(parent_camera_of(session.document(), wrapper), Some(camera));
}

#[test]
fn moving_an_attribute_group_onto_a_camera_needs_no_wrapper() {
    let mut document = ProjectDocument::new("Group move");
    let camera = posed_camera(
        &mut document,
        CameraPose {
            start: Mat4::identity().0,
            end: Mat4::identity().0,
        },
    );
    let plain = group_node("Plain group", Vec::new(), None);
    let plain_id = plain.id;
    append_world(&mut document, plain);
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::MoveNode {
                node: plain_id,
                parent: camera,
                after: None,
            }],
        )
        .unwrap();
    let document = session.document();
    assert_eq!(parent_camera_of(document, plain_id), Some(camera));
    assert!(world_children(&session).contains(&plain_id));
    session.undo(&session.revision()).unwrap();
    assert_eq!(parent_camera_of(session.document(), plain_id), None);
}

#[test]
fn duplicating_a_scene_reparents_camera_children_to_the_new_cameras() {
    let (session, _, group, _, _) = camera_scene();
    let mut session = session;
    let source = session.document().default_scene.unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::DuplicateScene {
                scene: source,
                location: "Scenes/copy.scene".into(),
            }],
        )
        .unwrap();
    let document = session.document();
    let copy_id = *(document
        .scenes
        .keys()
        .find(|id| **id != source)
        .expect("a duplicated scene"));
    let copy = &document.scenes[&copy_id];
    let copy_camera = copy.render_camera.expect("the render camera copies");
    assert_eq!(copy.options, vec![copy_camera]);
    let NodeKind::Scope { children, .. } = &copy.nodes[&copy.world].data else {
        unreachable!()
    };
    let copy_group = children[0];
    match &copy.nodes[&copy_group].data {
        NodeKind::Scope {
            parent_camera: Some(camera),
            ..
        } => assert_eq!(*camera, copy_camera, "the copy follows the copied camera"),
        other => panic!("the copied group keeps its parent_camera: {other:?}"),
    }
    assert_ne!(copy_group, group, "fresh ids");
    evaluate(&document.scene_view(Some(copy_id)).unwrap()).unwrap();
}

#[test]
fn create_camera_carries_an_identity_pose() {
    let (document, _, _) = native_scene();
    let scene_id = document.default_scene.unwrap();
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::CreateCamera { scene: scene_id }],
        )
        .unwrap();
    let document = session.document();
    let camera = scene(document).options[0];
    let pose = pose_of(document, camera).expect("a new camera is posed");
    assert_eq!(
        (pose.start, pose.end),
        (Mat4::identity().0, Mat4::identity().0)
    );
}
