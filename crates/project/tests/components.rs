//! the component add/remove edits — the area light
//! entity, the explicit local-binding clear, and the reverse-orientation
//! toggle — with the dedicated-group rules, the
//! shared-with-sibling cancellation, and undo restoring shapes and groups.
mod common;
use common::*;
use project::*;

fn material(document: &mut ProjectDocument) -> ResourceId {
    let resource = Resource {
        id: ResourceId::new(),
        label: "matte".into(),
        location: "Materials/matte.material".into(),
        edition: Revision::initial(),
        kind: ResourceKind::Material,
        texture_value_type: None,
        implementation: "matte".into(),
        parameters: Vec::new(),
        definition_scope: None,
        declared_in: None,
        origin: None,
    };
    let id = resource.id;
    document.resources.insert(id, resource);
    id
}

fn area_light(implementation: &str) -> Option<AreaLightContent> {
    Some(AreaLightContent {
        implementation: implementation.into(),
        parameters: vec![],
    })
}

fn area_light_with(implementation: &str, parameters: Vec<Parameter>) -> Option<AreaLightContent> {
    Some(AreaLightContent {
        implementation: implementation.into(),
        parameters,
    })
}

/// The children ids of one scope node.
fn children(document: &ProjectDocument, id: NodeId) -> Vec<NodeId> {
    let NodeKind::Scope { children, .. } = &scene(document).nodes[&id].data else {
        panic!("not a scope");
    };
    children.clone()
}

/// The dedicated group that holds `object` (findable via any scope parent).
fn group_of(document: &ProjectDocument, object: NodeId) -> NodeId {
    scene(document)
        .nodes
        .values()
        .find(|node| {
            matches!(&node.data, NodeKind::Scope { children, .. } if children.contains(&object))
        })
        .expect("the object should sit in a dedicated group by now")
        .id
}

/// The head member of the object's dedicated group (the component slot).
fn group_children_first(document: &ProjectDocument, object: NodeId) -> NodeId {
    children(document, group_of(document, object))[0]
}

/// The World's transform state child, i.e. the placement node.
fn placement_of(document: &ProjectDocument, world: NodeId) -> NodeId {
    children(document, world)
        .into_iter()
        .find(|child| {
            matches!(
                &scene(document).nodes[child].data,
                NodeKind::State {
                    operation: StateOperation::Transform(_),
                    ..
                }
            )
        })
        .expect("the placement state")
}

fn evaluate(session: &ProjectSession) -> EvaluatedScene {
    let view = session
        .document()
        .scene_view(session.document().default_scene.unwrap())
        .unwrap();
    project::evaluate(&view).unwrap()
}

/// An outer group whose area light entity two shapes share, plus both shapes.
fn shared_light_scene() -> (ProjectDocument, NodeId, NodeId, NodeId) {
    let mut document = ProjectDocument::new("Shared light");
    let light = Node::new(
        "Ceiling",
        NodeKind::Entity {
            entity: EntityKind::AreaLight,
            implementation: "diffuse".into(),
            parameters: vec![],
            pose: None,
        },
    );
    let shape = |label: &str| {
        Node::new(
            label,
            NodeKind::Entity {
                entity: EntityKind::Shape,
                implementation: "sphere".into(),
                parameters: vec![Parameter::floats("radius", [1.])],
                pose: None,
            },
        )
    };
    let first = shape("Left");
    let second = shape("Right");
    let group = Node::new(
        "Pair",
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children: vec![light.id, first.id, second.id],
            owned_object: None,
            parent_camera: None,
        },
    );
    append_world(&mut document, group);
    for node in [light.clone(), first.clone(), second.clone()] {
        scene_mut(&mut document).nodes.insert(node.id, node);
    }
    document.validate().unwrap();
    (document, light.id, first.id, second.id)
}

/// A shape inside a group whose head state toggles reverse orientation.
fn outer_toggle_scene() -> (ProjectDocument, NodeId, NodeId) {
    let mut document = ProjectDocument::new("Outer toggle");
    let flip = Node::new(
        "Flip state",
        NodeKind::State {
            operation: StateOperation::ReverseOrientation,
        },
    );
    let shape = Node::new(
        "Sphere",
        NodeKind::Entity {
            entity: EntityKind::Shape,
            implementation: "sphere".into(),
            parameters: vec![Parameter::floats("radius", [1.])],
            pose: None,
        },
    );
    let group = Node::new(
        "Pair",
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children: vec![flip.id, shape.id],
            owned_object: None,
            parent_camera: None,
        },
    );
    let group_id = group.id;
    append_world(&mut document, group);
    for node in [flip.clone(), shape.clone()] {
        scene_mut(&mut document).nodes.insert(node.id, node);
    }
    document.validate().unwrap();
    (document, group_id, shape.id)
}

fn apply(session: &mut ProjectSession, edits: &[Edit]) -> ChangeSet {
    session.apply(&session.revision(), edits).unwrap()
}

#[test]
fn set_area_light_wraps_rewrites_and_undoes() {
    let (document, placement, shape) = native_scene();
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    apply(
        &mut session,
        &[Edit::SetAreaLight {
            node: shape,
            light: area_light("diffuse"),
        }],
    );
    session.document().validate().unwrap();
    let group = group_of(session.document(), shape);
    let members = children(session.document(), group);
    let light_id = members[0];
    match &scene(session.document()).nodes[&light_id].data {
        NodeKind::Entity {
            entity: EntityKind::AreaLight,
            implementation,
            ..
        } => assert_eq!(implementation, "diffuse"),
        other => panic!("the group must open with an area light entity: {other:?}"),
    }
    assert_eq!(members[1], shape);
    // The wrapper replaced the shape in the World.
    let world_children = children(session.document(), world);
    assert_eq!(world_children.len(), 2);
    assert!(world_children.contains(&placement));

    // A second add rewrites the entity in place: no second node appears.
    let rewritten = apply(
        &mut session,
        &[Edit::SetAreaLight {
            node: shape,
            light: area_light_with("area", vec![Parameter::floats("scale", [1.])]),
        }],
    );
    assert!(rewritten.nodes.contains(&light_id));
    match &scene(session.document()).nodes[&light_id].data {
        NodeKind::Entity {
            implementation,
            parameters,
            ..
        } => {
            assert_eq!(implementation, "area");
            assert_eq!(parameters.len(), 1);
        }
        other => panic!("still an entity: {other:?}"),
    }
    session.document().validate().unwrap();

    // Undo both edits: the shape returns to the World, the entity and the
    // group disappear.
    session.undo(&session.revision()).unwrap();
    session.undo(&session.revision()).unwrap();
    assert_eq!(children(session.document(), world), vec![placement, shape]);
    assert!(!scene(session.document()).nodes.contains_key(&light_id));
}

#[test]
fn set_area_light_sits_at_the_head_of_the_binding_run() {
    let (mut document, _, shape) = native_scene();
    let material = material(&mut document);
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    apply(
        &mut session,
        &[Edit::SetLocalBinding {
            node: shape,
            material: Some(MaterialBinding::Resource(material)),
            medium: None,
        }],
    );
    apply(
        &mut session,
        &[Edit::SetAreaLight {
            node: shape,
            light: area_light("diffuse"),
        }],
    );
    let group = group_of(session.document(), shape);
    let members = children(session.document(), group);
    assert!(
        matches!(
            &scene(session.document()).nodes[&members[0]].data,
            NodeKind::Entity {
                entity: EntityKind::AreaLight,
                ..
            }
        ),
        "the area light leads the run: {members:?}"
    );
    assert!(matches!(
        &scene(session.document()).nodes[&members[1]].data,
        NodeKind::State {
            operation: StateOperation::Material(_),
            ..
        }
    ));
    assert_eq!(members[2], shape);
    session.document().validate().unwrap();
}

#[test]
fn set_area_light_null_removes_a_local_entity_and_unwraps() {
    let (document, placement, shape) = native_scene();
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    apply(
        &mut session,
        &[Edit::SetAreaLight {
            node: shape,
            light: area_light("diffuse"),
        }],
    );
    apply(
        &mut session,
        &[Edit::SetAreaLight {
            node: shape,
            light: None,
        }],
    );
    session.document().validate().unwrap();
    assert_eq!(children(session.document(), world), vec![placement, shape]);
    let evaluated = evaluate(&session);
    assert_eq!(evaluated.nodes[&shape].area_light, None);
}

#[test]
fn set_area_light_null_cancels_a_light_shared_with_a_sibling() {
    let (document, light, first, second) = shared_light_scene();
    let mut session = ProjectSession::new(document).unwrap();
    apply(
        &mut session,
        &[Edit::SetAreaLight {
            node: first,
            light: None,
        }],
    );
    session.document().validate().unwrap();
    // The first shape got its own group with a cancellation entity in
    // front — an empty implementation evaluates to no area light — while
    // the sibling keeps the shared entity itself.
    let cancel = group_children_first(session.document(), first);
    match &scene(session.document()).nodes[&cancel].data {
        NodeKind::Entity {
            entity: EntityKind::AreaLight,
            implementation,
            ..
        } => assert!(
            implementation.is_empty(),
            "the cancel entity carries no implementation"
        ),
        other => panic!("unexpected cancel node: {other:?}"),
    }
    assert_eq!(evaluate(&session).nodes[&first].area_light, None);
    assert_eq!(evaluate(&session).nodes[&second].area_light, Some(light));
    assert!(
        scene(session.document()).nodes.contains_key(&light),
        "the shared entity itself stays"
    );
    // Undo returns the plain shared pair.
    session.undo(&session.revision()).unwrap();
    let evaluated = evaluate(&session);
    assert_eq!(evaluated.nodes[&first].area_light, Some(light));
    assert_eq!(evaluated.nodes[&second].area_light, Some(light));
}

#[test]
fn clear_local_binding_removes_states_and_redo_restores() {
    let (mut document, _, shape) = native_scene();
    let material = material(&mut document);
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    apply(
        &mut session,
        &[Edit::SetLocalBinding {
            node: shape,
            material: Some(MaterialBinding::Resource(material)),
            medium: None,
        }],
    );
    let cleared = apply(
        &mut session,
        &[Edit::ClearLocalBinding {
            node: shape,
            material: true,
            medium: true,
        }],
    );
    assert!(!cleared.nodes.is_empty());
    assert_eq!(
        children(session.document(), world),
        vec![placement_of(session.document(), world), shape]
    );
    // Clearing on a shape without a dedicated group is a no-op, not an error.
    apply(
        &mut session,
        &[Edit::ClearLocalBinding {
            node: shape,
            material: true,
            medium: true,
        }],
    );
    // Undo restores the binding and its group; redo clears it again.
    session.undo(&session.revision()).unwrap();
    assert_eq!(children(session.document(), world).len(), 2);
    let redone = session.redo(&session.revision()).unwrap();
    assert!(!redone.nodes.is_empty());
    assert_eq!(
        children(session.document(), world),
        vec![placement_of(session.document(), world), shape]
    );
    session.document().validate().unwrap();
}

#[test]
fn set_reverse_orientation_is_effective_idempotent_and_undoable() {
    let (document, placement, shape) = native_scene();
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    // No outer toggle: enabling inserts one toggle in the dedicated group.
    apply(
        &mut session,
        &[Edit::SetReverseOrientation {
            node: shape,
            enabled: true,
        }],
    );
    assert!(evaluate(&session).nodes[&shape].reverse_orientation);
    let group = group_of(session.document(), shape);
    let mut toggles = |session: &ProjectSession| {
        children(session.document(), group)
            .into_iter()
            .filter(|child| {
                matches!(
                    &scene(session.document()).nodes[child].data,
                    NodeKind::State {
                        operation: StateOperation::ReverseOrientation,
                        ..
                    }
                )
            })
            .count()
    };
    assert_eq!(toggles(&session), 1);
    // Enabling again changes nothing — the edit is idempotent.
    apply(
        &mut session,
        &[Edit::SetReverseOrientation {
            node: shape,
            enabled: true,
        }],
    );
    assert_eq!(toggles(&session), 1);
    assert!(evaluate(&session).nodes[&shape].reverse_orientation);
    // Disabling removes the toggle and the group.
    apply(
        &mut session,
        &[Edit::SetReverseOrientation {
            node: shape,
            enabled: false,
        }],
    );
    assert!(!evaluate(&session).nodes[&shape].reverse_orientation);
    assert_eq!(children(session.document(), world), vec![placement, shape]);
    session.document().validate().unwrap();

    // With an outer toggle on, disabling keeps a local cancelling toggle;
    // enabling back removes it again.
    let (document, _, shape2) = outer_toggle_scene();
    let mut session = ProjectSession::new(document).unwrap();
    assert!(
        evaluate(&session).nodes[&shape2].reverse_orientation,
        "the outer toggle is on"
    );
    apply(
        &mut session,
        &[Edit::SetReverseOrientation {
            node: shape2,
            enabled: false,
        }],
    );
    assert!(!evaluate(&session).nodes[&shape2].reverse_orientation);
    session.document().validate().unwrap();
    apply(
        &mut session,
        &[Edit::SetReverseOrientation {
            node: shape2,
            enabled: true,
        }],
    );
    assert!(evaluate(&session).nodes[&shape2].reverse_orientation);
    session.document().validate().unwrap();
}

#[test]
fn group_targets_bind_components_at_the_head() {
    let (mut document, light, first, second) = shared_light_scene();
    let medium = Resource {
        id: ResourceId::new(),
        label: "fog".into(),
        location: "Media/fog.medium".into(),
        edition: Revision::initial(),
        kind: ResourceKind::Medium,
        texture_value_type: None,
        implementation: "homogeneous".into(),
        parameters: Vec::new(),
        definition_scope: None,
        declared_in: None,
        origin: None,
    };
    let medium_id = medium.id;
    document.resources.insert(medium_id, medium);
    let material = material(&mut document);
    let group = group_of(&document, first);
    let mut session = ProjectSession::new(document).unwrap();
    // Point the shared group instead of one shape: the states and the
    // light entity stay at the head, in the component order, and the
    // two shapes both see them.
    apply(
        &mut session,
        &[Edit::SetLocalBinding {
            node: group,
            material: Some(MaterialBinding::Resource(material)),
            medium: Some(MediumInterface {
                inside: None,
                outside: Some(medium_id),
            }),
        }],
    );
    let members = children(session.document(), group);
    let heads = &members[..members.len() - 2];
    assert_eq!(heads.len(), 3, "light, material, medium lead: {members:?}");
    matches_entity_area_light(&session, heads[0]);
    assert!(matches!(
        &scene(session.document()).nodes[&heads[1]].data,
        NodeKind::State { operation: StateOperation::Material(MaterialBinding::Resource(id)), .. } if *id == material
    ));
    assert!(matches!(
        &scene(session.document()).nodes[&heads[2]].data,
        NodeKind::State { operation: StateOperation::Medium { inside: None, outside: Some(id) }, .. } if *id == medium_id
    ));
    assert_eq!(&members[members.len() - 2..], &[first, second]);
    session.document().validate().unwrap();

    // A group target rewrites the head instead of growing: setting the area
    // light on the group replaces the leading entity in place.
    apply(
        &mut session,
        &[Edit::SetAreaLight {
            node: group,
            light: area_light_with("diffuse", vec![Parameter::floats("scale", [2.])]),
        }],
    );
    let members = children(session.document(), group);
    assert_eq!(
        members[0], light,
        "the head entity is rewritten, not duplicated"
    );
    if let NodeKind::Entity { parameters, .. } = &scene(session.document()).nodes[&light].data {
        assert_eq!(parameters.len(), 1);
    } else {
        panic!("the head stays an entity");
    }

    // Clearing a group's head states removes only them; the shared light
    // entity and the shapes stay.
    apply(
        &mut session,
        &[Edit::ClearLocalBinding {
            node: group,
            material: true,
            medium: true,
        }],
    );
    let members = children(session.document(), group);
    assert_eq!(members, vec![light, first, second]);
    session.document().validate().unwrap();

    // Removing the group's area light deletes the head entity; undo brings
    // the whole apply back.
    apply(
        &mut session,
        &[Edit::SetAreaLight {
            node: group,
            light: None,
        }],
    );
    assert_eq!(children(session.document(), group), vec![first, second]);
    session.undo(&session.revision()).unwrap();
    assert_eq!(
        children(session.document(), group),
        vec![light, first, second]
    );
}

fn matches_entity_area_light(session: &ProjectSession, id: NodeId) {
    assert!(
        matches!(
            &scene(session.document()).nodes[&id].data,
            NodeKind::Entity {
                entity: EntityKind::AreaLight,
                ..
            }
        ),
        "an area light leads the head"
    );
}

fn assert_medium(
    scene: &SceneDocument,
    id: NodeId,
    inside: Option<ResourceId>,
    outside: Option<ResourceId>,
) {
    match &scene.nodes[&id].data {
        NodeKind::State {
            operation:
                StateOperation::Medium {
                    inside: current_inside,
                    outside: current_outside,
                },
        } => {
            assert_eq!(*current_inside, inside, "inside medium");
            assert_eq!(*current_outside, outside, "outside medium");
        }
        other => panic!("a medium state: {other:?}"),
    }
}

#[test]
fn camera_medium_binds_in_the_options_list() {
    let mut document = ProjectDocument::new("Camera");
    let camera = Node::new(
        "Perspective",
        NodeKind::Entity {
            entity: EntityKind::Camera,
            implementation: "perspective".into(),
            parameters: vec![],
            pose: None,
        },
    );
    let camera_id = camera.id;
    let scene_id = document.default_scene.unwrap();
    {
        let scene = scene_mut(&mut document);
        scene.options.push(camera_id);
        scene.nodes.insert(camera_id, camera);
    }
    let medium = Resource {
        id: ResourceId::new(),
        label: "fog".into(),
        location: "Media/fog.medium".into(),
        edition: Revision::initial(),
        kind: ResourceKind::Medium,
        texture_value_type: None,
        implementation: "homogeneous".into(),
        parameters: Vec::new(),
        definition_scope: None,
        declared_in: None,
        origin: None,
    };
    let medium_id = medium.id;
    document.resources.insert(medium_id, medium);
    document.validate().unwrap();
    let mut session = ProjectSession::new(document).unwrap();
    // A camera takes no material, and its medium sits in the options list
    // right before the camera node.
    let error = {
        let revision = session.revision();
        session
            .apply(
                &revision,
                &[Edit::SetLocalBinding {
                    node: camera_id,
                    material: Some(MaterialBinding::None),
                    medium: None,
                }],
            )
            .unwrap_err()
            .to_string()
    };
    let _ = error;
    apply(
        &mut session,
        &[Edit::SetLocalBinding {
            node: camera_id,
            material: None,
            medium: Some(MediumInterface {
                inside: Some(medium_id),
                outside: Some(medium_id),
            }),
        }],
    );
    let snapshot = scene(session.document()).clone();
    assert_eq!(snapshot.options.len(), 2);
    assert_eq!(snapshot.options[1], camera_id);
    assert_medium(
        &snapshot,
        snapshot.options[0],
        Some(medium_id),
        Some(medium_id),
    );
    session.document().validate().unwrap();

    // Setting again rewrites the state node in place. The camera UI sends its
    // preserved inside value with the changed PBRT outside binding.
    apply(
        &mut session,
        &[Edit::SetLocalBinding {
            node: camera_id,
            material: None,
            medium: Some(MediumInterface {
                inside: Some(medium_id),
                outside: None,
            }),
        }],
    );
    let snapshot = scene(session.document()).clone();
    assert_eq!(snapshot.options.len(), 2, "no second state appears");
    assert_medium(&snapshot, snapshot.options[0], Some(medium_id), None);

    // The explicit clear removes the state from the options list; undo
    // restores it.
    apply(
        &mut session,
        &[Edit::ClearLocalBinding {
            node: camera_id,
            material: false,
            medium: true,
        }],
    );
    let snapshot = scene(session.document()).clone();
    assert_eq!(snapshot.options, vec![camera_id]);
    session.undo(&session.revision()).unwrap();
    let snapshot = scene(session.document()).clone();
    assert_eq!(snapshot.options.len(), 2);
    session.document().validate().unwrap();
}

/// The shared-case chain needs both directions to survive undo:
/// cancelling a group's inherited light, and re-adding a shape's after a
/// cancel rewrote the cancel slot.
#[test]
fn group_inherited_light_cancel_and_undo() {
    // An outer group holds the light; the inner pair inherits it.
    let mut document = ProjectDocument::new("Nested light");
    let light = Node::new(
        "Stage light",
        NodeKind::Entity {
            entity: EntityKind::AreaLight,
            implementation: "diffuse".into(),
            parameters: vec![],
            pose: None,
        },
    );
    let shape = |label: &str| {
        Node::new(
            label,
            NodeKind::Entity {
                entity: EntityKind::Shape,
                implementation: "sphere".into(),
                parameters: vec![Parameter::floats("radius", [1.])],
                pose: None,
            },
        )
    };
    let first = shape("Left");
    let second = shape("Right");
    let pair = Node::new(
        "Pair",
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children: vec![first.id, second.id],
            owned_object: None,
            parent_camera: None,
        },
    );
    let outer = Node::new(
        "Stage",
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children: vec![light.id, pair.id],
            owned_object: None,
            parent_camera: None,
        },
    );
    append_world(&mut document, outer.clone());
    for node in [light.clone(), first.clone(), second.clone(), pair.clone()] {
        scene_mut(&mut document).nodes.insert(node.id, node);
    }
    document.validate().unwrap();
    let (light, first, second, pair) = (light.id, first.id, second.id, pair.id);
    let mut session = ProjectSession::new(document).unwrap();
    assert_eq!(evaluate(&session).nodes[&first].area_light, Some(light));

    // The pair cancels its inherited light without touching the outer one.
    apply(
        &mut session,
        &[Edit::SetAreaLight {
            node: pair,
            light: None,
        }],
    );
    let evaluated = evaluate(&session);
    assert_eq!(evaluated.nodes[&first].area_light, None);
    assert_eq!(evaluated.nodes[&second].area_light, None);
    session.document().validate().unwrap();

    // Undo restores the inherited light — the cancel node must not survive
    // as an unreachable scene node.
    session.undo(&session.revision()).unwrap();
    let evaluated = evaluate(&session);
    assert_eq!(evaluated.nodes[&first].area_light, Some(light));
    session.document().validate().unwrap();
}

#[test]
fn a_shape_readd_after_a_cancel_rewrites_the_cancel_slot() {
    let (document, light, first, _second) = shared_light_scene();
    let mut session = ProjectSession::new(document).unwrap();
    // Cancel the shared light for the first shape only.
    apply(
        &mut session,
        &[Edit::SetAreaLight {
            node: first,
            light: None,
        }],
    );
    assert_eq!(evaluate(&session).nodes[&first].area_light, None);
    // Adding again must take effect: the cancel slot in the dedicated group
    // is rewritten in place, so the run never holds both an override and a
    // cancel that would still win in graphics-state order.
    apply(
        &mut session,
        &[Edit::SetAreaLight {
            node: first,
            light: area_light_with("diffuse", vec![Parameter::floats("scale", [2.])]),
        }],
    );
    let effective = evaluate(&session).nodes[&first]
        .area_light
        .expect("the re-added light must win");
    match &scene(session.document()).nodes[&effective].data {
        NodeKind::Entity {
            entity: EntityKind::AreaLight,
            implementation,
            parameters,
            ..
        } => {
            assert_eq!(implementation, "diffuse");
            assert_eq!(parameters.len(), 1);
        }
        other => panic!("an area light entity: {other:?}"),
    }
    let _ = light;
    session.document().validate().unwrap();
    // And undo unwraps everything again: two applies were made.
    session.undo(&session.revision()).unwrap();
    session.undo(&session.revision()).unwrap();
    session.document().validate().unwrap();
}
