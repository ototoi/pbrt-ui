use scene_view::{PreviewMode, ViewerCamera, render_preview};
use std::sync::Mutex;
static GPU: Mutex<()> = Mutex::new(());

fn parsed(source: &str) -> scene_parse::PreviewGeometry {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("emitter.pbrt");
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

/// sRGB-encode a linear value, as the preview output does.
fn srgb(v: f64) -> u8 {
    let x = if v <= 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.powf(1. / 2.4) - 0.055
    };
    (x.clamp(0., 1.) * 255.).round() as u8
}

fn center(frame: &scene_view::PreviewFrame) -> [u8; 3] {
    let i = ((frame.height / 2 * frame.width + frame.width / 2) * 4) as usize;
    [frame.rgba[i], frame.rgba[i + 1], frame.rgba[i + 2]]
}

fn near_rgb(got: [u8; 3], want: u8, tol: u8) {
    for c in got {
        assert!(c.abs_diff(want) <= tol, "{got:?} != {want}");
    }
}

/// The calibration framing of em1.pbrt / em2.pbrt: the camera at the origin
/// stares straight at the emitter quad at z = 3, so the centre pixels land
/// on the emitter's own surface.
const SCENE: &str = concat!(
    "Film \"rgb\" \"integer xresolution\" 64 \"integer yresolution\" 64\n",
    "LookAt 0 0 0  0 0 3  0 1 0\n",
    "Camera \"perspective\" \"float fov\" 40\n",
    "WorldBegin\n",
    "AttributeBegin\n",
    "AreaLightSource \"diffuse\" {LIGHT}\n",
    "Shape \"trianglemesh\" \"point3 P\" [-1 -1 3 1 -1 3 1 1 3 -1 1 3] ",
    "\"integer indices\" [0 2 1 0 3 2]\n",
    "AttributeEnd\n",
);

fn scene_with(light: &str) -> String {
    SCENE.replace("{LIGHT}", light)
}

fn assert_frames_agree(a: &scene_view::PreviewFrame, b: &scene_view::PreviewFrame) {
    assert_eq!(a.rgba.len(), b.rgba.len());
    let mut worst = 0;
    for (a, b) in a.rgba.iter().zip(b.rgba.iter()) {
        worst = worst.max(a.abs_diff(*b));
    }
    assert!(worst <= 1, "frames differ by {worst}/255");
}

/// A `scale 0.1` emitter's own surface renders like the same emitter
/// spelled `rgb L [0.1 0.1 0.1]` — r4 reads 0.099 off the board, pre-fix
/// Lit saturated at 1 before the fix.
#[test]
#[ignore = "requires a GPU adapter"]
fn scaled_emitter_surface_renders_like_its_dim_rgb_equivalent() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let scaled = render(&scene_with("\"rgb L\" [1 1 1] \"float scale\" [0.1]"));
    let dim = render(&scene_with("\"rgb L\" [0.1 0.1 0.1]"));
    assert_frames_agree(&scaled, &dim);
    near_rgb(center(&scaled), srgb(0.1), 2);
    assert!(
        center(&scaled)[0] < 250,
        "the emitter surface saturates: {:?}",
        center(&scaled)
    );
}

/// A scaled blackbody emitter's surface renders like the same emitter
/// spelled with `blackbody_rgb`'s luminance-1 colour × scale.
#[test]
#[ignore = "requires a GPU adapter"]
fn scaled_blackbody_emitter_surface_renders_like_its_dim_rgb_equivalent() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let rgb = scene_view::preview::blackbody_rgb(5500.).map(|v| v * 0.1);
    let light = format!("\"rgb L\" [{:?} {:?} {:?}]", rgb[0], rgb[1], rgb[2]);
    let blackbody = render(&scene_with("\"blackbody L\" [5500] \"float scale\" [0.1]"));
    let equivalent = render(&scene_with(&light));
    assert_frames_agree(&blackbody, &equivalent);
    let got = center(&blackbody);
    for (channel, value) in got.iter().zip(rgb.iter()) {
        assert!(
            channel.abs_diff(srgb(*value)) <= 2,
            "{got:?} != srgb({rgb:?})"
        );
    }
}
