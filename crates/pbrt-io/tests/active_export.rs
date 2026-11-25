use pbrt_io::export_project;
use project::*;

fn append(document: &mut ProjectDocument, node: Node) {
    let view = document.default_scene_view().unwrap();
    let scene_id = view.scene.id;
    let world = view.scene.world;
    let scene = std::sync::Arc::make_mut(document.scenes.get_mut(&scene_id).unwrap());
    let NodeKind::Scope { children, .. } = &mut scene.nodes.get_mut(&world).unwrap().data else {
        unreachable!()
    };
    children.push(node.id);
    scene.nodes.insert(node.id, node);
}

#[test]
fn inactive_objects_and_definition_children_are_omitted_from_export() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("project");
    let mut document = ProjectDocument::new("Active export");
    let active_shape = Node::new(
        "Visible",
        NodeKind::Entity {
            entity: EntityKind::Shape,
            implementation: "sphere".into(),
            parameters: vec![Parameter::floats("radius", [3.])],
            pose: None,
        },
    );
    let mut inactive_shape = Node::new(
        "Hidden",
        NodeKind::Entity {
            entity: EntityKind::Shape,
            implementation: "sphere".into(),
            parameters: vec![Parameter::floats("radius", [2.])],
            pose: None,
        },
    );
    inactive_shape.active = false;
    append(&mut document, active_shape);
    append(&mut document, inactive_shape);

    let grouped_shape = Node::new(
        "Hidden group child",
        NodeKind::Entity {
            entity: EntityKind::Shape,
            implementation: "sphere".into(),
            parameters: vec![Parameter::floats("radius", [5.])],
            pose: None,
        },
    );
    let mut group = Node::new(
        "Hidden group",
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children: vec![grouped_shape.id],
            owned_object: None,
            parent_camera: None,
        },
    );
    group.active = false;
    let scene_id = document.default_scene.unwrap();
    let scene = std::sync::Arc::make_mut(document.scenes.get_mut(&scene_id).unwrap());
    let world = scene.world;
    let NodeKind::Scope { children, .. } = &mut scene.nodes.get_mut(&world).unwrap().data else {
        unreachable!()
    };
    children.push(group.id);
    scene.nodes.insert(grouped_shape.id, grouped_shape);
    scene.nodes.insert(group.id, group);

    let mut wrapped_shape = Node::new(
        "Hidden wrapped shape",
        NodeKind::Entity {
            entity: EntityKind::Shape,
            implementation: "sphere".into(),
            parameters: vec![Parameter::floats("radius", [6.])],
            pose: None,
        },
    );
    wrapped_shape.active = false;
    let area = Node::new(
        "Attached area light",
        NodeKind::Entity {
            entity: EntityKind::AreaLight,
            implementation: "diffuse".into(),
            parameters: Vec::new(),
            pose: None,
        },
    );
    let wrapper = Node::new(
        "Hidden wrapped shape",
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children: vec![area.id, wrapped_shape.id],
            owned_object: Some(wrapped_shape.id),
            parent_camera: None,
        },
    );
    let scene_id = document.default_scene.unwrap();
    let scene = std::sync::Arc::make_mut(document.scenes.get_mut(&scene_id).unwrap());
    scene.nodes.insert(area.id, area);
    scene.nodes.insert(wrapped_shape.id, wrapped_shape);
    append(&mut document, wrapper);

    let mut light = Node::new(
        "Hidden light",
        NodeKind::Entity {
            entity: EntityKind::Light,
            implementation: "point".into(),
            parameters: Vec::new(),
            pose: None,
        },
    );
    light.active = false;
    append(&mut document, light);

    let mut defined_shape = Node::new(
        "Hidden definition shape",
        NodeKind::Entity {
            entity: EntityKind::Shape,
            implementation: "sphere".into(),
            parameters: vec![Parameter::floats("radius", [4.])],
            pose: None,
        },
    );
    defined_shape.active = false;
    let root = Node::new(
        "Definition root",
        NodeKind::Scope {
            scope: ScopeKind::ObjectDefinition,
            children: vec![defined_shape.id],
            owned_object: None,
            parent_camera: None,
        },
    );
    document
        .definition_nodes
        .insert(defined_shape.id, defined_shape);
    document.definition_nodes.insert(root.id, root.clone());
    let definition = Resource {
        id: ResourceId::new(),
        label: "Hidden definition".into(),
        location: "Objects/hidden.object".into(),
        edition: Revision::initial(),
        kind: ResourceKind::ObjectDefinition,
        texture_value_type: None,
        implementation: String::new(),
        parameters: Vec::new(),
        definition_scope: Some(root.id),
        declared_in: None,
        origin: None,
    };
    let mut instance = Node::new(
        "Hidden instance",
        NodeKind::Instance {
            definition: definition.id,
        },
    );
    instance.active = false;
    append(&mut document, instance);
    document.resources.insert(definition.id, definition);

    let snapshot = ProjectSession::new(document).unwrap().snapshot();
    let store = FolderStore::create(
        &path,
        snapshot.document().id,
        &snapshot.document().title,
        snapshot.revision().clone(),
    )
    .unwrap();
    let exported = export_project(
        &snapshot,
        &ProjectStore::Folder(store),
        &temp.path().join("out"),
        || false,
    )
    .unwrap();
    let text = std::fs::read_to_string(exported).unwrap();
    assert!(text.contains("\"float radius\" [ 3 ]"));
    assert!(!text.contains("[ 2 ]"));
    assert!(!text.contains("[ 4 ]"));
    assert!(!text.contains("[ 5 ]"));
    assert!(!text.contains("[ 6 ]"));
    assert!(!text.contains("AreaLightSource"));
    assert!(!text.contains("LightSource \"point\""));
    assert!(!text.contains("ObjectInstance"));
}
