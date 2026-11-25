mod common;
use common::*;
use project::*;
use std::collections::BTreeMap;

#[test]
fn cycles_missing_children_and_unreachable_nodes_are_rejected() {
    let (original, _, shape) = native_scene();
    let mut cycle = original.clone();
    let world = scene(&cycle).world;
    let NodeKind::Scope { children, .. } =
        &mut scene_mut(&mut cycle).nodes.get_mut(&world).unwrap().data
    else {
        unreachable!()
    };
    children.push(world);
    assert!(cycle.validate().is_err());
    let mut missing = original.clone();
    scene_mut(&mut missing).nodes.remove(&shape);
    assert!(missing.validate().is_err());
    let mut unreachable = original;
    let node = Node::new(
        "Orphan",
        NodeKind::State {
            operation: StateOperation::ReverseOrientation,
        },
    );
    scene_mut(&mut unreachable).nodes.insert(node.id, node);
    assert!(unreachable.validate().is_err());
}

#[test]
fn shared_resources_and_cyclic_references_are_rejected() {
    let mut document = ProjectDocument::new("Resources");
    let ids = [ResourceId::new(), ResourceId::new()];
    for (id, other) in [(ids[0], ids[1]), (ids[1], ids[0])] {
        document.resources.insert(
            id,
            Resource {
                id,
                label: "Mix".into(),
                location: format!("Materials/{id}.material"),
                edition: Revision::initial(),
                kind: ResourceKind::Material,
                texture_value_type: None,
                implementation: "mix".into(),
                parameters: vec![Parameter {
                    value_type: "string".into(),
                    name: "materials".into(),
                    color_space: ColorSpaceBinding::Inherit,
                    values: ParameterValues::Resources {
                        kind: ResourceKind::Material,
                        ids: vec![other],
                    },
                }],
                definition_scope: None,
                declared_in: None,
                origin: None,
            },
        );
    }
    assert!(document.validate().is_err());
    document
        .resources
        .get_mut(&ids[1])
        .unwrap()
        .parameters
        .clear();
    assert!(document.validate().is_ok());
}

#[test]
fn resource_reference_kinds_and_asset_paths_are_validated() {
    let mut document = ProjectDocument::new("Bindings");
    let id = ResourceId::new();
    document.resources.insert(
        id,
        Resource {
            id,
            label: "Texture".into(),
            location: "Textures/texture.texture".into(),
            edition: Revision::initial(),
            kind: ResourceKind::Texture,
            texture_value_type: Some(TextureValueType::Float),
            implementation: "constant".into(),
            parameters: vec![],
            definition_scope: None,
            declared_in: None,
            origin: None,
        },
    );
    append_world(
        &mut document,
        Node::new(
            "Wrong binding",
            NodeKind::State {
                operation: StateOperation::Material(MaterialBinding::Resource(id)),
            },
        ),
    );
    assert!(document.validate().is_err());
    let mut document = ProjectDocument::new("Assets");
    let id = AssetId::new();
    document.assets.insert(
        id,
        Asset {
            id,
            location: "../outside.exr".into(),
            media_type: "image/x-exr".into(),
            sha256: "a".repeat(64),
            size: 0,
            origin: None,
        },
    );
    assert!(document.validate().is_err());
    document.assets.get_mut(&id).unwrap().location = "Textures/image.exr".into();
    document.assets.get_mut(&id).unwrap().sha256 = "../outside".into();
    assert!(document.validate().is_err());
}

#[test]
fn source_occurrence_cycles_and_out_of_bounds_spans_are_rejected_without_panics() {
    let (mut document, _, shape) = native_scene();
    let import = ImportId::new();
    let source = SourceId::new();
    let root = OccurrenceId::new();
    let child = OccurrenceId::new();
    document.imports.insert(
        import,
        ImportRecord {
            id: import,
            importer_version: "test-1".into(),
            dialect: ImportDialect::PbrtR4,
            renderer_revision: None,
            diagnostics: vec![],
            root,
            sources: BTreeMap::from([(
                source,
                ImportedSource {
                    original_path: "/source/scene.pbrt".into(),
                    original: ObjectRef {
                        sha256: "a".repeat(64),
                        size: 100,
                    },
                    expanded: None,
                },
            )]),
            occurrences: BTreeMap::from([
                (
                    root,
                    SourceOccurrence {
                        source,
                        parent: None,
                        callsite: None,
                    },
                ),
                (
                    child,
                    SourceOccurrence {
                        source,
                        parent: Some(root),
                        callsite: Some(SourceSpan {
                            offset: 1,
                            length: 10,
                            line: 1,
                            column: 2,
                        }),
                    },
                ),
            ]),
        },
    );
    scene_mut(&mut document)
        .nodes
        .get_mut(&shape)
        .unwrap()
        .origin = Some(Origin {
        import,
        occurrence: child,
        span: SourceSpan {
            offset: 10,
            length: 10,
            line: 2,
            column: 1,
        },
    });
    assert!(document.validate().is_ok());
    scene_mut(&mut document)
        .nodes
        .get_mut(&shape)
        .unwrap()
        .origin
        .as_mut()
        .unwrap()
        .span
        .offset = u64::MAX;
    assert!(document.validate().is_err());
    scene_mut(&mut document)
        .nodes
        .get_mut(&shape)
        .unwrap()
        .origin = None;
    document
        .imports
        .get_mut(&import)
        .unwrap()
        .occurrences
        .get_mut(&child)
        .unwrap()
        .parent = Some(child);
    assert!(document.validate().is_err());
}

#[test]
fn duplicate_typed_record_ids_and_unknown_fields_are_not_silently_accepted() {
    let (mut document, _, shape) = native_scene();
    scene_mut(&mut document).nodes.get_mut(&shape).unwrap().id = NodeId::new();
    assert!(document.validate().is_err());
    let mut encoded = serde_json::to_value(native_scene().0).unwrap();
    encoded["unknown_field"] = serde_json::json!(true);
    assert!(serde_json::from_value::<ProjectDocument>(encoded).is_err());
}

#[test]
fn owned_attribute_scopes_require_one_direct_visible_object() {
    let (mut document, _, shape) = native_scene();
    let world = scene(&document).world;
    let wrapper = Node::new(
        "Internal wrapper",
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children: vec![shape],
            owned_object: Some(shape),
            parent_camera: None,
        },
    );
    let NodeKind::Scope { children, .. } =
        &mut scene_mut(&mut document).nodes.get_mut(&world).unwrap().data
    else {
        unreachable!()
    };
    let index = children.iter().position(|id| *id == shape).unwrap();
    children[index] = wrapper.id;
    scene_mut(&mut document)
        .nodes
        .insert(wrapper.id, wrapper.clone());
    assert!(document.validate().is_ok());

    let mut non_attribute = document.clone();
    let NodeKind::Scope { scope, .. } = &mut scene_mut(&mut non_attribute)
        .nodes
        .get_mut(&wrapper.id)
        .unwrap()
        .data
    else {
        unreachable!()
    };
    *scope = ScopeKind::Transform;
    assert!(non_attribute.validate().is_err());

    let mut not_direct = document.clone();
    let NodeKind::Scope { owned_object, .. } = &mut scene_mut(&mut not_direct)
        .nodes
        .get_mut(&wrapper.id)
        .unwrap()
        .data
    else {
        unreachable!()
    };
    *owned_object = Some(world);
    assert!(not_direct.validate().is_err());

    let mut multiple_objects = document;
    let extra = Node::new(
        "Second sphere",
        NodeKind::Entity {
            entity: EntityKind::Shape,
            implementation: "sphere".into(),
            parameters: vec![],
            pose: None,
        },
    );
    let NodeKind::Scope { children, .. } = &mut scene_mut(&mut multiple_objects)
        .nodes
        .get_mut(&wrapper.id)
        .unwrap()
        .data
    else {
        unreachable!()
    };
    children.push(extra.id);
    scene_mut(&mut multiple_objects)
        .nodes
        .insert(extra.id, extra);
    assert!(multiple_objects.validate().is_err());
}

#[test]
fn owned_attribute_scope_cannot_own_another_owned_scope() {
    let (mut document, _, shape) = native_scene();
    let world = scene(&document).world;
    let inner = Node::new(
        "Inner wrapper",
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children: vec![shape],
            owned_object: Some(shape),
            parent_camera: None,
        },
    );
    let outer = Node::new(
        "Outer wrapper",
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children: vec![inner.id],
            owned_object: Some(inner.id),
            parent_camera: None,
        },
    );
    let NodeKind::Scope { children, .. } =
        &mut scene_mut(&mut document).nodes.get_mut(&world).unwrap().data
    else {
        unreachable!()
    };
    let index = children.iter().position(|id| *id == shape).unwrap();
    children[index] = outer.id;
    scene_mut(&mut document).nodes.insert(inner.id, inner);
    scene_mut(&mut document).nodes.insert(outer.id, outer);
    assert!(document.validate().is_err());
}

#[test]
fn nested_object_definition_scopes_and_array_size_mismatches_are_rejected() {
    let mut document = ProjectDocument::new("Definition scope");
    append_world(
        &mut document,
        Node::new(
            "Unowned definition",
            NodeKind::Scope {
                scope: ScopeKind::ObjectDefinition,
                children: vec![],
                owned_object: None,
                parent_camera: None,
            },
        ),
    );
    assert!(document.validate().is_err());
    let (mut document, _, shape) = native_scene();
    let NodeKind::Entity { parameters, .. } =
        &mut scene_mut(&mut document).nodes.get_mut(&shape).unwrap().data
    else {
        unreachable!()
    };
    parameters.push(Parameter {
        value_type: "float".into(),
        name: "P".into(),
        color_space: ColorSpaceBinding::Inherit,
        values: ParameterValues::Array {
            asset: AssetId::new(),
            encoding: ArrayEncoding::F64Le,
            count: 2,
        },
    });
    assert!(document.validate().is_err());
}

#[test]
fn project_uuid_space_and_item_parent_folders_are_validated() {
    let mut document = ProjectDocument::new("UUID collision");
    let old_scene = document.default_scene.unwrap();
    let mut scene = (*document.scenes.remove(&old_scene).unwrap()).clone();
    scene.id = SceneId(document.id.0);
    document.default_scene = Some(scene.id);
    document.scenes.insert(scene.id, std::sync::Arc::new(scene));
    assert!(document.validate().is_err());

    let mut document = ProjectDocument::new("Missing folder");
    let asset = Asset {
        id: AssetId::new(),
        location: "Unlisted/mesh.ply".into(),
        media_type: "application/octet-stream".into(),
        sha256: "0".repeat(64),
        size: 0,
        origin: None,
    };
    document.assets.insert(asset.id, asset.clone());
    assert!(document.validate().is_err());

    document.assets.clear();
    let resource_id = ResourceId::new();
    document.resources.insert(
        resource_id,
        Resource {
            id: resource_id,
            label: "mesh".into(),
            location: "Unlisted/material.material".into(),
            edition: Revision::initial(),
            kind: ResourceKind::Material,
            texture_value_type: None,
            implementation: "matte".into(),
            parameters: Vec::new(),
            definition_scope: None,
            declared_in: None,
            origin: None,
        },
    );
    document.assets.insert(asset.id, asset);
    assert!(document.validate().is_ok());
}

#[test]
fn option_placement_and_texture_value_type_are_validated() {
    let mut document = ProjectDocument::new("Option placement");
    append_world(
        &mut document,
        Node::new(
            "Seed",
            NodeKind::Option {
                name: "seed".into(),
                value: OptionValue::Integer(42),
            },
        ),
    );
    assert!(document.validate().is_err());
    let mut document = ProjectDocument::new("Texture type");
    let id = ResourceId::new();
    document.resources.insert(
        id,
        Resource {
            id,
            label: "Texture".into(),
            location: "Textures/texture.texture".into(),
            edition: Revision::initial(),
            kind: ResourceKind::Texture,
            texture_value_type: None,
            implementation: "constant".into(),
            parameters: vec![],
            definition_scope: None,
            declared_in: None,
            origin: None,
        },
    );
    assert!(document.validate().is_err());
    document.resources.get_mut(&id).unwrap().texture_value_type = Some(TextureValueType::Float);
    assert!(document.validate().is_ok());
}

#[test]
fn a_pose_is_only_a_camera_s_own_value() {
    let (mut document, _, shape) = native_scene();
    let camera = Node::new(
        "Camera",
        NodeKind::Entity {
            entity: EntityKind::Camera,
            implementation: "perspective".into(),
            parameters: vec![Parameter::floats("fov", [40.])],
            pose: None,
        },
    );
    let camera_id = camera.id;
    scene_mut(&mut document).nodes.insert(camera_id, camera);
    scene_mut(&mut document).options.push(camera_id);
    let identity = project::math::Mat4::identity().0;
    // an options camera carries its pose as a value.
    if let NodeKind::Entity { pose, .. } = &mut scene_mut(&mut document)
        .nodes
        .get_mut(&camera_id)
        .unwrap()
        .data
    {
        *pose = Some(CameraPose {
            start: identity,
            end: identity,
        });
    }
    match document.validate() {
        Ok(()) => {}
        Err(e) => panic!("valid?: {e:?}"),
    }
    // Any other entity kind cannot hold one.
    if let NodeKind::Entity { pose, .. } =
        &mut scene_mut(&mut document).nodes.get_mut(&shape).unwrap().data
    {
        *pose = Some(CameraPose {
            start: identity,
            end: identity,
        });
    }
    let other = document;
    assert!(other.validate().is_err());
    let mut finite = other.clone();
    if let NodeKind::Entity { pose, .. } =
        &mut scene_mut(&mut finite).nodes.get_mut(&shape).unwrap().data
    {
        *pose = None;
    }
    assert!(finite.validate().is_ok());
    // A pose with a non-finite element is not a placement.
    if let NodeKind::Entity { pose, .. } = &mut scene_mut(&mut finite)
        .nodes
        .get_mut(&camera_id)
        .unwrap()
        .data
    {
        let mut broken = identity;
        broken[4] = f64::NAN;
        *pose = Some(CameraPose {
            start: broken,
            end: identity,
        });
    }
    assert!(finite.validate().is_err());
}

#[test]
fn a_camera_attached_group_needs_the_world_and_an_options_camera() {
    let (mut document, _, shape) = native_scene();
    let camera = Node::new(
        "Camera",
        NodeKind::Entity {
            entity: EntityKind::Camera,
            implementation: "perspective".into(),
            parameters: vec![Parameter::floats("fov", [40.])],
            pose: None,
        },
    );
    let camera_id = camera.id;
    scene_mut(&mut document).nodes.insert(camera_id, camera);
    scene_mut(&mut document).options.push(camera_id);
    let group = Node::new(
        "Follows camera",
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children: vec![shape],
            owned_object: None,
            parent_camera: Some(camera_id),
        },
    );
    let group_id = group.id;
    scene_mut(&mut document).nodes.insert(group_id, group);
    let world = scene(&document).world;
    let NodeKind::Scope { children, .. } =
        &mut scene_mut(&mut document).nodes.get_mut(&world).unwrap().data
    else {
        unreachable!()
    };
    let index = children.iter().position(|id| *id == shape).unwrap();
    children[index] = group_id;
    match document.validate() {
        Ok(()) => {}
        Err(e) => panic!("valid?: {e:?}"),
    }

    let mut transformed = document.clone();
    if let NodeKind::Scope { scope, .. } = &mut scene_mut(&mut transformed)
        .nodes
        .get_mut(&group_id)
        .unwrap()
        .data
    {
        *scope = ScopeKind::Transform;
    }
    assert!(
        transformed.validate().is_err(),
        "only an Attribute scope attaches"
    );

    let mut nested = document.clone();
    let outer = Node::new(
        "Wrapper",
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children: vec![group_id],
            owned_object: None,
            parent_camera: None,
        },
    );
    let outer_id = outer.id;
    scene_mut(&mut nested).nodes.insert(outer_id, outer);
    let world = scene(&nested).world;
    if let NodeKind::Scope { children, .. } =
        &mut scene_mut(&mut nested).nodes.get_mut(&world).unwrap().data
    {
        children.retain(|id| *id != group_id);
        children.push(outer_id);
    }
    assert!(
        nested.validate().is_err(),
        "an attachment must sit directly under World"
    );

    let mut misdirected = document;
    if let NodeKind::Scope { parent_camera, .. } = &mut scene_mut(&mut misdirected)
        .nodes
        .get_mut(&group_id)
        .unwrap()
        .data
    {
        *parent_camera = Some(shape);
    }
    assert!(
        misdirected.validate().is_err(),
        "the target must be an options camera"
    );
}
