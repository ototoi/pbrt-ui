use scene_view::{PreviewMode, ViewerCamera};
use std::sync::Mutex;

static GPU: Mutex<()> = Mutex::new(());

fn parsed(source: &str) -> scene_parse::PreviewGeometry {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lit-shadow.pbrt");
    std::fs::write(&path, source).unwrap();
    scene_parse::parse_scene(path, scene_parse::ParseOptions::default()).unwrap()
}

/// A parsed scene with a square sRGB control map next to it, like the
/// r4 audit scenes declare their lights.
fn parsed_with_map(source: &str, name: &str) -> scene_parse::PreviewGeometry {
    let dir = tempfile::tempdir().unwrap();
    let gray = image::RgbImage::from_pixel(128, 128, image::Rgb([125, 125, 125]));
    gray.save(dir.path().join(name)).unwrap();
    let path = dir.path().join("lit-shadow.pbrt");
    std::fs::write(&path, source).unwrap();
    let parsed = scene_parse::parse_scene(path, scene_parse::ParseOptions::default()).unwrap();
    // The map must ride as this light's blob for the test to exercise the
    // control-map path, not the mapless fallback.
    assert!(
        parsed.blobs.contains_key("light/0"),
        "the light map did not ride as a blob"
    );
    parsed
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_plate_casts_a_hard_point_light_shadow() {
    let _guard = GPU.lock().unwrap_or_else(|error| error.into_inner());
    let source = "Film \"rgb\" \"integer xresolution\" 96 \"integer yresolution\" 96\nLookAt 0 -3 1.5 0 0 0 0 0 1\nCamera \"perspective\" \"float fov\" 35\nWorldBegin\nLightSource \"point\" \"point from\" [0 0 4] \"rgb I\" [20 20 20]\nMaterial \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\nShape \"trianglemesh\" \"point3 P\" [-0.5 -0.5 1 0.5 -0.5 1 0.5 0.5 1 -0.5 0.5 1] \"integer indices\" [0 1 2 0 2 3]\nShape \"trianglemesh\" \"point3 P\" [-4 -4 0 4 -4 0 4 4 0 -4 4 0] \"integer indices\" [0 2 1 0 3 2]\n";
    let scene = parsed(source);
    let camera = ViewerCamera::from_manifest(&scene.manifest, 1.0);
    let frame = scene_view::render_preview(
        &scene.manifest,
        &scene.blobs,
        camera,
        96,
        96,
        PreviewMode::Lit,
    )
    .unwrap();
    // draws[0] is the plate, draws[1] the floor.
    let floor_id = scene.manifest.draws[1].pick_id;
    let pixels = frame
        .pick_ids
        .iter()
        .enumerate()
        .filter(|(_, id)| **id == floor_id)
        .map(|(index, _)| (index, frame.rgba[index * 4]))
        .collect::<Vec<_>>();
    assert!(!pixels.is_empty(), "camera should see the measured floor");
    let darkest = pixels.iter().map(|(_, value)| *value).min().unwrap();
    let brightest = pixels.iter().map(|(_, value)| *value).max().unwrap();
    eprintln!(
        "floor pixels={} darkest={darkest} brightest={brightest}",
        pixels.len()
    );
    assert!(
        darkest < 5,
        "plate shadow should contain an umbra, got {darkest}"
    );
    assert!(
        brightest > 30,
        "floor outside shadow should be lit, got {brightest}"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn camera_moves_do_not_rerender_shadows() {
    let _guard = GPU.lock().unwrap_or_else(|error| error.into_inner());
    let scene =
        parsed("WorldBegin\nLightSource \"point\" \"point from\" [0 0 2]\nShape \"sphere\"\n");
    let mut renderer = scene_view::PreviewRenderer::new().unwrap();
    renderer
        .upload_scene(&scene.manifest, &scene.blobs)
        .unwrap();
    let mut camera = ViewerCamera::from_manifest(&scene.manifest, 1.0);
    renderer
        .render(camera, 64, 64, PreviewMode::Lit, 0.0)
        .unwrap();
    camera.orbit(0.2, 0.0).unwrap();
    renderer
        .render(camera, 64, 64, PreviewMode::Lit, 0.0)
        .unwrap();
    assert_eq!(renderer.stats().shadow_renders, 1);
    renderer.update_draws(&scene.manifest.draws).unwrap();
    renderer
        .render(camera, 64, 64, PreviewMode::Lit, 0.0)
        .unwrap();
    assert_eq!(renderer.stats().shadow_renders, 2);
}

const FLOOR: &str = "Shape \"trianglemesh\" \"point3 P\" [-4 -4 0 4 -4 0 4 4 0 -4 4 0] \"integer indices\" [0 1 2 0 2 3]\n";
const HEADER_96: &str = "Film \"rgb\" \"integer xresolution\" 96 \"integer yresolution\" 96\n";

fn render(source: &str) -> (scene_parse::PreviewGeometry, scene_view::PreviewFrame) {
    let scene = parsed(source);
    let camera = ViewerCamera::from_manifest(&scene.manifest, 1.0);
    let frame = scene_view::render_preview(
        &scene.manifest,
        &scene.blobs,
        camera,
        96,
        96,
        PreviewMode::Lit,
    )
    .unwrap();
    (scene, frame)
}

/// Red channel of pixel (x, y), asserting it shows the draw `id`.
fn value_on(frame: &scene_view::PreviewFrame, x: u32, y: u32, id: u32) -> u8 {
    let index = (y * frame.width + x) as usize;
    assert_eq!(
        frame.pick_ids[index], id,
        "pixel ({x}, {y}) does not show the measured draw"
    );
    frame.rgba[index * 4]
}

#[test]
#[ignore = "requires a GPU adapter"]
fn distant_light_shadows_follow_the_light_direction() {
    let _guard = GPU.lock().unwrap_or_else(|error| error.into_inner());
    // Top view; the plate (z = 1) hides |x| < 0.56 of the floor, and a
    // light from (±1, 0, 1) throws its shadow to x = ∓1.
    let dark_side = |from_x: i32| {
        let source = format!(
            "{HEADER_96}LookAt 0 0 10 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" 28\nWorldBegin\nLightSource \"distant\" \"point3 from\" [{from_x} 0 1] \"point3 to\" [0 0 0] \"rgb L\" [3 3 3]\nMaterial \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\nShape \"trianglemesh\" \"point3 P\" [-0.5 -0.5 1 0.5 -0.5 1 0.5 0.5 1 -0.5 0.5 1] \"integer indices\" [0 1 2 0 2 3]\n{FLOOR}"
        );
        let (scene, frame) = render(&source);
        let floor = scene.manifest.draws[1].pick_id;
        // World x = ±1 is 0.2 of the image from the center.
        let left = value_on(&frame, 29, 48, floor);
        let right = value_on(&frame, 67, 48, floor);
        assert!(
            left.min(right) < 5 && left.max(right) > 60,
            "{left} {right}"
        );
        left < right
    };
    assert_ne!(dark_side(1), dark_side(-1));
}

/// Fraction of floor pixels the plate partly (not fully) shadows: the
/// frame with the plate divided by the frame without it, which cancels the
/// light's own falloff across the floor.
fn penumbra_fraction(light: &str) -> f64 {
    let header = format!(
        "{HEADER_96}LookAt 0 -4 0.8 0 0 0 0 0 1\nCamera \"perspective\" \"float fov\" 40\nWorldBegin\n{light}Material \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\n"
    );
    let plate = "Shape \"trianglemesh\" \"point3 P\" [-0.3 -0.3 1 0.3 -0.3 1 0.3 0.3 1 -0.3 0.3 1] \"integer indices\" [0 1 2 0 2 3]\n";
    let (with_scene, with) = render(&format!("{header}{plate}{FLOOR}"));
    let (without_scene, without) = render(&format!("{header}{FLOOR}"));
    let floor_with = with_scene.manifest.draws.last().unwrap().pick_id;
    let floor_without = without_scene.manifest.draws.last().unwrap().pick_id;
    let linear = |v: u8| {
        let c = v as f64 / 255.;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let (mut partial, mut shadowed) = (0, 0);
    for index in 0..with.pick_ids.len() {
        if with.pick_ids[index] != floor_with || without.pick_ids[index] != floor_without {
            continue;
        }
        let open = linear(without.rgba[index * 4]);
        if open < 0.01 {
            continue;
        }
        let visibility = linear(with.rgba[index * 4]) / open;
        if visibility < 0.9 {
            shadowed += 1;
            if visibility > 0.1 {
                partial += 1;
            }
        }
    }
    assert!(shadowed > 0, "the plate must cast a visible shadow");
    partial as f64 / shadowed as f64
}

#[test]
#[ignore = "requires a GPU adapter"]
fn quad_light_shadows_have_a_penumbra() {
    let _guard = GPU.lock().unwrap_or_else(|error| error.into_inner());
    // A 2×2 light at z = 3 facing down against a point light at its center:
    // the area light's shadow is mostly penumbra, the point light's is not.
    let area = penumbra_fraction(
        "AttributeBegin\nTranslate 0 0 3\nAreaLightSource \"diffuse\" \"rgb L\" [4 4 4]\nShape \"trianglemesh\" \"point3 P\" [-1 -1 0 1 -1 0 1 1 0 -1 1 0] \"integer indices\" [0 2 1 0 3 2]\nAttributeEnd\n",
    );
    let point =
        penumbra_fraction("LightSource \"point\" \"point3 from\" [0 0 3] \"rgb I\" [20 20 20]\n");
    eprintln!("penumbra fraction: area {area:.3}, point {point:.3}");
    assert!(area > 0.4, "area light penumbra fraction {area}");
    assert!(point < 0.25, "point light penumbra fraction {point}");
}

#[test]
#[ignore = "requires a GPU adapter"]
fn an_emitter_does_not_shadow_its_own_light() {
    let _guard = GPU.lock().unwrap_or_else(|error| error.into_inner());
    // The sphere light's own tessellated surface surrounds its shadow-map
    // origin; drawn into its own map it would black out the floor.
    let source = format!(
        "{HEADER_96}LookAt 0 -3 1 0 0 0 0 0 1\nCamera \"perspective\" \"float fov\" 40\nWorldBegin\nAttributeBegin\nTranslate 0 0 1.5\nAreaLightSource \"diffuse\" \"rgb L\" [4 4 4]\nShape \"sphere\" \"float radius\" 0.3\nAttributeEnd\nMaterial \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\n{FLOOR}"
    );
    let (scene, frame) = render(&source);
    let floor = scene.manifest.draws[1].pick_id;
    assert!(value_on(&frame, 48, 70, floor) > 20);
}

/// A scene with a light but no shapes renders in Lit instead of panicking
/// on the missing shadow instance buffer (review #264).
#[test]
#[ignore = "requires a GPU adapter"]
fn lit_renders_a_lit_scene_without_draws() {
    let _guard = GPU.lock().unwrap_or_else(|error| error.into_inner());
    let scene = parsed("WorldBegin\nLightSource \"point\" \"point3 from\" [0 0 2]\n");
    let mut renderer = scene_view::PreviewRenderer::new().unwrap();
    renderer
        .upload_scene(&scene.manifest, &scene.blobs)
        .unwrap();
    let camera = ViewerCamera::from_manifest(&scene.manifest, 1.0);
    renderer
        .render(camera, 32, 32, PreviewMode::Lit, 0.0)
        .unwrap();
}

/// The contrast of the floor pixels: shadowed region vs lit region, given
/// the floor draw's pick id (mirrors the point-light plate test's numbers).
fn floor_contrast(frame: &scene_view::PreviewFrame, floor_id: u32) -> (u8, u8) {
    let pixels = frame
        .pick_ids
        .iter()
        .enumerate()
        .filter(|(_, id)| **id == floor_id)
        .map(|(index, _)| frame.rgba[index * 4])
        .collect::<Vec<_>>();
    assert!(!pixels.is_empty(), "camera should see the measured floor");
    (*pixels.iter().min().unwrap(), *pixels.iter().max().unwrap())
}

/// Goniometric lights are positional: like a point light, the strongest
/// shadow candidate by their 4pi·mean-Y flux, the plate throws an umbra
/// on the floor through their cube shadow maps. Gray 125 makes the map's
/// Y about 0.216, so I = 100 lands at the point-light test's intensity.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_plate_casts_a_hard_goniometric_light_shadow() {
    let _guard = GPU.lock().unwrap_or_else(|error| error.into_inner());
    let source = "Film \"rgb\" \"integer xresolution\" 96 \"integer yresolution\" 96\nLookAt 0 -3 1.5 0 0 0 0 0 1\nCamera \"perspective\" \"float fov\" 35\nWorldBegin\nAttributeBegin\nTranslate 0 0 4\nLightSource \"goniometric\" \"string filename\" \"diagram.png\" \"rgb I\" [100 100 100]\nAttributeEnd\nMaterial \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\nShape \"trianglemesh\" \"point3 P\" [-0.5 -0.5 1 0.5 -0.5 1 0.5 0.5 1 -0.5 0.5 1] \"integer indices\" [0 1 2 0 2 3]\nShape \"trianglemesh\" \"point3 P\" [-4 -4 0 4 -4 0 4 4 0 -4 4 0] \"integer indices\" [0 1 2 0 2 3]\n";
    let scene = parsed_with_map(source, "diagram.png");
    let camera = ViewerCamera::from_manifest(&scene.manifest, 1.0);
    let frame = scene_view::render_preview(
        &scene.manifest,
        &scene.blobs,
        camera,
        96,
        96,
        PreviewMode::Lit,
    )
    .unwrap();
    let floor_id = scene.manifest.draws[1].pick_id;
    let (darkest, brightest) = floor_contrast(&frame, floor_id);
    eprintln!("goniometric floor darkest={darkest} brightest={brightest}");
    assert!(
        darkest < 5,
        "plate shadow should contain an umbra, got {darkest}"
    );
    assert!(
        brightest > 30,
        "floor outside shadow should be lit, got {brightest}"
    );
}

/// Projection lights are positional too: mirrored toward the scene like
/// r4's audit scene (rotated half a turn about x), the plate throws the
/// umbra and the projected map lights the floor inside the cone. Gray 125
/// with scale 100 lands at the point-light test's intensity.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_plate_casts_a_hard_projection_light_shadow() {
    let _guard = GPU.lock().unwrap_or_else(|error| error.into_inner());
    let source = "Film \"rgb\" \"integer xresolution\" 96 \"integer yresolution\" 96\nLookAt 0 -3 1.5 0 0 0 0 0 1\nCamera \"perspective\" \"float fov\" 35\nWorldBegin\nAttributeBegin\nTranslate 0 0 4\nRotate 180 1 0 0\nLightSource \"projection\" \"string filename\" \"slide.png\" \"float scale\" [100]\nAttributeEnd\nMaterial \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\nShape \"trianglemesh\" \"point3 P\" [-0.5 -0.5 1 0.5 -0.5 1 0.5 0.5 1 -0.5 0.5 1] \"integer indices\" [0 1 2 0 2 3]\nShape \"trianglemesh\" \"point3 P\" [-4 -4 0 4 -4 0 4 4 0 -4 4 0] \"integer indices\" [0 1 2 0 2 3]\n";
    let scene = parsed_with_map(source, "slide.png");
    let camera = ViewerCamera::from_manifest(&scene.manifest, 1.0);
    let frame = scene_view::render_preview(
        &scene.manifest,
        &scene.blobs,
        camera,
        96,
        96,
        PreviewMode::Lit,
    )
    .unwrap();
    let floor_id = scene.manifest.draws[1].pick_id;
    let (darkest, brightest) = floor_contrast(&frame, floor_id);
    eprintln!("projection floor darkest={darkest} brightest={brightest}");
    assert!(
        darkest < 5,
        "plate shadow should contain an umbra, got {darkest}"
    );
    assert!(
        brightest > 30,
        "floor inside the cone should be lit, got {brightest}"
    );
}

/// A shape's alpha cuts holes for Lit, pick and shadow rays alike. A checker
/// alpha (even cells solid, odd cells holes) over a backing quad: the pick
/// buffer sees through the holes, and `alpha 0` removes the shape entirely.
#[test]
#[ignore = "requires a GPU adapter"]
fn alpha_cuts_holes_for_pick_and_removes_shadows_at_zero() {
    let _guard = GPU.lock().unwrap_or_else(|error| error.into_inner());
    let header = "Film \"rgb\" \"integer xresolution\" 96 \"integer yresolution\" 96\nLookAt 0 0 4 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" 40\nWorldBegin\nTexture \"a\" \"float\" \"checkerboard\" \"float uscale\" [2] \"float vscale\" [2] \"float tex1\" [1] \"float tex2\" [0] \"string aamode\" \"none\"\nLightSource \"point\" \"point3 from\" [0 0 3] \"rgb I\" [20 20 20]\n";
    let back = "Shape \"trianglemesh\" \"point3 P\" [-3 -3 -1 3 -3 -1 3 3 -1 -3 3 -1] \"integer indices\" [0 1 2 0 2 3]\n";
    let front = |alpha: &str| {
        format!(
            "Shape \"trianglemesh\" \"point3 P\" [-1 -1 0 1 -1 0 1 1 0 -1 1 0] \"texcoord2 uv\" [0 0 1 0 1 1 0 1] \"integer indices\" [0 1 2 0 2 3] {alpha}\n"
        )
    };
    let (scene, frame) = render(&format!(
        "{header}{back}{}",
        front("\"texture alpha\" \"a\"")
    ));
    let (back_id, front_id) = (
        scene.manifest.draws[0].pick_id,
        scene.manifest.draws[1].pick_id,
    );
    let pick = |x: u32, y: u32| frame.pick_ids[(y * frame.width + x) as usize];
    // Cell centers either side of the quad's middle: one solid, one a hole.
    let (a, b) = (pick(34, 34), pick(62, 34));
    assert!(
        (a == front_id && b == back_id) || (a == back_id && b == front_id),
        "cells pick {a} and {b}; front {front_id}, back {back_id}"
    );
    assert_eq!(pick(34, 62), b, "the diagonal parity holds");

    // Without alpha, the solid shape shadows the backing quad behind its middle.
    let (_, solid) = render(&format!("{header}{back}{}", front("")));
    let (zero_scene, zero) = render(&format!("{header}{back}{}", front("\"float alpha\" [0]")));
    let zero_back = zero_scene.manifest.draws[0].pick_id;
    let center = (48 * zero.width + 48) as usize;
    assert_eq!(
        zero.pick_ids[center], zero_back,
        "alpha 0 removes the shape"
    );
    let (_, without) = render(&format!("{header}{back}"));
    assert_eq!(
        zero.rgba, without.rgba,
        "alpha 0 casts no shadow and draws nothing"
    );
    assert_ne!(solid.rgba, without.rgba);
}

/// A partly covered alpha rounds to its majority in every pass. A constant
/// texture stands in for a filtered mask's edge, which would otherwise grow
/// solid by a pixel or two.
#[test]
#[ignore = "requires a GPU adapter"]
fn minority_alpha_coverage_is_cut_from_view_pick_and_shadow() {
    let _guard = GPU.lock().unwrap_or_else(|error| error.into_inner());
    let header = "Film \"rgb\" \"integer xresolution\" 96 \"integer yresolution\" 96\nLookAt 0 0 4 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" 40\nWorldBegin\nTexture \"a\" \"float\" \"constant\" \"float value\" [0.25]\nLightSource \"point\" \"point3 from\" [0 0 3] \"rgb I\" [20 20 20]\n";
    let back = "Shape \"trianglemesh\" \"point3 P\" [-3 -3 -1 3 -3 -1 3 3 -1 -3 3 -1] \"integer indices\" [0 1 2 0 2 3]\n";
    let front = "Shape \"trianglemesh\" \"point3 P\" [-1 -1 0 1 -1 0 1 1 0 -1 1 0] \"integer indices\" [0 1 2 0 2 3] \"texture alpha\" \"a\"\n";
    let (scene, cut) = render(&format!("{header}{back}{front}"));
    let (_, without) = render(&format!("{header}{back}"));
    assert_eq!(
        cut.pick_ids[(48 * cut.width + 48) as usize],
        scene.manifest.draws[0].pick_id
    );
    assert_eq!(cut.rgba, without.rgba, "no trace in view or shadow");
}

/// A mapped alpha cuts shadows too, with its rows measured from the shadow
/// origin rather than the eye. The planar checker is the same cutout as a UV
/// one, so the shadow on the backing quad must match in both, whichever frame
/// the shadow pass runs in.
#[test]
#[ignore = "requires a GPU adapter"]
fn mapped_alpha_shadows_do_not_move_with_the_camera_origin() {
    let _guard = GPU.lock().unwrap_or_else(|error| error.into_inner());
    let scene = |texture: &str, uv: &str| {
        format!(
            "Film \"rgb\" \"integer xresolution\" 96 \"integer yresolution\" 96\nLookAt 0 0.01 8 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" 40\nWorldBegin\nTexture \"a\" \"float\" \"checkerboard\" {texture} \"float tex1\" [1] \"float tex2\" [0] \"string aamode\" \"none\"\nLightSource \"distant\" \"point3 from\" [0 0 1] \"point3 to\" [0 0 0] \"rgb L\" [3 3 3]\nShape \"trianglemesh\" \"point3 P\" [-3 -3 -1 3 -3 -1 3 3 -1 -3 3 -1] \"integer indices\" [0 1 2 0 2 3]\nShape \"trianglemesh\" \"point3 P\" [0 0 1 2 0 1 2 2 1 0 2 1] {uv} \"integer indices\" [0 1 2 0 2 3] \"texture alpha\" \"a\"\n"
        )
    };
    let (_, uv) = render(&scene(
        "\"float uscale\" [2] \"float vscale\" [2]",
        "\"point2 uv\" [0 0 1 0 1 1 0 1]",
    ));
    let (_, planar) = render(&scene("\"string mapping\" \"planar\"", ""));
    let different = uv
        .rgba
        .iter()
        .zip(&planar.rgba)
        .filter(|(a, b)| a.abs_diff(**b) > 8)
        .count();
    assert!(different < 60, "{different} channels differ");
    assert!(
        uv.rgba.chunks(4).any(|p| p[0] < 40) && uv.rgba.chunks(4).any(|p| p[0] > 100),
        "the scene has both shadow and light"
    );
}

/// A spot light's shadow falls under the occluder: the region the cone
/// lights outside the plate compares bright, the plate's umbra dark. The
/// wide cone reaches the whole floor, so the umbra is the only dark place.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_plate_casts_a_hard_spot_light_shadow() {
    let _guard = GPU.lock().unwrap_or_else(|error| error.into_inner());
    let source = "Film \"rgb\" \"integer xresolution\" 96 \"integer yresolution\" 96\nLookAt 0 -3 1.5 0 0 0 0 0 1\nCamera \"perspective\" \"float fov\" 35\nWorldBegin\nLightSource \"spot\" \"point from\" [0 0 4] \"point to\" [0 0 0] \"float coneangle\" 90 \"float conedelta\" 10 \"rgb I\" [20 20 20]\nMaterial \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\nShape \"trianglemesh\" \"point3 P\" [-0.5 -0.5 1 0.5 -0.5 1 0.5 0.5 1 -0.5 0.5 1] \"integer indices\" [0 1 2 0 2 3]\nShape \"trianglemesh\" \"point3 P\" [-4 -4 0 4 -4 0 4 4 0 -4 4 0] \"integer indices\" [0 2 1 0 3 2]\n";
    let scene = parsed(source);
    let camera = ViewerCamera::from_manifest(&scene.manifest, 1.0);
    let frame = scene_view::render_preview(
        &scene.manifest,
        &scene.blobs,
        camera,
        96,
        96,
        PreviewMode::Lit,
    )
    .unwrap();
    let floor_id = scene.manifest.draws[1].pick_id;
    let (darkest, brightest) = floor_contrast(&frame, floor_id);
    eprintln!("spot floor darkest={darkest} brightest={brightest}");
    assert!(
        darkest < 5,
        "plate shadow should contain an umbra, got {darkest}"
    );
    assert!(
        brightest > 30,
        "floor inside the cone should stay lit, got {brightest}"
    );
}
