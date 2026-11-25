//! Import folds a `LightSource` point, spot, or distant light's `from`
//! into the node's own transform run. pbrt-v4 composes `from` into the
//! transform current at the LightSource (lights.cpp: a point renders from
//! `CTM · Translate(from)`, a spot adds its LookAt-like frame toward `to`,
//! a distant uses the direction `to - from`), so a `from` off the origin
//! draws the light's wire-frame away from its Move gizmo. After the fold
//! the light's local run ends in `Translate(from)` — the run the
//! Inspector's transform and the Move gizmo edit — its parameters are
//! re-centred at the origin (`from` back to the default, `to` moved to
//! `to - from` for the two that read it), and the rendered light is
//! identical. Lights with an animated transform and lights inside object
//! definitions stay unchanged, and already-imported projects are not
//! migrated.
mod support;

use pbrt_io::{ImportOptions, export_project, import_project, parse_unresolved};
use project::{
    EntityKind, FloatValue, NodeId, NodeKind, Parameter, ParameterValues, ProjectDocument, SceneId,
    ScopeKind, StateOperation, TransformOperation, evaluate, fold_light_from_transforms,
    math::Mat4, resolve_transform_parents,
};
use std::path::{Path, PathBuf};
use support::signature::{difference, signature};

const CAMERA: &str = "Camera \"perspective\" \"float fov\" [40]\nFilm \"rgb\" \"integer xresolution\" [16] \"integer yresolution\" [16]\n";

/// The world body wrapped in a complete scene, imported through the real
/// import path (parse, resolve, fold, validate, publish).
fn imported(home: &Path, name: &str, world: &str) -> project::ProjectSession {
    let source = home.join(format!("{name}.pbrt"));
    std::fs::write(&source, format!("{CAMERA}WorldBegin\n{world}\nWorldEnd\n")).unwrap();
    let (_store, session) =
        import_project(&source, &home.join(name), &ImportOptions::default(), || {
            false
        })
        .unwrap_or_else(|error| panic!("import {name}: {error}"));
    session
}

fn scene(document: &ProjectDocument) -> SceneId {
    document.default_scene.expect("the import produced a scene")
}

fn nodes(
    document: &ProjectDocument,
    scene: SceneId,
) -> &std::collections::BTreeMap<NodeId, project::Node> {
    &document.scene(scene).expect("scene exists").nodes
}

/// The light entities of the scene tree (never the definition bodies).
fn scene_lights(document: &ProjectDocument) -> Vec<(NodeId, &str, &[Parameter])> {
    nodes(document, scene(document))
        .values()
        .filter(|node| {
            matches!(
                node.data,
                NodeKind::Entity {
                    entity: EntityKind::Light,
                    ..
                }
            )
        })
        .map(|node| match &node.data {
            NodeKind::Entity {
                implementation,
                parameters,
                ..
            } => (node.id, implementation.as_str(), parameters.as_slice()),
            _ => unreachable!("filtered to lights above"),
        })
        .collect()
}

fn floats(parameters: &[Parameter], name: &str) -> Option<Vec<f64>> {
    let row = parameters
        .iter()
        .find(|parameter| parameter.name.eq_ignore_ascii_case(name))?;
    match &row.values {
        ParameterValues::Floats(values) => Some(
            values
                .iter()
                .filter_map(|value| match value {
                    FloatValue::Finite(value) => Some(*value),
                    _ => None,
                })
                .collect(),
        ),
        _ => None,
    }
}

fn point3(parameters: &[Parameter], name: &str) -> Option<[f64; 3]> {
    floats(parameters, name).and_then(|values| values.try_into().ok())
}

/// The light's own local-transform run after the fold: its
/// dedicated group and the run's state nodes before the light. `None`
/// when the light was left unfolded.
fn folded_run(
    document: &ProjectDocument,
    scene: SceneId,
    light: NodeId,
) -> Option<(NodeId, Vec<NodeId>)> {
    let group = nodes(document, scene)
        .values()
        .find(|node| {
            matches!(
                &node.data,
                NodeKind::Scope { children, owned_object, .. }
                    if *owned_object == Some(light) && children.contains(&light)
            )
        })?
        .id;
    let NodeKind::Scope { children, .. } = &nodes(document, scene)[&group].data else {
        return None;
    };
    let run = children
        .iter()
        .take_while(|child| **child != light)
        .cloned()
        .collect();
    Some((group, run))
}

/// The one Translate state a folded run ends in.
fn folded_translate(document: &ProjectDocument, scene: SceneId, light: NodeId) -> Option<[f64; 3]> {
    let (_, run) = folded_run(document, scene, light)?;
    match &nodes(document, scene)[&run[0]].data {
        NodeKind::State {
            operation: StateOperation::Transform(TransformOperation::Translate(v)),
        } => Some(*v),
        other => panic!("a folded run starts with a Translate, got {other:?}"),
    }
}

fn near3(actual: Option<[f64; 3]>, expected: [f64; 3], what: &str) {
    let actual = actual.unwrap_or_else(|| panic!("{what}: no row"));
    for (a, b) in actual.iter().zip(expected.iter()) {
        assert!((a - b).abs() < 1e-9, "{what}: {actual:?} vs {expected:?}");
    }
}

fn near_matrix(actual: &Mat4, expected: &Mat4, what: &str) {
    for (a, b) in actual.0.iter().zip(expected.0.iter()) {
        assert!((a - b).abs() < 1e-9, "{what}: {actual:?} vs {expected:?}");
    }
}

// -- the fold takes each light's `from` into its own run --------------------------

#[test]
fn a_point_light_with_a_from_folds_into_its_own_run() {
    let home = tempfile::tempdir().unwrap();
    let session = imported(
        home.path(),
        "point-from",
        "LightSource \"point\" \"rgb I\" [10 10 10] \"point3 from\" [3 1 4]",
    );
    let document = session.document();
    let [(light, _, parameters)] = scene_lights(document)[..] else {
        panic!("one point light");
    };
    assert_eq!(
        folded_translate(document, scene(document), light),
        Some([3., 1., 4.]),
        "the node's local translation is from"
    );
    let (group, run) = folded_run(document, scene(document), light).unwrap();
    assert_eq!(run.len(), 1, "the run is its one Translate state");
    assert!(
        matches!(
            &nodes(document, scene(document))[&group].data,
            NodeKind::Scope {
                scope: ScopeKind::Attribute,
                owned_object: Some(owner),
                ..
            } if *owner == light
        ),
        "the run's group is the light's dedicated group"
    );
    // `from` is its default again — the model stores only declared rows —
    // and everything else is as declared.
    assert_eq!(point3(parameters, "from"), None, "the from row folds away");
    assert_eq!(floats(parameters, "I"), Some(vec![10., 10., 10.]));
    let evaluated = evaluate(&document.scene_view(scene(document)).unwrap()).unwrap();
    near_matrix(
        &evaluated.nodes[&light].start,
        &Mat4::translation(3., 1., 4.),
        "the world frame is Translate(from) under the identity",
    );
}

#[test]
fn a_spot_light_folds_its_from_and_moves_its_to() {
    let home = tempfile::tempdir().unwrap();
    let session = imported(
        home.path(),
        "spot-from",
        "LightSource \"spot\" \"rgb I\" [8 8 8] \"point3 from\" [3 1 4] \
         \"point3 to\" [6 5 4.5] \"float coneangle\" [50] \"float conedeltaangle\" [20]",
    );
    let document = session.document();
    let [(light, _, parameters)] = scene_lights(document)[..] else {
        panic!("one spot light");
    };
    assert_eq!(
        folded_translate(document, scene(document), light),
        Some([3., 1., 4.])
    );
    assert_eq!(point3(parameters, "from"), None);
    assert_eq!(
        point3(parameters, "to"),
        Some([3., 4., 0.5]),
        "to becomes to - from"
    );
    // The other rows ride along untouched, aliases included.
    assert_eq!(floats(parameters, "coneangle"), Some(vec![50.]));
    assert_eq!(floats(parameters, "conedeltaangle"), Some(vec![20.]));
    assert_eq!(floats(parameters, "I"), Some(vec![8., 8., 8.]));
}

#[test]
fn a_distant_light_folds_its_from_and_keeps_its_direction() {
    let home = tempfile::tempdir().unwrap();
    let session = imported(
        home.path(),
        "distant-from",
        "LightSource \"distant\" \"rgb L\" [1 1 1] \"point3 from\" [5 0 0] \"point3 to\" [0 0 0]",
    );
    let document = session.document();
    let [(light, _, parameters)] = scene_lights(document)[..] else {
        panic!("one distant light");
    };
    assert_eq!(
        folded_translate(document, scene(document), light),
        Some([5., 0., 0.])
    );
    assert_eq!(point3(parameters, "from"), None);
    assert_eq!(
        point3(parameters, "to"),
        Some([-5., 0., 0.]),
        "to becomes to - from"
    );
    // The direction the light shines along is unchanged: the world frame
    // now carries the translation, and translations never move vectors.
    let evaluated = evaluate(&document.scene_view(scene(document)).unwrap()).unwrap();
    let state = &evaluated.nodes[&light];
    let origin = state.start.transform_point([0., 0., 0.]);
    let toward = state.start.transform_point([-5., 0., 0.]);
    assert!(
        (toward[0] - origin[0] + 5.).abs() < 1e-9
            && (toward[1] - origin[1]).abs() < 1e-9
            && (toward[2] - origin[2]).abs() < 1e-9,
        "the direction to - from is unchanged, got {toward:?} - {origin:?}"
    );
}

/// A `to` value that the source did not declare becomes declared by the
/// fold — the light needs the moved row to keep its direction.
#[test]
fn an_undeclared_to_of_a_folded_light_becomes_declared() {
    let home = tempfile::tempdir().unwrap();
    let session = imported(
        home.path(),
        "spot-implicit-to",
        "LightSource \"spot\" \"point3 from\" [0 1 0] \"float coneangle\" [35]",
    );
    let document = session.document();
    let [(light, _, parameters)] = scene_lights(document)[..] else {
        panic!("one spot light");
    };
    assert!(folded_run(document, scene(document), light).is_some());
    assert_eq!(point3(parameters, "from"), None);
    assert_eq!(
        point3(parameters, "to"),
        Some([0., -1., 1.]),
        "the default to moves to to - from"
    );
}

// -- what the fold leaves alone ----------------------------------------------------

#[test]
fn a_light_with_the_default_from_stays_unchanged() {
    let home = tempfile::tempdir().unwrap();
    let session = imported(
        home.path(),
        "point-zero-from",
        "LightSource \"point\" \"rgb I\" [10 10 10] \"point3 from\" [0 0 0]\n\
         LightSource \"spot\" \"float coneangle\" [40]",
    );
    let document = session.document();
    let lights = scene_lights(document);
    assert_eq!(lights.len(), 2);
    for (light, _, parameters) in lights {
        assert!(
            folded_run(document, scene(document), light).is_none(),
            "nothing to fold"
        );
        assert_eq!(
            point3(parameters, "from").unwrap_or([0., 0., 0.]),
            [0., 0., 0.]
        );
    }
}

#[test]
fn an_animated_transform_light_stays_unchanged() {
    let home = tempfile::tempdir().unwrap();
    let source = home.path().join("animated.pbrt");
    std::fs::write(
        &source,
        format!(
            "{CAMERA}TransformTimes 0 10\nWorldBegin\n\
             ActiveTransform StartTime\nTranslate 1 0 0\n\
             ActiveTransform EndTime\nTranslate 2 0 0\n\
             ActiveTransform All\n\
             LightSource \"point\" \"rgb I\" [10 10 10] \"point3 from\" [0 3 0]\n\
             WorldEnd\n"
        ),
    )
    .unwrap();
    let (_store, session) = import_project(
        &source,
        &home.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let document = session.document();
    let [(light, _, parameters)] = scene_lights(document)[..] else {
        panic!("one point light");
    };
    assert!(
        folded_run(document, scene(document), light).is_none(),
        "an animated transform cannot take a static fold"
    );
    assert_eq!(
        point3(parameters, "from"),
        Some([0., 3., 0.]),
        "the from row stays"
    );
}

#[test]
fn a_light_inside_an_object_definition_stays_unchanged() {
    let home = tempfile::tempdir().unwrap();
    let session = imported(
        home.path(),
        "object-lamp",
        "ObjectBegin \"lamp\"\n\
         LightSource \"point\" \"rgb I\" [10 10 10] \"point3 from\" [4 0 0]\n\
         ObjectEnd\n\
         ObjectInstance \"lamp\"",
    );
    let document = session.document();
    assert!(
        scene_lights(document).is_empty(),
        "the definition's light is not a scene node"
    );
    let light = document
        .definition_nodes
        .values()
        .find(|node| {
            matches!(
                node.data,
                NodeKind::Entity {
                    entity: EntityKind::Light,
                    ..
                }
            )
        })
        .expect("the object definition holds the light");
    let NodeKind::Entity { parameters, .. } = &light.data else {
        unreachable!("filtered above");
    };
    assert_eq!(point3(parameters, "from"), Some([4., 0., 0.]));
}

// -- equivalence, by evaluation ----------------------------------------------------

/// Parse, resolve and fold by hand, and return the pre-fold copy beside
/// the folded document: light node ids survive both stages, so every
/// light pairs with itself.
fn resolve_and_fold(source: &Path) -> (ProjectDocument, ProjectDocument, SceneId) {
    let mut after = parse_unresolved(source, &ImportOptions::default()).unwrap();
    let scene = after.default_scene.expect("the parse produced a scene");
    resolve_transform_parents(&mut after, scene).unwrap();
    let before = after.clone();
    fold_light_from_transforms(&mut after, scene).unwrap();
    (before, after, scene)
}

/// The fold keeps every light exactly where pbrt renders it —
/// for every folded light the evaluated frame is the old one with
/// `Translate(from)` appended, under parent transforms too — and leaves
/// every other evaluated node (and every unfolded light) untouched.
#[test]
fn the_fold_keeps_every_evaluated_light_exactly_where_it_was() {
    let home = tempfile::tempdir().unwrap();
    let source = home.path().join("equivalence.pbrt");
    std::fs::write(
        &source,
        format!(
            "{CAMERA}WorldBegin\n\
             AttributeBegin\nTranslate 9 0 0\nRotate 90 0 0 1\n\
             LightSource \"point\" \"rgb I\" [5 5 5] \"point3 from\" [3 1 4]\n\
             AttributeEnd\n\
             AttributeBegin\nTranslate -2 0 0\n\
             LightSource \"spot\" \"point3 from\" [0 2 0] \"point3 to\" [1 2 -1] \"float coneangle\" [35]\n\
             LightSource \"point\" \"rgb I\" [2 2 2]\n\
             AttributeEnd\n\
             WorldEnd\n"
        ),
    )
    .unwrap();
    let (before, after, scene) = resolve_and_fold(&source);
    let before_evaluated = evaluate(&before.scene_view(scene).unwrap()).unwrap();
    let after_evaluated = evaluate(&after.scene_view(scene).unwrap()).unwrap();

    let moved_to = |parameters: &[Parameter], from: [f64; 3]| -> [f64; 3] {
        let to = point3(parameters, "to").unwrap_or([0., 0., 1.]);
        [to[0] - from[0], to[1] - from[1], to[2] - from[2]]
    };

    let mut folded = 0;
    let mut untouched = 0;
    for (id, node) in nodes(&after, scene).iter() {
        let NodeKind::Entity {
            entity: EntityKind::Light,
            implementation,
            parameters,
            ..
        } = &node.data
        else {
            // Non-light nodes evaluate to the very same matrices.
            let Some(was) = before_evaluated.nodes.get(id) else {
                // A node the fold added (the wrapper and its state).
                continue;
            };
            let now = after_evaluated
                .nodes
                .get(id)
                .unwrap_or_else(|| panic!("a node lost its evaluation"));
            assert_eq!(was.start, now.start, "node {id} moved");
            assert_eq!(was.end, now.end, "node {id} moved");
            continue;
        };
        let NodeKind::Entity {
            parameters: before_parameters,
            ..
        } = &nodes(&before, scene)[id].data
        else {
            unreachable!("the light kept its id through the fold");
        };
        let before_from = point3(before_parameters, "from").unwrap_or([0., 0., 0.]);
        let (was, now) = (
            &before_evaluated.nodes[id],
            after_evaluated
                .nodes
                .get(id)
                .expect("the light is evaluated"),
        );
        if before_from == [0., 0., 0.] {
            assert!(
                folded_run(&after, scene, *id).is_none(),
                "{implementation}: nothing to fold"
            );
            assert_eq!(was.start, now.start, "{implementation}");
            assert_eq!(
                point3(parameters, "from").unwrap_or([0., 0., 0.]),
                [0., 0., 0.]
            );
            assert_eq!(
                point3(parameters, "to").unwrap_or([0., 0., 1.]),
                point3(before_parameters, "to").unwrap_or([0., 0., 1.])
            );
            untouched += 1;
            continue;
        }
        folded += 1;
        assert!(
            folded_translate(&after, scene, *id).is_some(),
            "{implementation}: the from folds into the run"
        );
        // The world frame gains exactly Translate(from) at the light's
        // own place in the composition: the position (point and spot) and
        // the direction (spot and distant) the renderer uses are
        // identical.
        let translate = Mat4::translation(before_from[0], before_from[1], before_from[2]);
        for (was_sample, now_sample) in [(was.start, now.start), (was.end, now.end)] {
            near_matrix(
                &now_sample,
                &was_sample.compose(translate),
                &format!("{implementation}'s samples"),
            );
        }
        // `to` moves for the lights that read it; a point light has no
        // `to` and keeps whatever unused row it carried.
        if matches!(implementation.as_str(), "spot" | "distant") {
            near3(
                point3(parameters, "to"),
                moved_to(before_parameters, before_from),
                &format!("{implementation}: to moves with from"),
            );
        } else {
            assert_eq!(
                point3(parameters, "to"),
                point3(before_parameters, "to"),
                "{implementation}: an unused to row rides along"
            );
        }
    }
    assert_eq!(folded, 2, "the two folded lights");
    assert_eq!(untouched, 1, "the default-from light");
}

// -- export -----------------------------------------------------------------------

/// The exported scene keeps the light's meaning — the export
/// writes the Translate ahead of the LightSource with `from` at its
/// default, so re-importing reproduces the same rendering.
#[test]
fn the_export_of_a_folded_light_reimports_to_the_same_rendering() {
    let home = tempfile::tempdir().unwrap();
    let source = home.path().join("spot-roundtrip.pbrt");
    std::fs::write(
        &source,
        format!(
            "{CAMERA}WorldBegin\nAttributeBegin\nTranslate 9 0 0\n\
             LightSource \"spot\" \"rgb I\" [8 8 8] \"point3 from\" [0 2 0] \
             \"point3 to\" [1 2 -1] \"float coneangle\" [40]\n\
             AttributeEnd\nWorldEnd\n"
        ),
    )
    .unwrap();
    let (store, session) = import_project(
        &source,
        &home.path().join("original"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let original = signature(session.document(), &store);
    let exported = export_project(
        &session.snapshot(),
        &store,
        &home.path().join("bundle"),
        || false,
    )
    .unwrap();
    let text = std::fs::read_to_string(&exported).unwrap();
    assert!(
        text.contains("Translate 0 2 0"),
        "the export carries the fold's Translate: {text}"
    );
    assert!(
        !text.contains("from"),
        "the export leaves `from` at its default: {text}"
    );
    let (reimport_store, reimport_session) = import_project(
        &exported,
        &home.path().join("reimported"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let reimported = signature(reimport_session.document(), &reimport_store);
    if let Some(place) = difference(&original, &reimported) {
        panic!("export changed the folded light's meaning at {place}");
    }
}

// -- the real-world sanity check ---------------------------------------------------

fn villa_source() -> Option<PathBuf> {
    let root = std::env::var_os("PBRT_V4_SCENES")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../pbrt-v4-scenes"));
    let path = root.join("villa/villa-lights-on.pbrt");
    path.is_file().then_some(path)
}

/// The villa-lights-on scene contains four spot lights and two point lights
/// with a `from` off the origin; each folds with its frame and its
/// parameters exactly where the evaluation says it must.
#[test]
#[ignore = "requires the pbrt-v4-scenes checkout (PBRT_V4_SCENES=<path> or ../pbrt-v4-scenes)"]
fn villa_lights_fold_where_the_evaluation_says() {
    let Some(source) = villa_source() else {
        eprintln!("skipping: no pbrt-v4-scenes checkout; set PBRT_V4_SCENES");
        return;
    };
    let (before, after, scene) = resolve_and_fold(&source);
    let before_evaluated = evaluate(&before.scene_view(scene).unwrap()).unwrap();
    let after_evaluated = evaluate(&after.scene_view(scene).unwrap()).unwrap();
    let mut folded = 0;
    let mut unfolded = 0;
    for (id, node) in nodes(&after, scene).iter() {
        let NodeKind::Entity {
            entity: EntityKind::Light,
            implementation,
            parameters,
            ..
        } = &node.data
        else {
            continue;
        };
        let NodeKind::Entity {
            parameters: before_parameters,
            ..
        } = &nodes(&before, scene)[id].data
        else {
            unreachable!("ids survive the fold");
        };
        let before_from = point3(before_parameters, "from").unwrap_or([0., 0., 0.]);
        if before_from == [0., 0., 0.] {
            assert!(folded_run(&after, scene, *id).is_none(), "{implementation}");
            unfolded += 1;
            continue;
        }
        folded += 1;
        assert!(
            folded_translate(&after, scene, *id).is_some(),
            "{implementation}"
        );
        assert_eq!(point3(parameters, "from"), None, "{implementation}");
        let (was, now) = (
            &before_evaluated.nodes[id],
            after_evaluated.nodes.get(id).expect("evaluated"),
        );
        let translate = Mat4::translation(before_from[0], before_from[1], before_from[2]);
        near_matrix(
            &now.start,
            &was.start.compose(translate),
            &format!("{implementation}'s world frame"),
        );
        if implementation == "spot" {
            let before_to = point3(before_parameters, "to").unwrap_or([0., 0., 1.]);
            near3(
                point3(parameters, "to"),
                [
                    before_to[0] - before_from[0],
                    before_to[1] - before_from[1],
                    before_to[2] - before_from[2],
                ],
                &format!("{implementation}: to moves with from"),
            );
        }
    }
    assert_eq!(folded, 6, "four spots and two point lights fold");
    assert_eq!(unfolded, 1, "the infinite light keeps its origin from");
}
