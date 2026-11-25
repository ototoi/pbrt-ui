use scene_view::{PreviewMode, ViewerCamera, render_preview};
use std::sync::Mutex;
static GPU: Mutex<()> = Mutex::new(());

fn parsed(source: &str) -> scene_parse::PreviewGeometry {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("area.pbrt");
    std::fs::write(&path, source).unwrap();
    scene_parse::parse_scene(path, scene_parse::ParseOptions::default()).unwrap()
}

fn render(source: &str) -> scene_view::PreviewFrame {
    let scene = parsed(source);
    let camera = ViewerCamera::from_manifest(&scene.manifest, 1.);
    render_preview(
        &scene.manifest,
        &scene.blobs,
        camera,
        64,
        64,
        PreviewMode::Lit,
    )
    .unwrap()
}

/// Camera at z = 0.5 looking straight down at a floor at z = 0; the light
/// quad hangs at z = 2, behind the eye, so only the floor it lights is in
/// frame.
const SCENE: &str = concat!(
    "Film \"rgb\" \"integer xresolution\" 64 \"integer yresolution\" 64\n",
    "LookAt 0 0 0.5 0 0 0 0 1 0\n",
    "Camera \"perspective\" \"float fov\" 30\n",
    "WorldBegin\n",
    "Material \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\n",
    "Shape \"trianglemesh\" \"point3 P\" [-4 -4 0 4 -4 0 4 4 0 -4 4 0] ",
    "\"integer indices\" [0 1 2 0 2 3]\n",
    "AttributeBegin\n",
    "Translate 0 0 2\n",
);

/// A `blackbody L` area light must render like the same light spelled as
/// its luminance-normalised RGB (`blackbody_rgb`), the way r4 reads both:
/// Lit interprets the kelvin value as a temperature and uses its normalized color.
#[test]
#[ignore = "requires a GPU adapter"]
fn area_blackbody_light_renders_like_its_rgb_equivalent() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let rgb = scene_view::preview::blackbody_rgb(6500.);
    let rgb_values = format!("{:?} {:?} {:?}", rgb[0], rgb[1], rgb[2]);
    let shape = concat!(
        "Shape \"trianglemesh\" \"point3 P\" [-2 -2 0 2 -2 0 2 2 0 -2 2 0] ",
        "\"integer indices\" [0 2 1 0 3 2]\n",
        "AttributeEnd\n"
    );
    let blackbody = format!("{SCENE}AreaLightSource \"diffuse\" \"blackbody L\" [6500]\n{shape}");
    let equivalent =
        format!("{SCENE}AreaLightSource \"diffuse\" \"rgb L\" [{rgb_values}]\n{shape}");
    let blackbody_frame = render(&blackbody);
    let equivalent_frame = render(&equivalent);
    assert_eq!(blackbody_frame.rgba.len(), equivalent_frame.rgba.len());
    let mut worst = 0;
    for (a, b) in blackbody_frame
        .rgba
        .iter()
        .zip(equivalent_frame.rgba.iter())
    {
        worst = worst.max(a.abs_diff(*b));
    }
    assert!(worst <= 1, "frames differ by {worst}/255");
    // The lit floor must be neither black nor saturated: the calibration
    // would be vacuous if one spelling rendered flat white.
    let i = ((blackbody_frame.height / 2 * blackbody_frame.width + blackbody_frame.width / 2) * 4)
        as usize;
    for channel in 0..3 {
        let value = blackbody_frame.rgba[i + channel];
        assert!(value > 5 && value < 251, "floor pixel {value} is extreme");
    }
}
