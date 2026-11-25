use scene_view::{PreviewMode, ViewerCamera, render_preview};
use std::path::PathBuf;
use std::sync::Mutex;
static GPU: Mutex<()> = Mutex::new(());

const NAMES: [&str; 44] = [
    "point_sphere",
    "spot_plane",
    "distant_sphere",
    "infinite_sphere",
    "translated_point",
    "blackbody_sphere",
    "small_emitter",
    "quad_light_floor",
    "sphere_light_floor",
    "disk_light_floor",
    "quad_light_occluder",
    "point_shadow",
    "distant_shadow",
    "conductor_infinite",
    "conductor_quad",
    "coateddiffuse_point",
    "plastic_v3_quad",
    "coatedconductor_infinite",
    "alpha_shadow",
    "mix_shadow",
    "goniometric_floor",
    "goniometric_power",
    "projection_floor",
    "projection_power",
    "spot_shadow",
    "env_diffuse",
    "env_rotated",
    "env_scale",
    "env_illuminance",
    "env_mapname",
    "env_png",
    "env_two_maps",
    "env_and_uniform",
    "env_view_px",
    "env_view_nx",
    "env_view_py",
    "env_view_ny",
    "env_view_pz",
    "env_view_nz",
    "env_conductor_smooth",
    "env_conductor_mid",
    "env_conductor_rough",
    "env_coateddiffuse",
    "env_coatedconductor",
];
/// Relative tolerance per region and channel.
const TOLERANCE: f64 = 0.15;
/// Wider tolerance for what Lit only approximates:
/// the diffuse layer under a coat (pbrt's stochastic layered BxDF; Lit's
/// smooth-coat closed form is 17-18% high on high-albedo channels), and a
/// GGX reflection seen at grazing angles, where LTC's fit is weakest
/// (18% low at the top of the gold sphere).
const APPROXIMATE_TOLERANCE: f64 = 0.25;
const APPROXIMATE: [&str; 4] = [
    "coateddiffuse_point",
    "env_coateddiffuse",
    "plastic_v3_quad",
    "conductor_quad",
];

fn linear(value: u8) -> f64 {
    let value = value as f64 / 255.;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn parse(name: &str) -> scene_parse::PreviewGeometry {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/lit_reference");
    scene_parse::parse_scene(
        root.join(format!("{name}.pbrt")),
        scene_parse::ParseOptions::default(),
    )
    .unwrap()
}

#[test]
#[ignore = "requires a GPU adapter"]
fn lit_matches_r4_reference_regions() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/lit_reference");
    let mut failures = Vec::new();
    for name in NAMES {
        let scene = parse(name);
        let camera = ViewerCamera::from_manifest(&scene.manifest, 1.0);
        let frame = render_preview(
            &scene.manifest,
            &scene.blobs,
            camera,
            128,
            128,
            PreviewMode::Lit,
        )
        .unwrap();
        let reference = image::open(root.join(format!("{name}.png")))
            .unwrap()
            .to_rgb8();
        for ry in 0..4 {
            for rx in 0..4 {
                for channel in 0..3 {
                    let mut actual = 0.;
                    let mut expected = 0.;
                    for y in ry * 32..(ry + 1) * 32 {
                        for x in rx * 32..(rx + 1) * 32 {
                            let offset = ((y * 128 + x) * 4) as usize;
                            actual += linear(frame.rgba[offset + channel]);
                            expected += linear(reference.get_pixel(x, y)[channel]);
                        }
                    }
                    actual /= 1024.;
                    expected /= 1024.;
                    let tolerance = if APPROXIMATE.contains(&name) {
                        APPROXIMATE_TOLERANCE
                    } else {
                        TOLERANCE
                    };
                    let passes = if expected > 0.02 {
                        (actual / expected - 1.).abs() <= tolerance
                    } else {
                        (actual - expected).abs() <= 0.02
                    };
                    if !passes {
                        failures.push(format!("{name} region({rx},{ry}) channel {channel}: Lit={actual:.5}, r4={expected:.5}"));
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "Lit differs from r4 references:\n{}",
        failures.join("\n")
    );
}

/// How wide a highlight blurs only shows pixel by pixel: region means keep the
/// energy whatever the blur. A small strong sun on a metal, over the middle of the
/// sphere, where the split sum's view-along-the-reflection assumption holds best.
#[test]
#[ignore = "requires a GPU adapter"]
fn environment_highlights_blur_like_r4() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/lit_reference");
    for name in ["env_sun_mid", "env_sun_rough"] {
        let scene = parse(name);
        let frame = render_preview(
            &scene.manifest,
            &scene.blobs,
            ViewerCamera::from_manifest(&scene.manifest, 1.0),
            128,
            128,
            PreviewMode::Lit,
        )
        .unwrap();
        let reference = image::open(root.join(format!("{name}.png")))
            .unwrap()
            .to_rgb8();
        let (mut sum, mut count, mut worst) = (0., 0, 0f64);
        for y in 0..128i32 {
            for x in 0..128i32 {
                if (x - 64).pow(2) + (y - 64).pow(2) >= 30 * 30 {
                    continue;
                }
                let offset = ((y * 128 + x) * 4) as usize;
                let actual = linear(frame.rgba[offset]);
                let expected = linear(reference.get_pixel(x as u32, y as u32)[0]);
                let error = (actual - expected).abs();
                sum += error;
                count += 1;
                worst = worst.max(error);
            }
        }
        let mean = sum / count as f64;
        // Set from measurement (0.0016): 0.0037 is what a wrong roughness mapping gives.
        assert!(
            mean < 0.003,
            "{name}: mean error {mean:.4}, worst {worst:.4}"
        );
    }
}
