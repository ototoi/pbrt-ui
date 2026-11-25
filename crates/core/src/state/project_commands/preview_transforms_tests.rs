//! the transform-delta fast path and every fallback.
use super::*;
use project::{
    EntityKind, Node, NodeKind, Parameter, ProjectDocument, Resource, ResourceKind,
    Revision as ModelRevision, ScopeKind, StateOperation, TransformOperation,
};
use uuid::Uuid;

fn request(method: Method, params: Value) -> Request {
    Request {
        protocol: protocol::PROTOCOL_VERSION.into(),
        request_id: Uuid::new_v4().to_string(),
        method,
        params,
    }
}

fn state(label: &str, operation: StateOperation) -> Node {
    Node::new(label, NodeKind::State { operation })
}

fn shape(label: &str) -> Node {
    Node::new(
        label,
        NodeKind::Entity {
            entity: EntityKind::Shape,
            implementation: "sphere".into(),
            parameters: vec![Parameter::floats("radius", [1.])],
            pose: None,
        },
    )
}

fn entity(label: &str, entity: EntityKind, implementation: &str) -> Node {
    Node::new(
        label,
        NodeKind::Entity {
            entity,
            implementation: implementation.into(),
            parameters: Vec::new(),
            pose: None,
        },
    )
}

/// The default scene with world children (in order) and options.
fn document(world: &[&Node], options: &[&Node]) -> ProjectDocument {
    let mut document = ProjectDocument::new("P-b");
    let scene_id = document.default_scene.unwrap();
    let scene = std::sync::Arc::make_mut(document.scenes.get_mut(&scene_id).unwrap());
    let world_id = scene.world;
    {
        let NodeKind::Scope { children, .. } = &mut scene.nodes.get_mut(&world_id).unwrap().data
        else {
            unreachable!()
        };
        children.extend(world.iter().map(|node| node.id));
    }
    scene.options.extend(options.iter().map(|node| node.id));
    for node in world.iter().copied().chain(options.iter().copied()) {
        scene.nodes.insert(node.id, node.clone());
    }
    document.validate().unwrap();
    document
}

/// The fabricated-load pattern of the sibling tests: open, then hand the core
/// a finished I/O load carrying this document as an editable project.
fn load(document: ProjectDocument) -> CoreState {
    let mut core = CoreState::new(Limits::default());
    let context = core.projects.context();
    let response = core.handle(request(
        Method::ProjectOpen,
        json!({"path":crate::TEST_PROJECT,"expected_current":context}),
    ));
    assert!(response.error.is_none(), "{:?}", response.error);
    let job_id: JobId = serde_json::from_value(response.result.unwrap()["job_id"].clone()).unwrap();
    core.project_io_finished(
        job_id,
        Ok(IoOutput::Loaded {
            snapshot: ProjectSession::new(document).unwrap().snapshot(),
            store_id: Uuid::new_v4(),
            path: None,
            source_path: None,
            unregistered: vec![],
            published: false,
            report: None,
            mode: dto::ProjectMode::Project,
            asset_root: None,
            staging_owner: None,
        }),
    );
    core
}

/// Request the preview job whose success records the preview base /// diffs against (fabricated I/O like the load above returns its job id).
fn request_preview(core: &mut CoreState) -> JobId {
    let reference = core.projects.reference().unwrap();
    let response = core.handle(request(
        Method::ProjectPreview,
        json!({"project":reference,"destination":"/preview-slot"}),
    ));
    assert!(response.error.is_none(), "{:?}", response.error);
    serde_json::from_value(response.result.unwrap()["job_id"].clone()).unwrap()
}

fn record_preview_base(core: &mut CoreState) {
    let job_id = request_preview(core);
    finish_preview(core, job_id);
}

fn finish_preview(core: &mut CoreState, job_id: JobId) {
    core.project_io_finished(
        job_id,
        Ok(IoOutput::Previewed {
            path: "/preview-slot".into(),
            draws: 0,
            unsupported: 0,
            warnings: Vec::new(),
        }),
    );
}

fn apply(core: &mut CoreState, edits: Value) -> Value {
    let reference = core.projects.reference().unwrap();
    let response = core.handle(request(
        Method::ProjectApply,
        json!({"project":reference,"edits":edits}),
    ));
    assert!(response.error.is_none(), "{:?}", response.error);
    response.result.unwrap()
}

fn undo(core: &mut CoreState) -> Value {
    let reference = core.projects.reference().unwrap();
    let response = core.handle(request(Method::ProjectUndo, json!({"project":reference})));
    assert!(response.error.is_none(), "{:?}", response.error);
    response.result.unwrap()
}

fn ask(core: &mut CoreState, base_revision: &str) -> Value {
    let reference = core.projects.reference().unwrap();
    let response = core.handle(request(
        Method::ProjectPreviewTransforms,
        json!({"project":reference,"base_revision":base_revision}),
    ));
    assert!(response.error.is_none(), "{:?}", response.error);
    response.result.unwrap()
}

fn moved(shape: &Node) -> Value {
    let expected = project::math::Mat4::translation(3., 3., 3.).0;
    json!({
        "fast": true,
        "base_revision": "0",
        "revision": "1",
        "deltas": { shape.id.to_string(): [expected, expected] },
    })
}

/// A `set_transform` on a plain shape answers the draw's delta matrix,
/// keyed by the node id the preview pins draws by.
#[test]
fn a_set_transform_on_a_plain_shape_answers_the_draw_delta() {
    let translate = state(
        "Translate",
        StateOperation::Transform(TransformOperation::Translate([1., 2., 3.])),
    );
    let shape = shape("Sphere");
    let mut core = load(document(&[&translate, &shape], &[]));
    record_preview_base(&mut core);
    let applied = apply(
        &mut core,
        json!([{"kind":"set_transform","node_id":translate.id,
                "value":{"kind":"translate","value":[4.,5.,6.]}}]),
    );
    assert_eq!(
        applied["project"]["revision"],
        json!("1"),
        "the fixture moved once"
    );
    let answer = ask(&mut core, "0");
    assert_eq!(answer, moved(&shape), "{answer}");
}

/// The gizmo / Inspector's `set_local_trs` rebuilds the group's transform
/// run under new node ids; the probe ignores transform node identity, so
/// it stays fast once the object has its group.
#[test]
fn a_set_local_trs_that_rebuilds_the_transform_run_is_fast() {
    let shape = shape("Sphere");
    let mut core = load(document(&[&shape], &[]));
    let trs = |x: f64| {
        json!([{"kind":"set_local_trs","node_id":shape.id,
                "translation":[x,0.,0.],"rotation_euler_degrees":[0.,0.,0.],"scale":[1.,1.,1.]}])
    };
    apply(&mut core, trs(1.));
    record_preview_base(&mut core);
    apply(&mut core, trs(4.));
    let answer = ask(&mut core, "1");
    let expected = project::math::Mat4::translation(3., 0., 0.).0;
    assert_eq!(
        answer,
        json!({
            "fast": true,
            "base_revision": "1",
            "revision": "2",
            "deltas": { shape.id.to_string(): [expected, expected] },
        }),
        "{answer}"
    );
}

/// Undo of a move is fast against the moved revision as the base, with the
/// inverse delta; neither the command source nor the direction matters.
#[test]
fn undo_of_a_move_is_fast_with_the_inverse_delta() {
    let translate = state(
        "Translate",
        StateOperation::Transform(TransformOperation::Translate([1., 2., 3.])),
    );
    let shape = shape("Sphere");
    let mut core = load(document(&[&translate, &shape], &[]));
    apply(
        &mut core,
        json!([{"kind":"set_transform","node_id":translate.id,
                "value":{"kind":"translate","value":[4.,5.,6.]}}]),
    );
    record_preview_base(&mut core);
    undo(&mut core);
    let answer = ask(&mut core, "1");
    let expected = project::math::Mat4::translation(-3., -3., -3.).0;
    assert_eq!(
        answer,
        json!({
            "fast": true,
            "base_revision": "1",
            "revision": "2",
            "deltas": { shape.id.to_string(): [expected, expected] },
        }),
        "{answer}"
    );
}

/// A rename only takes the same path with no matrix deltas at all.
#[test]
fn a_rename_only_keeps_the_preview_fast_without_deltas() {
    let shape = shape("Sphere");
    let mut core = load(document(&[&shape], &[]));
    record_preview_base(&mut core);
    apply(
        &mut core,
        json!([{"kind":"rename_node","node_id":shape.id,"label":"Renamed"}]),
    );
    let answer = ask(&mut core, "0");
    assert_eq!(
        answer,
        json!({
            "fast": true,
            "base_revision": "0",
            "revision": "1",
            "deltas": {},
        }),
        "{answer}"
    );
}

/// The base never crosses an activation: a project switch clears it.
#[test]
fn a_project_switch_clears_the_preview_base() {
    let shape = shape("Sphere");
    let mut core = load(document(&[&shape], &[]));
    record_preview_base(&mut core);
    let context = core.projects.context();
    let response = core.handle(request(
        Method::ProjectOpen,
        json!({"path":crate::TEST_PROJECT,"expected_current":context}),
    ));
    let job_id: JobId = serde_json::from_value(response.result.unwrap()["job_id"].clone()).unwrap();
    core.project_io_finished(
        job_id,
        Ok(IoOutput::Loaded {
            snapshot: ProjectSession::new(document(&[&shape], &[]))
                .unwrap()
                .snapshot(),
            store_id: Uuid::new_v4(),
            path: None,
            source_path: None,
            unregistered: vec![],
            published: false,
            report: None,
            mode: dto::ProjectMode::Project,
            asset_root: None,
            staging_owner: None,
        }),
    );
    let base_revision = "0";
    let answer = ask(&mut core, base_revision);
    assert_eq!(
        answer,
        json!({"fast": false, "reason": "no_base"}),
        "{answer}"
    );
}

/// An in-flight preview that completes after a switch is not the new
/// activation's base: it published for a context the caller discarded.
#[test]
fn a_preview_completing_after_a_switch_is_not_the_base() {
    let shape = shape("Sphere");
    let mut core = load(document(&[&shape], &[]));
    let stale = request_preview(&mut core);
    // The switch lands before the preview's I/O completes.
    let context = core.projects.context();
    let response = core.handle(request(
        Method::ProjectOpen,
        json!({"path":crate::TEST_PROJECT,"expected_current":context}),
    ));
    let switched: JobId =
        serde_json::from_value(response.result.unwrap()["job_id"].clone()).unwrap();
    core.project_io_finished(
        switched,
        Ok(IoOutput::Loaded {
            snapshot: ProjectSession::new(document(&[&shape], &[]))
                .unwrap()
                .snapshot(),
            store_id: Uuid::new_v4(),
            path: None,
            source_path: None,
            unregistered: vec![],
            published: false,
            report: None,
            mode: dto::ProjectMode::Project,
            asset_root: None,
            staging_owner: None,
        }),
    );
    finish_preview(&mut core, stale);
    let answer = ask(&mut core, "0");
    assert_eq!(
        answer,
        json!({"fast": false, "reason": "no_base"}),
        "{answer}"
    );
}

/// a moved light no longer rebuilds. A static light
/// answers the same delta shape a Shape draw does, keyed by the light
/// node id the preview's light record pins, so `delta ·
/// world_from_light` is the new `world_from_light`.
#[test]
fn a_moved_light_answers_the_light_delta() {
    let translate = state(
        "Translate",
        StateOperation::Transform(TransformOperation::Translate([1., 2., 3.])),
    );
    let light = entity("Point", EntityKind::Light, "point");
    let mut core = load(document(&[&translate, &light], &[]));
    record_preview_base(&mut core);
    apply(
        &mut core,
        json!([{"kind":"set_transform","node_id":translate.id,
                "value":{"kind":"translate","value":[4.,5.,6.]}}]),
    );
    let answer = ask(&mut core, "0");
    assert_eq!(answer, moved(&light), "{answer}");
}

/// the viewer camera follows its owner, not the
/// manifest, so a camera move sends nothing to the display: the answer
/// stays fast with no deltas at all.
#[test]
fn a_moved_camera_stays_fast_without_deltas() {
    let translate = state(
        "Translate",
        StateOperation::Transform(TransformOperation::Translate([1., 2., 3.])),
    );
    let camera = entity("Camera", EntityKind::Camera, "perspective");
    let shape = shape("Sphere");
    let mut core = load(document(&[&shape], &[&translate, &camera]));
    record_preview_base(&mut core);
    apply(
        &mut core,
        json!([{"kind":"set_transform","node_id":translate.id,
                "value":{"kind":"translate","value":[4.,5.,6.]}}]),
    );
    let answer = ask(&mut core, "0");
    assert_eq!(
        answer,
        json!({
            "fast": true,
            "base_revision": "0",
            "revision": "1",
            "deltas": {},
        }),
        "{answer}"
    );
}

/// the viewport camera gizmo moves a posed camera with
/// `set_local_matrix`, which rewrites the Camera entity's `pose` — not a
/// transform state node. That is still a camera move: fast, no deltas.
#[test]
fn a_posed_camera_moved_by_set_local_matrix_stays_fast() {
    let mut camera = entity("Camera", EntityKind::Camera, "perspective");
    let NodeKind::Entity { pose, .. } = &mut camera.data else {
        unreachable!()
    };
    let identity = project::math::Mat4::translation(0., 0., 0.).0;
    *pose = Some(project::CameraPose {
        start: identity,
        end: identity,
    });
    let shape = shape("Sphere");
    let mut core = load(document(&[&shape], &[&camera]));
    record_preview_base(&mut core);
    apply(
        &mut core,
        json!([{"kind":"set_local_matrix","node_id":camera.id,
                "matrix":project::math::Mat4::translation(1., 2., 3.).0}]),
    );
    let answer = ask(&mut core, "0");
    assert_eq!(answer["fast"], true, "{answer}");
    assert_eq!(answer["deltas"], json!({}), "{answer}");
}

/// an area light owns no draw of its own — the shapes
/// it lights carry the move as ordinary Shapes, and the emitter the
/// renderer rebinds from a moved draw is the moved area light. A moved
/// area-light shape stays fast, the same delta as any other shape.
#[test]
fn a_shape_with_an_area_light_answers_the_draw_delta() {
    let area = entity("Area", EntityKind::AreaLight, "diffuse");
    let translate = state(
        "Translate",
        StateOperation::Transform(TransformOperation::Translate([1., 2., 3.])),
    );
    let shape = shape("Sphere");
    let mut core = load(document(&[&area, &translate, &shape], &[]));
    record_preview_base(&mut core);
    apply(
        &mut core,
        json!([{"kind":"set_transform","node_id":translate.id,
                "value":{"kind":"translate","value":[4.,5.,6.]}}]),
    );
    let answer = ask(&mut core, "0");
    assert_eq!(answer, moved(&shape), "{answer}");
}

/// a light with an animated transform keeps
/// rebuilding — the light record carries only the start sample, which an
/// endpoint pair cannot retarget (the same `animated` rule as a shape's).
#[test]
fn an_animated_moved_light_falls_back() {
    let start = state(
        "Start",
        StateOperation::ActiveTransform(project::ActiveSamples::Start),
    );
    let first = state(
        "Translate",
        StateOperation::Transform(TransformOperation::Translate([1., 2., 3.])),
    );
    let end = state(
        "End",
        StateOperation::ActiveTransform(project::ActiveSamples::End),
    );
    let last = state(
        "Translate",
        StateOperation::Transform(TransformOperation::Translate([4., 5., 6.])),
    );
    let all = state(
        "All",
        StateOperation::ActiveTransform(project::ActiveSamples::All),
    );
    let light = entity("Point", EntityKind::Light, "point");
    let mut core = load(document(&[&start, &first, &end, &last, &all, &light], &[]));
    record_preview_base(&mut core);
    apply(
        &mut core,
        json!([{"kind":"set_transform","node_id":last.id,
                "value":{"kind":"translate","value":[7.,8.,9.]}}]),
    );
    let answer = ask(&mut core, "0");
    assert_eq!(
        answer,
        json!({"fast": false, "reason": "animated"}),
        "{answer}"
    );
}

/// A light's parameters are not its transform: a light parameter edit
/// mixed with a move is a document change the probe rejects before any
/// entity answers a delta (`non_transform_change`, the same rule as a
/// shape).
#[test]
fn a_light_parameter_edit_mixed_with_a_move_falls_back() {
    let translate = state(
        "Translate",
        StateOperation::Transform(TransformOperation::Translate([1., 2., 3.])),
    );
    let light = entity("Point", EntityKind::Light, "point");
    let mut core = load(document(&[&translate, &light], &[]));
    record_preview_base(&mut core);
    apply(
        &mut core,
        json!([
            {"kind":"set_transform","node_id":translate.id,
             "value":{"kind":"translate","value":[4.,5.,6.]}},
            {"kind":"set_parameter","target":{"kind":"node","id":light.id},
             "name":"I","value_type":"rgb",
             "value":{"name":"I","value_type":"rgb","values":{"kind":"floats","value":[10.,10.,10.]}}},
        ]),
    );
    let answer = ask(&mut core, "0");
    assert_eq!(
        answer,
        json!({"fast": false, "reason": "non_transform_change"}),
        "{answer}"
    );
}

/// A transform edit inside an object definition is not a scene-node
/// transform change: the base document probe falls back.
#[test]
fn a_transform_inside_an_object_definition_falls_back() {
    let translate = state(
        "Translate",
        StateOperation::Transform(TransformOperation::Translate([1., 2., 3.])),
    );
    let defined = shape("Defined");
    let scope = Node::new(
        "Definition",
        NodeKind::Scope {
            scope: ScopeKind::ObjectDefinition,
            children: vec![translate.id, defined.id],
            owned_object: None,
            parent_camera: None,
        },
    );
    let world_shape = shape("Sphere");
    let mut document = document(&[&world_shape], &[]);
    document
        .definition_nodes
        .insert(translate.id, translate.clone());
    document.definition_nodes.insert(defined.id, defined);
    let scope_id = scope.id;
    document.definition_nodes.insert(scope.id, scope);
    let definition = Resource {
        id: project::ResourceId::new(),
        label: "Object".into(),
        location: "Objects/definition.object".into(),
        edition: ModelRevision::initial(),
        kind: ResourceKind::ObjectDefinition,
        texture_value_type: None,
        implementation: String::new(),
        parameters: Vec::new(),
        definition_scope: Some(scope_id),
        declared_in: None,
        origin: None,
    };
    document.resources.insert(definition.id, definition);
    document.validate().unwrap();

    let mut core = load(document);
    record_preview_base(&mut core);
    apply(
        &mut core,
        json!([{"kind":"set_transform","node_id":translate.id,
                "value":{"kind":"translate","value":[4.,5.,6.]}}]),
    );
    let answer = ask(&mut core, "0");
    assert_eq!(
        answer,
        json!({"fast": false, "reason": "non_transform_change"}),
        "{answer}"
    );
}

/// Even a scene-only transform edit can move a node inside an object
/// definition: a definition body that references a world-defined coordinate
/// system follows it, and the fast path has no draw to name for that.
#[test]
fn a_definition_body_moved_through_a_coordinate_reference_falls_back() {
    let translate = state(
        "Translate",
        StateOperation::Transform(TransformOperation::Translate([1., 2., 3.])),
    );
    let define = state("Define", StateOperation::DefineCoordinates("shared".into()));
    let world_shape = shape("Sphere");
    let use_shared = state("Use", StateOperation::UseCoordinates("shared".into()));
    let defined = shape("Defined");
    let scope = Node::new(
        "Definition",
        NodeKind::Scope {
            scope: ScopeKind::ObjectDefinition,
            children: vec![use_shared.id, defined.id],
            owned_object: None,
            parent_camera: None,
        },
    );
    let mut document = document(&[&translate, &define, &world_shape], &[]);
    document.definition_nodes.insert(use_shared.id, use_shared);
    document.definition_nodes.insert(defined.id, defined);
    let scope_id = scope.id;
    document.definition_nodes.insert(scope.id, scope);
    let definition = Resource {
        id: project::ResourceId::new(),
        label: "Object".into(),
        location: "Objects/shared.object".into(),
        edition: ModelRevision::initial(),
        kind: ResourceKind::ObjectDefinition,
        texture_value_type: None,
        implementation: String::new(),
        parameters: Vec::new(),
        definition_scope: Some(scope_id),
        declared_in: None,
        origin: None,
    };
    document.resources.insert(definition.id, definition);
    document.validate().unwrap();

    let mut core = load(document);
    record_preview_base(&mut core);
    apply(
        &mut core,
        json!([{"kind":"set_transform","node_id":translate.id,
                "value":{"kind":"translate","value":[4.,5.,6.]}}]),
    );
    let answer = ask(&mut core, "0");
    assert_eq!(
        answer,
        json!({"fast": false, "reason": "definition_node"}),
        "{answer}"
    );
}

/// A parameter edit mixed with a move is a document change the fast path
/// cannot name.
#[test]
fn a_parameter_edit_mixed_with_a_move_falls_back() {
    let translate = state(
        "Translate",
        StateOperation::Transform(TransformOperation::Translate([1., 2., 3.])),
    );
    let shape = shape("Sphere");
    let mut core = load(document(&[&translate, &shape], &[]));
    record_preview_base(&mut core);
    apply(
        &mut core,
        json!([
            {"kind":"set_transform","node_id":translate.id,
             "value":{"kind":"translate","value":[4.,5.,6.]}},
            {"kind":"set_parameter","target":{"kind":"node","id":shape.id},
             "name":"radius","value_type":"float",
             "value":{"name":"radius","value_type":"float","values":{"kind":"floats","value":[2.]}}},
        ]),
    );
    let answer = ask(&mut core, "0");
    assert_eq!(
        answer,
        json!({"fast": false, "reason": "non_transform_change"}),
        "{answer}"
    );
}

/// Without a successful preview behind it there is nothing to diff against.
#[test]
fn without_a_preview_base_the_answer_falls_back() {
    let shape = shape("Sphere");
    let mut core = load(document(&[&shape], &[]));
    let answer = ask(&mut core, "0");
    assert_eq!(
        answer,
        json!({"fast": false, "reason": "no_base"}),
        "{answer}"
    );
}

/// The caller must name the base revision it displays.
#[test]
fn a_wrong_base_revision_falls_back() {
    let translate = state(
        "Translate",
        StateOperation::Transform(TransformOperation::Translate([1., 2., 3.])),
    );
    let shape = shape("Sphere");
    let mut core = load(document(&[&translate, &shape], &[]));
    record_preview_base(&mut core);
    apply(
        &mut core,
        json!([{"kind":"set_transform","node_id":translate.id,
                "value":{"kind":"translate","value":[4.,5.,6.]}}]),
    );
    let answer = ask(&mut core, "42");
    assert_eq!(
        answer,
        json!({"fast": false, "reason": "base_revision_mismatch"}),
        "{answer}"
    );
}

/// A move that flips a static draw to animate (or back) changes the draw's
/// matrix per supported time, which plain deltas cannot express; so does a
/// move of an already animated draw (see the next test).
#[test]
fn a_static_animated_flip_falls_back() {
    let start = state(
        "Start",
        StateOperation::ActiveTransform(project::ActiveSamples::Start),
    );
    let first = state(
        "Translate",
        StateOperation::Transform(TransformOperation::Translate([1., 2., 3.])),
    );
    let end = state(
        "End",
        StateOperation::ActiveTransform(project::ActiveSamples::End),
    );
    let last = state(
        "Translate",
        StateOperation::Transform(TransformOperation::Translate([4., 5., 6.])),
    );
    let all = state(
        "All",
        StateOperation::ActiveTransform(project::ActiveSamples::All),
    );
    let shape = shape("Sphere");
    let mut core = load(document(&[&start, &first, &end, &last, &all, &shape], &[]));
    record_preview_base(&mut core);
    apply(
        &mut core,
        json!([{"kind":"set_transform","node_id":first.id,
                "value":{"kind":"translate","value":[4.,5.,6.]}}]),
    );
    let answer = ask(&mut core, "0");
    assert_eq!(
        answer,
        json!({"fast": false, "reason": "animated"}),
        "{answer}"
    );
}

/// An animated draw is sampled at shutter times between the transform
/// times; endpoint deltas cannot retarget those samples, so it rebuilds.
#[test]
fn a_move_of_an_animated_shape_falls_back() {
    let start = state(
        "Start",
        StateOperation::ActiveTransform(project::ActiveSamples::Start),
    );
    let first = state(
        "Translate",
        StateOperation::Transform(TransformOperation::Translate([1., 2., 3.])),
    );
    let end = state(
        "End",
        StateOperation::ActiveTransform(project::ActiveSamples::End),
    );
    let last = state(
        "Translate",
        StateOperation::Transform(TransformOperation::Translate([4., 5., 6.])),
    );
    let all = state(
        "All",
        StateOperation::ActiveTransform(project::ActiveSamples::All),
    );
    let shape = shape("Sphere");
    let mut core = load(document(&[&start, &first, &end, &last, &all, &shape], &[]));
    record_preview_base(&mut core);
    apply(
        &mut core,
        json!([{"kind":"set_transform","node_id":last.id,
                "value":{"kind":"translate","value":[7.,8.,9.]}}]),
    );
    let answer = ask(&mut core, "0");
    assert_eq!(
        answer,
        json!({"fast": false, "reason": "animated"}),
        "{answer}"
    );
}

/// A base matrix that cannot be inverted leaves no delta to multiply in.
#[test]
fn a_singular_base_matrix_falls_back() {
    let singular = state(
        "Flatten",
        StateOperation::Transform(TransformOperation::Scale([0., 1., 1.])),
    );
    let shape = shape("Sphere");
    let mut core = load(document(&[&singular, &shape], &[]));
    record_preview_base(&mut core);
    apply(
        &mut core,
        json!([{"kind":"set_transform","node_id":singular.id,
                "value":{"kind":"translate","value":[4.,5.,6.]}}]),
    );
    let answer = ask(&mut core, "0");
    assert_eq!(
        answer,
        json!({"fast": false, "reason": "singular_base_matrix"}),
        "{answer}"
    );
}
