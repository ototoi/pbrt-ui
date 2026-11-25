use scene_view::{PreviewMode, ViewerCamera, render_preview};
use std::sync::Mutex;
static GPU: Mutex<()> = Mutex::new(());

fn parsed(source: &str) -> scene_parse::PreviewGeometry {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lit.pbrt");
    std::fs::write(&path, source).unwrap();
    scene_parse::parse_scene(path, scene_parse::ParseOptions::default()).unwrap()
}
/// Render through the scene's own LookAt/Camera, as the Scene panel does.
fn render(source: &str, w: u32, h: u32, mode: PreviewMode) -> scene_view::PreviewFrame {
    let scene = parsed(source);
    let camera = ViewerCamera::from_manifest(&scene.manifest, w as f64 / h as f64);
    render_preview(&scene.manifest, &scene.blobs, camera, w, h, mode).unwrap()
}
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
const HEADER: &str = "Film \"rgb\" \"integer xresolution\" 64 \"integer yresolution\" 64\nLookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" 30\nWorldBegin\n";
/// Camera at z = 0.5 looking down at a floor at z = 0, below lights at z >= 1.
const UNDER_LIGHT: &str = "Film \"rgb\" \"integer xresolution\" 64 \"integer yresolution\" 64\nLookAt 0 0 0.5 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" 30\nWorldBegin\n";
/// Camera at z = -0.5 looking down at a floor at z = -1, under a light at z = 0.
const BELOW_PLANE: &str = "Film \"rgb\" \"integer xresolution\" 64 \"integer yresolution\" 64\nLookAt 0 0 -0.5 0 0 -1 0 1 0\nCamera \"perspective\" \"float fov\" 30\nWorldBegin\n";
const QUAD: &str = "Shape \"trianglemesh\" \"point3 P\" [-2 -2 0 2 -2 0 2 2 0 -2 2 0] \"integer indices\" [0 1 2 0 2 3]\n";
const DOWN_QUAD: &str = "Shape \"trianglemesh\" \"point3 P\" [-0.5 -0.5 0 0.5 -0.5 0 0.5 0.5 0 -0.5 0.5 0] \"integer indices\" [0 2 1 0 3 2]\n";

fn tiny_measured_bsdf() -> Vec<u8> {
    let fields = [
        ("wavelengths", vec![3u64], vec![465f32, 532., 630.]),
        ("spectra", vec![1u64, 1, 3, 1, 1], vec![0.2f32, 0.5, 0.9]),
    ];
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"tensor_file\0");
    bytes.extend([1, 0]);
    bytes.extend((fields.len() as u32).to_le_bytes());
    let mut offsets = Vec::new();
    for (name, shape, _) in &fields {
        bytes.extend((name.len() as u16).to_le_bytes());
        bytes.extend(name.as_bytes());
        bytes.extend((shape.len() as u16).to_le_bytes());
        bytes.push(10);
        offsets.push(bytes.len());
        bytes.extend([0; 8]);
        for dim in shape {
            bytes.extend(dim.to_le_bytes());
        }
    }
    let mut offset = bytes.len() as u64;
    for ((_, _, values), offset_at) in fields.iter().zip(offsets) {
        bytes[offset_at..offset_at + 8].copy_from_slice(&offset.to_le_bytes());
        offset += values.len() as u64 * 4;
    }
    for (_, _, values) in fields {
        for value in values {
            bytes.extend(value.to_le_bytes());
        }
    }
    bytes
}

#[test]
#[ignore = "requires a GPU adapter"]
fn measured_material_renders_its_average_color_or_gray_fallback_with_a_grouped_note() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("sample.bsdf"), tiny_measured_bsdf()).unwrap();
    let scene_path = dir.path().join("measured.pbrt");
    std::fs::write(
        &scene_path,
        format!("{HEADER}LightSource \"infinite\" \"rgb L\" [1 1 1]\nMaterial \"measured\" \"string filename\" \"sample.bsdf\"\nShape \"sphere\"\nShape \"sphere\"\n"),
    )
    .unwrap();
    let scene =
        scene_parse::parse_scene(&scene_path, scene_parse::ParseOptions::default()).unwrap();
    let mut renderer = scene_view::PreviewRenderer::new().unwrap();
    renderer
        .upload_scene(&scene.manifest, &scene.blobs)
        .unwrap();
    let camera = ViewerCamera::from_manifest(&scene.manifest, 1.);
    let loaded = renderer
        .render(camera, 64, 64, PreviewMode::Lit, 0.)
        .unwrap();
    let got = center(&loaded);
    assert!(
        got[0] > got[1] && got[1] > got[2],
        "representative color should be warm: {got:?}"
    );
    assert!(
        renderer
            .lit_notes()
            .iter()
            .any(|note| { note.reason.contains("representative-color") && note.draws == 2 }),
        "{:?}",
        renderer.lit_notes()
    );

    let missing_path = dir.path().join("missing.pbrt");
    std::fs::write(
        &missing_path,
        format!("{HEADER}LightSource \"infinite\" \"rgb L\" [1 1 1]\nMaterial \"measured\" \"string filename\" \"absent.bsdf\"\nShape \"sphere\"\nShape \"sphere\"\n"),
    )
    .unwrap();
    let missing =
        scene_parse::parse_scene(&missing_path, scene_parse::ParseOptions::default()).unwrap();
    renderer
        .upload_scene(&missing.manifest, &missing.blobs)
        .unwrap();
    let fallback = renderer
        .render(camera, 64, 64, PreviewMode::Lit, 0.)
        .unwrap();
    let gray = center(&fallback);
    assert!(
        gray.into_iter().max().unwrap() - gray.into_iter().min().unwrap() <= 1,
        "{gray:?}"
    );
    assert!(
        renderer
            .lit_notes()
            .iter()
            .any(|note| { note.reason.contains("file unread") && note.draws == 2 }),
        "{:?}",
        renderer.lit_notes()
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn hair_curve_fixture_renders_literal_texture_and_reflectance_precedence() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/hair-color/scene.pbrt");
    let scene = scene_parse::parse_scene(&path, scene_parse::ParseOptions::default()).unwrap();
    assert_eq!(scene.manifest.draws.len(), 4);
    assert_eq!(scene.manifest.meshes.len(), 4);
    assert!(
        scene
            .manifest
            .draws
            .iter()
            .all(|draw| draw.shape_type == "curve")
    );
    assert!(scene.manifest.meshes.iter().all(|mesh| {
        mesh.uv
            .as_ref()
            .is_some_and(|uv| uv.count == mesh.positions.count)
    }));

    let mut renderer = scene_view::PreviewRenderer::new().unwrap();
    renderer
        .upload_scene(&scene.manifest, &scene.blobs)
        .unwrap();
    let camera = ViewerCamera::from_manifest(&scene.manifest, 640. / 480.);
    let frame = renderer
        .render(camera, 640, 480, PreviewMode::Lit, 0.)
        .unwrap();

    let pixels = |x: std::ops::Range<usize>| {
        frame
            .rgba
            .chunks_exact(4)
            .enumerate()
            .filter_map(|(i, p)| {
                let px = i % frame.width as usize;
                let py = i / frame.width as usize;
                (x.contains(&px) && (80..400).contains(&py)).then_some([p[0], p[1], p[2]])
            })
            .filter(|p| *p != [255; 3])
            .collect::<Vec<_>>()
    };
    let default = pixels(80..220);
    let green = pixels(220..320);
    let checker = pixels(320..420);
    let ochre = pixels(420..560);
    assert!(
        default
            .iter()
            .any(|p| p[0] > p[1] && p[1] > p[2] && p[0] < 170),
        "{default:?}"
    );
    assert!(
        green.iter().any(|p| p[1] > p[0] + 20 && p[1] > p[2] + 20),
        "{green:?}"
    );
    let red = checker
        .iter()
        .filter(|p| p[0] > p[1] + 20 && p[0] > p[2] + 20)
        .count();
    let blue = checker
        .iter()
        .filter(|p| p[2] > p[0] + 20 && p[2] > p[1] + 20)
        .count();
    assert!(red > 0, "checker should have red texels; red pixels={red}");
    assert!(
        blue > 0,
        "checker should have blue texels; blue pixels={blue}"
    );
    assert!(
        ochre.iter().any(|p| p[0] > p[1] + 10 && p[1] > p[2] + 10),
        "{ochre:?}"
    );
    assert!(
        !ochre.iter().any(|p| p[2] > p[0] + 20),
        "reflectance should override texture: {ochre:?}"
    );
    assert!(
        renderer
            .lit_notes()
            .iter()
            .any(|note| { note.reason.contains("hair: no fibre model") && note.draws == 4 }),
        "{:?}",
        renderer.lit_notes()
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn uniform_infinite_light_shows_albedo_and_background() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let f = render(
        &format!(
            "{HEADER}LightSource \"infinite\" \"rgb L\" [1 1 1]\nMaterial \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\nShape \"sphere\"\n"
        ),
        64,
        64,
        PreviewMode::Lit,
    );
    near_rgb(center(&f), srgb(0.5), 2);
    assert_eq!([f.rgba[0], f.rgba[1], f.rgba[2]], [255; 3]);
}
#[test]
#[ignore = "requires a GPU adapter"]
fn point_light_follows_inverse_square() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let f = render(
        &format!(
            "{HEADER}LightSource \"point\" \"point from\" [0 0 1] \"rgb I\" [1.57079632679 1.57079632679 1.57079632679]\nMaterial \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\n{QUAD}"
        ),
        64,
        64,
        PreviewMode::Lit,
    );
    near_rgb(center(&f), srgb(0.25), 2);
}
#[test]
#[ignore = "requires a GPU adapter"]
fn film_iso_scales_the_output() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let f = render(
        &format!(
            "Film \"rgb\" \"integer xresolution\" 64 \"integer yresolution\" 64 \"float iso\" 200\n{HEADER}LightSource \"infinite\" \"rgb L\" [1 1 1]\nMaterial \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\nShape \"sphere\"\n"
        ),
        64,
        64,
        PreviewMode::Lit,
    );
    near_rgb(center(&f), 255, 2);
}
#[test]
#[ignore = "requires a GPU adapter"]
fn zero_iso_renders_black() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let f = render(
        &format!(
            "Film \"rgb\" \"integer xresolution\" 64 \"integer yresolution\" 64 \"float iso\" 0\n{HEADER}LightSource \"infinite\" \"rgb L\" [1 1 1]\nMaterial \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\nShape \"sphere\"\n"
        ),
        64,
        64,
        PreviewMode::Lit,
    );
    assert!(f.rgba.chunks_exact(4).all(|p| p[..3] == [0, 0, 0]));
}
#[test]
#[ignore = "requires a GPU adapter"]
fn point_light_on_the_surface_stays_finite() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let f = render(
        &format!("{HEADER}LightSource \"point\" \"point from\" [0 0 0]\n{QUAD}"),
        64,
        64,
        PreviewMode::Lit,
    );
    assert_eq!(f.rgba.len(), 64 * 64 * 4);
    assert!(f.rgba.chunks_exact(4).all(|p| p[3] == 255));
}
#[test]
#[ignore = "requires a GPU adapter"]
fn lit_without_lights_is_black_except_emitters() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let source = format!(
        "{HEADER}AttributeBegin\nTranslate 0 0 2\nAreaLightSource \"diffuse\" \"rgb L\" [0.5 0.5 0.5] \"bool twosided\" true\n{DOWN_QUAD}AttributeEnd\nMaterial \"diffuse\"\nShape \"sphere\"\n"
    );
    let f = render(&source, 64, 64, PreviewMode::Lit);
    assert_eq!([f.rgba[0], f.rgba[1], f.rgba[2]], [0; 3]);
    near_rgb(center(&f), srgb(0.5), 2);
    let sphere_id = parsed(&source).manifest.draws[1].pick_id;
    assert!(
        f.pick_ids
            .iter()
            .enumerate()
            .any(|(index, id)| *id == sphere_id
                && f.rgba[index * 4..index * 4 + 3].iter().any(|c| *c > 0)),
        "the emitter should illuminate visible sphere pixels"
    );
}
/// An area light whose `spectrum L`
/// arrives as interleaved wavelength/value pairs — the form
/// `program_from_project` expands a SpectrumData reference into — resolves
/// through the Lit channels exactly like the rgb light carrying the same
/// value, so a curve-backed light is colored, not degraded white.
#[test]
#[ignore = "requires a GPU adapter"]
fn an_area_light_spectrum_of_pairs_renders_like_the_equal_rgb_light() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let scene = |l: &str| {
        format!(
            "{HEADER}AttributeBegin\nTranslate 0 0 2\nAreaLightSource \"diffuse\" {l} \"bool twosided\" true\n{DOWN_QUAD}AttributeEnd\nMaterial \"diffuse\"\nShape \"sphere\"\n"
        )
    };
    let spectrum = render(
        &scene("\"spectrum L\" [400 0.5 700 0.5]"),
        64,
        64,
        PreviewMode::Lit,
    );
    let control = render(&scene("\"rgb L\" [0.5 0.5 0.5]"), 64, 64, PreviewMode::Lit);
    near_rgb(center(&spectrum), srgb(0.5), 2);
    let [r, g, b] = center(&spectrum);
    let [cr, cg, cb] = center(&control);
    assert!(
        [r, g, b]
            .iter()
            .zip([cr, cg, cb])
            .all(|(x, y)| x.abs_diff(y) <= 2),
        "pair spectrum {r} {g} {b} vs rgb control {cr} {cg} {cb}"
    );
}
#[test]
#[ignore = "requires a GPU adapter"]
fn pick_ids_match_between_lit_and_solid() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let s = parsed(&format!("{HEADER}{QUAD}"));
    let mut r = scene_view::PreviewRenderer::new().unwrap();
    r.upload_scene(&s.manifest, &s.blobs).unwrap();
    for (w, h) in [(64, 64), (80, 48)] {
        let camera = ViewerCamera::from_manifest(&s.manifest, w as f64 / h as f64);
        let a = r.render(camera, w, h, PreviewMode::Lit, 0.).unwrap();
        let b = r.render(camera, w, h, PreviewMode::Solid, 0.).unwrap();
        let c = r.render(camera, w, h, PreviewMode::Lit, 0.).unwrap();
        assert_eq!(a.pick_ids, b.pick_ids);
        assert_eq!(a.pick_ids, c.pick_ids);
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_huge_quad_light_fills_the_hemisphere() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let source = format!(
        "{UNDER_LIGHT}Material \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\nShape \"trianglemesh\" \"point3 P\" [-1 -1 0 1 -1 0 1 1 0 -1 1 0] \"integer indices\" [0 1 2 0 2 3]\nAttributeBegin\nTranslate 0 0 1\nAreaLightSource \"diffuse\" \"rgb L\" [1 1 1]\nShape \"trianglemesh\" \"point3 P\" [-200 -200 0 200 -200 0 200 200 0 -200 200 0] \"integer indices\" [0 3 2 0 2 1]\nAttributeEnd\n"
    );
    let frame = render(&source, 64, 64, PreviewMode::Lit);
    near_rgb(center(&frame), srgb(0.5), 3);
}

#[test]
#[ignore = "requires a GPU adapter"]
fn sphere_light_irradiance_matches_the_closed_form() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let source = format!(
        "{UNDER_LIGHT}Material \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\nShape \"trianglemesh\" \"point3 P\" [-1 -1 0 1 -1 0 1 1 0 -1 1 0] \"integer indices\" [0 1 2 0 2 3]\nAttributeBegin\nTranslate 0 0 2\nAreaLightSource \"diffuse\" \"rgb L\" [1 1 1]\nShape \"sphere\" \"float radius\" 1\nAttributeEnd\n"
    );
    let frame = render(&source, 64, 64, PreviewMode::Lit);
    near_rgb(center(&frame), srgb(0.125), 3);
}

#[test]
#[ignore = "requires a GPU adapter"]
fn polygon_light_behind_or_on_its_plane_is_zero() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let quad = "Shape \"trianglemesh\" \"point3 P\" [-2 -2 0 2 -2 0 2 2 0 -2 2 0] \"integer indices\" [0 1 2 0 2 3]\n";
    let base = format!(
        "{BELOW_PLANE}Material \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\nAttributeBegin\nTranslate 0 0 -1\n{quad}AttributeEnd\nAttributeBegin\nAreaLightSource \"diffuse\" \"rgb L\" [1 1 1]\n{quad}AttributeEnd\n"
    );
    let one_sided = render(&base, 64, 64, PreviewMode::Lit);
    assert_eq!(center(&one_sided), [0; 3]);
    let two_sided = render(
        &base.replace("[1 1 1]", "[1 1 1] \"bool twosided\" true"),
        64,
        64,
        PreviewMode::Lit,
    );
    assert!(center(&two_sided).iter().any(|c| *c > 0));
}

#[test]
#[ignore = "requires a GPU adapter"]
fn horizon_clipped_polygon_is_finite_and_symmetric() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let source = format!(
        "{HEADER}Material \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\nShape \"trianglemesh\" \"point3 P\" [-3 -3 0 3 -3 0 3 3 0 -3 3 0] \"integer indices\" [0 2 1 0 3 2]\nAttributeBegin\nTranslate 0 0 1\nAreaLightSource \"diffuse\" \"rgb L\" [1 1 1]\nShape \"trianglemesh\" \"point3 P\" [-1 -2 -1 1 -2 -1 1 2 1 -1 2 1] \"integer indices\" [0 1 2 0 2 3]\nAttributeEnd\n"
    );
    let frame = render(&source, 64, 64, PreviewMode::Lit);
    for y in 20..44 {
        assert_eq!(
            frame.rgba[((y * 64 + 20) * 4) as usize],
            frame.rgba[((y * 64 + 43) * 4) as usize]
        );
    }
}

/// Camera at z = 5 looking at a unit sphere at the origin, 64×64.
const SPHERE_VIEW: &str = "Film \"rgb\" \"integer xresolution\" 64 \"integer yresolution\" 64\nLookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" 30\nWorldBegin\n";

fn linear(value: u8) -> f64 {
    let c = value as f64 / 255.;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn red_at(frame: &scene_view::PreviewFrame, x: u32, y: u32) -> u8 {
    frame.rgba[((y * frame.width + x) * 4) as usize]
}

/// Under a uniform environment of radiance 1, a conductor seen head-on
/// reflects its directional albedo F0·magnitude + (1 − F0)·fresnel. With
/// α = sqrt(0.25) = 0.5 the committed table gives magnitude 0.68775 and
/// fresnel 0.00005, so F0 (0.9, 0.6, 0.3) shows 0.619, 0.413, 0.206.
#[test]
#[ignore = "requires a GPU adapter"]
fn rough_conductor_under_uniform_light_shows_its_directional_albedo() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let source = format!(
        "{SPHERE_VIEW}LightSource \"infinite\" \"rgb L\" [1 1 1]\nMaterial \"conductor\" \"rgb reflectance\" [0.9 0.6 0.3] \"float roughness\" 0.25\nShape \"sphere\"\n"
    );
    let frame = render(&source, 64, 64, PreviewMode::Lit);
    let got = center(&frame);
    for (channel, want) in [srgb(0.6190), srgb(0.4127), srgb(0.2064)]
        .into_iter()
        .enumerate()
    {
        assert!(
            got[channel].abs_diff(want) <= 4,
            "{got:?} vs {want} at {channel}"
        );
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn smooth_conductor_shows_a_point_light_highlight() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let source = format!(
        "{SPHERE_VIEW}LightSource \"point\" \"point3 from\" [0 0 5] \"rgb I\" [2 2 2]\nMaterial \"conductor\" \"rgb reflectance\" [0.9 0.9 0.9] \"float roughness\" 0.01\nShape \"sphere\"\n"
    );
    let frame = render(&source, 64, 64, PreviewMode::Lit);
    // A light weak enough that the peak stays below white, so the ratio is
    // measured, not clipped.
    let highlight = red_at(&frame, 32, 32);
    // About 30° around the sphere from the highlight.
    let off = red_at(&frame, 32 + 8, 32);
    assert!((150..255).contains(&highlight), "highlight {highlight}");
    assert!(
        u32::from(off) * 5 < u32::from(highlight),
        "off {off} vs {highlight}"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn mirror_conductor_reflects_a_quad_light_without_nan() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    // A roughness-0 mirror floor seen from z = 2; a 1×1 light at z = 3 faces
    // down, so its mirror image sits at the center of the view.
    let source = format!(
        "Film \"rgb\" \"integer xresolution\" 64 \"integer yresolution\" 64\nLookAt 0 0 2 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" 60\nWorldBegin\nAttributeBegin\nTranslate 0 0 3\nAreaLightSource \"diffuse\" \"rgb L\" [1 1 1]\nShape \"trianglemesh\" \"point3 P\" [-0.5 -0.5 0 0.5 -0.5 0 0.5 0.5 0 -0.5 0.5 0] \"integer indices\" [0 2 1 0 3 2]\nAttributeEnd\nMaterial \"conductor\" \"rgb reflectance\" [1 1 1] \"float roughness\" 0\nShape \"trianglemesh\" \"point3 P\" [-4 -4 0 4 -4 0 4 4 0 -4 4 0] \"integer indices\" [0 1 2 0 2 3]\n"
    );
    let frame = render(&source, 64, 64, PreviewMode::Lit);
    assert!(red_at(&frame, 32, 32) > 200, "the light's reflection");
    assert!(red_at(&frame, 2, 2) < 40, "outside the reflection");
}

#[test]
#[ignore = "requires a GPU adapter"]
fn coat_adds_a_highlight_over_diffuse() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let scene = |material: &str| {
        format!(
            "{SPHERE_VIEW}LightSource \"point\" \"point3 from\" [0 0 5] \"rgb I\" [10 10 10]\nMaterial \"{material}\" \"rgb reflectance\" [0.5 0.5 0.5] \"float roughness\" 0.01\nShape \"sphere\"\n"
        )
    };
    let coated = render(&scene("coateddiffuse"), 64, 64, PreviewMode::Lit);
    let plain = render(&scene("diffuse"), 64, 64, PreviewMode::Lit);
    // The coat puts a highlight on top of the diffuse layer; plain diffuse
    // has none.
    assert!(
        red_at(&coated, 32, 32) > red_at(&coated, 40, 32) + 30,
        "coat highlight"
    );
    assert!(
        red_at(&plain, 32, 32) < red_at(&plain, 40, 32) + 10,
        "no highlight without a coat"
    );
    // Away from the highlight the diffuse layer is dimmed by the coat:
    // ρ·T_in·T_out/(η²(1 − ρ·r_i)) ≈ 0.58 of plain diffuse for ρ = 0.5, η = 1.5.
    let ratio = linear(red_at(&coated, 40, 32)) / linear(red_at(&plain, 40, 32));
    assert!(
        (0.45..0.75).contains(&ratio),
        "coated/plain diffuse {ratio}"
    );
}

/// Inside a sphere light, a two-sided one surrounds the point with radiance
/// L (diffuse ρL, specular L·albedo), while a one-sided one emits only
/// outward and leaves the inside dark, as in pbrt (review #265).
#[test]
#[ignore = "requires a GPU adapter"]
fn inside_a_sphere_light_two_sided_glows_and_one_sided_is_dark() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let scene = |two_sided: bool, material: &str| {
        format!(
            "Film \"rgb\" \"integer xresolution\" 64 \"integer yresolution\" 64\nLookAt 0 0 2 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" 30\nWorldBegin\nAttributeBegin\nAreaLightSource \"diffuse\" \"rgb L\" [1 1 1] \"bool twosided\" {two_sided}\nShape \"sphere\" \"float radius\" 10\nAttributeEnd\n{material}\nShape \"trianglemesh\" \"point3 P\" [-1 -1 0 1 -1 0 1 1 0 -1 1 0] \"integer indices\" [0 1 2 0 2 3]\n"
        )
    };
    let diffuse = "Material \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]";
    let metal = "Material \"conductor\" \"rgb reflectance\" [0.9 0.9 0.9] \"float roughness\" 0.25";
    near_rgb(
        center(&render(&scene(true, diffuse), 64, 64, PreviewMode::Lit)),
        srgb(0.5),
        4,
    );
    assert!(center(&render(&scene(true, metal), 64, 64, PreviewMode::Lit))[0] > 150);
    assert_eq!(
        center(&render(&scene(false, diffuse), 64, 64, PreviewMode::Lit)),
        [0; 3]
    );
    assert_eq!(
        center(&render(&scene(false, metal), 64, 64, PreviewMode::Lit)),
        [0; 3]
    );
}

/// Texture inputs per pixel: a 2×2 checkerboard input shows, in each cell,
/// what a constant input of that cell's value shows. The quad is lit by a
/// uniform environment and two point lights mirrored into the top two cell
/// centers (world x = ±0.5), so reflectance and roughness both show there.
#[test]
#[ignore = "requires a GPU adapter"]
fn checkerboard_inputs_match_constant_inputs_per_cell() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let scene = |textures: &str, material: &str| {
        format!(
            "Film \"rgb\" \"integer xresolution\" 64 \"integer yresolution\" 64\nLookAt 0 0 3 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" 50\nWorldBegin\n{textures}LightSource \"infinite\" \"rgb L\" [0.4 0.4 0.4]\nLightSource \"point\" \"point3 from\" [0.83 0.83 2] \"rgb I\" [2 2 2]\nLightSource \"point\" \"point3 from\" [-0.83 0.83 2] \"rgb I\" [2 2 2]\nMaterial {material}\nShape \"trianglemesh\" \"point3 P\" [-1 -1 0 1 -1 0 1 1 0 -1 1 0] \"texcoord2 uv\" [0 0 1 0 1 1 0 1] \"integer indices\" [0 1 2 0 2 3]\n"
        )
    };
    let spectrum = "Texture \"t\" \"spectrum\" \"checkerboard\" \"float uscale\" [2] \"float vscale\" [2] \"rgb tex1\" [0.9 0.2 0.1] \"rgb tex2\" [0.1 0.3 0.8]\n";
    let float = "Texture \"t\" \"float\" \"checkerboard\" \"float uscale\" [2] \"float vscale\" [2] \"float tex1\" [0.02] \"float tex2\" [0.5]\n";
    let (red, blue) = (
        "\"rgb reflectance\" [0.9 0.2 0.1]",
        "\"rgb reflectance\" [0.1 0.3 0.8]",
    );
    // (material with the textured input, the same with each constant)
    let cases: Vec<(&str, String, String, String)> = vec![
        (
            "conductor reflectance",
            format!("\"conductor\" \"texture reflectance\" \"t\" \"float roughness\" [0.2]"),
            format!("\"conductor\" {red} \"float roughness\" [0.2]"),
            format!("\"conductor\" {blue} \"float roughness\" [0.2]"),
        ),
        (
            "coateddiffuse reflectance",
            "\"coateddiffuse\" \"texture reflectance\" \"t\"".into(),
            format!("\"coateddiffuse\" {red}"),
            format!("\"coateddiffuse\" {blue}"),
        ),
        (
            "coatedconductor reflectance",
            "\"coatedconductor\" \"texture reflectance\" \"t\"".into(),
            format!("\"coatedconductor\" {red}"),
            format!("\"coatedconductor\" {blue}"),
        ),
        (
            "diffusetransmission reflectance",
            "\"diffusetransmission\" \"texture reflectance\" \"t\"".into(),
            format!("\"diffusetransmission\" {red}"),
            format!("\"diffusetransmission\" {blue}"),
        ),
    ];
    let rough = |material: &str, name: &str, extra: &str| {
        (
            format!("\"{material}\" \"texture {name}\" \"t\" {extra}"),
            format!("\"{material}\" \"float {name}\" [0.02] {extra}"),
            format!("\"{material}\" \"float {name}\" [0.5] {extra}"),
        )
    };
    let float_cases = [
        ("conductor roughness", rough("conductor", "roughness", "")),
        (
            "conductor no remap",
            rough("conductor", "roughness", "\"bool remaproughness\" false"),
        ),
        (
            "coateddiffuse roughness",
            rough("coateddiffuse", "roughness", ""),
        ),
        (
            "coatedconductor conductor.roughness",
            rough("coatedconductor", "conductor.roughness", ""),
        ),
        (
            "coatedconductor interface.roughness",
            rough("coatedconductor", "interface.roughness", ""),
        ),
    ];
    let mut all = vec![];
    for (name, textured, a, b) in cases {
        all.push((name, spectrum, textured, a, b));
    }
    for (name, (textured, a, b)) in float_cases {
        all.push((name, float, textured, a, b));
    }
    for (name, textures, textured, first, second) in all {
        let frame = render(&scene(textures, &textured), 64, 64, PreviewMode::Lit);
        let one = render(&scene("", &first), 64, 64, PreviewMode::Lit);
        let two = render(&scene("", &second), 64, 64, PreviewMode::Lit);
        let at = |f: &scene_view::PreviewFrame, x: u32, y: u32| -> [u8; 3] {
            let i = ((y * f.width + x) * 4) as usize;
            [f.rgba[i], f.rgba[i + 1], f.rgba[i + 2]]
        };
        let close = |a: [u8; 3], b: [u8; 3]| (0..3).all(|c| a[c].abs_diff(b[c]) <= 3);
        // Cell centers on each side of the quad's UV midlines.
        for (x, y) in [(22, 22), (42, 22), (22, 42), (42, 42)] {
            let got = at(&frame, x, y);
            assert!(
                close(got, at(&one, x, y)) || close(got, at(&two, x, y)),
                "{name} at ({x},{y}): {got:?} vs {:?} / {:?}",
                at(&one, x, y),
                at(&two, x, y)
            );
        }
        // Parity decides the constant: diagonal cells share one, neighbours
        // take the other. Shading still varies with position, so each pixel
        // compares against the constant render at that same pixel.
        let first = |x: u32, y: u32| close(at(&frame, x, y), at(&one, x, y));
        assert_eq!(first(22, 22), first(42, 42), "{name}: diagonal cells");
        assert_ne!(first(22, 22), first(42, 22), "{name}: neighbouring cells");
    }
}

/// Eye-relative positions with f64 rows keep a pattern exact in a scene far
/// from the world's origin: the same scene moved 1e6 away looks the same.
#[test]
#[ignore = "requires a GPU adapter"]
fn mapped_patterns_hold_in_scenes_far_from_the_origin() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let scene = |x: f64| {
        format!(
            "Film \"rgb\" \"integer xresolution\" 64 \"integer yresolution\" 64\nLookAt {x} 0 4 {x} 0 0 0 1 0\nCamera \"perspective\" \"float fov\" 50\nWorldBegin\nLightSource \"infinite\" \"rgb L\" [1 1 1]\nAttributeBegin\nTranslate {x} 0 0\nTexture \"t\" \"spectrum\" \"checkerboard\" \"string mapping\" \"planar\" \"rgb tex1\" [0.9 0.2 0.1] \"rgb tex2\" [0.1 0.3 0.8]\nAttributeEnd\nMaterial \"diffuse\" \"texture reflectance\" \"t\"\nTranslate {x} 0 0\nShape \"trianglemesh\" \"point3 P\" [-2 -2 0 2 -2 0 2 2 0 -2 2 0] \"integer indices\" [0 1 2 0 2 3]\n"
        )
    };
    let near = render(&scene(0.), 64, 64, PreviewMode::Lit);
    let far = render(&scene(1e6), 64, 64, PreviewMode::Lit);
    let different = near
        .rgba
        .iter()
        .zip(&far.rgba)
        .filter(|(a, b)| a.abs_diff(**b) > 8)
        .count();
    assert!(different < 40, "{different} channels differ");
}

/// r4 clamps a diffuse reflectance to [0, 1], and noise and marble go beyond it.
#[test]
#[ignore = "requires a GPU adapter"]
fn out_of_range_reflectance_shades_as_clamped() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let scene = |textures: &str, material: &str| {
        format!(
            "Film \"rgb\" \"integer xresolution\" 32 \"integer yresolution\" 32\nLookAt 0 0 3 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" 50\nWorldBegin\n{textures}LightSource \"point\" \"point3 from\" [0 0 3] \"rgb I\" [2 2 2]\nMaterial \"diffuse\" {material}\nShape \"trianglemesh\" \"point3 P\" [-3 -3 0 3 -3 0 3 3 0 -3 3 0] \"integer indices\" [0 1 2 0 2 3]\n"
        )
    };
    let center = |source: &str| {
        let frame = render(source, 32, 32, PreviewMode::Lit);
        frame.rgba[((16 * 32 + 16) * 4) as usize]
    };
    let textured = "\"texture reflectance\" \"t\"";
    let over = "Texture \"t\" \"spectrum\" \"constant\" \"rgb value\" [4 4 4]\n";
    let white = center(&scene("", "\"rgb reflectance\" [1 1 1]"));
    assert_eq!(center(&scene(over, textured)), white);
    assert_eq!(center(&scene("", "\"rgb reflectance\" [4 4 4]")), white);
    let under = format!(
        "{over}Texture \"u\" \"spectrum\" \"scale\" \"texture tex\" \"t\" \"float scale\" [-1]\n"
    );
    let negative = scene(&under, "\"texture reflectance\" \"u\"");
    assert_eq!(
        center(&negative),
        center(&scene("", "\"rgb reflectance\" [0 0 0]"))
    );
}

/// Near a spherical mapping's pole phi races and theta barely moves, so an image
/// banded along t must filter to its average there. Derivatives swapped between
/// s and t leave the bands sharp.
#[test]
#[ignore = "requires a GPU adapter"]
fn spherical_images_filter_along_the_coordinate_that_changes() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let bands = image::RgbImage::from_fn(2, 256, |_, y| {
        image::Rgb(if (y / 8) % 2 == 0 { [255; 3] } else { [0; 3] })
    });
    bands.save(dir.path().join("bands.png")).unwrap();
    let path = dir.path().join("scene.pbrt");
    std::fs::write(
        &path,
        "Film \"rgb\" \"integer xresolution\" 64 \"integer yresolution\" 64\nLookAt 0 0 3 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" 50\nWorldBegin\nTexture \"t\" \"spectrum\" \"imagemap\" \"string filename\" \"bands.png\" \"string encoding\" \"linear\" \"string mapping\" \"spherical\"\nMaterial \"diffuse\" \"texture reflectance\" \"t\"\nShape \"sphere\"\n",
    )
    .unwrap();
    let scene = scene_parse::parse_scene(path, scene_parse::ParseOptions::default()).unwrap();
    let frame = scene_view::render_preview_at_time_with_options(
        &scene.manifest,
        &scene.blobs,
        ViewerCamera::from_manifest(&scene.manifest, 1.0),
        64,
        64,
        PreviewMode::Lit,
        0.0,
        scene_view::LitDebug::Albedo,
    )
    .unwrap();
    // A ring two to three pixels around the pole: every phi, one filtered gray.
    let mut ring = Vec::new();
    for y in 29..35i32 {
        for x in 29..35i32 {
            let d2 = (x - 32).pow(2) + (y - 32).pow(2);
            if (4..=9).contains(&d2) {
                ring.push(frame.rgba[((y * 64 + x) * 4) as usize]);
            }
        }
    }
    let (low, high) = (ring.iter().min().unwrap(), ring.iter().max().unwrap());
    assert!(high - low < 60, "the bands stay sharp: {ring:?}");
}

/// A constant map is the uniform light by another route: every level of the
/// prefiltered cube must hold that same color, for every roughness.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_constant_map_shades_like_the_uniform_light() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    image::RgbImage::from_pixel(8, 8, image::Rgb([188, 188, 188]))
        .save(dir.path().join("gray.png"))
        .unwrap();
    // sRGB 188 in linear.
    let gray = ((188. / 255. + 0.055f32) / 1.055).powf(2.4);
    let scene = |light: &str, material: &str| {
        let path = dir.path().join("scene.pbrt");
        std::fs::write(
            &path,
            format!("Film \"rgb\" \"integer xresolution\" 48 \"integer yresolution\" 48\nLookAt 0 0 3 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" 50\nWorldBegin\n{light}\nMaterial {material}\nShape \"sphere\" \"float radius\" 1\n"),
        )
        .unwrap();
        let scene = scene_parse::parse_scene(path, scene_parse::ParseOptions::default()).unwrap();
        render_preview(
            &scene.manifest,
            &scene.blobs,
            ViewerCamera::from_manifest(&scene.manifest, 1.0),
            48,
            48,
            PreviewMode::Lit,
        )
        .unwrap()
    };
    let map = "LightSource \"infinite\" \"string filename\" \"gray.png\"";
    let uniform = format!("LightSource \"infinite\" \"rgb L\" [{gray} {gray} {gray}]");
    for material in [
        "\"diffuse\" \"rgb reflectance\" [0.7 0.7 0.7]",
        "\"conductor\" \"rgb reflectance\" [0.9 0.9 0.9] \"float roughness\" [0.001]",
        "\"conductor\" \"rgb reflectance\" [0.9 0.9 0.9] \"float roughness\" [0.1]",
        "\"conductor\" \"rgb reflectance\" [0.9 0.9 0.9] \"float roughness\" [0.8]",
        "\"coatedconductor\" \"rgb reflectance\" [0.8 0.6 0.3]",
    ] {
        let (a, b) = (scene(map, material), scene(&uniform, material));
        let worst = a
            .rgba
            .iter()
            .zip(&b.rgba)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap();
        assert!(worst <= 3, "{material}: differs by up to {worst}");
    }
}
