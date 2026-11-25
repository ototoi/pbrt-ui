//! Lit screen-space AO (GTAO) from the PreZ depth and normals: convex
//! surfaces stay unoccluded, a sphere darkens the floor at its contact, the
//! screen edge adds no occlusion, and far coordinates change nothing.
use scene_view::{LitDebug, PreviewMode, PreviewRenderer, ViewerCamera};

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

const SIZE: u32 = 128;

/// A unit sphere resting on a 6×6 floor at y = 0, both moved by `offset`.
fn contact_scene(offset: [f64; 3]) -> String {
    let [x, y, z] = offset;
    format!(
        r#"WorldBegin
LightSource "infinite" "rgb L" [1 1 1]
AttributeBegin
Translate {x} {y} {z}
Shape "trianglemesh" "point3 P" [-3 0 -3 3 0 -3 3 0 3 -3 0 3] "integer indices" [0 1 2 0 2 3]
AttributeBegin
Translate 0 1 0
Shape "sphere" "float radius" [1]
AttributeEnd
AttributeEnd
"#
    )
}

fn camera(eye: [f64; 3], target: [f64; 3]) -> ViewerCamera {
    ViewerCamera {
        eye,
        target,
        up: [0., 1., 0.],
        fov_y_degrees: 45.,
        near: 0.01,
        far: 100.,
        mirror: false,
    }
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] + b[i])
}

/// Renders `source` in Lit and returns AO and depth, row by row.
fn ao(source: &str, camera: ViewerCamera) -> (Vec<f32>, Vec<f32>) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("scene.pbrt");
    std::fs::write(&path, source).unwrap();
    let scene = scene_parse::parse_scene(path, scene_parse::ParseOptions::default()).unwrap();
    let mut renderer = PreviewRenderer::new().unwrap();
    renderer
        .upload_scene(&scene.manifest, &scene.blobs)
        .unwrap();
    renderer
        .render(camera, SIZE, SIZE, PreviewMode::Lit, 0.)
        .unwrap();
    let ao = renderer.read_ao().unwrap();
    let (depth, _) = renderer.read_prez().unwrap();
    (ao, depth)
}

/// The pixel a world point lands in (no roll: up is +y).
fn pixel(camera: &ViewerCamera, p: [f64; 3]) -> usize {
    let sub = |a: [f64; 3], b: [f64; 3]| -> [f64; 3] { std::array::from_fn(|i| a[i] - b[i]) };
    let dot = |a: [f64; 3], b: [f64; 3]| -> f64 { (0..3).map(|i| a[i] * b[i]).sum() };
    let cross = |a: [f64; 3], b: [f64; 3]| -> [f64; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let unit = |a: [f64; 3]| {
        let l = dot(a, a).sqrt();
        a.map(|v| v / l)
    };
    let forward = unit(sub(camera.target, camera.eye));
    let right = unit(cross(camera.up, forward));
    let up = cross(forward, right);
    let d = sub(p, camera.eye);
    let t = (camera.fov_y_degrees.to_radians() / 2.).tan();
    let z = dot(d, forward);
    let sx = dot(d, right) / z / t;
    let sy = dot(d, up) / z / t;
    let x = ((sx + 1.) / 2. * SIZE as f64) as usize;
    let y = ((1. - sy) / 2. * SIZE as f64) as usize;
    y * SIZE as usize + x
}

#[test]
#[ignore = "requires a GPU adapter"]
fn convex_surfaces_stay_unoccluded() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    // A lone sphere, and a grazing plane reaching the screen edges.
    for (source, camera) in [
        (
            "WorldBegin\nLightSource \"infinite\" \"rgb L\" [1 1 1]\nShape \"sphere\" \"float radius\" [1]\n",
            camera([0., 0.5, 3.5], [0.; 3]),
        ),
        (
            "WorldBegin\nLightSource \"infinite\" \"rgb L\" [1 1 1]\nShape \"trianglemesh\" \"point3 P\" [-50 0 -50 50 0 -50 50 0 50 -50 0 50] \"integer indices\" [0 1 2 0 2 3]\n",
            camera([0., 1., 0.], [0., 0., -5.]),
        ),
    ] {
        let (ao, depth) = ao(source, camera);
        let covered: Vec<f32> = ao
            .iter()
            .zip(&depth)
            .filter(|(_, d)| **d > 0.)
            .map(|(a, _)| *a)
            .collect();
        assert!(covered.len() > 1000, "{source}");
        let lowest = covered.iter().copied().fold(1., f32::min);
        assert!(lowest > 0.95, "{source}: lowest AO {lowest}");
        assert!(ao.iter().zip(&depth).all(|(a, d)| *d > 0. || *a == 1.));
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_sphere_darkens_the_floor_at_its_contact_even_far_away() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let mut contact = Vec::new();
    for offset in [[0.; 3], [1.0e5, 2.0e4, -1.0e5]] {
        let camera = camera(add([0., 2., 4.], offset), add([0., 0.6, 0.], offset));
        let (ao, _) = ao(&contact_scene(offset), camera);
        // On the floor just in front of the contact point.
        let near = ao[pixel(&camera, add([0., 0., 0.25], offset))];
        // On the floor beyond the AO radius from the sphere.
        let open = ao[pixel(&camera, add([2.2, 0., 1.5], offset))];
        // The top of the sphere.
        let top = ao[pixel(&camera, add([0., 1.95, 0.3], offset))];
        assert!(near < 0.75, "contact AO {near} at offset {offset:?}");
        assert!(open > 0.95, "open floor AO {open} at offset {offset:?}");
        assert!(top > 0.95, "sphere top AO {top} at offset {offset:?}");
        contact.push(near);
    }
    assert!(
        (contact[0] - contact[1]).abs() < 0.03,
        "far coordinates change AO: {contact:?}"
    );
}

/// The AO capture shows the AO buffer as gray, sRGB-encoded like any color.
#[test]
#[ignore = "requires a GPU adapter"]
fn the_ao_capture_shows_the_ao_buffer() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("scene.pbrt");
    std::fs::write(&path, contact_scene([0.; 3])).unwrap();
    let scene = scene_parse::parse_scene(path, scene_parse::ParseOptions::default()).unwrap();
    let camera = camera([0., 2., 4.], [0., 0.6, 0.]);
    let mut renderer = PreviewRenderer::new().unwrap();
    renderer
        .upload_scene(&scene.manifest, &scene.blobs)
        .unwrap();
    let frame = renderer
        .render_with_options(camera, SIZE, SIZE, PreviewMode::Solid, 0., LitDebug::Ao)
        .unwrap();
    let ao = renderer.read_ao().unwrap();
    let srgb = |v: f32| {
        let v = if v <= 0.0031308 {
            12.92 * v
        } else {
            1.055 * v.powf(1. / 2.4) - 0.055
        };
        (v * 255.).round() as i32
    };
    for (i, ao) in ao.iter().enumerate() {
        let rgb = &frame.rgba[i * 4..i * 4 + 3];
        assert!(
            rgb[0] == rgb[1] && rgb[1] == rgb[2],
            "pixel {i} is gray: {rgb:?}"
        );
        assert!(
            (rgb[0] as i32 - srgb(*ao)).abs() <= 1,
            "pixel {i}: {rgb:?} for AO {ao}"
        );
    }
    let near = pixel(&camera, [0., 0., 0.25]);
    assert!(frame.rgba[near * 4] < 230, "the contact shows dark");
    // Picking still works in the AO capture.
    assert_ne!(frame.pick_ids[near], 0);
}
