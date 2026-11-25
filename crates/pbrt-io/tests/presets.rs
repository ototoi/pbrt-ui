//! Every built-in preset must survive validation, evaluation, PBRT export and
//! direct preview.
use pbrt_io::{export_project, program_from_project, stage_assets};
use project::presets::PresetView;
use project::{
    Edit, EntityKind, FolderStore, Node, NodeKind, Parameter, ProjectDocument, ProjectSession,
    ProjectStore, Resource, ResourceId, ResourceKind, Revision, ScopeKind, StateOperation,
    TransformOperation, evaluate, presets,
};
use scene_parse::{
    GeometryCache, ParseOptions, document::ReadOptions, document::read_document_with_options,
    preview_from_program,
};
use std::fs;

fn object_definition(document: &mut ProjectDocument) -> ResourceId {
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
    let id = definition.id;
    document.definition_nodes.insert(shape.id, shape);
    document.definition_nodes.insert(root.id, root);
    document.resources.insert(id, definition);
    id
}

#[test]
fn presets_create_previewable_and_exportable_nodes() {
    let temp = tempfile::tempdir().unwrap();
    let mut document = ProjectDocument::new("Presets");
    let definition = object_definition(&mut document);
    let world = document.default_scene_view().unwrap().scene.world;
    let store = ProjectStore::Folder(
        FolderStore::create(
            &temp.path().join("project"),
            document.id,
            &document.title,
            Revision::initial(),
        )
        .unwrap(),
    );
    let mut session = ProjectSession::new(document).unwrap();
    for preset in presets::BUILTIN_PRESETS {
        session
            .apply(
                &session.revision(),
                &[Edit::CreatePreset {
                    preset: preset.id,
                    parent: world,
                    after: None,
                    definition: (preset.id == presets::INSTANCE_OBJECT.id).then_some(definition),
                    view: None,
                }],
            )
            .unwrap_or_else(|error| panic!("{}: {error}", preset.name));
    }
    let snapshot = session.snapshot();
    snapshot.document().validate().unwrap();
    evaluate(&snapshot.scene_view().unwrap()).unwrap();

    let text = export_project(&snapshot, &store, &temp.path().join("text"), || false).unwrap();
    let exported = read_document_with_options(
        &text,
        ReadOptions {
            expand_imports: true,
            ..Default::default()
        },
    )
    .unwrap();
    let count = |name: &str| {
        exported
            .directives
            .iter()
            .filter(|directive| directive.name == name)
            .count()
    };
    // Sphere, disk, cylinder, plane, floor, cube, two area shapes, and the definition's sphere.
    // Eight world shapes (sphere, disk, cylinder, plane, floor, cube, the
    // two area quads) plus the definition's instanced sphere; the camera
    // lives in Options.
    assert_eq!(count("Shape"), 9);
    assert_eq!(count("Camera"), 1);
    assert_eq!(count("LightSource"), 4);
    assert_eq!(count("AreaLightSource"), 2);
    assert_eq!(count("ObjectInstance"), 1);

    let bundle = temp.path().join("direct");
    fs::create_dir_all(&bundle).unwrap();
    let assets = stage_assets(snapshot.document(), &store, &bundle, &|| false).unwrap();
    let program = program_from_project(&snapshot, &store, &assets, || false).unwrap();
    let origins = program.origins.clone();
    let preview = preview_from_program(
        program,
        Some(&origins),
        &bundle,
        ParseOptions::default(),
        &mut GeometryCache::new(0),
    )
    .unwrap();
    assert!(
        preview.manifest.unsupported.is_empty(),
        "{:?}",
        preview.manifest.unsupported
    );
    // Eight world shapes, the Floor, plus one instanced sphere.
    assert_eq!(preview.manifest.draws.len(), 9);
}

#[test]
fn definition_preset_placement_is_local_to_its_first_instance() {
    let temp = tempfile::tempdir().unwrap();
    let mut document = ProjectDocument::new("Definition preset placement");
    let definition = object_definition(&mut document);
    let scene_id = document.default_scene.unwrap();
    let world = document.scenes[&scene_id].world;
    let translate = Node::new(
        "Instance placement",
        NodeKind::State {
            operation: StateOperation::Transform(TransformOperation::Translate([5., 0., 0.])),
        },
    );
    let translate_id = translate.id;
    let scene = std::sync::Arc::make_mut(document.scenes.get_mut(&scene_id).unwrap());
    let NodeKind::Scope { children, .. } = &mut scene.nodes.get_mut(&world).unwrap().data else {
        unreachable!();
    };
    children.push(translate_id);
    scene.nodes.insert(translate_id, translate);

    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::CreatePreset {
                preset: presets::INSTANCE_OBJECT.id,
                parent: world,
                after: None,
                definition: Some(definition),
                view: None,
            }],
        )
        .unwrap();
    let root = session.document().resources[&definition]
        .definition_scope
        .unwrap();
    let created = session
        .apply(
            &session.revision(),
            &[Edit::CreatePreset {
                preset: presets::SHAPE_SPHERE.id,
                parent: root,
                after: None,
                definition: None,
                view: Some(PresetView {
                    position: [5., 0., 10.],
                    look_at: [5., 0., 0.],
                }),
            }],
        )
        .unwrap();
    let NodeKind::Scope { children, .. } =
        &session.document().definition_nodes[&created.created[0]].data
    else {
        unreachable!();
    };
    let local_translation =
        children
            .iter()
            .find_map(|id| match &session.document().definition_nodes[id].data {
                NodeKind::State {
                    operation: StateOperation::Transform(TransformOperation::Translate(position)),
                } => Some(*position),
                _ => None,
            });
    assert_eq!(local_translation, Some([0., 0., 0.]));

    let snapshot = session.snapshot();
    let bundle = temp.path().join("preview");
    fs::create_dir_all(&bundle).unwrap();
    let store = ProjectStore::Folder(
        FolderStore::create(
            &temp.path().join("project"),
            snapshot.document().id,
            &snapshot.document().title,
            Revision::initial(),
        )
        .unwrap(),
    );
    let assets = stage_assets(snapshot.document(), &store, &bundle, &|| false).unwrap();
    let program = program_from_project(&snapshot, &store, &assets, || false).unwrap();
    let origins = program.origins.clone();
    let preview = preview_from_program(
        program,
        Some(&origins),
        &bundle,
        ParseOptions::default(),
        &mut GeometryCache::new(0),
    )
    .unwrap();
    assert_eq!(preview.manifest.draws.len(), 2);
    for draw in &preview.manifest.draws {
        assert_eq!(draw.world_from_object[0][3], 5.);
    }
}

#[test]
fn make_object_definition_exports_the_group_as_object_begin_and_instance() {
    let temp = tempfile::tempdir().unwrap();
    let document = ProjectDocument::new("Instancing");
    let world = document.default_scene_view().unwrap().scene.world;
    let mut session = ProjectSession::new(document).unwrap();
    let group = session
        .apply(
            &session.revision(),
            &[Edit::CreatePreset {
                preset: presets::GROUP_EMPTY.id,
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
            &[Edit::CreatePreset {
                preset: presets::SHAPE_SPHERE.id,
                parent: world,
                after: None,
                definition: None,
                view: None,
            }],
        )
        .unwrap();
    let shape = session
        .document()
        .default_scene_view()
        .unwrap()
        .scene
        .nodes
        .values()
        .find(|node| {
            matches!(
                node.data,
                NodeKind::Entity {
                    entity: EntityKind::Shape,
                    ..
                }
            )
        })
        .unwrap()
        .id;
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
                label: Some("Tree".into()),
            }],
        )
        .unwrap();
    let snapshot = session.snapshot();
    let store = ProjectStore::Folder(
        FolderStore::create(
            &temp.path().join("project"),
            snapshot.document().id,
            &snapshot.document().title,
            Revision::initial(),
        )
        .unwrap(),
    );
    let path = export_project(&snapshot, &store, &temp.path().join("text"), || false).unwrap();
    let exported = read_document_with_options(
        &path,
        ReadOptions {
            expand_imports: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        exported
            .directives
            .iter()
            .any(|directive| directive.name == "ObjectBegin")
    );
    assert!(
        exported
            .directives
            .iter()
            .any(|directive| directive.name == "ObjectInstance")
    );
}

#[test]
fn unpacked_instance_preserves_preview_draw_transform_and_material() {
    use project::{
        ActiveSamples, CapturedGraphics, CapturedTransform, DeclarationState, MaterialBinding,
        StateOperation, TransformOperation,
    };

    let temp = tempfile::tempdir().unwrap();
    let mut document = ProjectDocument::new("Unpack parity");
    let scene_id = document.default_scene.unwrap();
    let world = document.scenes[&scene_id].world;
    let material = |label: &str, location: &str| Resource {
        id: ResourceId::new(),
        label: label.into(),
        location: location.into(),
        edition: Revision::initial(),
        kind: ResourceKind::Material,
        texture_value_type: None,
        implementation: "diffuse".into(),
        parameters: vec![Parameter::floats("Kd", [0.2, 0.3, 0.4])],
        definition_scope: None,
        declared_in: None,
        origin: None,
    };
    let site_material = material("At instance", "Materials/site.material");
    let site_id = site_material.id;
    document.resources.insert(site_id, site_material);

    let shape = Node::new(
        "Definition sphere",
        NodeKind::Entity {
            entity: EntityKind::Shape,
            implementation: "sphere".into(),
            parameters: vec![Parameter::floats("radius", [1.])],
            pose: None,
        },
    );
    let source_shape_id = shape.id;
    let body_translate = Node::new(
        "Translate",
        NodeKind::State {
            operation: StateOperation::Transform(TransformOperation::Translate([3., 0., 0.])),
        },
    );
    let identity = Node::new(
        "Identity",
        NodeKind::State {
            operation: StateOperation::Transform(TransformOperation::Identity),
        },
    );
    let after_identity = Node::new(
        "Translate",
        NodeKind::State {
            operation: StateOperation::Transform(TransformOperation::Translate([4., 0., 0.])),
        },
    );
    let replace = Node::new(
        "Transform",
        NodeKind::State {
            operation: StateOperation::Transform(TransformOperation::Replace(
                project::math::Mat4::translation(7., 0., 0.).0,
            )),
        },
    );
    let definition_root = Node::new(
        "Definition body",
        NodeKind::Scope {
            scope: ScopeKind::ObjectDefinition,
            children: vec![
                body_translate.id,
                identity.id,
                after_identity.id,
                replace.id,
                shape.id,
            ],
            owned_object: None,
            parent_camera: None,
        },
    );
    let definition_root_id = definition_root.id;
    let definition = Resource {
        id: ResourceId::new(),
        label: "Translated definition".into(),
        location: "Objects/translated.object".into(),
        edition: Revision::initial(),
        kind: ResourceKind::ObjectDefinition,
        texture_value_type: None,
        implementation: String::new(),
        parameters: Vec::new(),
        definition_scope: Some(definition_root.id),
        declared_in: Some(DeclarationState {
            transform: Some(CapturedTransform {
                start: project::math::Mat4::translation(2., 0., 0.).0,
                end: project::math::Mat4::translation(2., 0., 0.).0,
                active: ActiveSamples::All,
                times: [0., 1.],
            }),
            graphics: Some(CapturedGraphics {
                material: MaterialBinding::Default,
                inside_medium: None,
                outside_medium: None,
                reverse_orientation: true,
                color_space: "srgb".into(),
            }),
        }),
        origin: None,
    };
    let definition_id = definition.id;
    document.resources.insert(definition_id, definition);
    for node in [body_translate, identity, after_identity, replace] {
        document.definition_nodes.insert(node.id, node);
    }
    document.definition_nodes.insert(shape.id, shape);
    document
        .definition_nodes
        .insert(definition_root.id, definition_root);

    let instance = Node::new(
        "Placed instance",
        NodeKind::Instance {
            definition: definition_id,
        },
    );
    let placement_node = Node::new(
        "Placement",
        NodeKind::State {
            operation: StateOperation::Transform(TransformOperation::Translate([5., 0., 0.])),
        },
    );
    let placement_node_id = placement_node.id;
    let wrapper = Node::new(
        "Placed instance",
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children: vec![placement_node.id, instance.id],
            owned_object: Some(instance.id),
            parent_camera: None,
        },
    );
    let scene = document.scenes.get_mut(&scene_id).unwrap();
    let scene = std::sync::Arc::make_mut(scene);
    let NodeKind::Scope { children, .. } = &mut scene.nodes.get_mut(&world).unwrap().data else {
        unreachable!()
    };
    let site_state = Node::new(
        "Site material",
        NodeKind::State {
            operation: StateOperation::Material(MaterialBinding::Resource(site_id)),
        },
    );
    children.extend([site_state.id, wrapper.id]);
    scene.nodes.insert(site_state.id, site_state);
    scene.nodes.insert(placement_node.id, placement_node);
    scene.nodes.insert(instance.id, instance.clone());
    scene.nodes.insert(wrapper.id, wrapper.clone());

    let mut session = ProjectSession::new(document).unwrap();
    let before = session.snapshot();
    let before_draws = preview_draws(&before, &temp.path().join("before"));
    let change = session
        .apply(
            &session.revision(),
            &[Edit::UnpackObjectInstance { node: instance.id }],
        )
        .unwrap();
    let unpacked = &session.document().scene(scene_id).unwrap().nodes[&change.created[0]];
    assert_eq!(unpacked.label, "Placed instance");
    let NodeKind::Scope {
        children: unpacked_children,
        owned_object: None,
        ..
    } = &unpacked.data
    else {
        panic!("a plain group should replace the wrapper");
    };
    let unpacked_children = unpacked_children.clone();
    assert_eq!(unpacked_children[0], placement_node_id);
    assert!(
        session
            .document()
            .definition_nodes
            .contains_key(&definition_root_id)
    );
    let after = session.snapshot();
    let after_draws = preview_draws(&after, &temp.path().join("after"));
    assert_eq!(before_draws.len(), 1);
    assert_eq!(after_draws.len(), 1);
    assert_eq!(before_draws[0].0, after_draws[0].0);
    assert_eq!(before_draws[0].1, after_draws[0].1);
    assert_eq!(before_draws[0].2, None);
    let unpacked_material = after_draws[0].2.as_ref().unwrap();
    assert_eq!(unpacked_material.implementation, "diffuse");
    assert!(unpacked_material.parameters.is_empty());
    let copied_shape = unpacked_children
        .iter()
        .find(|id| {
            matches!(
                session.document().scene(scene_id).unwrap().nodes[id].data,
                NodeKind::Entity {
                    entity: EntityKind::Shape,
                    ..
                }
            )
        })
        .copied();
    assert!(copied_shape.is_some());
    assert_ne!(copied_shape.unwrap(), source_shape_id);
    session.undo(&session.revision()).unwrap();
    assert!(
        session
            .document()
            .scene(scene_id)
            .unwrap()
            .nodes
            .contains_key(&instance.id)
    );
    assert!(matches!(
        session.document().scene(scene_id).unwrap().nodes[&wrapper.id].data,
        NodeKind::Scope {
            owned_object: Some(_),
            ..
        }
    ));
}

fn preview_draws(
    snapshot: &project::ProjectSnapshot,
    bundle: &std::path::Path,
) -> Vec<(
    Vec<[f64; 16]>,
    bool,
    Option<protocol::geometry::MaterialRecord>,
)> {
    fs::create_dir_all(bundle).unwrap();
    let store = ProjectStore::Folder(
        FolderStore::create(
            &bundle.join("project"),
            snapshot.document().id,
            &snapshot.document().title,
            Revision::initial(),
        )
        .unwrap(),
    );
    let assets = stage_assets(snapshot.document(), &store, bundle, &|| false).unwrap();
    let program = program_from_project(snapshot, &store, &assets, || false).unwrap();
    let origins = program.origins.clone();
    let preview = preview_from_program(
        program,
        Some(&origins),
        bundle,
        ParseOptions::default(),
        &mut GeometryCache::new(0),
    )
    .unwrap();
    preview
        .manifest
        .draws
        .iter()
        .map(|draw| {
            (
                draw.world_from_object.clone(),
                draw.reverse_orientation,
                draw.material_id
                    .map(|id| preview.manifest.materials[id as usize].clone()),
            )
        })
        .collect()
}
