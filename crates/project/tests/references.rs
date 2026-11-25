//! the reference index reports *what* each
//! reference is through — a parameter's name, or the binding's side —
//! definition nodes name the resource owning them, and the session
//! builds one index per revision.
use project::{
    Edit, ItemId, MaterialBinding, Node, NodeKind, Parameter, ParameterValues, ProjectDocument,
    ProjectSession, ReferenceVia, Resource, ResourceId, ResourceKind, Revision, ScopeKind,
    StateOperation,
};

fn resource(kind: ResourceKind, label: &str, implementation: &str) -> Resource {
    Resource {
        id: ResourceId::new(),
        label: label.into(),
        location: format!("Materials/{label}.material"),
        edition: Revision::initial(),
        kind,
        texture_value_type: (kind == ResourceKind::Texture)
            .then_some(project::TextureValueType::Spectrum),
        implementation: implementation.into(),
        parameters: Vec::new(),
        definition_scope: None,
        declared_in: None,
        origin: None,
    }
}

fn collect<T, F>(document: &ProjectDocument, item: ItemId, pick: F) -> Vec<T>
where
    F: FnMut(&project::ReferenceLocation) -> Option<T>,
{
    document
        .reference_index()
        .references(item)
        .iter()
        .filter_map(pick)
        .collect()
}

fn with_world(document: &mut ProjectDocument, children: Vec<project::NodeId>, nodes: Vec<Node>) {
    let scene = document.default_scene.unwrap();
    let world = document.scenes[&scene].world;
    let scene = std::sync::Arc::make_mut(document.scenes.get_mut(&scene).expect("scene"));
    let Some(world_node) = scene.nodes.get_mut(&world) else {
        unreachable!();
    };
    let NodeKind::Scope {
        children: world_children,
        ..
    } = &mut world_node.data
    else {
        unreachable!();
    };
    world_children.extend(children);
    for node in nodes {
        scene.nodes.insert(node.id, node);
    }
}

/// One scene whose group binds a material and a medium on a shape whose
/// `bumpmap` parameter references a texture, plus an instance; one
/// object definition whose inner binding state binds the same material
/// again.
fn document() -> (
    ProjectDocument,
    ResourceId,      /* the material */
    ResourceId,      /* the medium */
    ResourceId,      /* the texture */
    ResourceId,      /* the object definition */
    project::NodeId, /* the shape */
    project::NodeId, /* the inner binding state */
) {
    let mut document = ProjectDocument::new("References");
    let id = |r: &Resource| (r.id, r.clone());
    let (bound, bound_resource) = id(&resource(ResourceKind::Material, "Bound", "diffuse"));
    let (fog, mut fog_resource) = id(&resource(ResourceKind::Medium, "Fog", "homogeneous"));
    fog_resource.location = "Media/Fog.medium".into();
    let (tex, tex_resource) = id(&resource(ResourceKind::Texture, "Tex", "checkerboard"));
    let (mut tree, mut tree_resource) = id(&resource(ResourceKind::ObjectDefinition, "Tree", ""));
    tree_resource.location = "Objects/Tree.object".into();
    document.resources.insert(bound, bound_resource);
    document.resources.insert(fog, fog_resource);
    document.resources.insert(tex, tex_resource);

    let shape = Node::new(
        "Sphere",
        NodeKind::Entity {
            entity: project::EntityKind::Shape,
            implementation: "trianglemesh".into(),
            parameters: vec![Parameter {
                name: "bumpmap".into(),
                value_type: "texture".into(),
                color_space: project::ColorSpaceBinding::Inherit,
                values: ParameterValues::Resources {
                    kind: ResourceKind::Texture,
                    ids: vec![tex],
                },
            }],
            pose: None,
        },
    );
    let mut medium_state = Node::new(
        "Medium state",
        NodeKind::State {
            operation: StateOperation::Medium {
                inside: Some(fog),
                outside: Some(fog),
            },
        },
    );
    medium_state.label = "Medium state".into();
    let _ = &mut medium_state;
    let instance = Node::new("Tree instance", NodeKind::Instance { definition: tree });
    // The definition's scope holds a lone shape whose own binding state
    // binds the same material.
    let inner_state = Node::new(
        "Inner binding",
        NodeKind::State {
            operation: StateOperation::Material(MaterialBinding::Resource(bound)),
        },
    );
    let inner_shape = Node::new(
        "Trunk",
        NodeKind::Entity {
            entity: project::EntityKind::Shape,
            implementation: "sphere".into(),
            parameters: vec![],
            pose: None,
        },
    );
    let scope = Node::new(
        "Tree",
        NodeKind::Scope {
            scope: ScopeKind::ObjectDefinition,
            children: vec![inner_state.id, inner_shape.id],
            owned_object: None,
            parent_camera: None,
        },
    );
    tree_resource.definition_scope = Some(scope.id);
    document.resources.insert(tree, tree_resource);
    document.definition_nodes.insert(scope.id, scope);
    for node in [inner_state.clone(), inner_shape] {
        document.definition_nodes.insert(node.id, node);
    }

    let group = Node::new(
        "Pair",
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children: vec![medium_state.id, shape.id, instance.id],
            owned_object: None,
            parent_camera: None,
        },
    );
    let shape_id = shape.id;
    with_world(
        &mut document,
        vec![group.id],
        vec![group, medium_state, shape, instance],
    );
    document.validate().unwrap();
    (document, bound, fog, tex, tree, shape_id, inner_state.id)
}

#[test]
fn bindings_report_their_side_and_kind() {
    let (document, bound, fog, _, _, _, _) = document();
    // The material is bound by the object definition's inner state only:
    // the scene binds nothing material (the group binds the medium).
    let vias = collect(&document, ItemId::Resource(bound), |location| {
        (location.node
            == document
                .definition_nodes
                .values()
                .find(|node| node.label == "Inner binding")
                .map(|node| node.id))
        .then(|| location.via.wire())
    });
    assert_eq!(vias, ["material".to_owned()]);
    let vias = collect(&document, ItemId::Resource(fog), |location| {
        Some(location.via.wire())
    });
    assert_eq!(
        vias,
        ["medium inside".to_owned(), "medium outside".to_owned()],
        "both sides of one binding count, with their sides named"
    );
}

#[test]
fn instances_report_via_instance() {
    let (document, _, _, _, tree, _, _) = document();
    let vias = collect(&document, ItemId::Resource(tree), |location| {
        Some(location.via.wire())
    });
    assert_eq!(vias, ["instance".to_owned()]);
}

#[test]
fn parameters_report_the_parameter_name() {
    let (document, _, _, tex, _, shape, _) = document();
    let sources = collect(&document, ItemId::Resource(tex), |location| {
        Some((location.node, location.via.wire()))
    });
    assert_eq!(
        sources,
        [(Some(shape), "parameter:bumpmap".to_owned())],
        "the shape's parameter names the texture"
    );
}

#[test]
fn definition_nodes_report_their_owner() {
    let (document, bound, _, _, tree, _, inner_state) = document();
    let index = document.reference_index();
    assert_eq!(
        index.definition_owner(inner_state),
        Some(tree),
        "the inner binding state belongs to the definition's resource"
    );
    // And the reference to the bound material says where it sits.
    let sources = collect(&document, ItemId::Resource(bound), |location| {
        Some((location.scene.is_none(), location.node))
    });
    assert_eq!(sources, [(true, Some(inner_state))]);
}

#[test]
fn one_index_per_revision() {
    let (document, bound, _, _, _, _, _) = document();
    let mut session = ProjectSession::new(document).unwrap();
    let first = session.reference_index();
    let snapshot_index = session.snapshot().reference_index();
    assert!(std::sync::Arc::ptr_eq(&first, &snapshot_index));
    let again = session.reference_index();
    assert!(
        std::sync::Arc::ptr_eq(&first, &again),
        "the same revision serves one index"
    );
    assert_eq!(first.count(ItemId::Resource(bound)), 1);
    // An edit brings a new revision and a rebuilt index.
    let scene = session.active_scene().unwrap();
    let world = session.document().scenes[&scene].world;
    let target = session.document().scenes[&scene].nodes[&world].id;
    let _ = target;
    let group = session.document().scenes[&scene].nodes[&world].id;
    let NodeKind::Scope { children, .. } = &session.document().scenes[&scene].nodes[&group].data
    else {
        unreachable!();
    };
    let renamed = children[0];
    session
        .apply(
            &session.revision(),
            &[Edit::RenameNode {
                node: renamed,
                label: "Renamed".into(),
            }],
        )
        .unwrap();
    let rebuilt = session.reference_index();
    assert!(
        !std::sync::Arc::ptr_eq(&first, &rebuilt),
        "a new revision rebuilds the index"
    );
    assert_eq!(rebuilt.count(ItemId::Resource(bound)), 1);
}

/// A snapshot never changes, so its reference index is built once and
/// shared by every reader (and by the snapshot's clones).
#[test]
fn a_snapshot_builds_its_reference_index_once() {
    let session = ProjectSession::new(ProjectDocument::new("Shared index")).unwrap();
    let snapshot = session.snapshot();
    let first = snapshot.reference_index();
    assert!(std::sync::Arc::ptr_eq(&first, &snapshot.reference_index()));
    assert!(std::sync::Arc::ptr_eq(
        &first,
        &snapshot.clone().reference_index()
    ));
    assert_eq!(*first, snapshot.document().reference_index());
}
