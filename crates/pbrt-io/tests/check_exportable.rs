//! Apply-time exportability: the light check that core
//! runs after every `project.apply`, limited to the nodes and resources the
//! edits touched instead of the whole-document walk.
use pbrt_io::check_exportable;
use project::*;
use std::sync::Arc;

fn shape(implementation: &str, parameters: Vec<Parameter>) -> Node {
    Node::new(
        "Shape",
        NodeKind::Entity {
            entity: EntityKind::Shape,
            implementation: implementation.into(),
            parameters,
            pose: None,
        },
    )
}

fn material_resource(label: &str, parameters: Vec<Parameter>) -> Resource {
    Resource {
        id: ResourceId::new(),
        label: label.into(),
        location: format!("Materials/{label}.material"),
        edition: Revision::initial(),
        kind: ResourceKind::Material,
        texture_value_type: None,
        implementation: "diffuse".into(),
        parameters,
        definition_scope: None,
        declared_in: None,
        origin: None,
    }
}

fn document_with_scene_nodes(nodes: Vec<Node>) -> ProjectDocument {
    let mut document = ProjectDocument::new("Check");
    let scene = Arc::make_mut(
        document
            .scenes
            .get_mut(&document.default_scene.unwrap())
            .unwrap(),
    );
    for node in nodes {
        scene.nodes.insert(node.id, node);
    }
    document
}

fn document_with_definition_nodes(nodes: Vec<Node>) -> ProjectDocument {
    let mut document = ProjectDocument::new("Check");
    for node in nodes {
        document.definition_nodes.insert(node.id, node);
    }
    document
}

#[test]
fn untouched_documents_are_never_rejected() {
    let sphere = shape("sphere", vec![Parameter::floats("radius", [1.])]);
    // Harmful content outside the touched set is not this check's business;
    // the full-document walk happens at save and export.
    let teapot = shape("teapot", Vec::new());
    let document = document_with_scene_nodes(vec![sphere.clone(), teapot]);
    check_exportable(&document, &[], &[]).unwrap();
    check_exportable(&document, &[sphere.id], &[]).unwrap();
}

#[test]
fn touched_scene_nodes_are_checked() {
    let sphere = shape("sphere", vec![Parameter::floats("radius", [1.])]);
    let teapot = shape("teapot", Vec::new());
    let document = document_with_scene_nodes(vec![sphere.clone(), teapot.clone()]);
    check_exportable(&document, &[sphere.id], &[]).unwrap();
    let error = check_exportable(&document, &[teapot.id], &[]).unwrap_err();
    assert!(
        error.to_string().to_lowercase().contains("teapot"),
        "unexpected error: {error}"
    );
}

#[test]
fn definition_nodes_are_found_by_id() {
    let sphere = shape("sphere", Vec::new());
    let document = document_with_definition_nodes(vec![sphere.clone()]);
    check_exportable(&document, &[sphere.id], &[]).unwrap();
    let teapot = shape("teapot", Vec::new());
    let document = document_with_definition_nodes(vec![teapot.clone()]);
    let error = check_exportable(&document, &[teapot.id], &[]).unwrap_err();
    assert!(
        error.to_string().to_lowercase().contains("teapot"),
        "unexpected error: {error}"
    );
}

#[test]
fn touched_resources_are_checked() {
    let strings = |name: &str| Parameter {
        name: name.into(),
        value_type: "string".into(),
        color_space: ColorSpaceBinding::Inherit,
        values: ParameterValues::Strings(vec!["value".into()]),
    };
    let clean = material_resource("clean", Vec::new());
    let typed = material_resource("typed", vec![strings("type")]);
    let surprise = material_resource("surprise", vec![strings("surprise")]);
    let mut document = ProjectDocument::new("Check");
    for resource in [&clean, &typed, &surprise] {
        document.resources.insert(resource.id, resource.clone());
    }
    check_exportable(&document, &[], &[clean.id]).unwrap();
    let error = check_exportable(&document, &[], &[typed.id]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("type is stored as implementation"),
        "unexpected error: {error}"
    );
    let error = check_exportable(&document, &[], &[surprise.id]).unwrap_err();
    assert!(
        error.to_string().contains(
            "uses the \"surprise\" string parameter, which the importer does not support"
        ),
        "unexpected error: {error}"
    );
}
