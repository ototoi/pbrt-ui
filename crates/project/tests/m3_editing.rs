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

#[test]
fn open_scene_preserves_history_and_undo_targets_the_original_scene() {
    let (mut document, _, shape) = native_scene();
    let first = document.default_scene.unwrap();
    let mut other = scene(&document).clone();
    let old_ids = other.nodes.keys().copied().collect::<Vec<_>>();
    let remap = old_ids
        .iter()
        .map(|old| (*old, NodeId::new()))
        .collect::<std::collections::BTreeMap<_, _>>();
    other.id = SceneId::new();
    other.location = "Scenes/Other.scene".into();
    other.world = remap[&other.world];
    other.options = other.options.iter().map(|id| remap[id]).collect();
    other.nodes = other
        .nodes
        .into_values()
        .map(|mut node| {
            node.id = remap[&node.id];
            if let NodeKind::Scope { children, .. } = &mut node.data {
                *children = children.iter().map(|id| remap[id]).collect();
            }
            (node.id, node)
        })
        .collect();
    let second = other.id;
    document.scenes.insert(second, std::sync::Arc::new(other));
    document.validate().unwrap();
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::RenameNode {
                node: shape,
                label: "Changed".into(),
            }],
        )
        .unwrap();
    assert!(session.can_undo());
    session.open_scene(second).unwrap();
    assert_eq!(session.active_scene(), Some(second));
    assert!(session.can_undo());
    session.undo(&session.revision()).unwrap();
    assert_eq!(
        session.document().scenes[&first].nodes[&shape].label,
        "Sphere"
    );
    assert!(session.can_redo());
    session.redo(&session.revision()).unwrap();
    session.open_scene(first).unwrap();
    assert_eq!(scene(session.document()).nodes[&shape].label, "Changed");
}

#[test]
fn scene_create_duplicate_and_move_require_reopenable_extensions() {
    let document = ProjectDocument::new("Scene locations");
    let first = document.default_scene.unwrap();
    let mut session = ProjectSession::new(document).unwrap();
    let revision = session.revision();
    assert!(
        session
            .apply(
                &revision,
                &[Edit::CreateScene {
                    location: "Second.txt".into(),
                }],
            )
            .is_err()
    );

    let created = session
        .apply(
            &revision,
            &[Edit::CreateScene {
                location: "Second.scene".into(),
            }],
        )
        .unwrap();
    let second = match created.created_items[0] {
        ItemId::Scene(id) => id,
        _ => unreachable!(),
    };
    let revision = session.revision();
    assert!(
        session
            .apply(
                &revision,
                &[Edit::DuplicateScene {
                    scene: first,
                    location: "Copy.txt".into(),
                }],
            )
            .is_err()
    );
    assert!(
        session
            .apply(
                &revision,
                &[Edit::MoveItem {
                    item: ItemId::Scene(second),
                    location: "Moved.txt".into(),
                }],
            )
            .is_err()
    );
    assert_eq!(session.document().scenes[&second].location, "Second.scene");
}

#[test]
fn create_preset_adds_isolated_group_and_undo_removes_it() {
    let document = ProjectDocument::new("Preset scene");
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    let change = session
        .apply(
            &session.revision(),
            &[Edit::CreatePreset {
                preset: presets::SHAPE_SPHERE.id,
                parent: world,
                after: None,
                definition: None,
                view: None,
            }],
        )
        .unwrap();
    assert_eq!(change.created.len(), 2);
    evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
    let NodeKind::Scope { children, .. } = &scene(session.document()).nodes[&world].data else {
        unreachable!();
    };
    let group_id = children[0];
    let group = &scene(session.document()).nodes[&group_id];
    assert_eq!(group.label, "Sphere");
    assert!(matches!(
        group.data,
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            ..
        }
    ));
    session.undo(&session.revision()).unwrap();
    assert!(
        matches!(scene(session.document()).nodes[&world].data, NodeKind::Scope { ref children, .. } if children.is_empty())
    );
    session.redo(&session.revision()).unwrap();
    assert_eq!(scene(session.document()).nodes[&group_id].label, "Sphere");
}

#[test]
fn set_local_binding_wraps_an_object_once_and_is_undoable() {
    let (mut document, _, shape) = native_scene();
    let material = material(&mut document);
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    for _ in 0..2 {
        session
            .apply(
                &session.revision(),
                &[Edit::SetLocalBinding {
                    node: shape,
                    material: Some(MaterialBinding::Resource(material)),
                    medium: None,
                }],
            )
            .unwrap();
    }
    let NodeKind::Scope { children, .. } = &scene(session.document()).nodes[&world].data else {
        unreachable!();
    };
    let wrapper_id = children[1];
    let NodeKind::Scope { children, .. } = &scene(session.document()).nodes[&wrapper_id].data
    else {
        panic!("object should be wrapped in an Attribute scope");
    };
    assert_eq!(children.len(), 2);
    assert!(
        matches!(scene(session.document()).nodes[&children[0]].data, NodeKind::State { operation: StateOperation::Material(MaterialBinding::Resource(id)) } if id == material)
    );
    session.undo(&session.revision()).unwrap();
    assert!(
        matches!(scene(session.document()).nodes[&world].data, NodeKind::Scope { ref children, .. } if children.len() == 2 && children[1] == shape)
    );
}

#[test]
fn local_material_and_medium_bindings_keep_canonical_order_and_can_be_cleared() {
    let (mut document, placement, shape) = native_scene();
    let material = material(&mut document);
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
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::SetLocalBinding {
                node: shape,
                material: Some(MaterialBinding::Resource(material)),
                medium: Some(MediumInterface {
                    inside: Some(medium_id),
                    outside: None,
                }),
            }],
        )
        .unwrap();
    let NodeKind::Scope { children, .. } = &scene(session.document()).nodes[&world].data else {
        unreachable!();
    };
    let group_id = children[1];
    let NodeKind::Scope { children, .. } = &scene(session.document()).nodes[&group_id].data else {
        unreachable!();
    };
    assert!(
        matches!(scene(session.document()).nodes[&children[0]].data, NodeKind::State { operation: StateOperation::Material(MaterialBinding::Resource(id)) } if id == material)
    );
    assert!(
        matches!(scene(session.document()).nodes[&children[1]].data, NodeKind::State { operation: StateOperation::Medium { inside: Some(id), outside: None } } if id == medium_id)
    );
    let state_ids = children[..2].to_vec();
    session
        .apply(
            &session.revision(),
            &[Edit::SetLocalBinding {
                node: shape,
                material: Some(MaterialBinding::Default),
                medium: Some(MediumInterface::default()),
            }],
        )
        .unwrap();
    // Clearing removes the binding states and the emptied dedicated group; the
    // outer binding applies again instead of an explicit Material(Default).
    let NodeKind::Scope { children, .. } = &scene(session.document()).nodes[&world].data else {
        unreachable!();
    };
    assert_eq!(children, &[placement, shape]);
    let mut touchable = state_ids;
    touchable.push(group_id);
    for id in &touchable {
        assert!(
            !scene(session.document()).nodes.contains_key(id),
            "cleared node {id:?} should be gone"
        );
    }
    let evaluated = evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
    assert_eq!(evaluated.nodes[&shape].material, MaterialBinding::Default);
    assert_eq!(
        session
            .document()
            .reference_index()
            .count(ItemId::Resource(material)),
        0
    );
    assert_eq!(
        session
            .document()
            .reference_index()
            .count(ItemId::Resource(medium_id)),
        0
    );
    // Undo restores the dedicated group with both binding states in order.
    session.undo(&session.revision()).unwrap();
    let NodeKind::Scope { children, .. } = &scene(session.document()).nodes[&world].data else {
        unreachable!();
    };
    assert_eq!(children, &[placement, group_id]);
    let NodeKind::Scope { children, .. } = &scene(session.document()).nodes[&group_id].data else {
        unreachable!();
    };
    assert_eq!(children.len(), 3);
    assert!(
        matches!(scene(session.document()).nodes[&children[0]].data, NodeKind::State { operation: StateOperation::Material(MaterialBinding::Resource(id)) } if id == material)
    );
    assert!(
        matches!(scene(session.document()).nodes[&children[1]].data, NodeKind::State { operation: StateOperation::Medium { inside: Some(id), outside: None } } if id == medium_id)
    );
    let evaluated = evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
    assert_eq!(
        evaluated.nodes[&shape].material,
        MaterialBinding::Resource(material)
    );
}

#[test]
fn reference_index_reports_resource_users_across_the_scene() {
    let (mut document, _, _) = native_scene();
    let material = material(&mut document);
    let binding = Node::new(
        "Material",
        NodeKind::State {
            operation: StateOperation::Material(MaterialBinding::Resource(material)),
        },
    );
    let binding_id = binding.id;
    append_world(&mut document, binding);
    let index = document.reference_index();
    assert_eq!(index.count(ItemId::Resource(material)), 1);
    assert_eq!(
        index.references(ItemId::Resource(material))[0].node,
        Some(binding_id)
    );
}

#[test]
fn built_in_preset_uuids_are_stable_and_unique() {
    let ids = presets::BUILTIN_PRESETS
        .iter()
        .map(|preset| preset.id)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), presets::BUILTIN_PRESETS.len());
    assert_eq!(
        presets::SHAPE_SPHERE.id.to_string(),
        "209d7c20-ca6d-4965-a471-405c97e13662"
    );
    assert_eq!(
        presets::INSTANCE_OBJECT.id.to_string(),
        "a62d8a1c-f165-40f7-adef-c5d3bf027006"
    );
}

#[test]
fn every_non_instance_preset_builds_a_valid_evaluable_scene() {
    for preset in presets::BUILTIN_PRESETS
        .iter()
        .filter(|preset| preset.name != "instance.object")
    {
        let document = ProjectDocument::new("Preset validation");
        let world = scene(&document).world;
        let mut session = ProjectSession::new(document).unwrap();
        session
            .apply(
                &session.revision(),
                &[Edit::CreatePreset {
                    preset: preset.id,
                    parent: world,
                    after: None,
                    definition: None,
                    view: None,
                }],
            )
            .unwrap_or_else(|error| panic!("{} failed to create: {error}", preset.name));
        evaluate(&session.snapshot().scene_view().unwrap())
            .unwrap_or_else(|error| panic!("{} failed to evaluate: {error}", preset.name));
    }
}

#[test]
fn object_instance_preset_requires_and_uses_a_valid_definition() {
    let mut document = ProjectDocument::new("Instance");
    let shape = Node::new(
        "Defined sphere",
        NodeKind::Entity {
            entity: EntityKind::Shape,
            implementation: "sphere".into(),
            parameters: vec![Parameter::floats("radius", [1.])],
            pose: None,
        },
    );
    let root = Node::new(
        "Definition",
        NodeKind::Scope {
            scope: ScopeKind::ObjectDefinition,
            children: vec![shape.id],
            owned_object: None,
            parent_camera: None,
        },
    );
    let definition = Resource {
        id: ResourceId::new(),
        label: "Object".into(),
        location: "Objects/object.object".into(),
        edition: Revision::initial(),
        kind: ResourceKind::ObjectDefinition,
        texture_value_type: None,
        implementation: String::new(),
        parameters: Vec::new(),
        definition_scope: Some(root.id),
        declared_in: None,
        origin: None,
    };
    let definition_id = definition.id;
    document.definition_nodes.insert(shape.id, shape);
    document.definition_nodes.insert(root.id, root);
    document.resources.insert(definition_id, definition);
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    assert!(
        session
            .apply(
                &session.revision(),
                &[Edit::CreatePreset {
                    preset: presets::INSTANCE_OBJECT.id,
                    parent: world,
                    after: None,
                    definition: None,
                    view: None,
                }],
            )
            .is_err()
    );
    session
        .apply(
            &session.revision(),
            &[Edit::CreatePreset {
                preset: presets::INSTANCE_OBJECT.id,
                parent: world,
                after: None,
                definition: Some(definition_id),
                view: None,
            }],
        )
        .unwrap();
    evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
}

#[test]
fn create_and_duplicate_scene_issue_fresh_node_ids_and_undo() {
    let document = ProjectDocument::new("Scenes");
    let original = document.default_scene.unwrap();
    let original_nodes = scene(&document)
        .nodes
        .keys()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let mut session = ProjectSession::new(document).unwrap();
    let created = session
        .apply(
            &session.revision(),
            &[Edit::DuplicateScene {
                scene: original,
                location: "Scenes/Copy.scene".into(),
            }],
        )
        .unwrap();
    let duplicate_id = match created.created_items[0] {
        ItemId::Scene(id) => id,
        _ => panic!("duplicate should report a scene id"),
    };
    let duplicate = session.document().scene(duplicate_id).unwrap();
    assert!(
        duplicate
            .nodes
            .keys()
            .all(|id| !original_nodes.contains(id))
    );
    assert_eq!(duplicate.label, "Copy");
    session.undo(&session.revision()).unwrap();
    assert!(!session.document().scenes.contains_key(&duplicate_id));
    session.redo(&session.revision()).unwrap();
    assert!(session.document().scenes.contains_key(&duplicate_id));
}

#[test]
fn referenced_resources_cannot_be_deleted() {
    let (mut document, _, _) = native_scene();
    let material_id = material(&mut document);
    let binding = Node::new(
        "Binding",
        NodeKind::State {
            operation: StateOperation::Material(MaterialBinding::Resource(material_id)),
        },
    );
    append_world(&mut document, binding);
    let mut session = ProjectSession::new(document).unwrap();
    assert!(
        session
            .apply(
                &session.revision(),
                &[Edit::DeleteItem {
                    item: ItemId::Resource(material_id),
                }],
            )
            .is_err()
    );
    assert!(session.document().resources.contains_key(&material_id));
}

#[test]
fn deleting_an_unreferenced_resource_is_undoable() {
    let (mut document, _, _) = native_scene();
    let material_id = material(&mut document);
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::DeleteItem {
                item: ItemId::Resource(material_id),
            }],
        )
        .unwrap();
    assert!(!session.document().resources.contains_key(&material_id));
    session.undo(&session.revision()).unwrap();
    assert!(session.document().resources.contains_key(&material_id));
}

#[test]
fn scene_creation_deletion_and_switching_keep_the_default_scene_valid() {
    let document = ProjectDocument::new("Scenes");
    let first = document.default_scene.unwrap();
    let mut session = ProjectSession::new(document).unwrap();
    let change = session
        .apply(
            &session.revision(),
            &[Edit::CreateScene {
                location: "Scenes/Second.scene".into(),
            }],
        )
        .unwrap();
    let second = match change.created_items[0] {
        ItemId::Scene(id) => id,
        _ => unreachable!(),
    };
    session.open_scene(second).unwrap();
    assert_eq!(session.document().default_scene.unwrap(), second);
    session
        .apply(&session.revision(), &[Edit::DeleteScene { scene: first }])
        .unwrap();
    assert!(!session.document().scenes.contains_key(&first));
    session.undo(&session.revision()).unwrap();
    assert!(session.document().scenes.contains_key(&first));
}

#[test]
fn folder_move_updates_all_placements_and_is_undoable() {
    let (mut document, _, _) = native_scene();
    let material = material(&mut document);
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[
                Edit::CreateFolder {
                    path: "Materials/Sub".into(),
                },
                Edit::MoveItem {
                    item: ItemId::Resource(material),
                    location: "Materials/Sub/matte.material".into(),
                },
            ],
        )
        .unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::MoveFolder {
                from: "Materials/Sub".into(),
                to: "Other/Renamed".into(),
            }],
        )
        .unwrap();
    assert_eq!(
        session.document().resources[&material].location,
        "Other/Renamed/matte.material"
    );
    assert!(session.document().folders.contains("Other/Renamed"));
    session.undo(&session.revision()).unwrap();
    assert_eq!(
        session.document().resources[&material].location,
        "Materials/Sub/matte.material"
    );
}

#[test]
fn mesh_asset_reference_can_be_replaced_by_id() {
    let (mut document, _, shape) = native_scene();
    let make_asset = |location: &str| Asset {
        id: AssetId::new(),
        location: location.into(),
        media_type: "application/octet-stream".into(),
        sha256: "0".repeat(64),
        size: 4,
        origin: None,
    };
    let original = make_asset("Meshes/original.ply");
    let replacement = make_asset("Meshes/replacement.ply");
    let replacement_id = replacement.id;
    document.assets.insert(original.id, original.clone());
    document.assets.insert(replacement.id, replacement);
    let NodeKind::Entity {
        parameters: params, ..
    } = &mut scene_mut(&mut document).nodes.get_mut(&shape).unwrap().data
    else {
        unreachable!();
    };
    params.push(Parameter {
        value_type: "string".into(),
        name: "filename".into(),
        color_space: Default::default(),
        values: ParameterValues::Assets(vec![original.id]),
    });
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::SetAssetReference {
                target: ParameterTarget::Node(shape),
                parameter: "filename".into(),
                asset: replacement_id,
            }],
        )
        .unwrap();
    assert!(
        matches!(&parameters(session.document(), shape)[1].values, ParameterValues::Assets(ids) if ids == &[replacement_id])
    );
}

#[test]
fn local_binding_leaves_unspecified_bindings_unchanged() {
    let (mut document, _, shape) = native_scene();
    let first = material(&mut document);
    let second = material(&mut document);
    document.resources.get_mut(&second).unwrap().location = "Materials/second.material".into();
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
    let mut session = ProjectSession::new(document).unwrap();
    for (material, medium) in [
        (
            Some(MaterialBinding::Resource(first)),
            Some(MediumInterface {
                inside: Some(medium_id),
                outside: None,
            }),
        ),
        (Some(MaterialBinding::Resource(second)), None),
    ] {
        session
            .apply(
                &session.revision(),
                &[Edit::SetLocalBinding {
                    node: shape,
                    material,
                    medium,
                }],
            )
            .unwrap();
    }
    let references = session.document().reference_index();
    assert_eq!(references.count(ItemId::Resource(first)), 0);
    assert_eq!(references.count(ItemId::Resource(second)), 1);
    assert_eq!(references.count(ItemId::Resource(medium_id)), 1);
}

#[test]
fn clearing_a_local_binding_lets_the_outer_binding_apply_and_undo_restores() {
    let (mut document, placement, shape) = native_scene();
    let outer = material(&mut document);
    document.resources.get_mut(&outer).unwrap().label = "outer".into();
    document.resources.get_mut(&outer).unwrap().location = "Materials/outer.material".into();
    let inner = material(&mut document);
    document.resources.get_mut(&inner).unwrap().label = "inner".into();
    document.resources.get_mut(&inner).unwrap().location = "Materials/inner.material".into();
    let state = Node::new(
        "Outer material",
        NodeKind::State {
            operation: StateOperation::Material(MaterialBinding::Resource(outer)),
        },
    );
    let state_id = state.id;
    // Keep the world state directly before the shape at the very start.
    let world = scene(&document).world;
    {
        let world_node = scene_mut(&mut document).nodes.get_mut(&world).unwrap();
        let NodeKind::Scope { children, .. } = &mut world_node.data else {
            unreachable!();
        };
        children.insert(0, state_id);
    }
    scene_mut(&mut document).nodes.insert(state_id, state);
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::SetLocalBinding {
                node: shape,
                material: Some(MaterialBinding::Resource(inner)),
                medium: None,
            }],
        )
        .unwrap();
    // Clearing the local binding must remove the state instead of pinning a
    // Material(Default) that export rejects; the outer binding applies again.
    session
        .apply(
            &session.revision(),
            &[Edit::SetLocalBinding {
                node: shape,
                material: Some(MaterialBinding::Default),
                medium: None,
            }],
        )
        .unwrap();
    let NodeKind::Scope { children, .. } = &scene(session.document()).nodes[&world].data else {
        unreachable!();
    };
    assert_eq!(children, &[state_id, placement, shape]);
    let evaluated = evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
    assert_eq!(
        evaluated.nodes[&shape].material,
        MaterialBinding::Resource(outer)
    );
    session.undo(&session.revision()).unwrap();
    let evaluated = evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
    assert_eq!(
        evaluated.nodes[&shape].material,
        MaterialBinding::Resource(inner)
    );
}

#[test]
fn set_material_binding_rejects_explicit_default() {
    let (mut document, _, _) = native_scene();
    let matte = material(&mut document);
    let state = Node::new(
        "Material",
        NodeKind::State {
            operation: StateOperation::Material(MaterialBinding::Resource(matte)),
        },
    );
    let state_id = state.id;
    append_world(&mut document, state);
    let mut session = ProjectSession::new(document).unwrap();
    let error = session
        .apply(
            &session.revision(),
            &[Edit::SetMaterialBinding {
                node: state_id,
                value: MaterialBinding::Default,
            }],
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("default material binding"),
        "unexpected error: {error}"
    );
}

fn medium(document: &mut ProjectDocument) -> ResourceId {
    let resource = Resource {
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
    let id = resource.id;
    document.resources.insert(id, resource);
    id
}

fn distant_light() -> Node {
    Node::new(
        "Sun",
        NodeKind::Entity {
            entity: EntityKind::Light,
            implementation: "distant".into(),
            parameters: Vec::new(),
            pose: None,
        },
    )
}

#[test]
fn set_local_binding_medium_works_on_a_light_and_is_undoable() {
    let (mut document, _, _) = native_scene();
    let fog = medium(&mut document);
    let light = distant_light();
    let light_id = light.id;
    append_world(&mut document, light);
    let world = scene(&document).world;
    let world_children = |session: &ProjectSession| {
        let NodeKind::Scope { children, .. } = &scene(session.document()).nodes[&world].data else {
            unreachable!();
        };
        children.clone()
    };
    let before = world_children(&ProjectSession::new(document.clone()).unwrap());
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::SetLocalBinding {
                node: light_id,
                material: None,
                medium: Some(MediumInterface {
                    inside: None,
                    outside: Some(fog),
                }),
            }],
        )
        .unwrap();
    let children = world_children(&session);
    let NodeKind::Scope { children, .. } = &scene(session.document()).nodes[&children[2]].data
    else {
        panic!("light should be wrapped in an Attribute scope");
    };
    assert_eq!(children.len(), 2);
    assert!(matches!(
        &scene(session.document()).nodes[&children[0]].data,
        NodeKind::State { operation: StateOperation::Medium { inside: None, outside } } if *outside == Some(fog)
    ));
    assert_eq!(children[1], light_id);
    let evaluated = evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
    assert_eq!(evaluated.nodes[&light_id].outside_medium, Some(fog));
    session.undo(&session.revision()).unwrap();
    assert_eq!(world_children(&session), before);
    let evaluated = evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
    assert_eq!(evaluated.nodes[&light_id].outside_medium, None);
}

#[test]
fn set_local_binding_material_is_rejected_on_a_light() {
    let (mut document, _, _) = native_scene();
    let matte = material(&mut document);
    let light = distant_light();
    let light_id = light.id;
    append_world(&mut document, light);
    let mut session = ProjectSession::new(document).unwrap();
    let error = session
        .apply(
            &session.revision(),
            &[Edit::SetLocalBinding {
                node: light_id,
                material: Some(MaterialBinding::Resource(matte)),
                medium: None,
            }],
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("lights take no material binding"),
        "unexpected error: {error}"
    );
}

#[test]
fn medium_binding_on_a_shape_does_not_reach_a_sibling_light() {
    let (mut document, _, shape) = native_scene();
    let fog = medium(&mut document);
    let light = distant_light();
    let light_id = light.id;
    let world = scene(&document).world;
    scene_mut(&mut document).nodes.insert(light_id, light);
    let group = Node::new(
        "Pair",
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children: vec![shape, light_id],
            owned_object: None,
            parent_camera: None,
        },
    );
    let group_id = group.id;
    {
        let world_node = scene_mut(&mut document).nodes.get_mut(&world).unwrap();
        let NodeKind::Scope { children, .. } = &mut world_node.data else {
            unreachable!();
        };
        children[1] = group_id;
    }
    scene_mut(&mut document).nodes.insert(group_id, group);
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::SetLocalBinding {
                node: shape,
                material: None,
                medium: Some(MediumInterface {
                    inside: Some(fog),
                    outside: None,
                }),
            }],
        )
        .unwrap();
    let evaluated = evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
    assert_eq!(evaluated.nodes[&shape].inside_medium, Some(fog));
    assert_eq!(evaluated.nodes[&light_id].inside_medium, None);
    assert_eq!(evaluated.nodes[&light_id].outside_medium, None);
}

fn object_definition(document: &mut ProjectDocument, label: &str) -> ResourceId {
    let shape = Node::new(
        format!("{label} sphere"),
        NodeKind::Entity {
            entity: EntityKind::Shape,
            implementation: "sphere".into(),
            parameters: vec![Parameter::floats("radius", [1.])],
            pose: None,
        },
    );
    let root = Node::new(
        label,
        NodeKind::Scope {
            scope: ScopeKind::ObjectDefinition,
            children: vec![shape.id],
            owned_object: None,
            parent_camera: None,
        },
    );
    let definition = Resource {
        id: ResourceId::new(),
        label: label.into(),
        location: format!("Objects/{label}.object"),
        edition: Revision::initial(),
        kind: ResourceKind::ObjectDefinition,
        texture_value_type: None,
        implementation: String::new(),
        parameters: Vec::new(),
        definition_scope: Some(root.id),
        declared_in: None,
        origin: None,
    };
    let id = definition.id;
    document.definition_nodes.insert(shape.id, shape);
    document.definition_nodes.insert(root.id, root);
    document.resources.insert(id, definition);
    id
}

#[test]
fn set_instance_definition_repoints_an_instance_and_is_undoable() {
    let mut document = ProjectDocument::new("Instances");
    let first = object_definition(&mut document, "first");
    let second = object_definition(&mut document, "second");
    let matte = material(&mut document);
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    let change = session
        .apply(
            &session.revision(),
            &[Edit::CreatePreset {
                preset: presets::INSTANCE_OBJECT.id,
                parent: world,
                after: None,
                definition: Some(first),
                view: None,
            }],
        )
        .unwrap();
    let instance = change
        .created
        .iter()
        .copied()
        .find(|id| {
            matches!(
                scene(session.document()).nodes[id].data,
                NodeKind::Instance { .. }
            )
        })
        .unwrap();
    let definition_of =
        |session: &ProjectSession| match &scene(session.document()).nodes[&instance].data {
            NodeKind::Instance { definition } => *definition,
            _ => unreachable!(),
        };
    session
        .apply(
            &session.revision(),
            &[Edit::SetInstanceDefinition {
                node: instance,
                definition: second,
            }],
        )
        .unwrap();
    assert_eq!(definition_of(&session), second);
    evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
    // Only object definitions are valid targets, and only instances change.
    for edit in [
        Edit::SetInstanceDefinition {
            node: instance,
            definition: matte,
        },
        Edit::SetInstanceDefinition {
            node: world,
            definition: first,
        },
    ] {
        assert!(session.apply(&session.revision(), &[edit]).is_err());
    }
    session.undo(&session.revision()).unwrap();
    assert_eq!(definition_of(&session), first);
}

#[test]
fn batched_clears_in_one_apply_stay_undoable() {
    let (mut document, _, shape) = native_scene();
    let matte = material(&mut document);
    let medium = medium(&mut document);
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::SetLocalBinding {
                node: shape,
                material: Some(MaterialBinding::Resource(matte)),
                medium: Some(MediumInterface {
                    inside: Some(medium),
                    outside: None,
                }),
            }],
        )
        .unwrap();
    let before = session.snapshot();
    let before_scene = before.scene();
    let before_world = before.document().scene(before_scene).unwrap().clone();
    // Two pure-clear edits in one apply: the second must not see a group
    // snapshot that predates the first removal.
    session
        .apply(
            &session.revision(),
            &[
                Edit::SetLocalBinding {
                    node: shape,
                    material: Some(MaterialBinding::Default),
                    medium: None,
                },
                Edit::SetLocalBinding {
                    node: shape,
                    material: None,
                    medium: Some(MediumInterface::default()),
                },
            ],
        )
        .unwrap();
    let after_undo = session.undo(&session.revision());
    assert!(
        after_undo.is_ok(),
        "undo after batched clears failed: {:?}",
        after_undo.err()
    );
    let restored = scene(session.document()).clone();
    let expected = before_world.nodes;
    assert_eq!(restored.nodes.len(), expected.len());
    for (id, node) in &expected {
        // Editions bump on every undo; the restored tree must match in
        // structure and content.
        let other = restored.nodes.get(id).unwrap();
        assert_eq!(other.label, node.label, "node {id:?} differs after undo");
        assert_eq!(other.data, node.data, "node {id:?} differs after undo");
    }
}

#[test]
fn batched_set_then_clear_restores_the_pre_apply_value_on_undo() {
    let (mut document, _, shape) = native_scene();
    let first = material(&mut document);
    document.resources.get_mut(&first).unwrap().label = "first".into();
    document.resources.get_mut(&first).unwrap().location = "Materials/first.material".into();
    let second = material(&mut document);
    document.resources.get_mut(&second).unwrap().label = "second".into();
    document.resources.get_mut(&second).unwrap().location = "Materials/second.material".into();
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::SetLocalBinding {
                node: shape,
                material: Some(MaterialBinding::Resource(first)),
                medium: None,
            }],
        )
        .unwrap();
    // Set then clear in one transaction: undo must restore the binding as it
    // was before the transaction, not the value the set edit wrote.
    session
        .apply(
            &session.revision(),
            &[
                Edit::SetLocalBinding {
                    node: shape,
                    material: Some(MaterialBinding::Resource(second)),
                    medium: None,
                },
                Edit::SetLocalBinding {
                    node: shape,
                    material: Some(MaterialBinding::Default),
                    medium: None,
                },
            ],
        )
        .unwrap();
    let evaluated = evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
    assert_eq!(
        scene(session.document()).nodes.len(),
        3,
        "cleared state and emptied group are gone"
    );
    assert_eq!(evaluated.nodes[&shape].material, MaterialBinding::Default);
    session.undo(&session.revision()).unwrap();
    let evaluated = evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
    assert_eq!(
        evaluated.nodes[&shape].material,
        MaterialBinding::Resource(first),
        "undo restores the pre-transaction binding"
    );
}

#[test]
fn batched_rename_and_delete_restores_the_original_node_on_undo() {
    let (document, _, shape) = native_scene();
    let world = scene(&document).world;
    let before_world = scene(&document).clone();
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[
                Edit::RenameNode {
                    node: shape,
                    label: "Renamed".into(),
                },
                Edit::DeleteNode { node: shape },
            ],
        )
        .unwrap();
    let restored = session.undo(&session.revision());
    assert!(
        restored.is_ok(),
        "undo after batched rename+delete failed: {:?}",
        restored.err()
    );
    let node = scene(session.document()).nodes.get(&shape).unwrap();
    assert_eq!(node.label, "Sphere");
    assert_eq!(node.data, before_world.nodes[&shape].data);
    let NodeKind::Scope { children, .. } = &scene(session.document()).nodes[&world].data else {
        unreachable!();
    };
    let NodeKind::Scope {
        children: before_children,
        ..
    } = &before_world.nodes[&world].data
    else {
        unreachable!();
    };
    assert_eq!(children, before_children);
}

#[test]
fn batched_bind_and_deletes_every_created_node_on_undo() {
    let (mut document, _, shape) = native_scene();
    let matte = material(&mut document);
    let world = scene(&document).world;
    let before_nodes = scene(&document).nodes.clone();
    let mut session = ProjectSession::new(document).unwrap();
    let deleted = session
        .apply(
            &session.revision(),
            &[
                Edit::SetLocalBinding {
                    node: shape,
                    material: Some(MaterialBinding::Resource(matte)),
                    medium: None,
                },
                Edit::DeleteNode { node: shape },
            ],
        )
        .unwrap();
    assert!(
        deleted.created.is_empty(),
        "created nodes are all deleted again"
    );
    let restored = session.undo(&session.revision());
    assert!(
        restored.is_ok(),
        "undo after batched bind+delete failed: {:?}",
        restored.err()
    );
    // Only the pre-apply nodes come back: no wrapper or binding state is
    // left orphaned in the table.
    let current = scene(session.document());
    assert_eq!(current.nodes.len(), before_nodes.len());
    for (id, node) in &before_nodes {
        assert_eq!(current.nodes[id].label, node.label);
        assert_eq!(current.nodes[id].data, node.data);
    }
    let NodeKind::Scope { children, .. } = &current.nodes[&world].data else {
        unreachable!();
    };
    let NodeKind::Scope {
        children: before_children,
        ..
    } = &before_nodes[&world].data
    else {
        unreachable!();
    };
    assert_eq!(children, before_children);
}
