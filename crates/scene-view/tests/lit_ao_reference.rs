//! Lit's screen-space AO against r4's visibility. Under a uniform `infinite`
//! light at `--maxdepth 1`, a diffuse surface of reflectance 0.5 shows
//! `0.5 · V`, so twice r4's linear pixel is the cosine-weighted visibility
//! the AO approximates. See `lit_ao_reference/README.md`.
use scene_view::{PreviewMode, PreviewRenderer, ViewerCamera};
use std::path::PathBuf;

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

const WIDTH: u32 = 160;
const HEIGHT: u32 = 120;
const TILE: u32 = 8;

/// Per scene, the maximum mean and largest absolute tile error allowed:
/// measured at radius 0.2 of the diagonal plus a margin. With AO off the
/// mean errors are 0.13, 0.54, 0.15 and 0.14.
const BUDGETS: [(&str, f64, f64); 4] = [
    ("ao_contact", 0.06, 0.30),
    // Off-screen walls and the floor's half-sky occlusion of vertical
    // surfaces reach beyond any screen-space radius (measured 0.35, 0.48).
    ("ao_corner", 0.40, 0.55),
    ("ao_stack", 0.07, 0.33),
    // Halos where the floor shows through the cutout's holes (0.08, 0.39).
    ("ao_alpha", 0.11, 0.45),
];

fn linear(value: u8) -> f64 {
    let value = value as f64 / 255.;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn lit_ao_matches_r4_visibility_per_tile() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/lit_ao_reference");
    let mut renderer = PreviewRenderer::new().unwrap();
    let mut report = Vec::new();
    let mut failed = false;
    for (name, mean_budget, max_budget) in BUDGETS {
        let scene = scene_parse::parse_scene(
            root.join(format!("{name}.pbrt")),
            scene_parse::ParseOptions::default(),
        )
        .unwrap();
        let camera = ViewerCamera::from_manifest(&scene.manifest, WIDTH as f64 / HEIGHT as f64);
        renderer
            .upload_scene(&scene.manifest, &scene.blobs)
            .unwrap();
        renderer
            .render(camera, WIDTH, HEIGHT, PreviewMode::Lit, 0.)
            .unwrap();
        let ao = renderer.read_ao().unwrap();
        let (depth, _) = renderer.read_prez().unwrap();
        let reference = image::open(root.join(format!("{name}.png")))
            .unwrap()
            .to_rgb8();
        let mut errors = Vec::new();
        for ty in 0..HEIGHT / TILE {
            for tx in 0..WIDTH / TILE {
                let pixels: Vec<(u32, u32)> = (0..TILE * TILE)
                    .map(|i| (tx * TILE + i % TILE, ty * TILE + i / TILE))
                    .collect();
                // Only tiles wholly on geometry; r4 shows the sky elsewhere.
                if pixels
                    .iter()
                    .any(|(x, y)| depth[(y * WIDTH + x) as usize] <= 0.)
                {
                    continue;
                }
                let n = pixels.len() as f64;
                let lit: f64 = pixels
                    .iter()
                    .map(|(x, y)| ao[(y * WIDTH + x) as usize] as f64)
                    .sum::<f64>()
                    / n;
                let r4: f64 = pixels
                    .iter()
                    .map(|(x, y)| 2. * linear(reference.get_pixel(*x, *y)[1]))
                    .sum::<f64>()
                    / n;
                errors.push((lit - r4).abs());
            }
        }
        let mean = errors.iter().sum::<f64>() / errors.len() as f64;
        let max = errors.iter().copied().fold(0., f64::max);
        failed |= mean > mean_budget || max > max_budget;
        report.push(format!(
            "{name}: {} tiles, mean {mean:.3} (≤ {mean_budget}), max {max:.3} (≤ {max_budget})",
            errors.len()
        ));
    }
    assert!(!failed, "AO differs from r4:\n{}", report.join("\n"));
    eprintln!("{}", report.join("\n"));
}

/// Per scene, as `BUDGETS`, for Lit's own color (linear, green): AO scales
/// the environment's diffuse, specular and coat. With AO off the mean errors
/// are 0.069, 0.078, 0.073, 0.273, 0.083, 0.106 and 0.092; with the
/// specular left unoccluded, 0.070 and 0.093 on the two metal floors.
const COLOR_BUDGETS: [(&str, f64, f64); 7] = [
    ("ao_contact", 0.035, 0.17),
    ("ao_stack", 0.035, 0.19),
    ("ao_alpha", 0.06, 0.26),
    ("ao_corner", 0.22, 0.30),
    ("ao_glossy_rough", 0.05, 0.20),
    ("ao_glossy_mid", 0.08, 0.33),
    ("ao_coated", 0.07, 0.16),
];

#[test]
#[ignore = "requires a GPU adapter"]
fn lit_matches_r4_under_a_uniform_sky_per_tile() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/lit_ao_reference");
    let mut renderer = PreviewRenderer::new().unwrap();
    let mut report = Vec::new();
    let mut failed = false;
    for (name, mean_budget, max_budget) in COLOR_BUDGETS {
        let scene = scene_parse::parse_scene(
            root.join(format!("{name}.pbrt")),
            scene_parse::ParseOptions::default(),
        )
        .unwrap();
        let camera = ViewerCamera::from_manifest(&scene.manifest, WIDTH as f64 / HEIGHT as f64);
        renderer
            .upload_scene(&scene.manifest, &scene.blobs)
            .unwrap();
        let frame = renderer
            .render(camera, WIDTH, HEIGHT, PreviewMode::Lit, 0.)
            .unwrap();
        let (depth, _) = renderer.read_prez().unwrap();
        let reference = image::open(root.join(format!("{name}.png")))
            .unwrap()
            .to_rgb8();
        let mut errors = Vec::new();
        for ty in 0..HEIGHT / TILE {
            for tx in 0..WIDTH / TILE {
                let pixels: Vec<(u32, u32)> = (0..TILE * TILE)
                    .map(|i| (tx * TILE + i % TILE, ty * TILE + i / TILE))
                    .collect();
                if pixels
                    .iter()
                    .any(|(x, y)| depth[(y * WIDTH + x) as usize] <= 0.)
                {
                    continue;
                }
                let n = pixels.len() as f64;
                let lit: f64 = pixels
                    .iter()
                    .map(|(x, y)| linear(frame.rgba[((y * WIDTH + x) * 4 + 1) as usize]))
                    .sum::<f64>()
                    / n;
                let r4: f64 = pixels
                    .iter()
                    .map(|(x, y)| linear(reference.get_pixel(*x, *y)[1]))
                    .sum::<f64>()
                    / n;
                errors.push((lit - r4).abs());
            }
        }
        let mean = errors.iter().sum::<f64>() / errors.len() as f64;
        let max = errors.iter().copied().fold(0., f64::max);
        failed |= mean > mean_budget || max > max_budget;
        report.push(format!(
            "{name}: {} tiles, mean {mean:.3} (≤ {mean_budget}), max {max:.3} (≤ {max_budget})",
            errors.len()
        ));
    }
    assert!(!failed, "Lit differs from r4:\n{}", report.join("\n"));
    eprintln!("{}", report.join("\n"));
}

/// Glass stays out of PreZ, so the floor beside it keeps AO 1. r4 darkens it;
/// Lit deliberately does not, as it shows glass without refraction.
#[test]
#[ignore = "requires a GPU adapter"]
fn glass_casts_no_ao() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/lit_ao_reference");
    let scene = scene_parse::parse_scene(
        root.join("ao_glass.pbrt"),
        scene_parse::ParseOptions::default(),
    )
    .unwrap();
    let camera = ViewerCamera::from_manifest(&scene.manifest, WIDTH as f64 / HEIGHT as f64);
    let mut renderer = PreviewRenderer::new().unwrap();
    renderer
        .upload_scene(&scene.manifest, &scene.blobs)
        .unwrap();
    renderer
        .render(camera, WIDTH, HEIGHT, PreviewMode::Lit, 0.)
        .unwrap();
    let ao = renderer.read_ao().unwrap();
    let lowest = ao.iter().copied().fold(1., f32::min);
    assert!(
        lowest > 0.95,
        "lowest AO {lowest} with only glass above the floor"
    );
}

/// Draws in the transparent pass take no AO: a mix of a matte and an
/// `interface` leaf is blended over the floor, and its own shading must not
/// pick up the AO of the floor behind it.
#[test]
#[ignore = "requires a GPU adapter"]
fn transparent_draws_receive_no_ao() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/lit_ao_reference");
    let base = std::fs::read_to_string(root.join("ao_contact.pbrt")).unwrap();
    let veil = "AttributeBegin\nMakeNamedMaterial \"matte\" \"string type\" \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\nMakeNamedMaterial \"none\" \"string type\" \"interface\"\nMaterial \"mix\" \"string materials\" [\"matte\" \"none\"] \"float amount\" [0.5]\nShape \"trianglemesh\" \"point3 P\" [-4 -2.5 0.2 4 -2.5 0.2 4 -2.5 1.7 -4 -2.5 1.7] \"integer indices\" [0 1 2 0 2 3]\nAttributeEnd\n";
    let directory = tempfile::tempdir().unwrap();
    let render = |source: &str| {
        let path = directory.path().join("scene.pbrt");
        std::fs::write(&path, source).unwrap();
        let scene = scene_parse::parse_scene(path, scene_parse::ParseOptions::default()).unwrap();
        let camera = ViewerCamera::from_manifest(&scene.manifest, WIDTH as f64 / HEIGHT as f64);
        let mut renderer = PreviewRenderer::new().unwrap();
        renderer
            .upload_scene(&scene.manifest, &scene.blobs)
            .unwrap();
        let frame = renderer
            .render(camera, WIDTH, HEIGHT, PreviewMode::Lit, 0.)
            .unwrap();
        let ao = renderer.read_ao().unwrap();
        let pick = scene.manifest.draws.last().unwrap().pick_id;
        (frame, ao, pick)
    };
    let (without, _, _) = render(&base);
    let (with, ao, veil_pick) = render(&format!("{base}{veil}"));
    // The veil's alpha is 0.5, so its own premultiplied color is what it adds
    // over half of the floor behind it.
    let mut own = Vec::new();
    for i in 0..(WIDTH * HEIGHT) as usize {
        if with.pick_ids[i] == veil_pick {
            let behind = linear(without.rgba[i * 4 + 1]);
            own.push((linear(with.rgba[i * 4 + 1]) - 0.5 * behind, ao[i]));
        }
    }
    assert!(own.len() > 1000, "the veil covers {} pixels", own.len());
    let ao_range = own.iter().map(|(_, a)| *a).fold(1., f32::min);
    assert!(
        ao_range < 0.6,
        "the veil must cover occluded floor (lowest AO {ao_range})"
    );
    let lowest = own.iter().map(|(c, _)| *c).fold(f64::MAX, f64::min);
    let highest = own.iter().map(|(c, _)| *c).fold(f64::MIN, f64::max);
    assert!(
        highest - lowest < 0.02,
        "the veil's own shading varies {lowest:.3}..{highest:.3} with the AO behind it"
    );
}
