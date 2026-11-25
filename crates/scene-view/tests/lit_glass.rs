//! Lit glass: dielectrics draw after opaque geometry, far to near, blended
//! by their transmission (no refraction, no OIT).
use scene_view::{PreviewMode, ViewerCamera, render_preview};
use std::sync::Mutex;

static GPU: Mutex<()> = Mutex::new(());

const HEADER: &str = "Film \"rgb\" \"integer xresolution\" 64 \"integer yresolution\" 64\nLookAt 0 0 3 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" 30\nWorldBegin\nLightSource \"distant\" \"point3 from\" [1 0.5 1] \"point3 to\" [0 0 0] \"rgb L\" [3 3 3]\n";

fn quad(z: f64, half: f64) -> String {
    format!(
        "Shape \"trianglemesh\" \"point3 P\" [-{half} -{half} {z} {half} -{half} {z} {half} {half} {z} -{half} {half} {z}] \"integer indices\" [0 1 2 0 2 3]\n"
    )
}

fn red_backdrop() -> String {
    format!(
        "AttributeBegin\nMaterial \"diffuse\" \"rgb reflectance\" [0.8 0.05 0.05]\n{}AttributeEnd\n",
        quad(0., 4.)
    )
}

fn glass(z: f64, eta: f64) -> String {
    format!(
        "AttributeBegin\nMaterial \"dielectric\" \"float eta\" {eta}\n{}AttributeEnd\n",
        quad(z, 3.)
    )
}

fn render(source: &str) -> (scene_parse::PreviewGeometry, scene_view::PreviewFrame) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("glass.pbrt");
    std::fs::write(&path, source).unwrap();
    let scene = scene_parse::parse_scene(path, scene_parse::ParseOptions::default()).unwrap();
    let camera = ViewerCamera::from_manifest(&scene.manifest, 1.0);
    let frame = render_preview(
        &scene.manifest,
        &scene.blobs,
        camera,
        64,
        64,
        PreviewMode::Lit,
    )
    .unwrap();
    (scene, frame)
}

fn center(frame: &scene_view::PreviewFrame) -> ([u8; 3], u32) {
    let i = (32 * 64 + 32) as usize;
    (
        frame.rgba[i * 4..i * 4 + 3].try_into().unwrap(),
        frame.pick_ids[i],
    )
}

#[test]
#[ignore = "requires a GPU adapter"]
fn glass_shows_what_is_behind_it() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let (_, frame) = render(&format!("{HEADER}{}{}", red_backdrop(), glass(1., 1.5)));
    let ([r, g, _], _) = center(&frame);
    assert!(r > 150 && g < 80, "red shows through the glass: {r} {g}");
}

#[test]
#[ignore = "requires a GPU adapter"]
fn glass_does_not_hide_opaque_geometry_in_front_of_it() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let green = format!(
        "AttributeBegin\nMaterial \"diffuse\" \"rgb reflectance\" [0.05 0.8 0.05]\n{}AttributeEnd\n",
        quad(2., 0.3)
    );
    for source in [
        format!("{HEADER}{}{}{green}", red_backdrop(), glass(1., 1.5)),
        format!("{HEADER}{green}{}{}", red_backdrop(), glass(1., 1.5)),
    ] {
        let ([r, g, _], _) = center(&render(&source).1);
        assert!(g > 150 && r < 80, "green in front stays opaque: {r} {g}");
    }
}

/// Tinted glass (a mix of a colored diffuse and a dielectric): unlike
/// clear dielectrics, whose reflection is proportional to their opacity,
/// tinted layers do not commute, so only far-to-near sorting gives the same
/// image whichever order the scene lists them in.
fn tinted(name: &str, z: f64, color: &str) -> String {
    format!(
        "MakeNamedMaterial \"{name}-tint\" \"string type\" \"diffuse\" \"rgb reflectance\" [{color}]\nMakeNamedMaterial \"{name}-clear\" \"string type\" \"dielectric\"\nMakeNamedMaterial \"{name}\" \"string type\" \"mix\" \"string materials\" [\"{name}-tint\" \"{name}-clear\"] \"float amount\" 0.5\nAttributeBegin\nNamedMaterial \"{name}\"\n{}AttributeEnd\n",
        quad(z, 3.)
    )
}

#[test]
#[ignore = "requires a GPU adapter"]
fn glass_order_follows_distance_not_draw_order() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let (a, b) = (
        tinted("green", 1., "0.1 0.9 0.1"),
        tinted("blue", 1.5, "0.1 0.1 0.9"),
    );
    let (_, near_last) = render(&format!("{HEADER}{}{a}{b}", red_backdrop()));
    let (_, near_first) = render(&format!("{HEADER}{}{b}{a}", red_backdrop()));
    let worst = near_last
        .rgba
        .iter()
        .zip(&near_first.rgba)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap();
    assert!(worst <= 2, "draw order changed the image by {worst}");
}

#[test]
#[ignore = "requires a GPU adapter"]
fn the_front_glass_wins_the_pick() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let (a, b) = (glass(1., 1.5), glass(1.5, 3.));
    for (source, front) in [
        (format!("{HEADER}{}{a}{b}", red_backdrop()), 2),
        (format!("{HEADER}{}{b}{a}", red_backdrop()), 1),
    ] {
        let (scene, frame) = render(&source);
        assert_eq!(center(&frame).1, scene.manifest.draws[front].pick_id);
    }
}

/// Glass sorting follows the rendered time sample: two panes that swap
/// depth between shutter open and close each win the pick at the time they
/// are in front (review #266).
#[test]
#[ignore = "requires a GPU adapter"]
fn animated_glass_sorts_at_the_rendered_time() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let moving = format!(
        "AttributeBegin\nActiveTransform StartTime\nTranslate 0 0 1\nActiveTransform EndTime\nTranslate 0 0 2\nActiveTransform All\nMaterial \"dielectric\"\n{}AttributeEnd\n",
        quad(0., 3.)
    );
    let source = format!("{HEADER}{}{moving}{}", red_backdrop(), glass(1.5, 1.5));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("moving.pbrt");
    std::fs::write(&path, source).unwrap();
    let scene = scene_parse::parse_scene(path, scene_parse::ParseOptions::default()).unwrap();
    let mut renderer = scene_view::PreviewRenderer::new().unwrap();
    renderer
        .upload_scene(&scene.manifest, &scene.blobs)
        .unwrap();
    let camera = ViewerCamera::from_manifest(&scene.manifest, 1.0);
    let times = &scene.manifest.supported_times;
    let (moving_id, fixed_id) = (
        scene.manifest.draws[1].pick_id,
        scene.manifest.draws[2].pick_id,
    );
    for (time, front) in [(times[0], fixed_id), (*times.last().unwrap(), moving_id)] {
        let frame = renderer
            .render(camera, 64, 64, PreviewMode::Lit, time)
            .unwrap();
        assert_eq!(center(&frame).1, front, "front pane at t = {time}");
    }
}

/// Only the first MAX_APPEARANCES (128) lobes reach the GPU; glass beyond
/// them keeps the opaque default shading instead of turning black
/// (review #266).
#[test]
#[ignore = "requires a GPU adapter"]
fn glass_beyond_the_appearance_limit_is_not_black() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let mut source = format!("{HEADER}");
    for i in 0..128 {
        // Distinct materials on tiny quads out of view.
        let shade = i as f64 / 128.;
        source += &format!(
            "AttributeBegin\nMaterial \"diffuse\" \"rgb reflectance\" [{shade} 0.5 0.5]\nTranslate 50 {i} 0\n{}AttributeEnd\n",
            quad(0., 0.01)
        );
    }
    source += &glass(1., 1.5);
    let (_, frame) = render(&source);
    let ([r, g, b], _) = center(&frame);
    assert!(
        r.max(g).max(b) > 20,
        "overflow glass shades as default diffuse, got {r} {g} {b}"
    );
}

/// Glass honors its shape's alpha like opaque shapes do: the backdrop picks
/// through a hole, and is unblended there.
#[test]
#[ignore = "requires a GPU adapter"]
fn glass_alpha_cuts_holes() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let alpha = "Texture \"a\" \"float\" \"constant\" \"float value\" [0]\n";
    let holed = format!(
        "AttributeBegin\nMaterial \"dielectric\"\nShape \"trianglemesh\" \"point3 P\" [-3 -3 1 3 -3 1 3 3 1 -3 3 1] \"integer indices\" [0 1 2 0 2 3] \"texture alpha\" \"a\"\nAttributeEnd\n"
    );
    let (scene, frame) = render(&format!("{HEADER}{alpha}{}{holed}", red_backdrop()));
    assert_eq!(center(&frame).1, scene.manifest.draws[0].pick_id);
}

/// A mix with a little glass is mostly opaque, so its nearer surface must hide
/// its farther one whatever order the mesh lists them in.
#[test]
#[ignore = "requires a GPU adapter"]
fn partly_transparent_mixes_hide_their_own_far_side() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    // One mesh: the near quad (green by uv) first, the far quad (blue) last.
    let mesh = "Shape \"trianglemesh\" \"point3 P\" [-3 -3 1 3 -3 1 3 3 1 -3 3 1 -3 -3 0.5 3 -3 0.5 3 3 0.5 -3 3 0.5] \"point2 uv\" [0 0 0 0 0 0 0 0 1 0 1 0 1 0 1 0] \"integer indices\" [0 1 2 0 2 3 4 5 6 4 6 7]\n";
    let scene = format!(
        "{HEADER}Texture \"sides\" \"spectrum\" \"bilerp\" \"rgb v00\" [0.05 0.8 0.05] \"rgb v01\" [0.05 0.8 0.05] \"rgb v10\" [0.05 0.05 0.8] \"rgb v11\" [0.05 0.05 0.8]\nMakeNamedMaterial \"solid\" \"string type\" \"diffuse\" \"texture reflectance\" \"sides\"\nMakeNamedMaterial \"clear\" \"string type\" \"dielectric\"\nMakeNamedMaterial \"most\" \"string type\" \"mix\" \"string materials\" [\"solid\" \"clear\"] \"float amount\" [0.1]\nNamedMaterial \"most\"\n{mesh}"
    );
    let (_, frame) = render(&scene);
    let ([_, green, blue], _) = center(&frame);
    assert!(
        green > blue,
        "the far side shows: green {green}, blue {blue}"
    );
}

/// Where a mix selects an invisible material nothing is there: the click goes
/// to what is behind.
#[test]
#[ignore = "requires a GPU adapter"]
fn mix_holes_pick_through() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let holed = format!(
        "AttributeBegin\nMakeNamedMaterial \"nothing\" \"string type\" \"interface\"\nMakeNamedMaterial \"solid\" \"string type\" \"diffuse\"\nMakeNamedMaterial \"holed\" \"string type\" \"mix\" \"string materials\" [\"solid\" \"nothing\"] \"float amount\" [1]\nNamedMaterial \"holed\"\n{}AttributeEnd\n",
        quad(1., 3.)
    );
    let (scene, frame) = render(&format!("{HEADER}{}{holed}", red_backdrop()));
    assert_eq!(center(&frame).1, scene.manifest.draws[0].pick_id);
}
