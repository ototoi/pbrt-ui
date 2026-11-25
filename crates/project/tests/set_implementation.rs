//! `set_implementation` renames an entity's,
//! configuration's or resource's implementation and keeps every parameter;
//! one change is one undo step. The table-driven keep/warn/refuse rules live
//! in pbrt-io and core; the session only refuses what can never change.
mod common;
use common::*;
use project::*;

fn apply(session: &mut ProjectSession, edits: &[Edit]) -> Result<ChangeSet> {
    session.apply(&session.revision(), edits)
}

fn set(target: ParameterTarget, implementation: &str) -> Edit {
    Edit::SetImplementation {
        target,
        implementation: implementation.into(),
    }
}

fn implementation(document: &ProjectDocument, id: NodeId) -> &str {
    match &scene(document).nodes[&id].data {
        NodeKind::Entity { implementation, .. }
        | NodeKind::Configuration { implementation, .. } => implementation,
        _ => panic!("no implementation"),
    }
}

fn resource(
    document: &mut ProjectDocument,
    kind: ResourceKind,
    implementation: &str,
) -> ResourceId {
    let resource = Resource {
        id: ResourceId::new(),
        label: "item".into(),
        location: "Materials/item.material".into(),
        edition: Revision::initial(),
        kind,
        texture_value_type: None,
        implementation: implementation.into(),
        parameters: vec![Parameter::floats("amount", [0.5])],
        definition_scope: None,
        declared_in: None,
        origin: None,
    };
    let id = resource.id;
    document.resources.insert(id, resource);
    id
}

#[test]
fn a_shape_changes_implementation_keeps_parameters_and_undoes() {
    let (document, _, shape) = native_scene();
    let before = parameters(&document, shape).to_vec();
    let mut session = ProjectSession::new(document).unwrap();
    let change = apply(&mut session, &[set(ParameterTarget::Node(shape), "disk")]).unwrap();
    assert_eq!(change.nodes, [shape]);
    assert_eq!(implementation(session.document(), shape), "disk");
    assert_eq!(parameters(session.document(), shape), before);
    session.undo(&session.revision()).unwrap();
    assert_eq!(implementation(session.document(), shape), "sphere");
    session.redo(&session.revision()).unwrap();
    assert_eq!(implementation(session.document(), shape), "disk");
}

#[test]
fn the_same_implementation_is_a_no_op() {
    let (document, _, shape) = native_scene();
    let mut session = ProjectSession::new(document).unwrap();
    let revision = session.revision();
    let change = apply(&mut session, &[set(ParameterTarget::Node(shape), "sphere")]).unwrap();
    assert_eq!(change.revision, revision);
    assert!(change.nodes.is_empty());
    assert!(!session.can_undo());
}

#[test]
fn options_list_nodes_change_in_place() {
    let mut document = ProjectDocument::new("Options");
    let camera = Node::new(
        "Camera",
        NodeKind::Entity {
            entity: EntityKind::Camera,
            implementation: "perspective".into(),
            parameters: vec![Parameter::floats("fov", [45.])],
            pose: None,
        },
    );
    let integrator = Node::new(
        "Integrator",
        NodeKind::Configuration {
            configuration: ConfigurationKind::Integrator,
            implementation: "path".into(),
            parameters: vec![],
        },
    );
    let (camera_id, integrator_id) = (camera.id, integrator.id);
    {
        let scene = scene_mut(&mut document);
        scene.options.extend([integrator_id, camera_id]);
        scene.nodes.insert(camera_id, camera);
        scene.nodes.insert(integrator_id, integrator);
    }
    document.validate().unwrap();
    let mut session = ProjectSession::new(document).unwrap();
    apply(
        &mut session,
        &[
            set(ParameterTarget::Node(integrator_id), "volpath"),
            set(ParameterTarget::Node(camera_id), "orthographic"),
        ],
    )
    .unwrap();
    assert_eq!(implementation(session.document(), integrator_id), "volpath");
    assert_eq!(
        implementation(session.document(), camera_id),
        "orthographic"
    );
    assert_eq!(
        scene(session.document()).options,
        [integrator_id, camera_id]
    );
    // One transaction, one undo step.
    session.undo(&session.revision()).unwrap();
    assert_eq!(implementation(session.document(), integrator_id), "path");
    assert_eq!(implementation(session.document(), camera_id), "perspective");
    assert!(!session.can_undo());
}

#[test]
fn resources_change_but_object_definitions_have_no_implementation() {
    let (mut document, _, _) = native_scene();
    let material = resource(&mut document, ResourceKind::Material, "mix");
    let (definition, _) = object_definition(&mut document);
    let mut session = ProjectSession::new(document).unwrap();
    let change = apply(
        &mut session,
        &[set(ParameterTarget::Resource(material), "diffuse")],
    )
    .unwrap();
    assert_eq!(change.resources, [material]);
    let edited = &session.document().resources[&material];
    assert_eq!(edited.implementation, "diffuse");
    assert_eq!(edited.parameters, [Parameter::floats("amount", [0.5])]);
    assert!(
        apply(
            &mut session,
            &[set(ParameterTarget::Resource(definition), "diffuse")]
        )
        .is_err()
    );
}

#[test]
fn refusals_leave_the_document_alone() {
    let mut document = ProjectDocument::new("Cancel");
    let cancel = Node::new(
        "No light",
        NodeKind::Entity {
            entity: EntityKind::AreaLight,
            implementation: String::new(),
            parameters: vec![],
            pose: None,
        },
    );
    let shape = Node::new(
        "Sphere",
        NodeKind::Entity {
            entity: EntityKind::Shape,
            implementation: "sphere".into(),
            parameters: vec![],
            pose: None,
        },
    );
    let group = Node::new(
        "Group",
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children: vec![cancel.id, shape.id],
            owned_object: None,
            parent_camera: None,
        },
    );
    let (cancel_id, shape_id, group_id) = (cancel.id, shape.id, group.id);
    append_world(&mut document, group);
    for node in [cancel, shape] {
        scene_mut(&mut document).nodes.insert(node.id, node);
    }
    document.validate().unwrap();
    let mut session = ProjectSession::new(document).unwrap();
    let revision = session.revision();
    for edit in [
        set(ParameterTarget::Node(cancel_id), "diffuse"),
        set(ParameterTarget::Node(shape_id), ""),
        set(ParameterTarget::Node(group_id), "sphere"),
        set(ParameterTarget::Node(NodeId::new()), "sphere"),
        set(ParameterTarget::Resource(ResourceId::new()), "diffuse"),
    ] {
        assert!(
            apply(&mut session, std::slice::from_ref(&edit)).is_err(),
            "{edit:?}"
        );
    }
    assert_eq!(session.revision(), revision);
}
