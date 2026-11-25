//! Coordinate-system inference on import: a fresh PBRT
//! import estimates the scene's up axis and handedness from the render
//! camera, reports it as an info diagnostic, and export→reimport agrees —
//! including a right-handed mirror and Z-up scenes.
use pbrt_io::{ImportOptions, export_project, import_project_with_report, prepare_scene};
use project::{Axis, CoordinateSystem, Handedness, ImportSeverity, ProjectDocument};

const FILM: &str = "Film \"rgb\" \"integer xresolution\" [32] \"integer yresolution\" [32]\n";

fn write_source(home: &std::path::Path, name: &str, text: &str) -> std::path::PathBuf {
    let path = home.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

fn scene(document: &ProjectDocument) -> &project::SceneDocument {
    document.scene(document.default_scene.unwrap()).unwrap()
}

fn coordinate_diagnostic(report: &pbrt_io::ImportReport) -> &project::ImportDiagnostic {
    report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "scene_coordinate_system")
        .expect("the estimate reaches the report as its own diagnostic")
}

#[test]
fn z_up_look_at_imports_as_z_up_left_handed() {
    let temp = tempfile::tempdir().unwrap();
    let text = format!(
        "LookAt 0 -10 3  0 0 0  0 0 1\n\
Camera \"perspective\" \"float fov\" [40]\n\
{FILM}\
WorldBegin\n\
Shape \"sphere\" \"float radius\" [1]\n\
WorldEnd\n"
    );
    let source = write_source(temp.path(), "z-up.pbrt", &text);
    let (_store, session, report) = import_project_with_report(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    assert_eq!(
        scene(session.document()).coordinate_system,
        CoordinateSystem::new(Axis::PosZ, Handedness::Left)
    );
    let diagnostic = coordinate_diagnostic(&report);
    assert_eq!(diagnostic.severity, Some(ImportSeverity::Info));
    assert_eq!(diagnostic.message, "coordinate system: +Z up, left-handed");
    // The diagnostic is persisted with the import record, like the others.
    let snapshot = session.snapshot();
    let document = snapshot.document();
    let record = document.imports.values().next().unwrap();
    assert!(
        record
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "scene_coordinate_system"),
        "the estimate is recorded in the document: {:?}",
        record.diagnostics
    );
}

#[test]
fn a_scale_mirror_imports_as_right_handed() {
    let temp = tempfile::tempdir().unwrap();
    // The right-handed-tool export idiom: `Scale -1 1 1` in the camera
    // chain flips the handedness without moving the up direction.
    let text = format!(
        "LookAt 0 2 -6  0 0.5 0  0 1 0\n\
Scale -1 1 1\n\
Camera \"perspective\" \"float fov\" [40]\n\
{FILM}\
WorldBegin\n\
Shape \"sphere\" \"float radius\" [1]\n\
WorldEnd\n"
    );
    let source = write_source(temp.path(), "mirror.pbrt", &text);
    let (_store, session, report) = import_project_with_report(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    assert_eq!(
        scene(session.document()).coordinate_system,
        CoordinateSystem::new(Axis::PosY, Handedness::Right)
    );
    assert_eq!(
        coordinate_diagnostic(&report).message,
        "coordinate system: +Y up, right-handed (camera has a mirror)"
    );
}

#[test]
fn a_z_mirror_flips_the_up_image_with_the_handedness() {
    let temp = tempfile::tempdir().unwrap();
    // `Scale 1 1 -1` in the camera chain mirrors the world the camera
    // sees about Z, so the world image of camera-space +y flips to -Z
    // while the mirror still reads as right-handed.
    let text = format!(
        "LookAt 0 -10 3  0 0 0  0 0 1\n\
Scale 1 1 -1\n\
Camera \"perspective\" \"float fov\" [40]\n\
{FILM}\
WorldBegin\n\
Shape \"sphere\" \"float radius\" [1]\n\
WorldEnd\n"
    );
    let source = write_source(temp.path(), "mirror-z-world.pbrt", &text);
    let (_store, session, report) = import_project_with_report(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    assert_eq!(
        scene(session.document()).coordinate_system,
        CoordinateSystem::new(Axis::NegZ, Handedness::Right)
    );
    assert!(
        coordinate_diagnostic(&report)
            .message
            .contains("right-handed (camera has a mirror)"),
        "the mirror is reported: {:?}",
        coordinate_diagnostic(&report).message
    );
}

#[test]
fn export_reimport_estimates_the_same_y_up_left_handed_scene() {
    let temp = tempfile::tempdir().unwrap();
    let text = format!(
        "LookAt 0 2 -6  0 0.5 0  0 1 0\n\
Camera \"perspective\" \"float fov\" [40]\n\
{FILM}\
WorldBegin\n\
Shape \"sphere\" \"float radius\" [1]\n\
WorldEnd\n"
    );
    let source = write_source(temp.path(), "y-up.pbrt", &text);
    let (store, session, _report) = import_project_with_report(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let bundle = temp.path().join("bundle");
    let exported = export_project(&session.snapshot(), &store, &bundle, || false).unwrap();
    let (_, reimported, report) = import_project_with_report(
        &exported,
        &temp.path().join("reimport"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    assert_eq!(
        scene(reimported.document()).coordinate_system,
        CoordinateSystem::default()
    );
    assert_eq!(
        coordinate_diagnostic(&report).message,
        "coordinate system: +Y up, left-handed"
    );
}

#[test]
fn export_reimport_finds_the_mirror_in_a_z_up_right_handed_scene() {
    let temp = tempfile::tempdir().unwrap();
    // A right-handed-source export: the canonical `Scale -1 1 1` mirror
    // flips the handedness while the up axis stays +Z.
    let text = format!(
        "LookAt 0 -10 3  0 0 0  0 0 1\n\
Scale -1 1 1\n\
Camera \"perspective\" \"float fov\" [40]\n\
{FILM}\
WorldBegin\n\
Shape \"sphere\" \"float radius\" [1]\n\
WorldEnd\n"
    );
    let source = write_source(temp.path(), "mirror-z.pbrt", &text);
    let (store, session, _report) = import_project_with_report(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let expected = CoordinateSystem::new(Axis::PosZ, Handedness::Right);
    assert_eq!(scene(session.document()).coordinate_system, expected);
    let bundle = temp.path().join("bundle");
    let exported = export_project(&session.snapshot(), &store, &bundle, || false).unwrap();
    let (_, reimported, report) = import_project_with_report(
        &exported,
        &temp.path().join("reimport"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    assert_eq!(scene(reimported.document()).coordinate_system, expected);
    assert!(
        coordinate_diagnostic(&report)
            .message
            .contains("(camera has a mirror)"),
        "the mirror survives the round trip: {:?}",
        coordinate_diagnostic(&report).message
    );
}

#[test]
fn disagreeing_cameras_report_the_render_camera_choice() {
    let temp = tempfile::tempdir().unwrap();
    // Two cameras: the last one renders (PBRT order), so the estimate
    // follows it and the disagreement is reported.
    let text = format!(
        "LookAt 0 2 -6  0 0.5 0  0 1 0\n\
Camera \"perspective\" \"float fov\" [40]\n\
LookAt 0 -10 3  0 0 0  0 0 1\n\
Camera \"perspective\" \"float fov\" [50]\n\
{FILM}\
WorldBegin\n\
Shape \"sphere\" \"float radius\" [1]\n\
WorldEnd\n"
    );
    let source = write_source(temp.path(), "two-cameras.pbrt", &text);
    let (_store, session, report) = import_project_with_report(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    assert_eq!(
        scene(session.document()).coordinate_system,
        CoordinateSystem::new(Axis::PosZ, Handedness::Left)
    );
    assert_eq!(
        coordinate_diagnostic(&report).message,
        "coordinate system: +Z up, left-handed; \
another camera suggests +Y up, left-handed"
    );
}

#[test]
fn cameraless_scenes_default_to_y_up_left_handed() {
    let temp = tempfile::tempdir().unwrap();
    let text = format!(
        "{FILM}\
WorldBegin\n\
Shape \"sphere\" \"float radius\" [1]\n\
WorldEnd\n"
    );
    let source = write_source(temp.path(), "no-camera.pbrt", &text);
    let (_store, session, report) = import_project_with_report(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    assert_eq!(
        scene(session.document()).coordinate_system,
        CoordinateSystem::default()
    );
    // No camera, no source line to point at: the scene keeps the default
    // silently instead of adding an origin-less diagnostic.
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "scene_coordinate_system"),
        "cameraless scenes stay silent: {:?}",
        report.diagnostics
    );
}

#[test]
fn prepared_scenes_carry_the_inference_too() {
    let temp = tempfile::tempdir().unwrap();
    let text = format!(
        "LookAt 0 -10 3  0 0 0  0 0 1\n\
Camera \"perspective\" \"float fov\" [40]\n\
{FILM}\
WorldBegin\n\
Shape \"sphere\" \"float radius\" [1]\n\
WorldEnd\n"
    );
    let source = write_source(temp.path(), "prepared.pbrt", &text);
    let (prepared, report) = prepare_scene(&source, &ImportOptions::default(), || false).unwrap();
    assert_eq!(
        prepared
            .document()
            .scenes
            .values()
            .next()
            .unwrap()
            .coordinate_system,
        CoordinateSystem::new(Axis::PosZ, Handedness::Left),
        "the Add-Scene-to-Project path estimates too"
    );
    assert!(
        coordinate_diagnostic(&report)
            .message
            .starts_with("coordinate system: +Z up, left-handed")
    );
}

/// A real-scene survey of the import estimate: the official
/// pbrt-v4 scenes in the fixture set, through the same
/// parse → resolve → infer steps the importer takes. The eyeball is
/// the assertion, so one row per scene prints the estimate and every
/// disagreeing camera; nothing about the rows is committed as data.
/// Machine-bound, so ignored without the checkout: pass
/// `PBRT_V4_SCENES=<path>`, or keep it at `../pbrt-v4-scenes`.
#[test]
#[ignore = "requires the pbrt-v4-scenes checkout (PBRT_V4_SCENES=<path> or ../pbrt-v4-scenes)"]
fn real_scene_estimates_survey_what_the_importer_would_report() {
    let root = std::env::var_os("PBRT_V4_SCENES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../pbrt-v4-scenes")
        });
    let Ok(root) = protocol::path::canonicalize(root) else {
        eprintln!("skipped: no pbrt-v4-scenes checkout; set PBRT_V4_SCENES");
        return;
    };
    for tail in [
        "killeroos/killeroo-simple.pbrt",
        "killeroos/killeroo-gold.pbrt",
        "bmw-m6/bmw-m6.pbrt",
        "barcelona-pavilion/pavilion-day.pbrt",
        "sportscar/sportscar-sky.pbrt",
        "clouds/clouds.pbrt",
    ] {
        let started = std::time::Instant::now();
        let mut document = pbrt_io::parse_unresolved(&root.join(tail), &ImportOptions::default())
            .unwrap_or_else(|error| panic!("{tail} parses: {error}"));
        let scene_id = document.default_scene.expect("import produced a scene");
        project::resolve_transform_parents(&mut document, scene_id)
            .unwrap_or_else(|error| panic!("{tail} resolves: {error}"));
        let scene = document.scene(scene_id).unwrap();
        let inference = project::coordinate::infer_scene(scene);
        let conflicts = inference
            .conflicts
            .iter()
            .map(|(_, estimate)| estimate.to_string())
            .collect::<Vec<_>>();
        eprintln!(
            "{tail}: {}{} ({:?})",
            inference.coordinate_system,
            if conflicts.is_empty() {
                String::new()
            } else {
                format!("; disagrees: {conflicts:?}")
            },
            started.elapsed()
        );
    }
}
