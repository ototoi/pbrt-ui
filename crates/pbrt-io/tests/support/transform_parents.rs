//! Shared helpers for transform-parent resolution tests:
//! the unresolved/resolved `Case` pair, model walking, and the
//! equivalence and nothing-remains comparisons.
use pbrt_io::{ImportOptions, parse_unresolved};
use project::{
    ActiveSamples, EntityKind, EvaluatedScene, ImportDiagnostic, Node, NodeId, NodeKind,
    ProjectDocument, SceneId, ScopeKind, StateOperation, TransformOperation, evaluate,
    resolve_transform_parents,
};
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Clone, Copy, PartialEq)]
pub enum Tolerance {
    Exact,
    Baked,
}

pub const BAKED_EPSILON: f64 = 1e-12;

pub struct Case {
    pub before: ProjectDocument,
    pub after: ProjectDocument,
    pub scene: SceneId,
    pub diagnostics: Vec<ImportDiagnostic>,
}

pub fn resolve_case(source: &Path) -> Case {
    let before = parse_unresolved(source, &ImportOptions::default()).unwrap();
    let scene = before.default_scene.expect("import produced a scene");
    let mut after = before.clone();
    let diagnostics = resolve_transform_parents(&mut after, scene).unwrap();
    Case {
        before,
        after,
        scene,
        diagnostics,
    }
}

pub fn case(body: &str) -> Case {
    let text = format!(
        "Camera \"perspective\" \"float fov\" [40]\n\
         Film \"rgb\" \"integer xresolution\" [320] \"integer yresolution\" [240]\n\
         WorldBegin\n{body}\nWorldEnd\n"
    );
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    std::fs::write(&source, &text).unwrap();
    resolve_case(&source)
}

// -- model walking -----------------------------------------------------------

pub fn scene_doc(document: &ProjectDocument, scene: SceneId) -> &project::SceneDocument {
    document.scene(scene).expect("scene exists")
}

pub fn node(document: &ProjectDocument, scene: SceneId, id: NodeId) -> &Node {
    scene_doc(document, scene)
        .nodes
        .get(&id)
        .or_else(|| document.definition_nodes.get(&id))
        .expect("node exists in document")
}

pub fn children(document: &ProjectDocument, scene: SceneId, id: NodeId) -> Vec<NodeId> {
    match &node(document, scene, id).data {
        NodeKind::Scope { children: ids, .. } => ids.clone(),
        _ => Vec::new(),
    }
}

pub fn world(document: &ProjectDocument, scene: SceneId) -> NodeId {
    scene_doc(document, scene).world
}

pub fn world_children(document: &ProjectDocument, scene: SceneId) -> Vec<NodeId> {
    children(document, scene, world(document, scene))
}

/// The World tree and the object-definition bodies, in preorder.
pub fn trees(document: &ProjectDocument, scene: SceneId) -> Vec<NodeId> {
    let mut stack = vec![world(document, scene)];
    stack.extend(definition_roots(document).into_iter().rev());
    let mut out = Vec::new();
    while let Some(id) = stack.pop() {
        out.push(id);
        stack.extend(children(document, scene, id).into_iter().rev());
    }
    out
}

pub fn definition_roots(document: &ProjectDocument) -> Vec<NodeId> {
    document
        .resources
        .values()
        .filter(|r| r.kind == project::ResourceKind::ObjectDefinition)
        .filter_map(|r| r.definition_scope)
        .collect()
}

pub fn kind(document: &ProjectDocument, scene: SceneId, id: NodeId) -> &project::NodeKind {
    &node(document, scene, id).data
}

pub fn label(document: &ProjectDocument, scene: SceneId, id: NodeId) -> &str {
    &node(document, scene, id).label
}

pub fn labels_of<'a>(
    document: &'a ProjectDocument,
    scene: SceneId,
    among: &'a [NodeId],
) -> Vec<&'a str> {
    among.iter().map(|id| label(document, scene, *id)).collect()
}

pub fn node_at(
    document: &ProjectDocument,
    scene: SceneId,
    among: &[NodeId],
    wanted: &str,
) -> NodeId {
    *among
        .iter()
        .find(|id| label(document, scene, **id) == wanted)
        .unwrap_or_else(|| panic!("no node labelled {wanted}"))
}

pub fn shape_by_radius(document: &ProjectDocument, scene: SceneId, radius: f64) -> NodeId {
    for id in trees(document, scene) {
        if let NodeKind::Entity {
            entity: EntityKind::Shape,
            parameters,
            ..
        } = kind(document, scene, id)
        {
            let found = parameters.iter().any(|p| {
                p.name == "radius"
                    && matches!(&p.values, project::ParameterValues::Floats(values)
                        if values.first() == Some(&project::FloatValue::Finite(radius)))
            });
            if found {
                return id;
            }
        }
    }
    panic!("no shape with radius {radius}")
}

pub fn resource_id(document: &ProjectDocument, label: &str) -> project::ResourceId {
    document
        .resources
        .values()
        .find(|r| r.label == label)
        .unwrap_or_else(|| panic!("no resource labelled {label}"))
        .id
}

pub fn is_reset(kind: &project::NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::State {
            operation: StateOperation::Transform(
                TransformOperation::Identity | TransformOperation::Replace(_)
            ) | StateOperation::DefineCoordinates(_)
                | StateOperation::UseCoordinates(_)
        }
    )
}

pub fn state_node_at(document: &ProjectDocument, scene: SceneId, id: NodeId) -> &StateOperation {
    match kind(document, scene, id) {
        NodeKind::State { operation } => operation,
        other => panic!("expected a state node, got {other:?}"),
    }
}

pub fn concat_node(document: &ProjectDocument, scene: SceneId, id: NodeId) {
    match kind(document, scene, id) {
        NodeKind::State {
            operation: StateOperation::Transform(TransformOperation::Concat(_)),
        } => assert_eq!(label(document, scene, id), "ConcatTransform"),
        other => panic!("expected a concat, got {other:?}"),
    }
}

pub fn concat_matrix(document: &ProjectDocument, scene: SceneId, id: NodeId) -> [f64; 16] {
    match kind(document, scene, id) {
        NodeKind::State {
            operation: StateOperation::Transform(TransformOperation::Concat(m)),
        } => *m,
        other => panic!("expected a concat, got {other:?}"),
    }
}

// -- equivalence -------------------------------------------------------------

pub fn entities(document: &ProjectDocument, scene: SceneId) -> BTreeSet<NodeId> {
    trees(document, scene)
        .into_iter()
        .filter(|id| {
            matches!(
                kind(document, scene, *id),
                NodeKind::Entity {
                    entity: EntityKind::Shape | EntityKind::Light,
                    ..
                } | NodeKind::Instance { .. }
            )
        })
        .collect()
}

pub fn cameras(document: &ProjectDocument, scene: SceneId) -> BTreeSet<NodeId> {
    scene_doc(document, scene)
        .options
        .iter()
        .copied()
        .filter(|id| {
            matches!(
                kind(document, scene, *id),
                NodeKind::Entity {
                    entity: EntityKind::Camera,
                    ..
                }
            )
        })
        .collect()
}

/// Area-light state compared by implementation and parameters, not node id.
pub fn area_signature(
    document: &ProjectDocument,
    scene: SceneId,
    state: &project::GraphicsState,
) -> Option<(String, Vec<project::Parameter>)> {
    state
        .area_light
        .map(|id| match &node(document, scene, id).data {
            NodeKind::Entity {
                entity: EntityKind::AreaLight,
                implementation,
                parameters,
                ..
            } => (implementation.clone(), parameters.clone()),
            other => panic!("area light state references {other:?}"),
        })
}

pub fn matrix_close(a: &[f64; 16], b: &[f64; 16], tolerance: Tolerance, what: &str) {
    match tolerance {
        Tolerance::Exact => assert_eq!(a, b, "{what}"),
        Tolerance::Baked => {
            for (index, (left, right)) in a.iter().zip(b).enumerate() {
                let scale = right.abs().max(1.0);
                assert!(
                    (left - right).abs() <= BAKED_EPSILON * scale,
                    "{what}[{index}]: {left} vs {right}"
                );
            }
        }
    }
}

pub fn camera_to_world(
    document: &ProjectDocument,
    scene: SceneId,
    evaluated: &EvaluatedScene,
    camera: NodeId,
) -> ([f64; 16], [f64; 16]) {
    // The pose is the value a resolved camera carries; a pose-less
    // camera (the unresolved document) derives it from the Options
    // transform stream instead.
    if let NodeKind::Entity {
        pose: Some(pose), ..
    } = kind(document, scene, camera)
    {
        return (pose.start, pose.end);
    }
    let state = evaluated
        .nodes
        .get(&camera)
        .expect("camera state is evaluated");
    (
        project::math::inverse(state.start)
            .expect("camera transform is invertible")
            .0,
        project::math::inverse(state.end)
            .expect("camera transform is invertible")
            .0,
    )
}

pub fn assert_equivalent(c: &Case, tolerance: Tolerance) {
    let before = evaluate(&c.before.scene_view(c.scene).unwrap()).unwrap();
    let after = evaluate(&c.after.scene_view(c.scene).unwrap()).unwrap();
    assert_eq!(
        entities(&c.before, c.scene),
        entities(&c.after, c.scene),
        "the set of rendered entities changed"
    );
    assert_eq!(
        cameras(&c.before, c.scene),
        cameras(&c.after, c.scene),
        "the set of cameras changed"
    );
    for id in entities(&c.before, c.scene) {
        let b = before.nodes.get(&id).expect("before: entity is evaluated");
        let a = after.nodes.get(&id).expect("after: entity is evaluated");
        matrix_close(&b.start.0, &a.start.0, tolerance, "entity world start");
        matrix_close(&b.end.0, &a.end.0, tolerance, "entity world end");
        assert_eq!(b.active, a.active, "entity active samples");
        assert_eq!(b.times, a.times, "entity transform times");
        assert_eq!(b.material, a.material, "entity material binding");
        assert_eq!(b.inside_medium, a.inside_medium, "entity inside medium");
        assert_eq!(b.outside_medium, a.outside_medium, "entity outside medium");
        assert_eq!(
            b.reverse_orientation, a.reverse_orientation,
            "entity reverse orientation"
        );
        assert_eq!(b.color_space, a.color_space, "entity color space");
        assert_eq!(
            area_signature(&c.before, c.scene, b),
            area_signature(&c.after, c.scene, a),
            "entity area light"
        );
    }
    for id in cameras(&c.before, c.scene) {
        let b = before.nodes.get(&id).expect("before: camera is evaluated");
        let a = after.nodes.get(&id).expect("after: camera is evaluated");
        assert_eq!(b.times, a.times, "camera transform times");
        let (bs, be) = camera_to_world(&c.before, c.scene, &before, id);
        let (as_, ae) = camera_to_world(&c.after, c.scene, &after, id);
        matrix_close(&bs, &as_, tolerance, "camera to world start");
        matrix_close(&be, &ae, tolerance, "camera to world end");
    }
}

/// No reset or coordinate states and no Transform
/// scopes in the World tree or the definitions.
pub fn assert_no_resets(c: &Case) {
    for id in trees(&c.after, c.scene) {
        let kind = kind(&c.after, c.scene, id);
        assert!(
            !is_reset(kind),
            "reset state remains: {}",
            label(&c.after, c.scene, id)
        );
        assert!(
            !matches!(
                kind,
                NodeKind::Scope {
                    scope: ScopeKind::Transform,
                    ..
                }
            ),
            "Transform scope remains: {}",
            label(&c.after, c.scene, id)
        );
    }
    // The Options list keeps no transform stream at all.
    for id in scene_doc(&c.after, c.scene).options.clone() {
        assert!(
            !matches!(
                kind(&c.after, c.scene, id),
                NodeKind::State {
                    operation: StateOperation::Transform(_)
                        | StateOperation::ActiveTransform(_)
                        | StateOperation::DefineCoordinates(_)
                        | StateOperation::UseCoordinates(_)
                }
            ),
            "an Options transform state remains: {}",
            label(&c.after, c.scene, id)
        );
    }
    // Every imported camera carries its pose as a value.
    for id in cameras(&c.after, c.scene) {
        match kind(&c.after, c.scene, id) {
            NodeKind::Entity { pose: Some(_), .. } => {}
            other => panic!("a camera without a pose remains: {other:?}"),
        }
    }
}

pub fn assert_resolved_exact(c: &Case) {
    c.before.validate().expect("unresolved document validates");
    assert_equivalent(c, Tolerance::Exact);
    assert_no_resets(c);
    c.after.validate().expect("resolved document validates");
}

pub fn assert_resolved_baked(c: &Case) {
    c.before.validate().expect("unresolved document validates");
    assert_equivalent(c, Tolerance::Baked);
    assert_no_resets(c);
    c.after.validate().expect("resolved document validates");
}

// -- shared fixture pieces ----------------------------------------------------

/// Column-vector translation stored in the model.
pub fn translate(x: f64, y: f64, z: f64) -> [f64; 16] {
    [
        1.0, 0.0, 0.0, x, 0.0, 1.0, 0.0, y, 0.0, 0.0, 1.0, z, 0.0, 0.0, 0.0, 1.0,
    ]
}

pub fn rotation(degrees: f64) -> [f64; 16] {
    project::math::Mat4::rotation(degrees, 0.0, 1.0, 0.0).0
}

/// The group head shared by the split-move fixtures: a translated blocker
/// shape, then the reset.
pub const SPLIT_BODY: &str = "Translate 1 2 3\nShape \"sphere\" \"float radius\" [0.25]\n";

/// The test's `Transform` matrix: a translation of 2, 3, 4 in the model's
/// row-major column-vector order.
pub const M: [f64; 16] = [
    1.0, 0.0, 0.0, 2.0, 0.0, 1.0, 0.0, 3.0, 0.0, 0.0, 1.0, 4.0, 0.0, 0.0, 0.0, 1.0,
];
/// The lexical form the importer reads as `M`.
pub const M_PBRT: &str = "Transform [ 1 0 0 0  0 1 0 0  0 0 1 0  2 3 4 1 ]";

/// The reset rewrite shared by in-place fixtures: read `id` as a reset in the
/// before document, expect a relabelled concat with `M` in the after one.
pub fn assert_in_place_concat(c: &Case, id: NodeId) {
    assert!(is_reset(kind(&c.before, c.scene, id)));
    concat_node(&c.after, c.scene, id);
    assert_eq!(concat_matrix(&c.after, c.scene, id), M);
}

pub fn camera_case(camera_body: &str, body: &str) -> Case {
    let text = format!(
        "{camera_body}\nCamera \"perspective\" \"float fov\" [40]\n\
         Film \"rgb\" \"integer xresolution\" [320] \"integer yresolution\" [240]\n\
         WorldBegin\n{body}\nWorldEnd\n"
    );
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    std::fs::write(&source, &text).unwrap();
    resolve_case(&source)
}
pub fn options_labels(document: &ProjectDocument, scene: SceneId) -> Vec<String> {
    scene_doc(document, scene)
        .options
        .iter()
        .map(|id| label(document, scene, *id).to_string())
        .collect()
}
pub fn pose_of(document: &ProjectDocument, scene: SceneId, camera: NodeId) -> project::CameraPose {
    match kind(document, scene, camera) {
        NodeKind::Entity {
            pose: Some(pose), ..
        } => *pose,
        other => panic!("expected a posed camera, got {other:?}"),
    }
}
pub fn parent_camera_of(
    document: &ProjectDocument,
    scene: SceneId,
    id: NodeId,
) -> Option<project::NodeId> {
    match kind(document, scene, id) {
        NodeKind::Scope { parent_camera, .. } => *parent_camera,
        other => panic!("expected a scope, got {other:?}"),
    }
}
pub fn options_cameras(document: &ProjectDocument, scene: SceneId) -> Vec<NodeId> {
    scene_doc(document, scene)
        .options
        .iter()
        .copied()
        .filter(|id| {
            matches!(
                kind(document, scene, *id),
                NodeKind::Entity {
                    entity: EntityKind::Camera,
                    ..
                }
            )
        })
        .collect()
}

// -- equivalence across separate imports ------------------------------------
//
// Two imports of the same text assign fresh node and resource ids, so the
// id-matched comparison cannot apply between them. Entities and cameras
// group by content (kind, implementation, parameters / resource contents),
// then compare as multisets ordered by their world matrices.

/// Parameters as text, with asset and resource ids replaced by their
/// content: a reimport stores the same content under new ids.
fn parameter_key(document: &ProjectDocument, parameters: &[project::Parameter]) -> String {
    let mut text = serde_json::to_string(parameters).expect("parameters serialize");
    for (id, asset) in &document.assets {
        text = text.replace(&id.0.to_string(), &asset.sha256);
    }
    // Referenced resources (textures) likewise compare by content.
    for id in document.resources.keys() {
        let key = id.0.to_string();
        if text.contains(&key) {
            text = text.replace(&key, &resource_key(document, *id));
        }
    }
    text
}

fn resource_key(document: &ProjectDocument, resource: project::ResourceId) -> String {
    let found = document.resources.get(&resource).expect("resource exists");
    format!(
        "{}|{}",
        found.implementation,
        parameter_key(document, &found.parameters)
    )
}

fn material_key(binding: &project::MaterialBinding, document: &ProjectDocument) -> String {
    match binding {
        project::MaterialBinding::Default => "default".into(),
        project::MaterialBinding::None => "none".into(),
        project::MaterialBinding::Resource(id) => format!("r|{}", resource_key(document, *id)),
    }
}

fn area_light_key(
    document: &ProjectDocument,
    scene: SceneId,
    state: &project::GraphicsState,
) -> Option<String> {
    state.area_light.map(|id| match kind(document, scene, id) {
        NodeKind::Entity {
            implementation,
            parameters,
            ..
        } => format!("{implementation}|{}", parameter_key(document, parameters)),
        other => panic!("area light state references {other:?}"),
    })
}

struct EntityCard {
    key: String,
    start: [f64; 16],
    end: [f64; 16],
    active: ActiveSamples,
    times: [f64; 2],
    material: String,
    inside_medium: Option<String>,
    outside_medium: Option<String>,
    reverse_orientation: bool,
    color_space: String,
    area_light: Option<String>,
}

fn entity_cards(
    document: &ProjectDocument,
    scene: SceneId,
    evaluated: &EvaluatedScene,
) -> Vec<EntityCard> {
    trees(document, scene)
        .into_iter()
        .filter_map(|id| {
            let key = match kind(document, scene, id) {
                NodeKind::Entity {
                    entity: EntityKind::Shape,
                    implementation,
                    parameters,
                    ..
                } => format!(
                    "shape|{implementation}|{}",
                    parameter_key(document, parameters)
                ),
                NodeKind::Entity {
                    entity: EntityKind::Light,
                    implementation,
                    parameters,
                    ..
                } => format!(
                    "light|{implementation}|{}",
                    parameter_key(document, parameters)
                ),
                NodeKind::Instance { definition } => {
                    format!("instance|{}", resource_key(document, *definition))
                }
                _ => return None,
            };
            let state = evaluated.nodes.get(&id).expect("entity is evaluated");
            Some(EntityCard {
                key,
                start: state.start.0,
                end: state.end.0,
                active: state.active,
                times: state.times,
                material: material_key(&state.material, document),
                inside_medium: state.inside_medium.map(|id| resource_key(document, id)),
                outside_medium: state.outside_medium.map(|id| resource_key(document, id)),
                reverse_orientation: state.reverse_orientation,
                color_space: state.color_space.clone(),
                area_light: area_light_key(document, scene, state),
            })
        })
        .collect()
}

struct CameraCard {
    start: [f64; 16],
    end: [f64; 16],
    times: [f64; 2],
    key: String,
}

fn camera_cards(
    document: &ProjectDocument,
    scene: SceneId,
    evaluated: &EvaluatedScene,
) -> Vec<CameraCard> {
    options_cameras(document, scene)
        .into_iter()
        .map(|id| {
            let (start, end) = camera_to_world(document, scene, evaluated, id);
            let times = evaluated.nodes.get(&id).expect("camera is evaluated").times;
            let key = match kind(document, scene, id) {
                NodeKind::Entity {
                    implementation,
                    parameters,
                    ..
                } => format!(
                    "camera|{implementation}|{}",
                    parameter_key(document, parameters)
                ),
                other => panic!("expected a camera, got {other:?}"),
            };
            CameraCard {
                start,
                end,
                times,
                key,
            }
        })
        .collect()
}

fn matrix_total_order(a: &[f64; 16], b: &[f64; 16]) -> std::cmp::Ordering {
    for (left, right) in a.iter().zip(b) {
        match left.total_cmp(right) {
            std::cmp::Ordering::Equal => {}
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

/// Equivalence between two documents from separate imports (fresh ids on
/// both sides): every entity and every camera keeps its evaluated meaning
/// within the baked tolerance (1e-12 relative).
pub fn assert_documents_equivalent(before: &ProjectDocument, after: &ProjectDocument) {
    let before_scene = before.default_scene.expect("document has a scene");
    let after_scene = after.default_scene.expect("document has a scene");
    let before_evaluated = evaluate(&before.scene_view(before_scene).unwrap()).unwrap();
    let after_evaluated = evaluate(&after.scene_view(after_scene).unwrap()).unwrap();

    let mut before_cards = entity_cards(before, before_scene, &before_evaluated);
    let mut after_cards = entity_cards(after, after_scene, &after_evaluated);
    before_cards.sort_by(|a, b| {
        a.key
            .cmp(&b.key)
            .then_with(|| matrix_total_order(&a.start, &b.start))
    });
    after_cards.sort_by(|a, b| {
        a.key
            .cmp(&b.key)
            .then_with(|| matrix_total_order(&a.start, &b.start))
    });
    assert_eq!(
        before_cards.len(),
        after_cards.len(),
        "the number of rendered entities changed"
    );
    for (b, a) in before_cards.iter().zip(&after_cards) {
        assert_eq!(b.key, a.key, "entity content changed");
        matrix_close(&b.start, &a.start, Tolerance::Baked, "entity world start");
        matrix_close(&b.end, &a.end, Tolerance::Baked, "entity world end");
        assert_eq!(b.active, a.active, "entity active samples");
        assert_eq!(b.times, a.times, "entity transform times");
        assert_eq!(b.material, a.material, "entity material content");
        assert_eq!(
            b.inside_medium, a.inside_medium,
            "entity inside medium content"
        );
        assert_eq!(
            b.outside_medium, a.outside_medium,
            "entity outside medium content"
        );
        assert_eq!(
            b.reverse_orientation, a.reverse_orientation,
            "entity reverse orientation"
        );
        assert_eq!(b.color_space, a.color_space, "entity color space");
        assert_eq!(b.area_light, a.area_light, "entity area light content");
    }

    let mut before_cameras = camera_cards(before, before_scene, &before_evaluated);
    let mut after_cameras = camera_cards(after, after_scene, &after_evaluated);
    before_cameras.sort_by(|a, b| {
        a.key
            .cmp(&b.key)
            .then_with(|| matrix_total_order(&a.start, &b.start))
    });
    after_cameras.sort_by(|a, b| {
        a.key
            .cmp(&b.key)
            .then_with(|| matrix_total_order(&a.start, &b.start))
    });
    assert_eq!(
        before_cameras.len(),
        after_cameras.len(),
        "the number of cameras changed"
    );
    for (b, a) in before_cameras.iter().zip(&after_cameras) {
        assert_eq!(b.key, a.key, "camera content changed");
        assert_eq!(b.times, a.times, "camera transform times");
        matrix_close(
            &b.start,
            &a.start,
            Tolerance::Baked,
            "camera to world start",
        );
        matrix_close(&b.end, &a.end, Tolerance::Baked, "camera to world end");
    }
}
