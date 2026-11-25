mod common;
use common::*;
use project::*;

fn world_children(session: &ProjectSession) -> Vec<NodeId> {
    let scene = scene(session.document());
    match &scene.nodes[&scene.world].data {
        NodeKind::Scope { children, .. } => children.clone(),
        _ => unreachable!(),
    }
}

#[test]
fn unpack_instance_without_owned_wrapper_creates_a_group_and_is_undoable() {
    let (mut document, _, original_shape) = native_scene();
    let (definition, _) = common::object_definition(&mut document);
    let instance = Node::new("Imported instance", NodeKind::Instance { definition });
    append_world(&mut document, instance.clone());
    let mut session = ProjectSession::new(document).unwrap();

    let change = session
        .apply(
            &session.revision(),
            &[Edit::UnpackObjectInstance { node: instance.id }],
        )
        .unwrap();
    let group_id = change.created[0];
    let group = &scene(session.document()).nodes[&group_id];
    assert_eq!(group.label, "Imported instance");
    let NodeKind::Scope {
        scope: ScopeKind::Attribute,
        children,
        owned_object: None,
        ..
    } = &group.data
    else {
        panic!("unpacked instance should be a plain Attribute group");
    };
    assert!(children.iter().any(|child| {
        matches!(
            scene(session.document()).nodes[child].data,
            NodeKind::Entity {
                entity: EntityKind::Shape,
                ..
            }
        )
    }));
    assert!(!scene(session.document()).nodes.contains_key(&instance.id));
    session.document().validate().unwrap();
    assert!(
        session
            .apply(
                &session.revision(),
                &[Edit::UnpackObjectInstance {
                    node: original_shape,
                }],
            )
            .is_err()
    );

    session.undo(&session.revision()).unwrap();
    assert!(scene(session.document()).nodes.contains_key(&instance.id));
    assert!(!scene(session.document()).nodes.contains_key(&group_id));
    session.document().validate().unwrap();
}

#[test]
fn unpack_keeps_the_wrappers_other_children_in_place() {
    let (mut document, _, _) = native_scene();
    let (definition, _) = common::object_definition(&mut document);
    let translate = Node::new(
        "Translate",
        NodeKind::State {
            operation: StateOperation::Transform(TransformOperation::Translate([1., 0., 0.])),
        },
    );
    let flip = Node::new(
        "ReverseOrientation",
        NodeKind::State {
            operation: StateOperation::ReverseOrientation,
        },
    );
    let instance = Node::new("Placed", NodeKind::Instance { definition });
    let identity = math::Mat4::identity().0;
    let camera = Node::new(
        "Camera",
        NodeKind::Entity {
            entity: EntityKind::Camera,
            implementation: "perspective".into(),
            parameters: Vec::new(),
            pose: Some(CameraPose {
                start: identity,
                end: identity,
            }),
        },
    );
    let wrapper = Node::new(
        "Placed",
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children: vec![translate.id, flip.id, instance.id],
            owned_object: Some(instance.id),
            parent_camera: Some(camera.id),
        },
    );
    {
        let scene_id = document.default_scene.unwrap();
        let scene = std::sync::Arc::make_mut(document.scenes.get_mut(&scene_id).unwrap());
        scene.options.push(camera.id);
        for node in [
            translate.clone(),
            flip.clone(),
            instance.clone(),
            camera.clone(),
        ] {
            scene.nodes.insert(node.id, node);
        }
    }
    append_world(&mut document, wrapper.clone());
    document.validate().unwrap();
    let mut session = ProjectSession::new(document).unwrap();

    let change = session
        .apply(
            &session.revision(),
            &[Edit::UnpackObjectInstance { node: instance.id }],
        )
        .unwrap();
    // The first created node is the plain group that replaced the wrapper.
    let group = change.created[0];
    assert_eq!(scene(session.document()).nodes[&group].label, "Placed");
    assert!(!scene(session.document()).nodes.contains_key(&wrapper.id));
    let kept = children(&session, group);
    assert_eq!(&kept[..2], &[translate.id, flip.id]);
    assert!(kept.len() > 2 && !kept.contains(&instance.id));
    assert!(matches!(
        scene(session.document()).nodes[&group].data,
        NodeKind::Scope { parent_camera: Some(attached), .. } if attached == camera.id
    ));
    session.document().validate().unwrap();

    session.undo(&session.revision()).unwrap();
    assert_eq!(
        children(&session, wrapper.id),
        vec![translate.id, flip.id, instance.id]
    );
    assert!(!scene(session.document()).nodes.contains_key(&group));
    session.document().validate().unwrap();
}

#[test]
fn unpack_keeps_captured_transform_times_out_of_the_world() {
    let (mut document, _, _) = native_scene();
    let (definition, _) = common::object_definition(&mut document);
    let mut start = math::Mat4::identity().0;
    start[3] = 2.;
    document.resources.get_mut(&definition).unwrap().declared_in = Some(DeclarationState {
        transform: Some(CapturedTransform {
            start,
            end: start,
            active: ActiveSamples::All,
            times: [0., 0.5],
        }),
        graphics: None,
    });
    let instance = Node::new("Timed", NodeKind::Instance { definition });
    append_world(&mut document, instance.clone());
    let mut session = ProjectSession::new(document).unwrap();

    session
        .apply(
            &session.revision(),
            &[Edit::UnpackObjectInstance { node: instance.id }],
        )
        .unwrap();
    assert!(
        !scene(session.document())
            .nodes
            .values()
            .any(|node| matches!(
                node.data,
                NodeKind::State {
                    operation: StateOperation::TransformTimes(_)
                }
            ))
    );
    session.document().validate().unwrap();
}

#[test]
fn unpack_rebases_identity_and_replace_transforms_to_keep_instance_placement() {
    let (mut document, _, shape) = native_scene();
    let (definition, root) = common::object_definition(&mut document);
    let translate = Node::new(
        "Translate",
        NodeKind::State {
            operation: StateOperation::Transform(TransformOperation::Translate([2., 0., 0.])),
        },
    );
    let identity = Node::new(
        "Identity",
        NodeKind::State {
            operation: StateOperation::Transform(TransformOperation::Identity),
        },
    );
    let replace = Node::new(
        "Transform",
        NodeKind::State {
            operation: StateOperation::Transform(TransformOperation::Replace(
                math::Mat4::translation(7., 0., 0.).0,
            )),
        },
    );
    let NodeKind::Scope { children, .. } =
        &mut document.definition_nodes.get_mut(&root).unwrap().data
    else {
        unreachable!()
    };
    children.splice(0..0, [translate.id, identity.id, replace.id]);
    document.definition_nodes.insert(translate.id, translate);
    document.definition_nodes.insert(identity.id, identity);
    document.definition_nodes.insert(replace.id, replace);
    let placement = Node::new(
        "Placement",
        NodeKind::State {
            operation: StateOperation::Transform(TransformOperation::Translate([5., 0., 0.])),
        },
    );
    append_world(&mut document, placement);
    let instance = Node::new("Reset", NodeKind::Instance { definition });
    append_world(&mut document, instance.clone());
    let mut session = ProjectSession::new(document).unwrap();

    session
        .apply(
            &session.revision(),
            &[Edit::UnpackObjectInstance { node: instance.id }],
        )
        .unwrap();
    let evaluated = evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
    let copied_shape = scene(session.document())
        .nodes
        .values()
        .find(|node| {
            matches!(
                node.data,
                NodeKind::Entity {
                    entity: EntityKind::Shape,
                    ..
                }
            ) && node.id != shape
        })
        .unwrap();
    assert_eq!(
        evaluated.nodes[&copied_shape.id]
            .start
            .transform_point([0., 0., 0.]),
        [13., 2., 3.]
    );
    assert!(!scene(session.document()).nodes.contains_key(&instance.id));
}

#[test]
fn unpack_materializes_default_material_under_a_bound_instance_and_undoes_together() {
    let (mut document, _, _) = native_scene();
    let (definition, _) = common::object_definition(&mut document);
    document.resources.get_mut(&definition).unwrap().declared_in = Some(DeclarationState {
        transform: None,
        graphics: Some(CapturedGraphics {
            material: MaterialBinding::Default,
            inside_medium: None,
            outside_medium: None,
            reverse_orientation: false,
            color_space: "srgb".into(),
        }),
    });
    let material = Node::new(
        "Material",
        NodeKind::State {
            operation: StateOperation::Material(MaterialBinding::None),
        },
    );
    let instance = Node::new("Bound", NodeKind::Instance { definition });
    append_world(&mut document, material);
    append_world(&mut document, instance.clone());
    let mut session = ProjectSession::new(document).unwrap();

    let change = session
        .apply(
            &session.revision(),
            &[Edit::UnpackObjectInstance { node: instance.id }],
        )
        .unwrap();
    let material_id = session
        .document()
        .resources
        .values()
        .find(|resource| {
            resource.label == "Default Material"
                && resource.kind == ResourceKind::Material
                && resource.implementation == "diffuse"
                && resource.parameters.is_empty()
        })
        .unwrap()
        .id;
    assert_eq!(
        session.document().resources[&material_id].location,
        "Materials/Default Material.material"
    );
    assert!(
        change
            .created_items
            .contains(&ItemId::Resource(material_id))
    );
    let group = &scene(session.document()).nodes[&change.created[0]];
    let NodeKind::Scope { children, .. } = &group.data else {
        panic!("unpacked instance should become a group");
    };
    assert!(children.iter().any(|child| matches!(
        scene(session.document()).nodes[child].data,
        NodeKind::State {
            operation: StateOperation::Material(MaterialBinding::Resource(id)),
        } if id == material_id
    )));
    session.undo(&session.revision()).unwrap();
    assert!(!session.document().resources.contains_key(&material_id));
    assert!(scene(session.document()).nodes.contains_key(&instance.id));
}

#[test]
fn unpack_reuses_existing_default_material() {
    let (mut document, _, _) = native_scene();
    let (definition, _) = common::object_definition(&mut document);
    document.resources.get_mut(&definition).unwrap().declared_in = Some(DeclarationState {
        transform: None,
        graphics: Some(CapturedGraphics {
            material: MaterialBinding::Default,
            inside_medium: None,
            outside_medium: None,
            reverse_orientation: false,
            color_space: "srgb".into(),
        }),
    });
    let existing = Resource {
        id: ResourceId::new(),
        label: "Default Material".into(),
        location: "Materials/Default Material.material".into(),
        edition: Revision::initial(),
        kind: ResourceKind::Material,
        texture_value_type: None,
        implementation: "diffuse".into(),
        parameters: Vec::new(),
        definition_scope: None,
        declared_in: None,
        origin: None,
    };
    let existing_id = existing.id;
    document.resources.insert(existing_id, existing);
    let material = Node::new(
        "Material",
        NodeKind::State {
            operation: StateOperation::Material(MaterialBinding::None),
        },
    );
    let instance = Node::new("Bound", NodeKind::Instance { definition });
    append_world(&mut document, material);
    append_world(&mut document, instance.clone());
    let mut session = ProjectSession::new(document).unwrap();

    let change = session
        .apply(
            &session.revision(),
            &[Edit::UnpackObjectInstance { node: instance.id }],
        )
        .unwrap();
    assert!(
        !change
            .created_items
            .contains(&ItemId::Resource(existing_id))
    );
    assert_eq!(
        session
            .document()
            .resources
            .values()
            .filter(|resource| resource.label == "Default Material")
            .count(),
        1
    );
    let group = &scene(session.document()).nodes[&change.created[0]];
    let NodeKind::Scope { children, .. } = &group.data else {
        panic!("unpacked instance should become a group");
    };
    assert!(children.iter().any(|child| matches!(
        scene(session.document()).nodes[child].data,
        NodeKind::State {
            operation: StateOperation::Material(MaterialBinding::Resource(id)),
        } if id == existing_id
    )));
}

fn children(session: &ProjectSession, id: NodeId) -> Vec<NodeId> {
    match &scene(session.document()).nodes[&id].data {
        NodeKind::Scope { children, .. } => children.clone(),
        _ => Vec::new(),
    }
}

/// World ▸ [Placement, Sphere, Sphere group ▸ [Translate, Sphere]] where the
/// group is the preset's dedicated group, moved by `set_local_transform`.
fn scene_with_preset() -> (ProjectSession, NodeId, NodeId, NodeId) {
    let (document, _, shape) = native_scene();
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    let change = session
        .apply(
            &session.revision(),
            &[Edit::CreatePreset {
                preset: presets::BUILTIN_PRESETS
                    .iter()
                    .find(|preset| preset.name == "shape.sphere")
                    .unwrap()
                    .id,
                parent: world,
                after: None,
                definition: None,
                view: None,
            }],
        )
        .unwrap();
    let group = change.created[0];
    let preset_shape = *children(&session, group).last().unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::SetLocalTransform {
                node: preset_shape,
                value: LocalTransform::Trs(transform_trs::Trs {
                    translation: [1., 0., 0.],
                    rotation_euler_degrees: [0.; 3],
                    scale: [1.; 3],
                }),
            }],
        )
        .unwrap();
    assert_eq!(children(&session, group).len(), 2);
    (session, shape, group, preset_shape)
}

#[test]
fn delete_node_takes_the_dedicated_group_and_undo_restores_it() {
    let (mut session, shape, group, preset_shape) = scene_with_preset();
    let before = world_children(&session);
    session
        .apply(
            &session.revision(),
            &[Edit::DeleteNode { node: preset_shape }],
        )
        .unwrap();
    assert!(!world_children(&session).contains(&group));
    assert!(!scene(session.document()).nodes.contains_key(&preset_shape));
    // A shape in a shared scope goes alone; the transform before it stays.
    session
        .apply(&session.revision(), &[Edit::DeleteNode { node: shape }])
        .unwrap();
    assert_eq!(
        world_children(&session).len(),
        1,
        "the Placement state stays"
    );
    session.undo(&session.revision()).unwrap();
    session.undo(&session.revision()).unwrap();
    assert_eq!(world_children(&session), before);
    assert!(scene(session.document()).nodes.contains_key(&preset_shape));
    evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
}

#[test]
fn duplicate_node_copies_the_unit_after_itself_with_new_ids() {
    let (mut session, _, group, preset_shape) = scene_with_preset();
    let change = session
        .apply(
            &session.revision(),
            &[Edit::DuplicateNode { node: preset_shape }],
        )
        .unwrap();
    let world = world_children(&session);
    let index = world.iter().position(|id| *id == group).unwrap();
    let copy = world[index + 1];
    assert_ne!(copy, group);
    assert_eq!(change.created[0], copy);
    assert_eq!(change.created.len(), 3, "group, translate and sphere");
    let document = session.document();
    let original = &scene(document).nodes[&group].label;
    assert_eq!(
        scene(document).nodes[&copy].label,
        format!("{original} (1)")
    );
    let copied = children(&session, copy);
    assert!(
        copied
            .iter()
            .all(|id| !children(&session, group).contains(id))
    );
    let copied_owner = match &scene(document).nodes[&copy].data {
        NodeKind::Scope { owned_object, .. } => *owned_object,
        _ => unreachable!(),
    };
    assert_eq!(copied_owner, copied.last().copied());
    assert_ne!(copied_owner, Some(preset_shape));
    assert_eq!(
        scene(document).nodes[&copied_owner.unwrap()].label,
        format!("{original} (1)")
    );
    evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
    session.undo(&session.revision()).unwrap();
    assert!(!world_children(&session).contains(&copy));
    assert!(!scene(session.document()).nodes.contains_key(&copied[1]));
}

#[test]
fn create_and_rename_use_unique_visible_owner_labels() {
    let (document, _, _) = native_scene();
    let world = scene(&document).world;
    let preset = presets::SHAPE_SPHERE.id;
    let mut session = ProjectSession::new(document).unwrap();
    let first = session
        .apply(
            &session.revision(),
            &[Edit::CreatePreset {
                preset,
                parent: world,
                after: None,
                definition: None,
                view: None,
            }],
        )
        .unwrap();
    let first_wrapper = first.created[0];
    let first_owner = match scene(session.document()).nodes[&first_wrapper].data {
        NodeKind::Scope {
            owned_object: Some(owner),
            ..
        } => owner,
        _ => unreachable!(),
    };
    let second = session
        .apply(
            &session.revision(),
            &[Edit::CreatePreset {
                preset,
                parent: world,
                after: None,
                definition: None,
                view: None,
            }],
        )
        .unwrap();
    let second_wrapper = second.created[0];
    let second_owner = match scene(session.document()).nodes[&second_wrapper].data {
        NodeKind::Scope {
            owned_object: Some(owner),
            ..
        } => owner,
        _ => unreachable!(),
    };
    assert_ne!(
        scene(session.document()).nodes[&first_owner].label,
        scene(session.document()).nodes[&second_owner].label
    );
    assert_eq!(
        scene(session.document()).nodes[&second_wrapper].label,
        scene(session.document()).nodes[&second_owner].label
    );

    let before = session.document().clone();
    session
        .apply(
            &session.revision(),
            &[Edit::RenameNode {
                node: second_owner,
                label: "Renamed sphere".into(),
            }],
        )
        .unwrap();
    assert_eq!(
        scene(session.document()).nodes[&second_wrapper].label,
        "Renamed sphere"
    );
    session.undo(&session.revision()).unwrap();
    assert_eq!(
        project::ContentId::of(session.document()),
        project::ContentId::of(&before)
    );
    assert_eq!(
        scene(session.document()).nodes[&second_wrapper].label,
        scene(session.document()).nodes[&second_owner].label
    );
    session.redo(&session.revision()).unwrap();
    assert_eq!(
        scene(session.document()).nodes[&second_wrapper].label,
        "Renamed sphere"
    );
    assert_eq!(
        scene(session.document()).nodes[&second_owner].label,
        "Renamed sphere"
    );
}

#[test]
fn move_node_reparents_the_unit_and_rejects_cycles() {
    let (mut document, _, shape) = native_scene();
    let world = scene(&document).world;
    let ordinary_group = Node::new(
        "Ordinary group",
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children: Vec::new(),
            owned_object: None,
            parent_camera: None,
        },
    );
    let group = ordinary_group.id;
    append_world(&mut document, ordinary_group);
    let mut session = ProjectSession::new(document).unwrap();
    let created = session
        .apply(
            &session.revision(),
            &[Edit::CreatePreset {
                preset: presets::BUILTIN_PRESETS
                    .iter()
                    .find(|preset| preset.name == "shape.sphere")
                    .unwrap()
                    .id,
                parent: world,
                after: None,
                definition: None,
                view: None,
            }],
        )
        .unwrap();
    let owned_group = created.created[0];
    let preset_shape = *children(&session, owned_group).last().unwrap();
    let world = scene(session.document()).world;
    // A visible child can be moved into an ordinary Attribute group.
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
    assert_eq!(children(&session, group), vec![shape]);
    assert!(!world_children(&session).contains(&shape));
    // Selecting a child of an explicitly owned group moves the whole unit.
    session
        .apply(
            &session.revision(),
            &[Edit::MoveNode {
                node: preset_shape,
                parent: group,
                after: None,
            }],
        )
        .unwrap();
    assert!(children(&session, group).contains(&owned_group));
    assert_eq!(children(&session, owned_group).last(), Some(&preset_shape));
    for edit in [
        Edit::MoveNode {
            node: group,
            parent: group,
            after: None,
        },
        Edit::MoveNode {
            node: shape,
            parent: preset_shape,
            after: None,
        },
        Edit::MoveNode {
            node: shape,
            parent: owned_group,
            after: None,
        },
        Edit::DeleteNode { node: world },
    ] {
        assert!(session.apply(&session.revision(), &[edit]).is_err());
    }
    session.undo(&session.revision()).unwrap();
    session.undo(&session.revision()).unwrap();
    assert!(world_children(&session).contains(&shape));
    assert!(!children(&session, group).contains(&owned_group));
    assert_eq!(children(&session, owned_group).len(), 1);
}

/// deleting a render configuration removes it from Options; undo
/// puts it back exactly where it was. (The Inspector's Remove Component
/// drives the same wire delete_node.)
#[test]
fn delete_node_removes_a_configuration_and_undo_restores_it() {
    let (document, _, _) = native_scene();
    let scene_id = document.default_scene.unwrap();
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[
                Edit::CreateConfiguration {
                    scene: scene_id,
                    configuration: ConfigurationKind::Film,
                    implementation: None,
                    parameters: Vec::new(),
                },
                Edit::CreateConfiguration {
                    scene: scene_id,
                    configuration: ConfigurationKind::Sampler,
                    implementation: None,
                    parameters: Vec::new(),
                },
            ],
        )
        .unwrap();
    let configuration = |kind: ConfigurationKind| {
        let scene = session.document().scene(scene_id).unwrap();
        scene
            .options
            .iter()
            .copied()
            .find(|id| {
                matches!(
                    scene.nodes.get(id).map(|node| &node.data),
                    Some(NodeKind::Configuration {
                        configuration: found,
                        ..
                    }) if *found == kind
                )
            })
            .unwrap()
    };
    let (film, sampler) = (
        configuration(ConfigurationKind::Film),
        configuration(ConfigurationKind::Sampler),
    );
    session
        .apply(&session.revision(), &[Edit::DeleteNode { node: film }])
        .unwrap();
    let scene = session.document().scene(scene_id).unwrap();
    assert!(!scene.options.contains(&film), "out of Options");
    assert!(!scene.nodes.contains_key(&film), "out of the scene");
    assert!(scene.options.contains(&sampler), "the rest stay");
    session.undo(&session.revision()).unwrap();
    let scene = session.document().scene(scene_id).unwrap();
    assert!(
        scene.options.contains(&film)
            && scene.options.iter().position(|id| *id == film) == Some(scene.options.len() - 2),
        "back where it was, before the Sampler: {:?}",
        scene.options
    );
    assert!(matches!(
        scene.nodes.get(&film).map(|node| &node.data),
        Some(NodeKind::Configuration { .. })
    ));
    evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
    // A second Film is refused, like Add Component never offers one.
    let duplicate = Edit::CreateConfiguration {
        scene: scene_id,
        configuration: ConfigurationKind::Film,
        implementation: None,
        parameters: Vec::new(),
    };
    assert!(session.apply(&session.revision(), &[duplicate]).is_err());
}

#[test]
fn make_object_definition_moves_group_contents_keeps_transform_and_undoes_as_one_edit() {
    let (mut session, _, _, shape) = scene_with_preset();
    let world = scene(session.document()).world;
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
    session
        .apply(
            &session.revision(),
            &[Edit::SetLocalTransform {
                node: group,
                value: LocalTransform::Trs(transform_trs::Trs {
                    translation: [3., 0., 0.],
                    rotation_euler_degrees: [0.; 3],
                    scale: [1.; 3],
                }),
            }],
        )
        .unwrap();
    let before = evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
    let change = session
        .apply(
            &session.revision(),
            &[Edit::MakeObjectDefinition {
                node: group,
                label: None,
            }],
        )
        .unwrap();
    // The visible instance comes first; its wrapper is hidden in the hierarchy.
    let replacement = &scene(session.document()).nodes[&change.created[0]];
    assert!(matches!(replacement.data, NodeKind::Instance { .. }));
    let definition = session
        .document()
        .resources
        .values()
        .find(|resource| {
            resource.kind == ResourceKind::ObjectDefinition && resource.label == "Group"
        })
        .unwrap();
    let root = session
        .document()
        .definition_nodes
        .get(&definition.definition_scope.unwrap())
        .unwrap();
    let NodeKind::Scope { children, .. } = &root.data else {
        panic!("definition root")
    };
    assert!(!children.is_empty());
    assert!(session.document().definition_nodes.contains_key(&shape));
    assert!(!scene(session.document()).nodes.contains_key(&group));
    assert!(!scene(session.document()).nodes.contains_key(&shape));
    assert!(session.document().definition_nodes.contains_key(&shape));
    assert!(scene(session.document()).nodes.values().any(
        |node| matches!(node.data, NodeKind::Instance { definition: id } if id == definition.id)
    ));
    session.document().validate().unwrap();
    session.undo(&session.revision()).unwrap();
    assert!(scene(session.document()).nodes.contains_key(&group));
    assert!(scene(session.document()).nodes.contains_key(&shape));
    assert!(
        !session
            .document()
            .resources
            .values()
            .any(|resource| resource.kind == ResourceKind::ObjectDefinition
                && resource.label == "Group")
    );
    let after = evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
    assert_eq!(before.nodes[&shape].start, after.nodes[&shape].start);
}

#[test]
fn definition_nodes_can_be_created_duplicated_moved_and_deleted_with_undo() {
    let (mut session, _, _, shape) = scene_with_preset();
    let world = scene(session.document()).world;
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
    session
        .apply(
            &session.revision(),
            &[Edit::MakeObjectDefinition {
                node: group,
                label: None,
            }],
        )
        .unwrap();
    let scope = session
        .document()
        .resources
        .values()
        .find(|resource| resource.kind == ResourceKind::ObjectDefinition)
        .unwrap()
        .definition_scope
        .unwrap();

    let created = session
        .apply(
            &session.revision(),
            &[Edit::CreatePreset {
                preset: presets::BUILTIN_PRESETS
                    .iter()
                    .find(|preset| preset.name == "shape.cube")
                    .unwrap()
                    .id,
                parent: scope,
                after: None,
                definition: None,
                view: None,
            }],
        )
        .unwrap();
    assert!(!created.created.is_empty());
    let cube_group = *match &session.document().definition_nodes[&scope].data {
        NodeKind::Scope { children, .. } => children.last().unwrap(),
        _ => panic!("definition root"),
    };
    let cube = match &session.document().definition_nodes[&cube_group].data {
        NodeKind::Scope { children, .. } => *children.last().unwrap(),
        _ => panic!("cube preset group"),
    };
    assert!(
        session
            .apply(
                &session.revision(),
                &[Edit::CreatePreset {
                    preset: presets::BUILTIN_PRESETS
                        .iter()
                        .find(|preset| preset.name == "light.point")
                        .unwrap()
                        .id,
                    parent: scope,
                    after: None,
                    definition: None,
                    view: None,
                }]
            )
            .is_err()
    );

    let sibling_group = session
        .apply(
            &session.revision(),
            &[Edit::CreatePreset {
                preset: presets::BUILTIN_PRESETS
                    .iter()
                    .find(|preset| preset.name == "shape.cube")
                    .unwrap()
                    .id,
                parent: scope,
                after: None,
                definition: None,
                view: None,
            }],
        )
        .unwrap()
        .created[0];
    let sibling_shape = match &session.document().definition_nodes[&sibling_group].data {
        NodeKind::Scope { children, .. } => *children.last().unwrap(),
        _ => panic!("cube preset group"),
    };
    session
        .apply(
            &session.revision(),
            &[Edit::MoveNode {
                node: cube,
                parent: scope,
                after: Some(sibling_shape),
            }],
        )
        .expect("after resolves to the sibling shape's object unit");
    let root_children = match &session.document().definition_nodes[&scope].data {
        NodeKind::Scope { children, .. } => children,
        _ => panic!("definition root"),
    };
    assert_eq!(root_children.last(), Some(&cube_group));

    let duplicate = session
        .apply(&session.revision(), &[Edit::DuplicateNode { node: cube }])
        .unwrap()
        .created[0];
    assert!(session.document().definition_nodes.contains_key(&duplicate));
    session.undo(&session.revision()).unwrap();
    assert!(!session.document().definition_nodes.contains_key(&duplicate));
    let subgroup = session
        .apply(
            &session.revision(),
            &[Edit::CreatePreset {
                preset: group_preset,
                parent: scope,
                after: None,
                definition: None,
                view: None,
            }],
        )
        .unwrap()
        .created[0];
    assert!(
        session
            .apply(
                &session.revision(),
                &[Edit::MoveNode {
                    node: cube,
                    parent: subgroup,
                    after: None
                }]
            )
            .is_ok()
    );
    session.undo(&session.revision()).unwrap();
    session
        .apply(&session.revision(), &[Edit::DeleteNode { node: cube }])
        .unwrap();
    assert!(!session.document().definition_nodes.contains_key(&cube));
    session.undo(&session.revision()).unwrap();
    assert!(session.document().definition_nodes.contains_key(&cube));
    assert!(
        session
            .apply(
                &session.revision(),
                &[Edit::MoveNode {
                    node: cube,
                    parent: world,
                    after: None
                }]
            )
            .is_err()
    );
    let other = session
        .apply(
            &session.revision(),
            &[Edit::CreateResource {
                id: None,
                resource_kind: ResourceKind::ObjectDefinition,
                implementation: String::new(),
                label: "Other".into(),
                folder: None,
                texture_value_type: None,
                parameters: Vec::new(),
            }],
        )
        .unwrap()
        .resources[0];
    let other_scope = session.document().resources[&other]
        .definition_scope
        .unwrap();
    let into_other = [Edit::MoveNode {
        node: cube,
        parent: other_scope,
        after: None,
    }];
    assert!(
        session.apply(&session.revision(), &into_other).is_err(),
        "a node stays in its own definition"
    );
    // Back out of a sub-group onto its own definition's root.
    let into_subgroup = [Edit::MoveNode {
        node: cube,
        parent: subgroup,
        after: None,
    }];
    session.apply(&session.revision(), &into_subgroup).unwrap();
    let to_root = [Edit::MoveNode {
        node: cube,
        parent: scope,
        after: None,
    }];
    session.apply(&session.revision(), &to_root).unwrap();
    session.document().validate().unwrap();
}

/// World ▸ [Material none, <scope> ▸ [Sphere]].
fn group_under_outer_material(scope: ScopeKind, inner: Option<Node>) -> (ProjectSession, NodeId) {
    let mut document = ProjectDocument::new("Outer material");
    append_world(
        &mut document,
        Node::new(
            "Interface",
            NodeKind::State {
                operation: StateOperation::Material(MaterialBinding::None),
            },
        ),
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
    let mut children = Vec::new();
    if let Some(inner) = inner {
        children.push(inner.id);
        scene_mut(&mut document).nodes.insert(inner.id, inner);
    }
    children.push(shape.id);
    scene_mut(&mut document).nodes.insert(shape.id, shape);
    let group = Node::new(
        "Group",
        NodeKind::Scope {
            scope,
            children,
            owned_object: None,
            parent_camera: None,
        },
    );
    let id = group.id;
    append_world(&mut document, group);
    (ProjectSession::new(document).unwrap(), id)
}

#[test]
fn make_object_definition_keeps_the_material_the_group_inherits() {
    let (mut session, group) = group_under_outer_material(ScopeKind::Attribute, None);
    session
        .apply(
            &session.revision(),
            &[Edit::MakeObjectDefinition {
                node: group,
                label: None,
            }],
        )
        .unwrap();
    let definition = session
        .document()
        .resources
        .values()
        .find(|resource| resource.kind == ResourceKind::ObjectDefinition)
        .unwrap();
    let declared = definition.declared_in.as_ref().unwrap();
    assert!(declared.transform.is_none());
    assert_eq!(
        declared.graphics.as_ref().unwrap().material,
        MaterialBinding::None
    );
    session.document().validate().unwrap();
}

#[test]
fn make_object_definition_refuses_a_transform_group_whose_state_leaks_to_siblings() {
    let leak = Node::new(
        "Reverse",
        NodeKind::State {
            operation: StateOperation::ReverseOrientation,
        },
    );
    let (mut session, group) = group_under_outer_material(ScopeKind::Transform, Some(leak));
    let edit = [Edit::MakeObjectDefinition {
        node: group,
        label: None,
    }];
    assert!(session.apply(&session.revision(), &edit).is_err());
    let (mut session, group) = group_under_outer_material(ScopeKind::Transform, None);
    let edit = [Edit::MakeObjectDefinition {
        node: group,
        label: None,
    }];
    session.apply(&session.revision(), &edit).unwrap();
}
