mod common;
use common::*;
use project::*;

#[test]
fn batch_edits_preserve_ids_and_advance_each_touched_edition_once() {
    let (document, transform, shape) = native_scene();
    let id = document.id;
    let mut session = ProjectSession::new(document).unwrap();
    let old = session.snapshot();
    let change = session
        .apply(
            &session.revision(),
            &[
                Edit::RenameNode {
                    node: shape,
                    label: "Edited sphere".into(),
                },
                set_parameter(shape, "radius", Some(Parameter::floats("radius", [2.]))),
                Edit::SetTransform {
                    node: transform,
                    value: TransformOperation::Translate([4., 5., 6.]),
                },
            ],
        )
        .unwrap();
    assert_eq!(session.document().id, id);
    assert_eq!(session.revision(), Revision::from_u64(1));
    assert_eq!(change.nodes.len(), 2);
    assert_eq!(scene(session.document()).nodes[&shape].id, shape);
    assert_eq!(
        scene(session.document()).nodes[&shape].edition,
        Revision::from_u64(1)
    );
    assert_eq!(
        scene(session.document()).nodes[&transform].edition,
        Revision::from_u64(1)
    );
    assert_eq!(
        parameters(session.document(), shape),
        &[Parameter::floats("radius", [2.])]
    );
    assert_eq!(scene(old.document()).nodes[&shape].label, "Sphere");
    assert_eq!(
        parameters(old.document(), shape),
        &[Parameter::floats("radius", [1.])]
    );
}

#[test]
fn array_value_windows_keep_length_and_round_trip_through_history() {
    let (mut document, _, shape) = native_scene();
    let NodeKind::Entity {
        parameters: params, ..
    } = &mut scene_mut(&mut document).nodes.get_mut(&shape).unwrap().data
    else {
        unreachable!()
    };
    params.push(Parameter {
        name: "P".into(),
        value_type: "point3".into(),
        color_space: ColorSpaceBinding::Inherit,
        values: ParameterValues::Floats((0..9).map(|n| FloatValue::Finite(n as f64)).collect()),
    });
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::SetParameterValues {
                target: ParameterTarget::Node(shape),
                name: "P".into(),
                offset: 3,
                values: ParameterValues::Floats(vec![
                    FloatValue::Finite(30.),
                    FloatValue::Finite(31.),
                    FloatValue::Finite(32.),
                ]),
            }],
        )
        .unwrap();
    let parameter = parameters(session.document(), shape)
        .iter()
        .find(|p| p.name == "P")
        .unwrap();
    let ParameterValues::Floats(values) = &parameter.values else {
        panic!("wrong type")
    };
    assert_eq!(values.len(), 9);
    assert_eq!(
        &values[3..6],
        &[
            FloatValue::Finite(30.),
            FloatValue::Finite(31.),
            FloatValue::Finite(32.)
        ]
    );
    session.undo(&session.revision()).unwrap();
    let ParameterValues::Floats(values) = &parameters(session.document(), shape)
        .iter()
        .find(|p| p.name == "P")
        .unwrap()
        .values
    else {
        panic!("wrong type")
    };
    assert_eq!(
        values.as_slice(),
        (0..9)
            .map(|n| FloatValue::Finite(n as f64))
            .collect::<Vec<_>>()
            .as_slice()
    );
    session.redo(&session.revision()).unwrap();
    let ParameterValues::Floats(values) = &parameters(session.document(), shape)
        .iter()
        .find(|p| p.name == "P")
        .unwrap()
        .values
    else {
        panic!("wrong type")
    };
    assert_eq!(
        &values[3..6],
        &[
            FloatValue::Finite(30.),
            FloatValue::Finite(31.),
            FloatValue::Finite(32.)
        ]
    );
}

/// A window edit on a resource that does not exist is an error, not a panic
/// (`project::Edit` is public; core's own checks are not the only caller).
#[test]
fn an_array_window_on_a_missing_resource_is_refused() {
    let (document, _, _) = native_scene();
    let mut session = ProjectSession::new(document.clone()).unwrap();
    let result = session.apply(
        &session.revision(),
        &[Edit::SetParameterValues {
            target: ParameterTarget::Resource(ResourceId::new()),
            name: "P".into(),
            offset: 0,
            values: ParameterValues::Floats(vec![FloatValue::Finite(1.)]),
        }],
    );
    assert!(result.is_err());
    assert_eq!(session.document(), &document);
}

#[test]
fn invalid_array_window_rejects_the_whole_transaction() {
    let (mut document, _, shape) = native_scene();
    let NodeKind::Entity { parameters, .. } =
        &mut scene_mut(&mut document).nodes.get_mut(&shape).unwrap().data
    else {
        unreachable!()
    };
    parameters.push(Parameter::floats("samples", [1., 2., 3.]));
    let mut session = ProjectSession::new(document.clone()).unwrap();
    let result = session.apply(
        &session.revision(),
        &[Edit::SetParameterValues {
            target: ParameterTarget::Node(shape),
            name: "samples".into(),
            offset: 2,
            values: ParameterValues::Floats(vec![FloatValue::Finite(9.), FloatValue::Finite(10.)]),
        }],
    );
    assert!(result.is_err());
    assert_eq!(session.document(), &document);
    assert!(!session.can_undo());
}

#[test]
fn invalid_batch_rolls_back_values_editions_revision_and_history() {
    let (document, transform, shape) = native_scene();
    let mut session = ProjectSession::new(document.clone()).unwrap();
    assert!(
        session
            .apply(
                &session.revision(),
                &[
                    Edit::RenameNode {
                        node: shape,
                        label: "Must roll back".into()
                    },
                    Edit::SetTransform {
                        node: transform,
                        value: TransformOperation::Scale([f64::NAN, 1., 1.])
                    },
                ]
            )
            .is_err()
    );
    assert_eq!(session.document(), &document);
    assert_eq!(session.revision(), Revision::initial());
    assert!(!session.can_undo());
    assert!(!session.can_redo());
}

#[test]
fn stale_revision_rejects_a_second_edit_without_changing_the_scene() {
    let (document, _, shape) = native_scene();
    let mut session = ProjectSession::new(document).unwrap();
    let old = session.revision();
    session
        .apply(
            &old,
            &[Edit::RenameNode {
                node: shape,
                label: "First".into(),
            }],
        )
        .unwrap();
    assert!(matches!(
        session.apply(
            &old,
            &[Edit::RenameNode {
                node: shape,
                label: "Stale".into()
            }]
        ),
        Err(ProjectError::RevisionConflict { .. })
    ));
    assert_eq!(scene(session.document()).nodes[&shape].label, "First");
}

#[test]
fn undo_redo_preserve_parameter_order_and_keep_revisions_and_editions_monotonic() {
    let (mut document, _, shape) = native_scene();
    let NodeKind::Entity {
        parameters: params, ..
    } = &mut scene_mut(&mut document).nodes.get_mut(&shape).unwrap().data
    else {
        unreachable!()
    };
    params.extend([
        Parameter::floats("zmin", [-1.]),
        Parameter::floats("zmax", [1.]),
    ]);
    let original = params.clone();
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(&session.revision(), &[set_parameter(shape, "zmin", None)])
        .unwrap();
    assert_eq!(parameters(session.document(), shape).len(), 2);
    session.undo(&session.revision()).unwrap();
    assert_eq!(parameters(session.document(), shape), original);
    assert_eq!(
        scene(session.document()).nodes[&shape].edition,
        Revision::from_u64(2)
    );
    session.redo(&session.revision()).unwrap();
    assert_eq!(session.revision(), Revision::from_u64(3));
    assert_eq!(
        scene(session.document()).nodes[&shape].edition,
        Revision::from_u64(3)
    );
    session.undo(&session.revision()).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::RenameNode {
                node: shape,
                label: "New branch".into(),
            }],
        )
        .unwrap();
    assert!(!session.can_redo());
}

#[test]
fn undo_parameter_addition_restores_omission_instead_of_inventing_a_default() {
    let (document, _, shape) = native_scene();
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[set_parameter(
                shape,
                "phimax",
                Some(Parameter::floats("phimax", [180.])),
            )],
        )
        .unwrap();
    session.undo(&session.revision()).unwrap();
    assert_eq!(
        parameters(session.document(), shape),
        &[Parameter::floats("radius", [1.])]
    );
}

#[test]
fn duplicate_declarations_are_preserved_but_ambiguous_edits_are_rejected() {
    let (mut document, _, shape) = native_scene();
    let NodeKind::Entity {
        parameters: params, ..
    } = &mut scene_mut(&mut document).nodes.get_mut(&shape).unwrap().data
    else {
        unreachable!()
    };
    params.push(Parameter::floats("radius", [2.]));
    let mut session = ProjectSession::new(document.clone()).unwrap();
    assert!(
        session
            .apply(
                &session.revision(),
                &[set_parameter(
                    shape,
                    "radius",
                    Some(Parameter::floats("radius", [3.]))
                )]
            )
            .is_err()
    );
    assert_eq!(session.document(), &document);
}

#[test]
fn no_op_edits_do_not_invalidate_editions_or_create_undo_entries() {
    let (document, _, shape) = native_scene();
    let mut session = ProjectSession::new(document).unwrap();
    let result = session
        .apply(
            &session.revision(),
            &[Edit::RenameNode {
                node: shape,
                label: "Sphere".into(),
            }],
        )
        .unwrap();
    assert!(result.nodes.is_empty());
    assert_eq!(result.revision, Revision::initial());
    assert!(!session.can_undo());
}

#[test]
fn snapshot_is_send_and_sync_without_renderer_or_ui_objects() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<ProjectSnapshot>();
}

#[test]
fn edition_overflow_fails_atomically_instead_of_reusing_an_old_cache_version() {
    let (mut document, _, shape) = native_scene();
    scene_mut(&mut document)
        .nodes
        .get_mut(&shape)
        .unwrap()
        .edition = Revision::from_u64(u64::MAX);
    let mut session = ProjectSession::new(document.clone()).unwrap();
    assert!(
        session
            .apply(
                &session.revision(),
                &[Edit::RenameNode {
                    node: shape,
                    label: "Overflow".into()
                }]
            )
            .is_err()
    );
    assert_eq!(session.document(), &document);
    assert_eq!(session.revision(), Revision::initial());
    assert!(!session.can_undo());
}
