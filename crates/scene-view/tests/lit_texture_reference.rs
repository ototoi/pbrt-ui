//! Pixel comparisons for texture programs; deliberately separate from Lit region tests.
use scene_view::{PreviewMode, ViewerCamera, render_preview_at_time_with_options};
use std::path::PathBuf;
use std::sync::Mutex;

// GPU device creation is not safe across parallel test threads.
static GPU: Mutex<()> = Mutex::new(());

fn linear(value: u8) -> f64 {
    let value = value as f64 / 255.;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

// The projected square spans x/y 33.1..94.9. Exclude its silhouette and
// two pixels around the predefined UV discontinuities at x/y 64. These
// bounds come from the scene geometry, never from observed error pixels.
fn compared(x: u32, y: u32) -> bool {
    (35..93).contains(&x)
        && (35..93).contains(&y)
        && !(62..66).contains(&x)
        && !(62..66).contains(&y)
}

#[test]
fn texture_reference_mask_has_fixed_coverage() {
    let valid = (0..128)
        .flat_map(|y| (0..128).map(move |x| (x, y)))
        .filter(|&(x, y)| compared(x, y))
        .count();
    assert_eq!(valid, 2916);
    assert_eq!(128 * 128 - valid, 13468);
}

#[test]
#[ignore = "requires a GPU adapter"]
fn texture_vm_matches_r4_per_pixel() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/lit_texture_reference");
    for name in ["imagemap", "scale", "mix"] {
        let scene = scene_parse::parse_scene(
            root.join(format!("{name}.pbrt")),
            scene_parse::ParseOptions::default(),
        )
        .unwrap();
        let frame = render_preview_at_time_with_options(
            &scene.manifest,
            &scene.blobs,
            ViewerCamera::from_manifest(&scene.manifest, 1.0),
            128,
            128,
            PreviewMode::Lit,
            0.0,
            scene_view::LitDebug::Albedo,
        )
        .unwrap();
        let reference = image::open(root.join(format!("{name}.png")))
            .unwrap()
            .to_rgb8();
        assert_eq!(reference.dimensions(), (128, 128));
        let mut failures = Vec::new();
        let mut valid = 0;
        for y in 0..128 {
            for x in 0..128 {
                if !compared(x, y) {
                    continue;
                }
                valid += 1;
                for channel in 0..3 {
                    let actual = linear(frame.rgba[((y * 128 + x) * 4) as usize + channel]);
                    let expected = linear(reference.get_pixel(x, y)[channel]);
                    let error = (actual - expected).abs();
                    if error > 0.02 + 0.03 * expected.abs() {
                        failures.push(format!(
                            "({x},{y}) channel {channel}: actual={actual:.5}, r4={expected:.5}"
                        ));
                    }
                }
            }
        }
        assert_eq!(valid, 2916);
        assert!(
            failures.is_empty(),
            "{name}: {} failing channels of {}; first errors: {:?}",
            failures.len(),
            valid * 3,
            &failures[..failures.len().min(8)]
        );
    }
}

/// Render `name` as debug_albedo and collect r4's linear image alongside it.
fn albedo_and_reference(root: &std::path::Path, name: &str) -> (Vec<[f64; 3]>, Vec<[f64; 3]>) {
    let scene = scene_parse::parse_scene(
        root.join(format!("{name}.pbrt")),
        scene_parse::ParseOptions::default(),
    )
    .unwrap();
    let frame = render_preview_at_time_with_options(
        &scene.manifest,
        &scene.blobs,
        ViewerCamera::from_manifest(&scene.manifest, 1.0),
        128,
        128,
        PreviewMode::Lit,
        0.0,
        scene_view::LitDebug::Albedo,
    )
    .unwrap();
    let reference = image::open(root.join(format!("{name}.png")))
        .unwrap()
        .to_rgb8();
    let mut lit = Vec::new();
    let mut r4 = Vec::new();
    for y in 0..128 {
        for x in 0..128 {
            let offset = ((y * 128 + x) * 4) as usize;
            lit.push(std::array::from_fn(|c| linear(frame.rgba[offset + c])));
            let pixel = reference.get_pixel(x, y);
            r4.push(std::array::from_fn(|c| linear(pixel[c])));
        }
    }
    (lit, r4)
}

/// Pixels of r4's image away from a discontinuity: nothing within two pixels
/// differs by more than 0.1. The mask comes from the reference alone, so a
/// misplaced pattern in Lit still fails everywhere it lands off the edges.
fn away_from_reference_edges(r4: &[[f64; 3]], x: i32, y: i32) -> bool {
    let at = |x: i32, y: i32| r4[(y * 128 + x) as usize];
    let center = at(x, y);
    (-2..=2).all(|dy| {
        (-2..=2).all(|dx| {
            let (nx, ny) = (x + dx, y + dy);
            !(0..128).contains(&nx)
                || !(0..128).contains(&ny)
                || (0..3).all(|c| (at(nx, ny)[c] - center[c]).abs() <= 0.1)
        })
    })
}

/// Each scene with the pixels it must at least compare; a mask that grew to
/// swallow the pattern would fail here. The quad scenes compare inside its
/// silhouette (projected 33.1..94.9); the sphere projects to radius 48.6.
#[test]
#[ignore = "requires a GPU adapter"]
fn procedural_textures_match_r4_per_pixel() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/lit_texture_reference");
    // Noise compares away from the limb: see the README.
    for (name, radius, minimum) in [
        ("checkerboard", None, 1400),
        ("bilerp", None, 3000),
        ("dots", None, 1500),
        ("uv", None, 1500),
        ("composite", None, 600),
        ("material_mix", None, 1500),
        ("alpha", None, 1500),
        ("sphere_uv", None, 1000),
        ("cylinder_uv", None, 1000),
        ("cone_uv", None, 1000),
        ("disk_uv", None, 1000),
        ("bilinear_uv", None, 1000),
        ("bilinear_uv_tex", None, 1000),
        ("heightfield_uv", None, 1000),
        // The bowls' rims alias differently in Lit's rasterizer and r4's
        // gaussian-filtered sampler, so they compare inside the disk half
        // their 37/31-pixel silhouettes (as `directionmix` does near the
        // sphere's limb) and away from Lit's root-flip bands.
        ("paraboloid_uv", Some(24), 1000),
        ("hyperboloid_uv", Some(21), 1000),
        ("nurbs_uv", None, 1000),
        ("directionmix", Some(46), 5000),
        ("map_planar", None, 1500),
        ("map_instance", None, 1000),
        ("map_spherical", Some(46), 5000),
        ("map_cylindrical", Some(46), 5000),
        ("solid_checkerboard", Some(46), 3000),
        ("solid_fbm", Some(40), 4500),
        ("solid_wrinkled", Some(40), 2000),
        ("solid_windy", Some(40), 4000),
        ("solid_marble", Some(40), 2500),
    ] {
        let (lit, r4) = albedo_and_reference(&root, name);
        let mut failures = Vec::new();
        let mut valid = 0;
        for y in 0..128i32 {
            for x in 0..128i32 {
                let inside = match radius {
                    Some(radius) => (x - 64).pow(2) + (y - 64).pow(2) < radius * radius,
                    None => (35..93).contains(&x) && (35..93).contains(&y),
                };
                // Marble's spline jumps by little, and r4 averages the jump across a
                // pixel, which leaves it under the threshold on both sides. The
                // two bowls also compare away from Lit's root-flip transitions:
                // rays that reach their inner wall through the rim blend in the
                // rasterizer's own AA band, which r4's mask cannot see.
                if !inside
                    || !away_from_reference_edges(&r4, x, y)
                    || matches!(name, "solid_marble" | "paraboloid_uv" | "hyperboloid_uv")
                        && !away_from_reference_edges(&lit, x, y)
                {
                    continue;
                }
                valid += 1;
                let index = (y * 128 + x) as usize;
                for c in 0..3 {
                    let error = (lit[index][c] - r4[index][c]).abs();
                    if error > 0.02 + 0.03 * r4[index][c].abs() {
                        failures.push(format!(
                            "({x},{y}) channel {c}: lit={:.4} r4={:.4}",
                            lit[index][c], r4[index][c]
                        ));
                    }
                }
            }
        }
        assert!(valid >= minimum, "{name}: only {valid} pixels compared");
        assert!(
            failures.is_empty(),
            "{name}: {} failing channels of {}; first errors: {:?}",
            failures.len(),
            valid * 3,
            &failures[..failures.len().min(6)]
        );
    }
}
