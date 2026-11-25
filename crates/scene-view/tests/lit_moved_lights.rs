//! The GPU side of a light delta: `update_scene`
//! with a moved `world_from_light` relights at the new position while the
//! meshes stay uploaded (the desktop's manifest override costs no
//! re-transfer), and `update_draws` rebinds a moved area-light shape's
//! emitter from the draw, the same retarget `apply_transform_deltas`
//! produces. The brightness peak over the floor follows the light.

use scene_view::{PreviewMode, ViewerCamera};
use std::sync::Mutex;

static GPU: Mutex<()> = Mutex::new(());

const HEADER_96: &str = "Film \"rgb\" \"integer xresolution\" 96 \"integer yresolution\" 96\n";
const LOOK_DOWN: &str = "LookAt 0 0 10 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" 28\n";
const FLOOR: &str = "Material \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\nShape \"trianglemesh\" \"point3 P\" [-4 -4 0 4 -4 0 4 4 0 -4 4 0] \"integer indices\" [0 2 1 0 3 2]\n";

fn parsed(source: &str) -> scene_parse::PreviewGeometry {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lit-moved.pbrt");
    std::fs::write(&path, source).unwrap();
    scene_parse::parse_scene(path, scene_parse::ParseOptions::default()).unwrap()
}

/// Row-major `a · b`, the convention every preview matrix uses — the same
/// product the desktop's fast path retargets with.
fn product(a: [f64; 16], b: [f64; 16]) -> [f64; 16] {
    std::array::from_fn(|i| {
        let (row, column) = (i / 4, i % 4);
        (0..4).map(|k| a[row * 4 + k] * b[k * 4 + column]).sum()
    })
}

fn translation(x: f64, y: f64, z: f64) -> [f64; 16] {
    [1., 0., 0., x, 0., 1., 0., y, 0., 0., 1., z, 0., 0., 0., 1.]
}

/// The brightest floor pixel: (index, brightness).
fn floor_peak(frame: &scene_view::PreviewFrame, floor: u32) -> (usize, u8) {
    frame
        .pick_ids
        .iter()
        .enumerate()
        .filter(|(_, id)| **id == floor)
        .map(|(index, _)| (index, frame.rgba[index * 4]))
        .max_by_key(|(_, brightness)| *brightness)
        .expect("the camera sees the measured floor")
}

/// After a moved light the peak brightness over the floor follows the
/// move: the same light, still on, still fully bright — only its place
/// changed (and nothing re-uploads: the GPUs meshes are the desktop's
/// whole saving).
#[test]
#[ignore = "requires a GPU adapter"]
fn a_moved_point_light_relights_through_update_scene() {
    let _guard = GPU.lock().unwrap_or_else(|error| error.into_inner());
    let scene = parsed(&format!(
        "{HEADER_96}{LOOK_DOWN}WorldBegin\n\
         LightSource \"point\" \"point from\" [-2 0 3] \"rgb I\" [40 40 40]\n{FLOOR}"
    ));
    let floor = scene.manifest.draws[0].pick_id;
    let mut renderer = scene_view::PreviewRenderer::new().unwrap();
    renderer
        .upload_scene(&scene.manifest, &scene.blobs)
        .unwrap();
    let camera = ViewerCamera::from_manifest(&scene.manifest, 1.0);
    let before = renderer
        .render(camera, 96, 96, PreviewMode::Lit, 0.)
        .unwrap();
    let (before_index, before_bright) = floor_peak(&before, floor);
    let uploads = renderer.stats().mesh_uploads;
    let before_x = before_index as i32 % 96;

    // The fast path's move: `delta · world_from_light` over the base.
    let mut moved = scene.manifest.clone();
    assert_eq!(moved.lights.len(), 1, "the fixture declares one light");
    let delta = translation(4., 0., 0.);
    moved.lights[0].world_from_light = product(delta, moved.lights[0].world_from_light);
    renderer
        .update_scene(&moved, &scene.blobs, &std::collections::BTreeMap::new())
        .unwrap();
    let after = renderer
        .render(camera, 96, 96, PreviewMode::Lit, 0.)
        .unwrap();
    let (after_index, after_bright) = floor_peak(&after, floor);
    let after_x = after_index as i32 % 96;
    // The screen's x may mirror a right-handed scene, so the
    // assertion is the distance: 4 world units at this camera are about
    // 76 px of floor, and the peak must move with the light.
    assert!(
        (after_x - before_x).abs() > 30,
        "the brightness peak moved with the light: {before_x} -> {after_x}"
    );
    assert_eq!(
        renderer.stats().mesh_uploads,
        uploads,
        "a moved light re-evaluates Lit without re-uploading meshes"
    );
    // The same light, still on: the peak keeps its brightness, and the
    // old peak dims because the light left.
    assert!(
        (after_bright as i32 - before_bright as i32).abs() <= 2,
        "the peak brightness stays: {before_bright} -> {after_bright}"
    );
    assert!(
        after.rgba[before_index * 4] + 8 <= before_bright,
        "the floor below the old light position dims: {} -> {}",
        before_bright,
        after.rgba[before_index * 4]
    );
}

/// A moved area-light SHAPE relights through `update_draws`: `lit::rebind`
/// rebuilds the emitter from the draw, so the same retargeted matrix the
/// fast path installs moves the light.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_moved_area_light_shape_relights_through_update_draws() {
    let _guard = GPU.lock().unwrap_or_else(|error| error.into_inner());
    let scene = parsed(&format!(
        "{HEADER_96}{LOOK_DOWN}WorldBegin\nAttributeBegin\nTranslate 2 0 3\n\
         AreaLightSource \"diffuse\" \"rgb L\" [10 10 10]\n\
         Shape \"trianglemesh\" \"point3 P\" [-0.5 -0.5 0 0.5 -0.5 0 0.5 0.5 0 -0.5 0.5 0] \"integer indices\" [0 2 1 0 3 2]\n\
         AttributeEnd\n{FLOOR}"
    ));
    let floor = scene.manifest.draws[1].pick_id;
    let mut renderer = scene_view::PreviewRenderer::new().unwrap();
    renderer
        .upload_scene(&scene.manifest, &scene.blobs)
        .unwrap();
    let camera = ViewerCamera::from_manifest(&scene.manifest, 1.0);
    let before = renderer
        .render(camera, 96, 96, PreviewMode::Lit, 0.)
        .unwrap();
    let (before_index, before_bright) = floor_peak(&before, floor);
    let before_x = before_index as i32 % 96;

    // The fast path's draw retarget: `delta · world_from_object` for
    // every sample of the moved emitter draw.
    let delta = translation(-4., 0., 0.);
    let draws = scene
        .manifest
        .draws
        .iter()
        .map(|draw| {
            let mut draw = draw.clone();
            draw.world_from_object = draw
                .world_from_object
                .iter()
                .map(|matrix| product(delta, *matrix))
                .collect();
            draw
        })
        .collect::<Vec<_>>();
    renderer.update_draws(&draws).unwrap();
    let after = renderer
        .render(camera, 96, 96, PreviewMode::Lit, 0.)
        .unwrap();
    let (after_index, after_bright) = floor_peak(&after, floor);
    let after_x = after_index as i32 % 96;
    // The screen's x may mirror a right-handed scene, so the
    // assertion is the distance: the emitter moved 4 world units.
    assert!(
        (after_x - before_x).abs() > 30,
        "the emitter's brightness peak moved with the shape: {before_x} -> {after_x}"
    );
    assert!(
        (after_bright as i32 - before_bright as i32).abs() <= 2,
        "the peak brightness stays: {before_bright} -> {after_bright}"
    );
    assert!(
        after.rgba[before_index * 4] + 8 <= before_bright,
        "the floor below the old emitter position dims: {} -> {}",
        before_bright,
        after.rgba[before_index * 4]
    );
}
