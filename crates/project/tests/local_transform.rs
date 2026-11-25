//! `set_local_transform` rewrites a node's own transform run so
//! the node gets the requested local transform, and nothing else moves.
mod common;
use common::*;
use project::math::Mat4;
use project::transform_trs::{Trs, approx_eq, compose};
use project::*;

fn op(label: &str, operation: TransformOperation) -> Node {
    Node::new(
        label,
        NodeKind::State {
            operation: StateOperation::Transform(operation),
        },
    )
}

fn sphere(label: &str) -> Node {
    Node::new(
        label,
        NodeKind::Entity {
            entity: EntityKind::Shape,
            implementation: "sphere".into(),
            parameters: Vec::new(),
            pose: None,
        },
    )
}

/// An Attribute group of `children`, with the children added to the scene.
fn group(document: &mut ProjectDocument, label: &str, children: Vec<Node>) -> Node {
    let group = Node::new(
        label,
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children: children.iter().map(|child| child.id).collect(),
            owned_object: None,
            parent_camera: None,
        },
    );
    for child in children {
        scene_mut(document).nodes.insert(child.id, child);
    }
    group
}

fn children(session: &ProjectSession, id: NodeId) -> Vec<NodeId> {
    match &scene(session.document()).nodes[&id].data {
        NodeKind::Scope { children, .. } => children.clone(),
        _ => Vec::new(),
    }
}

fn operations(session: &ProjectSession, id: NodeId) -> Vec<TransformOperation> {
    children(session, id)
        .iter()
        .filter_map(|child| match &scene(session.document()).nodes[child].data {
            NodeKind::State {
                operation: StateOperation::Transform(operation),
            } => Some(operation.clone()),
            _ => None,
        })
        .collect()
}

fn worlds(session: &ProjectSession) -> std::collections::BTreeMap<NodeId, Mat4> {
    evaluate(&session.snapshot().scene_view().unwrap())
        .unwrap()
        .nodes
        .into_iter()
        .map(|(id, state)| (id, state.start))
        .collect()
}

fn set(session: &mut ProjectSession, node: NodeId, value: LocalTransform) -> Result<ChangeSet> {
    session.apply(
        &session.revision(),
        &[Edit::SetLocalTransform { node, value }],
    )
}

fn trs(translation: [f64; 3], rotation: [f64; 3], scale: [f64; 3]) -> Trs {
    Trs {
        translation,
        rotation_euler_degrees: rotation,
        scale,
    }
}

/// World ▸ [Move, Parent ▸ [Translate 5, Rotate 30 y, A, B]], plus a
/// replacing group `R ▸ [Identity, Translate 0 1 0, C]` inside Parent.
struct Fixture {
    session: ProjectSession,
    parent: NodeId,
    a: NodeId,
    b: NodeId,
    replacing: NodeId,
    c: NodeId,
}

fn fixture() -> Fixture {
    let mut document = ProjectDocument::new("Local transform");
    let (a, b, c) = (sphere("A"), sphere("B"), sphere("C"));
    let ids = (a.id, b.id, c.id);
    let mut replacing = group(
        &mut document,
        "R",
        vec![
            op("Identity", TransformOperation::Identity),
            op("Translate", TransformOperation::Translate([0., 1., 0.])),
            c,
        ],
    );
    let replacing_id = replacing.id;
    if let NodeKind::Scope {
        children,
        owned_object,
        ..
    } = &mut replacing.data
    {
        *owned_object = children.iter().find(|child| **child == ids.2).copied();
    }
    let parent = group(
        &mut document,
        "Parent",
        vec![
            op("Translate", TransformOperation::Translate([5., 0., 0.])),
            op(
                "Rotate",
                TransformOperation::Rotate {
                    degrees: 30.,
                    axis: [0., 1., 0.],
                },
            ),
            a,
            b,
            replacing,
        ],
    );
    let parent_id = parent.id;
    append_world(
        &mut document,
        op("Move", TransformOperation::Scale([2., 2., 2.])),
    );
    append_world(&mut document, parent);
    Fixture {
        session: ProjectSession::new(document).unwrap(),
        parent: parent_id,
        a: ids.0,
        b: ids.1,
        replacing: replacing_id,
        c: ids.2,
    }
}

#[test]
fn an_object_gets_a_dedicated_group_and_nothing_else_moves() {
    let Fixture {
        mut session,
        parent,
        a,
        b,
        ..
    } = fixture();
    let before = worlds(&session);
    let parent_world = before[&a];
    let value = trs([1., 2., 3.], [10., 20., 30.], [-1., 2., 0.5]);
    set(&mut session, a, LocalTransform::Trs(value)).unwrap();
    let after = worlds(&session);
    assert!(approx_eq(
        &after[&a],
        &parent_world.compose(compose(&value))
    ));
    for (id, world) in &before {
        if *id != a {
            assert_eq!(after.get(id), Some(world), "{id:?} moved");
        }
    }
    // A sits alone in a new group holding T, Ry, Rx, Rz, S; B is untouched.
    let wrapper = children(&session, parent)[2];
    assert_eq!(*children(&session, wrapper).last().unwrap(), a);
    assert_eq!(operations(&session, wrapper).len(), 5);
    assert_eq!(children(&session, parent)[3], b);
    // One undo step restores the original document.
    session.undo(&session.revision()).unwrap();
    assert_eq!(worlds(&session), before);
    assert_eq!(children(&session, parent)[2], a);
}

#[test]
fn a_group_run_is_replaced_in_place_and_keeps_other_states() {
    let Fixture {
        session,
        parent,
        a,
        b,
        c,
        ..
    } = fixture();
    let material = Node::new(
        "Material",
        NodeKind::State {
            operation: StateOperation::Material(MaterialBinding::None),
        },
    );
    let material_id = material.id;
    let mut document = session.document().clone();
    scene_mut(&mut document).nodes.insert(material.id, material);
    if let NodeKind::Scope { children, .. } = &mut scene_mut(&mut document)
        .nodes
        .get_mut(&parent)
        .unwrap()
        .data
    {
        children.insert(1, material_id);
    }
    let mut session = ProjectSession::new(document).unwrap();
    let before = worlds(&session);
    let entry = before[&parent];
    let mut matrix = Mat4::translation(1., 0., 0.).0;
    matrix[1] = 0.5; // shear: only a matrix can say this
    set(&mut session, parent, LocalTransform::Matrix(matrix)).unwrap();
    assert_eq!(
        operations(&session, parent),
        [TransformOperation::Concat(matrix)]
    );
    // The new state sits where the first transform was, before the material.
    assert_eq!(children(&session, parent)[1], material_id);
    let after = worlds(&session);
    for child in [a, b] {
        assert!(approx_eq(&after[&child], &entry.compose(Mat4(matrix))));
    }
    // The replacing group below does not follow its parent.
    assert_eq!(after[&c], before[&c]);
}

#[test]
fn an_older_import_reset_run_refuses_local_edits() {
    let Fixture {
        mut session,
        replacing,
        c,
        ..
    } = fixture();
    let value = trs([0., 0., 4.], [0., 90., 0.], [1., 1., 1.]);
    // R still carries `Identity`: an older import's re-parenting
    // residue. Neither the group nor its object takes a local edit until
    // the scene re-imports under the new rules.
    for target in [c, replacing] {
        let error = set(&mut session, target, LocalTransform::Trs(value)).unwrap_err();
        assert!(
            error.to_string().contains("older import"),
            "{target:?}: {error}"
        );
    }
}

#[test]
fn an_identity_local_writes_no_identity() {
    let Fixture {
        mut session,
        parent,
        a,
        ..
    } = fixture();
    let identity = trs([0.; 3], [0.; 3], [1.; 3]);
    set(&mut session, parent, LocalTransform::Trs(identity)).unwrap();
    assert!(operations(&session, parent).is_empty());
    set(
        &mut session,
        a,
        LocalTransform::Trs(trs([1., 0., 0.], [0.; 3], [1.; 3])),
    )
    .unwrap();
    let wrapper = children(&session, parent)[0];
    assert_ne!(wrapper, a);
    // Back to identity: the emptied dedicated group goes away again.
    set(&mut session, a, LocalTransform::Matrix(Mat4::identity().0)).unwrap();
    assert_eq!(children(&session, parent)[0], a);
    assert!(!scene(session.document()).nodes.contains_key(&wrapper));
}

#[test]
fn refuses_what_has_no_single_local_transform() {
    let mut document = ProjectDocument::new("Refusals");
    let (h, k) = (sphere("H"), sphere("K"));
    let ids = (h.id, k.id);
    let uses = Node::new(
        "CoordSysTransform",
        NodeKind::State {
            operation: StateOperation::UseCoordinates("camera".into()),
        },
    );
    let h_group = group(&mut document, "HG", vec![uses, h]);
    let active = Node::new(
        "ActiveTransform",
        NodeKind::State {
            operation: StateOperation::ActiveTransform(ActiveSamples::End),
        },
    );
    let k_group = group(&mut document, "KG", vec![active, k]);
    append_world(&mut document, h_group);
    append_world(&mut document, k_group);
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    let identity = LocalTransform::Matrix(Mat4::identity().0);
    for node in [ids.0, ids.1, world] {
        assert!(set(&mut session, node, identity.clone()).is_err());
    }
    let Fixture { mut session, a, .. } = fixture();
    let mut matrix = Mat4::identity().0;
    matrix[3] = f64::NAN;
    assert!(set(&mut session, a, LocalTransform::Matrix(matrix)).is_err());
    let value = trs([0.; 3], [f64::INFINITY, 0., 0.], [1.; 3]);
    assert!(set(&mut session, a, LocalTransform::Trs(value)).is_err());
}

#[test]
fn refuses_an_inherited_single_sample_selection() {
    // `ActiveTransform EndTime` before the group: new states would set only
    // the end sample and make the static sphere animated.
    let mut document = ProjectDocument::new("Inherited active");
    let s = sphere("S");
    let s_id = s.id;
    let g = group(&mut document, "G", vec![s]);
    append_world(
        &mut document,
        Node::new(
            "ActiveTransform",
            NodeKind::State {
                operation: StateOperation::ActiveTransform(ActiveSamples::End),
            },
        ),
    );
    append_world(&mut document, g);
    let mut session = ProjectSession::new(document).unwrap();
    let error = set(
        &mut session,
        s_id,
        LocalTransform::Trs(trs([1., 0., 0.], [0.; 3], [1.; 3])),
    )
    .unwrap_err();
    assert!(error.to_string().contains("animated"), "{error}");
}

#[test]
fn edits_objects_inside_an_object_definition() {
    let mut document = ProjectDocument::new("Definition");
    let (_, root) = object_definition(&mut document);
    let shape = match &document.definition_nodes[&root].data {
        NodeKind::Scope { children, .. } => children[0],
        _ => unreachable!(),
    };
    let mut session = ProjectSession::new(document).unwrap();
    let value = trs([0., 2., 0.], [0., 45., 0.], [1.; 3]);
    set(&mut session, shape, LocalTransform::Trs(value)).unwrap();
    let world = |session: &ProjectSession| {
        evaluate(&session.snapshot().scene_view().unwrap())
            .unwrap()
            .nodes[&shape]
            .start
    };
    assert!(approx_eq(&world(&session), &compose(&value)));
    // The dedicated group lives in the definition table too.
    let wrapper = match &session.document().definition_nodes[&root].data {
        NodeKind::Scope { children, .. } => children[0],
        _ => unreachable!(),
    };
    assert_ne!(wrapper, shape);
    assert!(session.document().definition_nodes.contains_key(&wrapper));
    session.undo(&session.revision()).unwrap();
    assert_eq!(world(&session), Mat4::identity());
    assert!(!session.document().definition_nodes.contains_key(&wrapper));
}
