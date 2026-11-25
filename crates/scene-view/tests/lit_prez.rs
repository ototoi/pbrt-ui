//! Lit PreZ: the depth and eye-facing normals of the opaque draws, which the
//! opaque pass then tests `Equal` and screen-space AO reads. Glass stays out
//! and alpha cutouts leave their holes.
use scene_view::{PreviewMode, PreviewRenderer, ViewerCamera};

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

const SIZE: u32 = 96;
const EYE: [f64; 3] = [0., 0., 5.];
const TAN: f64 = 0.41421356237309503; // tan(22.5°)
// pbrt's LookAt is left-handed: camera-right is up × forward.
const RIGHT: [f64; 3] = [-1., 0., 0.];

const SCENE: &str = r#"LookAt 0 0 5  0 0 0  0 1 0
Camera "perspective" "float fov" [45]
WorldBegin
LightSource "infinite" "rgb L" [1 1 1]
Texture "a" "float" "checkerboard" "float uscale" [2] "float vscale" [1] "float tex1" [1] "float tex2" [0]
AttributeBegin
# Wound to face away from the eye: PreZ must face its normal toward it.
Shape "trianglemesh" "point3 P" [-2 -2 -2 2 -2 -2 2 2 -2 -2 2 -2] "integer indices" [0 2 1 0 3 2]
AttributeEnd
AttributeBegin
Material "dielectric"
Shape "trianglemesh" "point3 P" [0.3 0.3 1 1.1 0.3 1 1.1 1.1 1 0.3 1.1 1] "integer indices" [0 1 2 0 2 3]
AttributeEnd
# Solid where u < 0.5 (x < -0.7), a hole elsewhere.
Shape "trianglemesh" "point3 P" [-1.1 0.3 1 -0.3 0.3 1 -0.3 1.1 1 -1.1 1.1 1] "texcoord2 uv" [0 0 1 0 1 1 0 1] "integer indices" [0 1 2 0 2 3] "texture alpha" "a"
# A roof with a ridge along y: two 45° faces sharing the ridge vertices.
Shape "trianglemesh" "point3 P" [1 -0.3 0 1.5 -0.3 0.5 2 -0.3 0 1 0.3 0 1.5 0.3 0.5 2 0.3 0] "integer indices" [0 1 4 0 4 3 1 2 5 1 5 4]
# A mix with a transmitting leaf, tilted: it stays out of PreZ.
AttributeBegin
MakeNamedMaterial "clear" "string type" "dielectric"
MakeNamedMaterial "matte" "string type" "diffuse"
Material "mix" "string materials" ["matte" "clear"] "float amount" [0.5]
Shape "trianglemesh" "point3 P" [-1.5 -0.3 0.3 -0.9 -0.3 0 -0.9 0.3 0 -1.5 0.3 0.3] "integer indices" [0 1 2 0 2 3]
AttributeEnd
AttributeBegin
Translate 0 -1 0
Shape "sphere" "float radius" [0.6]
AttributeEnd
"#;

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}

/// The pixel a world point lands in.
fn pixel(p: [f64; 3]) -> usize {
    let d = sub(p, EYE);
    let z = -d[2];
    let sx = dot(d, RIGHT) / z / TAN;
    let sy = d[1] / z / TAN;
    let x = ((sx + 1.) / 2. * SIZE as f64) as usize;
    let y = ((1. - sy) / 2. * SIZE as f64) as usize;
    y * SIZE as usize + x
}

/// The unit ray through a pixel's center.
fn ray(index: usize) -> [f64; 3] {
    let (x, y) = (index % SIZE as usize, index / SIZE as usize);
    let sx = ((x as f64 + 0.5) / SIZE as f64 * 2. - 1.) * TAN;
    let sy = (1. - (y as f64 + 0.5) / SIZE as f64 * 2.) * TAN;
    let d = [sx * RIGHT[0], sy, -1.];
    let l = dot(d, d).sqrt();
    d.map(|v| v / l)
}

fn assert_normal(actual: [f32; 3], expected: [f64; 3], tolerance: f64, what: &str) {
    let error = (0..3)
        .map(|i| (actual[i] as f64 - expected[i]).abs())
        .fold(0., f64::max);
    assert!(
        error < tolerance,
        "{what}: normal {actual:?}, expected {expected:?}"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn prez_writes_opaque_depth_and_eye_facing_normals() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("scene.pbrt");
    std::fs::write(&path, SCENE).unwrap();
    let scene = scene_parse::parse_scene(path, scene_parse::ParseOptions::default()).unwrap();
    let camera = ViewerCamera {
        eye: EYE,
        target: [0.; 3],
        up: [0., 1., 0.],
        fov_y_degrees: 45.,
        near: 0.1,
        far: 100.,
        mirror: false,
    };
    let mut renderer = PreviewRenderer::new().unwrap();
    renderer
        .upload_scene(&scene.manifest, &scene.blobs)
        .unwrap();
    renderer
        .render(camera, SIZE, SIZE, PreviewMode::Lit, 0.)
        .unwrap();
    let (depth, normal) = renderer.read_prez().unwrap();
    // Depth is read after the frame, when a transmitting mix has written its
    // own; the normals are PreZ's alone.
    let up = [0., 0., 1.];

    assert_eq!(depth[0], 0., "background keeps the cleared depth");
    let backdrop = pixel([1.6, -1.6, -2.]);
    let far = depth[backdrop];
    assert!(far > 0.);
    assert_normal(normal[backdrop], up, 0.01, "backdrop");

    let glass = pixel([0.7, 0.7, 1.]);
    assert_eq!(depth[glass], far, "glass stays out of PreZ");
    assert_normal(normal[glass], up, 0.01, "behind glass");

    let solid = pixel([-0.9, 0.7, 1.]);
    assert!(depth[solid] > far, "the cutout's solid half is in front");
    assert_normal(normal[solid], up, 0.01, "cutout");
    let hole = pixel([-0.5, 0.7, 1.]);
    assert_eq!(depth[hole], far, "the cutout's hole shows the backdrop");

    assert_normal(normal[pixel([-1.2, 0., 0.15])], up, 0.01, "behind the mix");

    // Smooth normals: the analytic sphere normal where the pixel's ray hits
    // it, within the tessellation's interpolation error (facet normals are off
    // by more).
    let center = [0., -1., 0.];
    for offset in [[0., 0., 0.6], [0.35, 0.2, 0.45], [-0.4, -0.3, 0.37]] {
        let index = pixel(std::array::from_fn(|i| center[i] + offset[i]));
        let d = ray(index);
        let oc = sub(EYE, center);
        let b = dot(oc, d);
        let t = -b - (b * b - (dot(oc, oc) - 0.36)).sqrt();
        let hit: [f64; 3] = std::array::from_fn(|i| EYE[i] + t * d[i]);
        let n = sub(hit, center).map(|v| v / 0.6);
        assert_normal(normal[index], n, 0.05, "sphere");
    }
    // Smooth, not facet, normals: near the roof's ridge the normal leans far
    // less than the 45° left face (x = -0.707).
    let near_ridge = normal[pixel([1.4, 0., 0.4])];
    assert!(
        near_ridge[0] < 0. && near_ridge[0] > -0.5,
        "roof normal near the ridge {near_ridge:?}"
    );
}

/// Coincident opaque surfaces: the first draw wins color, pick and normals,
/// as PreZ's `Greater` keeps it (the opaque pass draws in reverse).
#[test]
#[ignore = "requires a GPU adapter"]
fn the_first_of_coincident_opaque_draws_wins() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let quad = "Shape \"trianglemesh\" \"point3 P\" [-1 -1 0 1 -1 0 1 1 0 -1 1 0] \"integer indices\" [0 1 2 0 2 3]";
    let source = format!(
        "LookAt 0 0 5  0 0 0  0 1 0\nCamera \"perspective\" \"float fov\" [45]\nWorldBegin\nLightSource \"infinite\" \"rgb L\" [1 1 1]\nAttributeBegin\nMaterial \"diffuse\" \"rgb reflectance\" [0.8 0 0]\n{quad}\nAttributeEnd\nAttributeBegin\nMaterial \"diffuse\" \"rgb reflectance\" [0 0.8 0]\n{quad}\nAttributeEnd\n"
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("scene.pbrt");
    std::fs::write(&path, source).unwrap();
    let scene = scene_parse::parse_scene(path, scene_parse::ParseOptions::default()).unwrap();
    let camera = ViewerCamera::from_manifest(&scene.manifest, 1.0);
    let frame = scene_view::render_preview(
        &scene.manifest,
        &scene.blobs,
        camera,
        32,
        32,
        PreviewMode::Lit,
    )
    .unwrap();
    let i = (16 * 32 + 16) as usize;
    let rgb = &frame.rgba[i * 4..i * 4 + 3];
    assert!(
        rgb[0] > 100 && rgb[1] < 20,
        "the red, first quad shows: {rgb:?}"
    );
    assert_eq!(frame.pick_ids[i], scene.manifest.draws[0].pick_id);
}
