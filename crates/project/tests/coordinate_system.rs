//! The scene-level coordinate system: scene file format 4 keeps
//! it stored, older scene files infer it from their cameras.
mod common;
use common::*;
use project::math::{Mat4, inverse, look_at};
use project::*;
use std::fs;

/// A perspective camera carrying the pose a `LookAt eye look up` import
/// would carve (camera-to-world).
fn posed_camera(label: &str, eye: [f64; 3], look: [f64; 3], up: [f64; 3]) -> Node {
    let args = [
        eye[0], eye[1], eye[2], look[0], look[1], look[2], up[0], up[1], up[2],
    ];
    let matrix = inverse(look_at(&args).unwrap()).unwrap();
    Node::new(
        label,
        NodeKind::Entity {
            entity: EntityKind::Camera,
            implementation: "perspective".into(),
            parameters: Vec::new(),
            pose: Some(CameraPose {
                start: matrix.0,
                end: matrix.0,
            }),
        },
    )
}

/// Places a camera in the document's scene options and makes it the
/// render camera.
fn add_camera(document: &mut ProjectDocument, camera: Node) -> NodeId {
    let id = camera.id;
    let scene = scene_mut(document);
    scene.options.push(id);
    scene.nodes.insert(id, camera);
    scene.render_camera = Some(id);
    id
}

/// The single saved scene file's path and rewritten bytes; keeps `.meta`
/// untouched like a project from before coordinate systems existed.
fn rewrite_saved_scene(
    path: &std::path::Path,
    document: &ProjectDocument,
    mut edit: impl FnMut(&mut serde_json::Value),
) {
    let file = path.join("Assets").join(&scene(document).location);
    let mut json: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    edit(&mut json);
    fs::write(&file, serde_json::to_vec(&json).unwrap()).unwrap();
}

fn saved_closed(path: &std::path::Path, document: ProjectDocument) {
    let session = ProjectSession::new(document.clone()).unwrap();
    let mut store =
        FolderStore::create_v3(path, document.id, &document.title, Revision::initial()).unwrap();
    store.save(&session.snapshot()).unwrap();
    drop(store);
}

#[test]
fn saves_scenes_in_format_four_with_their_coordinate_system() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("project");
    let mut document = ProjectDocument::new("Format four");
    scene_mut(&mut document).coordinate_system =
        CoordinateSystem::new(Axis::PosZ, Handedness::Right);
    add_camera(
        &mut document,
        posed_camera("Camera", [0., -10., 3.], [0., 0., 0.], [0., 0., 1.]),
    );
    saved_closed(&path, document.clone());
    let (_, reopened) = ProjectStore::open(&path).unwrap();
    assert_eq!(reopened.document(), &document);
    let file = fs::read_to_string(path.join("Assets").join(&scene(&document).location)).unwrap();
    assert!(
        file.contains("pbrt-ui.scene/4"),
        "saved as format four: {file}"
    );
    let saved: serde_json::Value = serde_json::from_str(&file).unwrap();
    assert_eq!(saved["format"], "pbrt-ui.scene/4");
    assert_eq!(
        saved["coordinate_system"],
        serde_json::json!({"up":"+Z","handedness":"right"}),
        "the coordinate system is stored: {file}"
    );
}

#[test]
fn opening_format_three_scenes_infers_the_coordinate_system() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("project");
    let mut document = ProjectDocument::new("Format three");
    add_camera(
        &mut document,
        posed_camera("Camera", [0., -10., 3.], [0., 0., 0.], [0., 0., 1.]),
    );
    saved_closed(&path, document.clone());
    rewrite_saved_scene(&path, &document, |json| {
        json["format"] = "pbrt-ui.scene/3".into();
        json.as_object_mut().unwrap().remove("coordinate_system");
    });
    let (_, reopened) = ProjectStore::open(&path).unwrap();
    assert_eq!(
        scene(reopened.document()).coordinate_system,
        CoordinateSystem::new(Axis::PosZ, Handedness::Left),
        "inferred from the render camera's pose"
    );
}

#[test]
fn opening_format_three_scenes_with_a_mirrored_camera_reads_right_handed() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("project");
    let mut document = ProjectDocument::new("Mirrored");
    let camera_id = add_camera(
        &mut document,
        posed_camera("Camera", [0., 2., -6.], [0., 0.5, 0.], [0., 1., 0.]),
    );
    // The right-handed-tool idiom: a mirror composed into the camera pose.
    let scene_slot = scene_mut(&mut document);
    let node = scene_slot.nodes.get_mut(&camera_id).unwrap();
    let pose = node_pose_mut(node);
    let mirrored = Mat4(pose.start).compose(Mat4::scale(-1., 1., 1.)).0;
    pose.start = mirrored;
    pose.end = mirrored;
    saved_closed(&path, document.clone());
    rewrite_saved_scene(&path, &document, |json| {
        json["format"] = "pbrt-ui.scene/3".into();
        json.as_object_mut().unwrap().remove("coordinate_system");
    });
    let (_, reopened) = ProjectStore::open(&path).unwrap();
    assert_eq!(
        scene(reopened.document()).coordinate_system,
        CoordinateSystem::new(Axis::PosY, Handedness::Right),
        "the mirror flips the handedness, the up stays"
    );
}

#[test]
fn format_four_needs_its_coordinate_system() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("project");
    let mut document = ProjectDocument::new("Strict");
    add_camera(
        &mut document,
        posed_camera("Camera", [0., -10., 3.], [0., 0., 0.], [0., 0., 1.]),
    );
    saved_closed(&path, document.clone());
    rewrite_saved_scene(&path, &document, |json| {
        json.as_object_mut().unwrap().remove("coordinate_system");
    });
    let error = match ProjectStore::open(&path) {
        Ok(_) => panic!("format four without a coordinate system must be rejected"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("coordinate_system"),
        "the refusal names the missing field: {error}"
    );
}

#[test]
fn cameraless_older_scenes_default_to_y_up_left_handed() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("project");
    let (document, _, _) = native_scene();
    saved_closed(&path, document.clone());
    rewrite_saved_scene(&path, &document, |json| {
        json["format"] = "pbrt-ui.scene/1".into();
        json.as_object_mut().unwrap().remove("render_camera");
        json.as_object_mut().unwrap().remove("coordinate_system");
    });
    let (_, reopened) = ProjectStore::open(&path).unwrap();
    assert_eq!(
        scene(reopened.document()).coordinate_system,
        CoordinateSystem::default()
    );
    assert_eq!(scene(reopened.document()).render_camera, None);
}

#[test]
fn opening_format_one_scenes_infers_from_their_last_camera() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("project");
    let mut document = ProjectDocument::new("Format one");
    let first = add_camera(
        &mut document,
        posed_camera("First", [0., 2., -6.], [0., 0.5, 0.], [0., 1., 0.]),
    );
    let last = add_camera(
        &mut document,
        posed_camera("Last", [0., -10., 3.], [0., 0., 0.], [0., 0., 1.]),
    );
    saved_closed(&path, document.clone());
    rewrite_saved_scene(&path, &document, |json| {
        json["format"] = "pbrt-ui.scene/1".into();
        json.as_object_mut().unwrap().remove("render_camera");
        json.as_object_mut().unwrap().remove("coordinate_system");
    });
    let (_, reopened) = ProjectStore::open(&path).unwrap();
    let opened = scene(reopened.document());
    assert_eq!(opened.render_camera, Some(last), "the last camera renders");
    assert_ne!(opened.render_camera, Some(first));
    assert_eq!(
        opened.coordinate_system,
        CoordinateSystem::new(Axis::PosZ, Handedness::Left),
        "inferred from the render camera's pose"
    );
}

#[test]
fn the_coordinate_system_is_part_of_the_content_identity() {
    let plain = ProjectDocument::new("Content");
    let mut tilted = plain.clone();
    scene_mut(&mut tilted).coordinate_system = CoordinateSystem::new(Axis::PosZ, Handedness::Left);
    assert_ne!(
        ContentId::of(&plain),
        ContentId::of(&tilted),
        "a scene's coordinate system is saved content"
    );
}

#[test]
fn the_default_scene_starts_y_up_left_handed() {
    let document = ProjectDocument::new("Defaults");
    assert_eq!(
        scene(&document).coordinate_system,
        CoordinateSystem::default()
    );
}

#[test]
fn setting_the_coordinate_system_is_undoable_and_redoable() {
    let document = ProjectDocument::new("Editable");
    let scene_id = document.default_scene.unwrap();
    let mut session = ProjectSession::new(document).unwrap();
    let revision = session.revision();
    session
        .apply(
            &revision,
            &[Edit::SetSceneCoordinateSystem {
                scene: scene_id,
                up: Axis::PosZ,
                handedness: Handedness::Right,
            }],
        )
        .unwrap();
    let changed = CoordinateSystem::new(Axis::PosZ, Handedness::Right);
    assert_eq!(scene(session.document()).coordinate_system, changed);
    session.undo(&session.revision()).unwrap();
    assert_eq!(
        scene(session.document()).coordinate_system,
        CoordinateSystem::default(),
        "undo restores the previous coordinate system"
    );
    session.redo(&session.revision()).unwrap();
    assert_eq!(
        scene(session.document()).coordinate_system,
        changed,
        "redo re-applies the new coordinate system"
    );
}

#[test]
fn setting_the_coordinate_system_of_a_missing_scene_is_refused() {
    let document = ProjectDocument::new("Editable");
    let mut session = ProjectSession::new(document).unwrap();
    let missing = SceneId::new();
    let error = session
        .apply(
            &session.revision(),
            &[Edit::SetSceneCoordinateSystem {
                scene: missing,
                up: Axis::NegY,
                handedness: Handedness::Left,
            }],
        )
        .unwrap_err();
    assert!(
        error.to_string().to_lowercase().contains("scene"),
        "the refusal names the scene: {error}"
    );
}

#[test]
fn duplicate_scenes_carry_the_coordinate_system_over() {
    let mut session = ProjectSession::new(ProjectDocument::new("Source")).unwrap();
    let scene_id = session.document().default_scene.unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::SetSceneCoordinateSystem {
                scene: scene_id,
                up: Axis::NegZ,
                handedness: Handedness::Right,
            }],
        )
        .unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::DuplicateScene {
                scene: scene_id,
                location: "Scenes/Copy.scene".into(),
            }],
        )
        .unwrap();
    let copy = session
        .document()
        .scenes
        .values()
        .find(|scene| scene.label == "Copy")
        .unwrap();
    assert_eq!(
        copy.coordinate_system,
        CoordinateSystem::new(Axis::NegZ, Handedness::Right),
        "duplicating a scene preserves its coordinate system"
    );
}

/// Borrows the camera pose of an entity node for editing.
fn node_pose_mut(node: &mut Node) -> &mut CameraPose {
    let NodeKind::Entity {
        pose: Some(pose), ..
    } = &mut node.data
    else {
        panic!("the camera carries a pose");
    };
    pose
}
