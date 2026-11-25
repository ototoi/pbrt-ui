//! Export→reimport: a resolved model exports camera poses
//! (`Transform [inverse(pose)]` before every posed camera) and
//! camera-relative groups (`Transform [camera-to-world]` inside the
//! group); the export never reproduces import-time resets, and the whole
//! round-trip re-imports resolved and equivalent to the first import at
//! within a tolerance of 1e-12 relative for baked matrices.
mod support;

use pbrt_io::{
    ImportOptions, export_project, import_project, import_project_with_report, prepare_scene,
};
use project::math::Mat4;
use project::{
    CameraPose, EntityKind, NodeId, NodeKind, ProjectDocument, ProjectSession, SceneId, ScopeKind,
    StateOperation, TransformOperation,
};
use std::path::{Path, PathBuf};
use support::transform_parents::{
    BAKED_EPSILON, Case, assert_documents_equivalent, assert_no_resets, kind, label,
    parent_camera_of, scene_doc, trees, world_children,
};

/// A sportscar/ganesha-like battery: world-level transforms,
/// a depth-1 Transform group, a whole move from an Identity,
/// a Transform scope leaking a material, an object definition with
/// a Transform (sanmiguel), and an area-light materialization on a whole
/// move.
const ROUNDTRIP_SOURCE: &str = "LookAt 1 2 8  0 1 0  0 1 0\n\
Camera \"perspective\" \"float fov\" [40]\n\
Film \"rgb\" \"integer xresolution\" [64] \"integer yresolution\" [64]\n\
WorldBegin\n\
Translate 1 0 0\n\
Shape \"sphere\" \"float radius\" [0.5]\n\
AttributeBegin\n\
Transform [1 0 0 0  0 1 0 0  0 0 1 0  2 0 0 1]\n\
Shape \"sphere\" \"float radius\" [1]\n\
AttributeEnd\n\
AttributeBegin\n\
Translate 0 1 0\n\
Shape \"sphere\" \"float radius\" [2]\n\
Identity\n\
Shape \"sphere\" \"float radius\" [3]\n\
AttributeEnd\n\
AttributeBegin\n\
Material \"diffuse\" \"rgb reflectance\" [0.4 0.4 0.4]\n\
TransformBegin\n\
Rotate 30 0 1 0\n\
Shape \"sphere\" \"float radius\" [4]\n\
TransformEnd\n\
Shape \"sphere\" \"float radius\" [5]\n\
AttributeEnd\n\
AttributeBegin\n\
ObjectBegin \"part\"\n\
Transform [1 0 0 0  0 1 0 0  0 0 1 0  -3 0 0 1]\n\
Shape \"sphere\" \"float radius\" [6]\n\
ObjectEnd\n\
ObjectInstance \"part\"\n\
AttributeEnd\n\
AttributeBegin\n\
AreaLightSource \"diffuse\" \"rgb L\" [2 2 2]\n\
Shape \"sphere\" \"float radius\" [7]\n\
Identity\n\
Shape \"sphere\" \"float radius\" [8]\n\
AttributeEnd\n\
WorldEnd\n";

fn write_source(home: &Path, name: &str, text: &str) -> PathBuf {
    let path = home.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

/// Import PBRT text through the real import path (which resolves transform
/// parents), returning the store, the session and the document.
fn import_text(
    home: &Path,
    name: &str,
    text: &str,
) -> (project::ProjectStore, ProjectSession, ProjectDocument) {
    let source = write_source(home, name, text);
    let (store, session) = import_project(
        &source,
        &home.join(format!("{name}-project")),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let document = session.snapshot().document().clone();
    (store, session, document)
}

/// Export a session's project and hand back the exported scene.pbrt path.
fn export(home: &Path, store: &project::ProjectStore, session: &ProjectSession) -> PathBuf {
    export_project(&session.snapshot(), store, &home.join("bundle"), || false).unwrap()
}

fn reimport(
    home: &Path,
    name: &str,
    exported: &Path,
) -> (project::ProjectStore, ProjectSession, ProjectDocument) {
    let (store, session) = import_project(
        exported,
        &home.join(format!("{name}-again")),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let document = session.snapshot().document().clone();
    (store, session, document)
}

/// For a document imported through the resolved path: no reset state, no
/// Transform scope, every camera carries its pose.
fn assert_resolved(document: &ProjectDocument, scene: SceneId) {
    assert_no_resets(&Case {
        before: document.clone(),
        after: document.clone(),
        scene,
        diagnostics: Vec::new(),
    });
    assert_document_has_no_transform_stream(document, scene);
}

/// For one document imported through the resolved path (fresh import with
/// a default scene).
fn assert_resolved_document(document: &ProjectDocument) {
    assert_resolved(document, default_scene_of(document));
}

fn default_scene_of(document: &ProjectDocument) -> SceneId {
    document.default_scene.expect("import produced a scene")
}

fn camera_pose_of(document: &ProjectDocument, camera: NodeId) -> CameraPose {
    match kind(document, default_scene_of(document), camera) {
        NodeKind::Entity {
            pose: Some(pose), ..
        } => *pose,
        other => panic!("expected a posed camera, got {other:?}"),
    }
}

fn options_cameras_of(document: &ProjectDocument) -> Vec<NodeId> {
    let scene = default_scene_of(document);
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

/// Parse the emitter's bracketed matrix spelling back into model order. The
/// file's 16 numbers are column-major relative to the row-major model, the
/// same convention the preview scanner applies to `Transform`/`ConcatTransform`.
fn parsed_matrix(text: &str) -> [f64; 16] {
    let body = text
        .split_once('[')
        .map(|(_, rest)| rest)
        .unwrap_or(text)
        .trim_end_matches(']')
        .trim();
    let numbers: Vec<f64> = body
        .split_whitespace()
        .map(|number| number.parse().unwrap())
        .collect();
    assert_eq!(
        numbers.len(),
        16,
        "a matrix directive has 16 entries: {text}"
    );
    let mut model = [0f64; 16];
    for row in 0..4 {
        for col in 0..4 {
            model[row * 4 + col] = numbers[col * 4 + row];
        }
    }
    model
}

fn matrices_close(a: &[f64; 16], b: &[f64; 16], what: &str) {
    for (index, (left, right)) in a.iter().zip(b).enumerate() {
        let scale = right.abs().max(1.0);
        assert!(
            (left - right).abs() <= BAKED_EPSILON * scale,
            "{what}[{index}]: {left} vs {right}"
        );
    }
}

/// The zero-resets rule over every node of the document, whatever tree it
/// lives in for World/definitions,
/// applied to the whole resolved import).
fn assert_document_has_no_transform_stream(document: &ProjectDocument, scene: SceneId) {
    for id in trees(document, scene) {
        let kind = kind(document, scene, id);
        assert!(
            !matches!(
                kind,
                NodeKind::State {
                    operation: StateOperation::Transform(
                        TransformOperation::Identity | TransformOperation::Replace(_)
                    ) | StateOperation::DefineCoordinates(_)
                        | StateOperation::UseCoordinates(_)
                } | NodeKind::Scope {
                    scope: ScopeKind::Transform,
                    ..
                }
            ),
            "import-time reset remains: {}",
            label(document, scene, id)
        );
    }
    for id in scene_doc(document, scene).options.clone() {
        assert!(
            !matches!(
                kind(document, scene, id),
                NodeKind::State {
                    operation: StateOperation::Transform(_)
                        | StateOperation::ActiveTransform(_)
                        | StateOperation::DefineCoordinates(_)
                        | StateOperation::UseCoordinates(_)
                }
            ),
            "an Options transform state remains: {}",
            label(document, scene, id)
        );
    }
}

#[test]
fn every_import_path_resolves_transform_parents() {
    let temp = tempfile::tempdir().unwrap();
    let (_store, session, document) = import_text(temp.path(), "scene.pbrt", ROUNDTRIP_SOURCE);
    assert_resolved_document(&document);
    for camera in options_cameras_of(&document) {
        let _ = camera_pose_of(&document, camera);
    }
    // Resolution is equivalent to the unresolved parse of the same text and
    // to the session that stores it.
    let source = write_source(temp.path(), "unresolved.pbrt", ROUNDTRIP_SOURCE);
    let unresolved = pbrt_io::parse_unresolved(&source, &ImportOptions::default()).unwrap();
    let stored = session.snapshot().document().clone();
    assert!(stored == document, "the stored document is the import");
    let unresolved_scene = unresolved.default_scene.expect("import produced a scene");
    let mut resolved = unresolved.clone();
    let diagnostics = project::resolve_transform_parents(&mut resolved, unresolved_scene).unwrap();
    assert!(diagnostics.is_empty(), "the fixture needs no bake");
    assert_documents_equivalent(&resolved, &document);
}

#[test]
fn resolution_diagnostics_reach_the_import_report() {
    let temp = tempfile::tempdir().unwrap();
    // A cross-tree coordinate-system use (defined in the world, used inside an
    // object definition) bakes the matrix and reports `coordinate_system_fixed`.
    let text = "Camera \"perspective\" \"float fov\" [40]\n\
Film \"rgb\" \"integer xresolution\" [32] \"integer yresolution\" [32]\n\
WorldBegin\n\
AttributeBegin\nTranslate 3 0 0\nCoordinateSystem \"w0\"\nAttributeEnd\n\
ObjectBegin \"obj\"\n\
CoordSysTransform \"w0\"\n\
Shape \"sphere\" \"float radius\" [0.4]\nObjectEnd\n\
ObjectInstance \"obj\"\n\
WorldEnd\n";
    let source = write_source(temp.path(), "fixed.pbrt", text);
    let (_store, session, report) = import_project_with_report(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "coordinate_system_fixed"),
        "the bake diagnostic must reach the report: {:?}",
        report.diagnostics
    );
    let document = session.snapshot().document().clone();
    let record_id = document.imports.keys().copied().next().unwrap();
    assert!(
        document.imports[&record_id]
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "coordinate_system_fixed"),
        "the diagnostic must be recorded in the document"
    );
}

#[test]
fn resolution_beyond_the_node_budget_fails_the_import() {
    let temp = tempfile::tempdir().unwrap();
    // A split whole move: the head carries its own nested scope, while the
    // moved tail inherited a material, a color space, reverse orientation
    // and an area light — every one of them turns into an explicit state
    // node inside the fresh group, so the resolution grows the model well
    // past its unresolved node count.
    let text = "Camera \"perspective\" \"float fov\" [40]\n\
Film \"rgb\" \"integer xresolution\" [32] \"integer yresolution\" [32]\n\
WorldBegin\n\
AttributeBegin\n\
Material \"diffuse\" \"rgb reflectance\" [0.4 0.4 0.4]\n\
ColorSpace \"ACEScg\"\n\
ReverseOrientation\n\
AreaLightSource \"diffuse\" \"rgb L\" [1 1 1]\n\
AttributeBegin\n\
Shape \"sphere\" \"float radius\" [0.25]\n\
AttributeEnd\n\
Translate 1 2 3\n\
Identity\n\
Shape \"sphere\" \"float radius\" [1]\n\
AttributeEnd\n\
WorldEnd\n";
    let source = write_source(temp.path(), "budget.pbrt", text);
    let parsed = pbrt_io::parse_unresolved(&source, &ImportOptions::default()).unwrap();
    let scene = parsed.default_scene.unwrap();
    let unresolved_count = parsed.scenes[&scene].nodes.len() + parsed.definition_nodes.len();
    let mut resolved = parsed.clone();
    project::resolve_transform_parents(&mut resolved, scene).unwrap();
    let resolved_count = resolved.scenes[&scene].nodes.len() + resolved.definition_nodes.len();
    assert!(
        resolved_count > unresolved_count,
        "the fixture must grow under resolution: {resolved_count} vs {unresolved_count}"
    );
    assert!(
        resolved_count > unresolved_count + 1,
        "a budget between the two counts must exist: {resolved_count} vs {unresolved_count}"
    );
    let options = ImportOptions {
        max_nodes: resolved_count - 1,
        ..ImportOptions::default()
    };
    let error = import_project(&source, &temp.path().join("project"), &options, || false)
        .err()
        .expect("the resolved node count must fail the budget");
    assert!(
        matches!(
            &error,
            pbrt_io::Error::Project(project::ProjectError::ResourceLimit(message))
                if message.contains("nodes (import limit)")
        ),
        "the resolved node count must re-check the budget: {error:?}"
    );
}

#[test]
fn prepare_scene_resolves_transform_parents() {
    let temp = tempfile::tempdir().unwrap();
    let text = "LookAt 0 2 8  0 0 0  0 1 0\n\
Camera \"perspective\" \"float fov\" [40]\n\
WorldBegin\n\
AttributeBegin\n\
Translate 1 2 3\n\
Shape \"sphere\" \"float radius\" [0.25]\n\
Identity\n\
Shape \"sphere\" \"float radius\" [1]\n\
AttributeEnd\n\
WorldEnd\n";
    let source = write_source(temp.path(), "prepared.pbrt", text);
    let (prepared, report) = prepare_scene(&source, &ImportOptions::default(), || false).unwrap();
    let document = prepared.document().clone();
    // The prepared fixture stays silent apart from the estimate note
    // every camera scene import reports on the console.
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code == "scene_coordinate_system"),
        "{:?}",
        report.diagnostics
    );
    // A prepared document carries no default scene (the caller places it),
    // so assert on the one scene the import produced.
    let scene = *document.scenes.keys().next().expect("a prepared scene");
    assert_resolved(&document, scene);
}

#[test]
fn resolved_project_roundtrips_without_resets_and_stays_equivalent() {
    let temp = tempfile::tempdir().unwrap();
    let (store, session, document) = import_text(temp.path(), "roundtrip.pbrt", ROUNDTRIP_SOURCE);
    assert_resolved_document(&document);
    let exported = export(temp.path(), &store, &session);
    let text = std::fs::read_to_string(&exported).unwrap();
    assert!(
        !text.contains("Identity") && !text.contains("CoordSysTransform"),
        "export never reproduces import-time resets: {text}"
    );
    let (_store2, _session2, reimported) = reimport(temp.path(), "roundtrip.pbrt", &exported);
    assert_resolved_document(&reimported);
    for camera in options_cameras_of(&reimported) {
        let _ = camera_pose_of(&reimported, camera);
    }
    assert_documents_equivalent(&document, &reimported);
}

/// hair-actual-bsdf shape: a light group follows the camera through
/// `CoordSysTransform "camera"`. Export bakes the camera-to-world matrix
/// into the group, and the reimport keeps no camera reference.
#[test]
fn camera_child_group_reimports_as_a_baked_concat_transform() {
    let temp = tempfile::tempdir().unwrap();
    let text = "LookAt 1 2 8  0 0 0  0 1 0\n\
Camera \"perspective\" \"float fov\" [40]\n\
Film \"rgb\" \"integer xresolution\" [32] \"integer yresolution\" [32]\n\
WorldBegin\n\
AttributeBegin\n\
Translate 5 0 0\n\
Shape \"sphere\" \"float radius\" [2]\n\
AttributeEnd\n\
AttributeBegin\n\
CoordSysTransform \"camera\"\n\
LightSource \"point\" \"rgb I\" [10 10 10]\n\
Shape \"sphere\" \"float radius\" [0.25]\n\
AttributeEnd\n\
WorldEnd\n";
    let (store, session, document) = import_text(temp.path(), "hair.pbrt", text);
    let scene = default_scene_of(&document);
    // The import attaches the group to the render camera.
    let camera = scene_doc(&document, scene).render_camera.unwrap();
    let group = world_children(&document, scene)
        .into_iter()
        .find(|id| parent_camera_of(&document, scene, *id) == Some(camera))
        .expect("the light group attaches to the camera");
    assert_eq!(parent_camera_of(&document, scene, group), Some(camera));
    let exported = export(temp.path(), &store, &session);
    let body = std::fs::read_to_string(&exported).unwrap();
    // The group exports as `AttributeBegin` + a camera-to-world Transform
    // before its children, so the exported light follows the camera.
    let pose = camera_pose_of(&document, camera);
    let lines: Vec<&str> = body.lines().collect();
    let light = lines
        .iter()
        .position(|line| line.starts_with("LightSource \"point\""))
        .expect("the exported light");
    let attribute = (0..light)
        .rev()
        .find(|&index| lines[index].starts_with("AttributeBegin"))
        .expect("the camera group's AttributeBegin");
    let transforms: Vec<&str> = lines[attribute + 1..light]
        .iter()
        .filter(|line| line.starts_with("Transform ["))
        .copied()
        .collect();
    assert_eq!(
        transforms.len(),
        1,
        "one camera-to-world transform leads the camera group: {body}"
    );
    matrices_close(
        &parsed_matrix(transforms[0]),
        &pose.start,
        "the camera group's camera-to-world transform",
    );
    let (_store2, _session2, reimported) = reimport(temp.path(), "hair.pbrt", &exported);
    let reimport_scene = default_scene_of(&reimported);
    for id in trees(&reimported, reimport_scene) {
        assert!(
            !matches!(
                kind(&reimported, reimport_scene, id),
                NodeKind::Scope {
                    parent_camera: Some(_),
                    ..
                }
            ),
            "the reimported model bakes the camera relationship as a matrix: {}",
            label(&reimported, reimport_scene, id)
        );
    }
    assert_resolved_document(&reimported);
    assert_documents_equivalent(&document, &reimported);
    let reimport_camera = options_cameras_of(&reimported)[0];
    matrices_close(
        &camera_pose_of(&reimported, reimport_camera).start,
        &pose.start,
        "reimported camera pose",
    );
}

/// A camera group entered under `ActiveTransform StartTime` still gets the
/// camera pose on both samples, and its children keep the StartTime selector.
#[test]
fn camera_child_group_under_a_start_time_selector_round_trips() {
    let temp = tempfile::tempdir().unwrap();
    let text = "LookAt 1 2 8  0 0 0  0 1 0\n\
Camera \"perspective\" \"float fov\" [40]\n\
Film \"rgb\" \"integer xresolution\" [32] \"integer yresolution\" [32]\n\
WorldBegin\n\
ActiveTransform StartTime\n\
AttributeBegin\n\
CoordSysTransform \"camera\"\n\
Translate 0 0 -1\n\
Shape \"sphere\" \"float radius\" [0.25]\n\
AttributeEnd\n\
WorldEnd\n";
    let (store, session, document) = import_text(temp.path(), "start.pbrt", text);
    let scene = default_scene_of(&document);
    let camera = scene_doc(&document, scene).render_camera.unwrap();
    assert!(
        world_children(&document, scene)
            .into_iter()
            .filter(|id| matches!(kind(&document, scene, *id), NodeKind::Scope { .. }))
            .any(|id| parent_camera_of(&document, scene, id) == Some(camera)),
        "the group attaches to the camera"
    );
    let exported = export(temp.path(), &store, &session);
    let (_store2, _session2, reimported) = reimport(temp.path(), "start.pbrt", &exported);
    assert_resolved_document(&reimported);
    assert_documents_equivalent(&document, &reimported);
}

/// A static pose camera exports as `Transform [inverse(pose)]` directly
/// before its `Camera` directive: one transform, no ActiveTransform dance.
#[test]
fn a_static_pose_camera_exports_its_inverse_pose_before_camera() {
    let temp = tempfile::tempdir().unwrap();
    let text = "LookAt 0 2 8  0 0 0  0 1 0\n\
Camera \"perspective\" \"float fov\" [40]\n\
Camera \"orthographic\"\n\
Film \"rgb\" \"integer xresolution\" [32] \"integer yresolution\" [32]\n\
WorldBegin\n\
Shape \"sphere\" \"float radius\" [1]\n\
WorldEnd\n";
    let (store, session, document) = import_text(temp.path(), "staticcamera.pbrt", text);
    let cameras = options_cameras_of(&document);
    assert_eq!(cameras.len(), 2);
    let exported = export(temp.path(), &store, &session);
    let body = std::fs::read_to_string(&exported).unwrap();
    // The selected (last Camera wins) is written at the end of Options; the
    // perspective camera is a non-selected one inside Options. Both carry
    // their own inverse-pose transform before their Camera directive.
    let ortho_index = body
        .rfind("Camera \"orthographic\"")
        .expect("orthographic camera");
    let perspective_index = body
        .find("Camera \"perspective\"")
        .expect("perspective camera");
    let ortho_pose = project::math::inverse(Mat4(camera_pose_of(&document, cameras[1]).start))
        .expect("invertible");
    let perspective_pose =
        project::math::inverse(Mat4(camera_pose_of(&document, cameras[0]).start))
            .expect("invertible");
    let before_ortho = &body[..ortho_index];
    let last_transform = before_ortho
        .lines()
        .rev()
        .find(|line| line.starts_with("Transform ["))
        .expect("a Transform line before the selected camera");
    matrices_close(
        &parsed_matrix(last_transform),
        &ortho_pose.0,
        "selected camera inverse pose",
    );
    // No ActiveTransform choreography belongs to a static camera.
    let between_cameras = &body[perspective_index..ortho_index];
    assert!(
        !between_cameras.contains("ActiveTransform"),
        "a static pose camera needs no sample split: {body}"
    );
    // The non-selected perspective camera inside Options carries its own
    // inverse-pose transform right before its Camera directive.
    let before_perspective: Vec<&str> = body[..perspective_index]
        .lines()
        .filter(|line| line.starts_with("Transform ["))
        .collect();
    assert_eq!(before_perspective.len(), 1, "{body}");
    matrices_close(
        &parsed_matrix(before_perspective[0]),
        &perspective_pose.0,
        "non-selected camera inverse pose",
    );
    let (_store2, _session2, reimported) = reimport(temp.path(), "staticcamera.pbrt", &exported);
    assert_resolved_document(&reimported);
    assert_documents_equivalent(&document, &reimported);
}

/// matrix-preview shape: only the start sample of the camera transform is
/// set before `Camera`, so the pose is animated and exports per sample.
#[test]
fn an_animated_pose_camera_exports_per_sample_transforms() {
    let temp = tempfile::tempdir().unwrap();
    let text = "TransformTimes 0 2\n\
ActiveTransform StartTime\n\
LookAt 3 2 6  0 0 0  0 1 0\n\
Camera \"perspective\" \"float fov\" [45]\n\
Film \"rgb\" \"integer xresolution\" [16] \"integer yresolution\" [16]\n\
WorldBegin\n\
AttributeBegin\n\
Transform [1 0 0 0  0 1 0 0  0 0 1 0  -1.5 0 0.5 1]\n\
Shape \"sphere\" \"float radius\" [1]\n\
AttributeEnd\n\
ActiveTransform EndTime\n\
AttributeBegin\n\
Translate 2 0 0\n\
Shape \"cylinder\" \"float radius\" [0.25] \"float zmin\" [0] \"float zmax\" [1]\n\
AttributeEnd\n\
WorldEnd\n";
    let (store, session, document) = import_text(temp.path(), "animated.pbrt", text);
    let camera = options_cameras_of(&document)[0];
    let pose = camera_pose_of(&document, camera);
    assert!(pose.start != pose.end, "the camera is animated");
    let exported = export(temp.path(), &store, &session);
    let body = std::fs::read_to_string(&exported).unwrap();
    let camera_at = body
        .lines()
        .position(|line| line.starts_with("Camera \"perspective\""))
        .expect("the camera directive");
    let lines: Vec<&str> = body.lines().collect();
    let around = &lines[camera_at.saturating_sub(5)..camera_at + 1];
    let joined = around.join("\n");
    assert!(
        joined.contains("ActiveTransform StartTime")
            && joined.contains("ActiveTransform EndTime")
            && joined.contains("ActiveTransform All"),
        "an animated pose camera writes a per-sample split then All: {joined}"
    );
    // The two bracketed transforms before the camera are the inverse poses.
    let transforms_before: Vec<&str> = lines
        .iter()
        .take(camera_at)
        .rev()
        .take_while(|line| line.starts_with("Transform [") || line.starts_with("ActiveTransform "))
        .filter_map(|line| line.starts_with("Transform [").then_some(*line))
        .collect();
    assert_eq!(
        transforms_before.len(),
        2,
        "an animated camera writes both samples: {joined}"
    );
    let expected_start = project::math::inverse(Mat4(pose.start)).expect("invertible");
    let expected_end = project::math::inverse(Mat4(pose.end)).expect("invertible");
    // transforms_before is reversed document order: [end, start].
    matrices_close(
        &parsed_matrix(transforms_before[0]),
        &expected_end.0,
        "animated end inverse pose",
    );
    matrices_close(
        &parsed_matrix(transforms_before[1]),
        &expected_start.0,
        "animated start inverse pose",
    );
    let (_store2, _session2, reimported) = reimport(temp.path(), "animated.pbrt", &exported);
    assert_resolved_document(&reimported);
    assert_documents_equivalent(&document, &reimported);
}

/// clouds/smoke-plume shape: a world-level transform wraps into a group,
/// exports as any other group, and the preview manifest cannot tell the
/// resolved export from the original source text.
#[test]
fn preview_manifests_match_before_and_after_resolution() {
    let temp = tempfile::tempdir().unwrap();
    let text = "LookAt 0 2 8  0 0 0  0 1 0\n\
Camera \"perspective\" \"float fov\" [40]\n\
Film \"rgb\" \"integer xresolution\" [32] \"integer yresolution\" [32]\n\
WorldBegin\n\
Translate 1 0 0\n\
Shape \"sphere\" \"float radius\" [0.5]\n\
Rotate 90 0 1 0\n\
AttributeBegin\n\
Translate 0 3 0\n\
Shape \"sphere\" \"float radius\" [1.5]\n\
Identity\n\
Shape \"sphere\" \"float radius\" [2.5]\n\
AttributeEnd\n\
WorldEnd\n";
    let source = write_source(temp.path(), "clouds.pbrt", text);
    let (store, session, _document) = import_text(temp.path(), "clouds.pbrt", text);
    let exported = export(temp.path(), &store, &session);

    let original = scene_parse::parse_scene(&source, Default::default()).unwrap();
    let resolved = scene_parse::parse_scene(&exported, Default::default()).unwrap();
    let original_draws = draw_matrices(&original);
    let resolved_draws = draw_matrices(&resolved);
    assert_eq!(
        original_draws.len(),
        resolved_draws.len(),
        "the resolved export draws the same shape count"
    );
    for (before, after) in original_draws.iter().zip(&resolved_draws) {
        for (index, (left, right)) in before.iter().zip(after.iter()).enumerate() {
            let scale = right.abs().max(1.0);
            assert!(
                (left - right).abs() <= BAKED_EPSILON * scale,
                "draw world matrix {index}: {left} vs {right}"
            );
        }
    }
}

/// Sort each manifest's `world_from_object` matrices into a stable order
/// (drawing order is not part of equivalence: whole moves reorder nodes).
fn draw_matrices(preview: &scene_parse::PreviewGeometry) -> Vec<[f64; 16]> {
    let mut matrices: Vec<[f64; 16]> = preview
        .manifest
        .draws
        .iter()
        .map(|draw| draw.world_from_object.first().copied().expect("a matrix"))
        .collect();
    matrices.sort_by(|a, b| {
        a.iter()
            .zip(b.iter())
            .map(|(x, y)| x.total_cmp(y))
            .find(|order| order.is_ne())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    matrices
}

/// sportscar/ganesha shape: the world matrices of each
/// draw keep their exact positions while the model composes the same chain
/// through different (concat) forms.
#[test]
fn user_defined_coordinate_system_round_trips_through_its_own_group() {
    // A name system defined in the world attaches the use site to the
    // system's group; export writes the group and its child plainly.
    let temp = tempfile::tempdir().unwrap();
    let text = "LookAt 0 2 8  0 0 0  0 1 0\n\
Camera \"perspective\" \"float fov\" [40]\n\
Film \"rgb\" \"integer xresolution\" [32] \"integer yresolution\" [32]\n\
WorldBegin\n\
AttributeBegin\n\
Translate 4 0 0\n\
CoordinateSystem \"frame\"\n\
Shape \"sphere\" \"float radius\" [1]\n\
AttributeEnd\n\
AttributeBegin\n\
CoordSysTransform \"frame\"\n\
LightSource \"point\" \"rgb I\" [3 3 3]\n\
AttributeEnd\n\
WorldEnd\n";
    let (store, session, document) = import_text(temp.path(), "named.pbrt", text);
    let exported = export(temp.path(), &store, &session);
    let body = std::fs::read_to_string(&exported).unwrap();
    assert!(!body.contains("CoordSysTransform"), "{body}");
    assert!(!body.contains("CoordinateSystem"), "{body}");
    let (_store2, _session2, reimported) = reimport(temp.path(), "named.pbrt", &exported);
    assert_resolved_document(&reimported);
    assert_documents_equivalent(&document, &reimported);
}

#[test]
fn a_mirrored_camera_pose_round_trips() {
    // Camera list: `LookAt` composed with `Scale -1 1 1`.
    let temp = tempfile::tempdir().unwrap();
    let text = "Scale -1 1 1\n\
LookAt 0 2 8  0 0 0  0 1 0\n\
Camera \"perspective\" \"float fov\" [40]\n\
WorldBegin\n\
Shape \"sphere\" \"float radius\" [1]\n\
WorldEnd\n";
    let (store, session, document) = import_text(temp.path(), "mirror.pbrt", text);
    let camera = options_cameras_of(&document)[0];
    let pose = camera_pose_of(&document, camera);
    assert!(project::math::inverse(Mat4(pose.start)).is_some());
    let exported = export(temp.path(), &store, &session);
    let (_store2, _session2, reimported) = reimport(temp.path(), "mirror.pbrt", &exported);
    assert_resolved_document(&reimported);
    assert_documents_equivalent(&document, &reimported);
    let reimport_camera = options_cameras_of(&reimported)[0];
    matrices_close(
        &camera_pose_of(&reimported, reimport_camera).start,
        &pose.start,
        "mirrored camera pose",
    );
}
