//! G10: local transforms set through `set_local_transform` export and
//! reimport to the same world matrices. Imports resolve transform resets
//! so the edited runs never contain `Identity` and a posed camera
//! exports as a single inverse-pose `Transform`.
use pbrt_io::{ImportOptions, export_project, import_project};
use project::math::{Mat4, inverse};
use project::transform_trs::{Trs, approx_eq};
use project::*;

/// Every shape's world matrix, ordered by its translation.
fn shape_worlds(session: &ProjectSession) -> Vec<Mat4> {
    let snapshot = session.snapshot();
    let view = snapshot.scene_view().unwrap();
    let evaluated = evaluate(&view).unwrap();
    let mut worlds: Vec<Mat4> = view
        .scene
        .nodes
        .values()
        .filter(|node| {
            matches!(
                node.data,
                NodeKind::Entity {
                    entity: EntityKind::Shape,
                    ..
                }
            )
        })
        .map(|node| evaluated.nodes[&node.id].start)
        .collect();
    worlds.sort_by(|a, b| {
        [a.0[3], a.0[7], a.0[11]]
            .partial_cmp(&[b.0[3], b.0[7], b.0[11]])
            .unwrap()
    });
    worlds
}

fn shape_named(session: &ProjectSession, radius: f64) -> NodeId {
    let snapshot = session.snapshot();
    let view = snapshot.scene_view().unwrap();
    view.scene
        .nodes
        .values()
        .find(|node| match &node.data {
            NodeKind::Entity { parameters, .. } => parameters.iter().any(|parameter| {
                parameter.name == "radius"
                    && matches!(&parameter.values, ParameterValues::Floats(values)
                        if values.first().is_some_and(|value| *value == FloatValue::Finite(radius)))
            }),
            _ => false,
        })
        .unwrap()
        .id
}

fn camera_pose(session: &ProjectSession) -> Mat4 {
    let snapshot = session.snapshot();
    let view = snapshot.scene_view().unwrap();
    let evaluated = evaluate(&view).unwrap();
    let camera = view
        .scene
        .options
        .iter()
        .find(|id| {
            matches!(
                view.node(**id).map(|node| &node.data),
                Some(NodeKind::Entity {
                    entity: EntityKind::Camera,
                    ..
                })
            )
        })
        .copied()
        .unwrap();
    match &view.node(camera).unwrap().data {
        NodeKind::Entity {
            pose: Some(pose), ..
        } => Mat4(pose.start),
        NodeKind::Entity { pose: None, .. } => inverse(evaluated.nodes[&camera].start).unwrap(),
        other => panic!("expected a camera, got {other:?}"),
    }
}

#[test]
fn edited_local_transforms_survive_export_and_reimport() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    std::fs::write(
        &source,
        r#"LookAt 0 2 8  0 0 0  0 1 0
Camera "perspective"
WorldBegin
AttributeBegin
Translate 5 0 0
Rotate 30 0 1 0
Shape "sphere" "float radius" [1]
Shape "sphere" "float radius" [2]
AttributeBegin
Identity
Translate 0 1 0
Shape "sphere" "float radius" [3]
AttributeEnd
AttributeEnd
"#,
    )
    .unwrap();
    let (store, mut session) = import_project(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let (one, three) = (shape_named(&session, 1.), shape_named(&session, 3.));
    session
        .apply(
            &session.revision(),
            &[
                Edit::SetLocalTransform {
                    node: one,
                    value: LocalTransform::Trs(Trs {
                        translation: [0.25, -1., 2.],
                        rotation_euler_degrees: [15., -40., 120.],
                        scale: [-1., 0.5, 2.],
                    }),
                },
                Edit::SetLocalTransform {
                    node: three,
                    value: LocalTransform::Matrix(
                        Mat4::translation(-3., 0., 1.)
                            .compose(Mat4::rotation(60., 0., 0., 1.))
                            .0,
                    ),
                },
            ],
        )
        .unwrap();
    let edited = shape_worlds(&session);
    let exported = export_project(
        &session.snapshot(),
        &store,
        &temp.path().join("bundle"),
        || false,
    )
    .unwrap();
    let text = std::fs::read_to_string(&exported).unwrap();
    // The import resolved the fixture's `Identity`: its tail moved to its
    // own world-direct group, so editing three's local transform adds an
    // ordinary dedicated group holding a ConcatTransform. The only bracketed
    // matrices are that concat and the selected camera's inverse pose.
    assert_eq!(text.matches("Identity").count(), 0, "{text}");
    assert_eq!(text.matches("ConcatTransform").count(), 1, "{text}");
    assert_eq!(text.matches("Transform [").count(), 2, "{text}");
    assert!(
        !text.contains("ActiveTransform"),
        "a static camera needs no sample split: {text}"
    );
    assert!(
        text.contains(
            "ConcatTransform [ 0.5000000000000001 0.8660254037844386 0 0 -0.8660254037844386 0.5000000000000001 0 0 0 0 1 0 -3 0 1 1 ]"
        ),
        "{text}"
    );
    let (_, reimported) = import_project(
        &exported,
        &temp.path().join("again"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let again = shape_worlds(&reimported);
    assert_eq!(edited.len(), 3);
    for (a, b) in edited.iter().zip(&again) {
        assert!(approx_eq(a, b), "{a:?} != {b:?}\n{text}");
    }
}

/// A camera imports with a pose; exporting it and reimporting
/// keeps the placement. The session camera edit — writing `pose` through
/// `SetLocalTransform` — lands with the session change, where this test
/// grows its edit back.
#[test]
fn an_imported_camera_pose_survives_export_and_reimport() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("camera.pbrt");
    std::fs::write(
        &source,
        r#"LookAt 0 2 8  0 0 0  0 1 0
Film "rgb" "integer xresolution" [32] "integer yresolution" [24]
Camera "perspective"
WorldBegin
Shape "sphere"
"#,
    )
    .unwrap();
    let (store, session) = import_project(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let expected = camera_pose(&session);
    let exported = export_project(
        &session.snapshot(),
        &store,
        &temp.path().join("bundle"),
        || false,
    )
    .unwrap();
    let text = std::fs::read_to_string(&exported).unwrap();
    // The selected camera writes its inverse pose as a single `Transform`.
    assert_eq!(text.matches("Transform [").count(), 1, "{text}");
    let (_, reimported) = import_project(
        &exported,
        &temp.path().join("again"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    assert!(approx_eq(&expected, &camera_pose(&reimported)), "{text}");
}

/// The edit form that returned with the Commit-4 session change: writing
/// `pose` through `SetLocalTransform` exports as the edited camera's
/// inverse pose and reimports to the same camera-to-world placement.
#[test]
fn a_session_edited_camera_pose_exports_and_reimports() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("camera.pbrt");
    std::fs::write(
        &source,
        r#"LookAt 0 2 8  0 0 0  0 1 0
Film "rgb" "integer xresolution" [32] "integer yresolution" [24]
Camera "perspective"
WorldBegin
Shape "sphere"
"#,
    )
    .unwrap();
    let (store, mut session) = import_project(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let snapshot = session.snapshot();
    let view = snapshot.scene_view().unwrap();
    let camera = view
        .scene
        .options
        .iter()
        .copied()
        .find(|id| {
            matches!(
                view.node(*id).map(|node| &node.data),
                Some(NodeKind::Entity {
                    entity: EntityKind::Camera,
                    ..
                })
            )
        })
        .expect("the imported camera");
    let edited = Mat4::translation(-1., 0.5, 4.)
        .compose(Mat4::rotation(-35., 0., 1., 0.))
        .compose(Mat4::scale(2., 2., 2.));
    session
        .apply(
            &session.revision(),
            &[Edit::SetLocalTransform {
                node: camera,
                value: LocalTransform::Matrix(edited.0),
            }],
        )
        .unwrap();
    assert!(
        approx_eq(&camera_pose(&session), &edited),
        "the edit is the pose"
    );
    let exported = export_project(
        &session.snapshot(),
        &store,
        &temp.path().join("bundle"),
        || false,
    )
    .unwrap();
    let text = std::fs::read_to_string(&exported).unwrap();
    // One static pose: a single inverse-pose Transform, no sample split.
    assert_eq!(text.matches("Transform [").count(), 1, "{text}");
    assert!(
        !text.contains("ActiveTransform") && !text.contains("LookAt"),
        "{text}"
    );
    // The written matrix is the inverse pose in model order: the file
    // spells bracketed matrices column-major relative to the model
    // the same decoding the preview scanner applies.
    let printed = text
        .split("Transform [")
        .nth(1)
        .and_then(|rest| rest.split_once(']').map(|(numbers, _)| numbers))
        .expect("a bracketed matrix");
    let written: Vec<f64> = printed
        .split_whitespace()
        .map(|number| number.parse().expect("a number"))
        .collect();
    assert_eq!(written.len(), 16, "{text}");
    let mut model = [0f64; 16];
    for row in 0..4 {
        for col in 0..4 {
            model[row * 4 + col] = written[col * 4 + row];
        }
    }
    let expected_printed = inverse(edited).unwrap();
    for (index, (left, right)) in model.iter().zip(expected_printed.0).enumerate() {
        assert!(
            (left - right).abs() <= 1e-12 * right.abs().max(1.),
            "the export is the inverse of the edited pose [{index}]: {text}"
        );
    }
    let (_, reimported) = import_project(
        &exported,
        &temp.path().join("again"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    assert!(approx_eq(&edited, &camera_pose(&reimported)), "{text}");
}
