#![allow(dead_code)]
use project::*;

pub fn scene(document: &ProjectDocument) -> &SceneDocument {
    document.scene(document.default_scene.unwrap()).unwrap()
}

/// An `integer` parameter with plain values.
pub fn integers(name: &str, values: &[i64]) -> Parameter {
    Parameter {
        value_type: "integer".into(),
        name: name.into(),
        color_space: ColorSpaceBinding::Inherit,
        values: ParameterValues::Integers(values.to_vec()),
    }
}

/// A `string` parameter with plain values.
pub fn text(name: &str, values: &[&str]) -> Parameter {
    Parameter {
        value_type: "string".into(),
        name: name.into(),
        color_space: ColorSpaceBinding::Inherit,
        values: ParameterValues::Strings(values.iter().map(|value| (*value).into()).collect()),
    }
}

pub fn scene_mut(document: &mut ProjectDocument) -> &mut SceneDocument {
    let id = document.default_scene.unwrap();
    std::sync::Arc::make_mut(document.scenes.get_mut(&id).unwrap())
}
pub fn append_world(document: &mut ProjectDocument, node: Node) {
    let world = scene(document).world;
    let NodeKind::Scope { children, .. } =
        &mut scene_mut(document).nodes.get_mut(&world).unwrap().data
    else {
        panic!("missing World");
    };
    children.push(node.id);
    scene_mut(document).nodes.insert(node.id, node);
}
pub fn native_scene() -> (ProjectDocument, NodeId, NodeId) {
    let mut document = ProjectDocument::new("Native scene");
    let translate = Node::new(
        "Placement",
        NodeKind::State {
            operation: StateOperation::Transform(TransformOperation::Translate([1., 2., 3.])),
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
    let ids = (translate.id, shape.id);
    append_world(&mut document, translate);
    append_world(&mut document, shape);
    (document, ids.0, ids.1)
}
pub fn parameters(document: &ProjectDocument, node: NodeId) -> &[Parameter] {
    let NodeKind::Entity { parameters, .. } = &scene(document).nodes[&node].data else {
        panic!("not an entity");
    };
    parameters
}
pub fn set_parameter(node: NodeId, name: &str, value: Option<Parameter>) -> Edit {
    Edit::SetParameter {
        target: ParameterTarget::Node(node),
        name: name.into(),
        value_type: "float".into(),
        value,
    }
}
/// Object definition holding one sphere; returns the resource and its root scope.
pub fn object_definition(document: &mut ProjectDocument) -> (ResourceId, NodeId) {
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
    let ids = (definition.id, root.id);
    document.definition_nodes.insert(shape.id, shape);
    document.definition_nodes.insert(root.id, root);
    document.resources.insert(definition.id, definition);
    ids
}
