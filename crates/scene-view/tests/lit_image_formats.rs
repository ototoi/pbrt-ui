//! The binding-array path stores lit images in the
//! format their bits allow. For every image kind the preview takes — 8-bit
//! sRGB and linear, gamma-declared, 16-bit, HDR, and float uses through a
//! mix amount and an alpha mask — the stored format must shade exactly like
//! today's fixed `Rgba16Float` array path, and the byte budget must count
//! the stored format's real texel bytes.
//!
//! Tolerance: the albedo readback of the two paths is compared pixel by
//! pixel over the quad's projected interior (x and y in 35..93 of the
//! 128-px frame, the bounds the reference fixtures use), allowing 3/255
//! per channel — u8 storage quantizes texels by up to half a byte and the
//! 8-bit mip chain box-averages where the f32 chain triangles, everything
//! else is the same arithmetic.
use scene_view::{LitDebug, PreviewMode, PreviewRenderer, ViewerCamera};
use std::path::Path;
use std::sync::Mutex;
use tempfile::TempDir;

// GPU device creation is not safe across parallel test threads.
static GPU: Mutex<()> = Mutex::new(());

const EDITIONS: std::collections::BTreeMap<u64, u64> = std::collections::BTreeMap::new();

/// One image case: the file the scene reads, the texture and material
/// declarations that shade the albedo with it, and the byte count its
/// stored format must account for (16×16 stores 341 texels over the mip
/// chain: 16² + 8² + 4² + 2² + 1).
struct Case {
    name: &'static str,
    expected_bytes: u64,
    /// Writes the case's image file into `dir`.
    write: fn(&Path),
    /// The `Texture` and `Material` lines.
    material: String,
}

/// 16×16 ramp, every channel moving against x, alpha opaque. The steps
/// stay small (≤17 per texel): a stored-format comparison needs smooth
/// content, since the two paths resample through different mip ladders.
fn ramp_rgba8() -> image::RgbaImage {
    image::RgbaImage::from_fn(16, 16, |x, _| {
        image::Rgba([
            (x * 17) as u8,
            (38 + x * 13) as u8,
            (255 - x * 14) as u8,
            255,
        ])
    })
}

fn write_png(dir: &Path, name: &str, image: image::RgbaImage) {
    image.save(dir.join(name)).unwrap();
}

fn write_dat_png(dir: &Path, image: image::RgbaImage) {
    // A non-png extension keeps the byte-compatible file out of the
    // encoding default: the declaration names its encoding instead.
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    std::fs::write(dir.join("map.dat"), bytes.into_inner()).unwrap();
}

fn spectrum_material(filename: &str, parameters: &str) -> String {
    format!(
        "Texture \"image\" \"spectrum\" \"imagemap\" \"string filename\" \"{filename}\"{parameters}\n\
         Material \"diffuse\" \"texture reflectance\" \"image\"\n"
    )
}

/// The float imagemap drives a spectrum mix's amount, so the albedo shows
/// the scalar the shader widened from R.
fn float_material(filename: &str, parameters: &str) -> String {
    format!(
        "Texture \"ramp\" \"float\" \"imagemap\" \"string filename\" \"{filename}\"{parameters}\n\
         Texture \"mixed\" \"spectrum\" \"mix\" \"rgb tex1\" [1 1 1] \"rgb tex2\" [0 0 0] \"texture amount\" \"ramp\"\n\
         Material \"diffuse\" \"texture reflectance\" \"mixed\"\n"
    )
}

fn cases() -> Vec<Case> {
    const TEXELS: u64 = 16 * 16 + 8 * 8 + 4 * 4 + 2 * 2 + 1;
    vec![
        // 8-bit png without an encoding declaration: sRGB, the GPU decodes.
        Case {
            name: "eight-bit srgb png → Rgba8UnormSrgb",
            expected_bytes: TEXELS * 4,
            write: |dir| write_png(dir, "map.png", ramp_rgba8()),
            material: spectrum_material("map.png", ""),
        },
        // The same bytes declared linear: plain Unorm, identical sampling.
        Case {
            name: "eight-bit linear png → Rgba8Unorm",
            expected_bytes: TEXELS * 4,
            write: |dir| write_dat_png(dir, ramp_rgba8()),
            material: spectrum_material("map.dat", " \"string encoding\" \"linear\""),
        },
        // A declared gamma curve keeps today's f16 pipeline.
        Case {
            name: "gamma 2.2 png → Rgba16Float",
            expected_bytes: TEXELS * 8,
            write: |dir| write_png(dir, "map.png", ramp_rgba8()),
            material: spectrum_material("map.png", " \"float gamma\" [2.2]"),
        },
        // 16-bit samples are float-stored, as today.
        Case {
            name: "sixteen-bit png → Rgba16Float",
            expected_bytes: TEXELS * 8,
            write: |dir| {
                let image =
                    image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_fn(16, 16, |x, _| {
                        image::Rgba([
                            ((x * 17) as u16) * 257,
                            ((38 + x * 13) as u16) * 257,
                            ((255 - x * 14) as u16) * 257,
                            65535,
                        ])
                    });
                image.save(dir.join("map.png")).unwrap();
            },
            material: spectrum_material("map.png", ""),
        },
        // Radiance HDR: float light, linear, f16 as today.
        Case {
            name: "radiance hdr → Rgba16Float",
            expected_bytes: TEXELS * 8,
            write: |dir| {
                let mut bytes = Vec::new();
                let encoder = image::codecs::hdr::HdrEncoder::new(&mut bytes);
                let pixels: Vec<image::Rgb<f32>> = (0..256)
                    .map(|index| {
                        let x = (index % 16) as f32;
                        image::Rgb([0.05 + x / 45., 0.2 + x / 60., 0.9 - x / 45.])
                    })
                    .collect();
                encoder.encode(&pixels, 16, 16).unwrap();
                std::fs::write(dir.join("map.hdr"), bytes).unwrap();
            },
            material: spectrum_material("map.hdr", ""),
        },
        // A float use of an 8-bit linear image narrows to one channel.
        Case {
            name: "float use of an 8-bit linear image → R8Unorm",
            expected_bytes: TEXELS,
            write: |dir| write_dat_png(dir, ramp_rgba8()),
            material: float_material("map.dat", " \"string encoding\" \"linear\""),
        },
        // A float use of an sRGB png keeps the linear scalar in R16Float.
        Case {
            name: "float use of an 8-bit srgb png → R16Float",
            expected_bytes: TEXELS * 2,
            write: |dir| write_png(dir, "map.png", ramp_rgba8()),
            material: float_material("map.png", ""),
        },
        // An alpha channel rides along untouched (straight alpha, no
        // sRGB decode): the RGB comparison carries the layout.
        Case {
            name: "alpha png → Rgba8UnormSrgb",
            expected_bytes: TEXELS * 4,
            write: |dir| write_png(dir, "map.png", ramp_rgba8()),
            material: spectrum_material("map.png", ""),
        },
        // A float use whose scalar is a nonopaque alpha mask narrows to
        // the alpha channel in R8.
        Case {
            name: "float use of an alpha mask → R8Unorm (alpha)",
            expected_bytes: TEXELS,
            write: |dir| {
                write_png(
                    dir,
                    "mask.dat.png",
                    image::RgbaImage::from_fn(16, 16, |x, _| {
                        image::Rgba([
                            (x * 17) as u8,
                            (38 + x * 13) as u8,
                            (255 - x * 14) as u8,
                            (128 + x * 8) as u8,
                        ])
                    }),
                )
            },
            material: float_material("mask.dat.png", " \"string encoding\" \"linear\""),
        },
    ]
}

fn parse_scene_text(dir: &Path, scene: &str, text: &str) -> scene_parse::PreviewGeometry {
    std::fs::write(dir.join(scene), text.as_bytes()).unwrap();
    scene_parse::parse_scene(dir.join(scene), scene_parse::ParseOptions::default()).unwrap()
}

/// The sRGB pixel value the Lit output writes for a linear colour.
fn output_srgb(linear: f64) -> u8 {
    let encoded = if linear <= 0.0031308 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (encoded.clamp(0., 1.) * 255.).round() as u8
}

/// Render one textured quad through the binding-array path and return the
/// pixels of its 3×3 centre block. The quad projects to ~8 pixels on a
/// 128-pixel frame, so the sampler's gradients land a whole mip level deep
/// in the 16×16 checkerboard — level 1 at the least — where a correct
/// chain holds the box average of every 2×2 checker cell, which is uniform.
fn render_minified_checker(material: &str, write: fn(&Path)) -> [[u8; 3]; 9] {
    let dir = TempDir::new().unwrap();
    write(dir.path());
    let scene = [
        "LookAt 0 0 5  0 0 0  0 1 0",
        "Camera \"perspective\" \"float fov\" [45]",
        "WorldBegin",
        material,
        "Shape \"trianglemesh\" \"point3 P\" [-0.13 -0.13 0  0.13 -0.13 0  0.13 0.13 0  -0.13 0.13 0] \"texcoord2 uv\" [0 0 1 0 1 1 0 1] \"integer indices\" [0 1 2 0 2 3]",
    ]
    .join("\n");
    let geometry = parse_scene_text(dir.path(), "scene.pbrt", &scene);
    let mut renderer = PreviewRenderer::new_with_texture_arrays(true).unwrap();
    renderer
        .update_scene(&geometry.manifest, &geometry.blobs, &EDITIONS)
        .unwrap();
    let frame = renderer
        .render_with_options(
            ViewerCamera {
                eye: [0., 0., 5.],
                target: [0.; 3],
                up: [0., 1., 0.],
                fov_y_degrees: 45.,
                near: 0.1,
                far: 100.,
                mirror: false,
            },
            128,
            128,
            PreviewMode::Lit,
            geometry
                .manifest
                .supported_times
                .first()
                .copied()
                .unwrap_or(0.),
            LitDebug::Albedo,
        )
        .unwrap();
    let mut centre = [[0u8; 3]; 9];
    for (index, (x, y)) in (63..=65_u32)
        .flat_map(|x| (63..=65_u32).map(move |y| (x, y)))
        .enumerate()
    {
        let offset = ((y * 128 + x) * 4) as usize;
        centre[index] = [
            frame.rgba[offset],
            frame.rgba[offset + 1],
            frame.rgba[offset + 2],
        ];
    }
    centre
}

/// The Albedo-debug pixel a square textured quad of half-size `half` shows
/// `rows_above` pixels above the frame centre.
fn quad_pixel(dir: &Path, half: f32, rows_above: u32) -> [u8; 3] {
    let scene = [
        "LookAt 0 0 5  0 0 0  0 1 0".to_string(),
        "Camera \"perspective\" \"float fov\" [45]".into(),
        "WorldBegin".into(),
        spectrum_material("halves.png", ""),
        format!(
            "Shape \"trianglemesh\" \"point3 P\" [-{half} -{half} 0  {half} -{half} 0  {half} {half} 0  -{half} {half} 0] \"texcoord2 uv\" [0 0 1 0 1 1 0 1] \"integer indices\" [0 1 2 0 2 3]"
        ),
    ]
    .join("\n");
    let geometry = parse_scene_text(dir, "scene.pbrt", &scene);
    let mut renderer = PreviewRenderer::new_with_texture_arrays(true).unwrap();
    renderer
        .update_scene(&geometry.manifest, &geometry.blobs, &EDITIONS)
        .unwrap();
    let frame = renderer
        .render_with_options(
            ViewerCamera {
                eye: [0., 0., 5.],
                target: [0.; 3],
                up: [0., 1., 0.],
                fov_y_degrees: 45.,
                near: 0.1,
                far: 100.,
                mirror: false,
            },
            128,
            128,
            PreviewMode::Lit,
            0.,
            LitDebug::Albedo,
        )
        .unwrap();
    let offset = (((64 - rows_above) * 128 + 64) * 4) as usize;
    [
        frame.rgba[offset],
        frame.rgba[offset + 1],
        frame.rgba[offset + 2],
    ]
}

#[test]
#[ignore = "requires a GPU adapter"]
fn gpu_mips_keep_the_image_orientation() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    // Top half red, bottom half blue: a pass that flips v mirrors every
    // other mip level, so a deep level shows the opposite half.
    let dir = TempDir::new().unwrap();
    write_png(
        dir.path(),
        "halves.png",
        image::RgbaImage::from_fn(64, 64, |_, y| {
            image::Rgba(if y < 32 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            })
        }),
    );
    let red_on_top = |pixel: [u8; 3]| pixel[0] > pixel[2];
    // ~93 pixels for 64 texels: level 0.
    let level_0 = quad_pixel(dir.path(), 1.5, 30);
    // ~8 pixels: level 3 (8×8), three mip passes deep.
    let deep = quad_pixel(dir.path(), 0.13, 2);
    assert_eq!(
        red_on_top(deep),
        red_on_top(level_0),
        "deep mip {deep:?} vs level 0 {level_0:?}"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn minified_checkers_sample_the_gpu_mips_box_average() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    // Rgba8UnormSrgb: a black/white checker's deep mips average in linear
    // light and re-encode — sRGB ~188, not the byte-average 128 and not
    // the level-0 speckle.
    let srgb_checker = |dir: &Path| {
        image::RgbaImage::from_fn(16, 16, |x, y| {
            image::Rgba(if (x + y) % 2 == 0 { [255; 4] } else { [0; 4] })
        })
        .save(dir.join("checker.png"))
        .unwrap();
    };
    let pixels = render_minified_checker(&spectrum_material("checker.png", ""), srgb_checker);
    let expected = output_srgb(0.5);
    for pixels in pixels {
        for channel in pixels {
            assert!(
                channel.abs_diff(expected) <= 3,
                "srgb checker mip: {pixels:?} != {expected} (the linear average)"
            );
        }
    }
    // R16Float: a float use of a 16-bit checker of 0 and 0.25 (declared
    // linear, so the png's sRGB default does not curve the scalar)
    // narrows to one f16 channel; its deep mips box-average to 0.125, and
    // the mix leaves albedo 1 - 0.125. A missing or byte-domain chain
    // shows speckle or the wrong mean instead.
    let scalar_checker = |dir: &Path| {
        let checker = image::ImageBuffer::<image::Luma<u16>, Vec<u16>>::from_fn(16, 16, |x, y| {
            image::Luma([if (x + y) % 2 == 0 { 16384 } else { 0 }])
        });
        let mut bytes = std::io::Cursor::new(Vec::new());
        checker
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        std::fs::write(dir.join("checker.dat"), bytes.into_inner()).unwrap();
    };
    let pixels = render_minified_checker(
        &float_material("checker.dat", " \"string encoding\" \"linear\""),
        scalar_checker,
    );
    let expected = output_srgb(1.0 - 0.125);
    for pixels in pixels {
        for channel in pixels {
            assert!(
                channel.abs_diff(expected) <= 3,
                "r16 checker mip: {pixels:?} != {expected} (the scalar box average)"
            );
        }
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn stored_formats_sample_like_the_f16_array_path() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let mut array = PreviewRenderer::new_with_texture_arrays(false).unwrap();
    let mut stored = PreviewRenderer::new_with_texture_arrays(true).unwrap();
    for case in cases() {
        let dir = TempDir::new().unwrap();
        (case.write)(dir.path());
        let scene = [
            "LookAt 0 0 5  0 0 0  0 1 0",
            "Camera \"perspective\" \"float fov\" [45]",
            "WorldBegin",
            &case.material,
            "Shape \"trianglemesh\" \"point3 P\" [-1 -1 0 1 -1 0 1 1 0 -1 1 0] \"texcoord2 uv\" [0 0 1 0 1 1 0 1] \"integer indices\" [0 1 2 0 2 3]",
        ]
        .join("\n");
        let geometry = parse_scene_text(dir.path(), "scene.pbrt", &scene);
        let update = |renderer: &mut PreviewRenderer| {
            renderer
                .update_scene(&geometry.manifest, &geometry.blobs, &EDITIONS)
                .unwrap();
            renderer
                .render_with_options(
                    ViewerCamera {
                        eye: [0., 0., 5.],
                        target: [0.; 3],
                        up: [0., 1., 0.],
                        fov_y_degrees: 45.,
                        near: 0.1,
                        far: 100.,
                        mirror: false,
                    },
                    128,
                    128,
                    PreviewMode::Lit,
                    geometry
                        .manifest
                        .supported_times
                        .first()
                        .copied()
                        .unwrap_or(0.),
                    LitDebug::Albedo,
                )
                .unwrap()
                .rgba
        };
        let stored_pixels = update(&mut stored);
        let array_pixels = update(&mut array);
        assert_eq!(
            stored.stats().lit_image_bytes,
            case.expected_bytes,
            "{}: the stored format's texel bytes across all mips",
            case.name
        );
        let mut failures = Vec::new();
        for y in 35..93_u32 {
            for x in 35..93_u32 {
                let at = ((y * 128 + x) * 4) as usize;
                for channel in 0..3 {
                    let error = (stored_pixels[at + channel] as i16)
                        .abs_diff(array_pixels[at + channel] as i16);
                    if error > 3 {
                        failures.push(format!(
                            "({x},{y}) channel {channel}: stored={} array={}",
                            stored_pixels[at + channel],
                            array_pixels[at + channel]
                        ));
                    }
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{}: {} channels over 3/255, first: {:?}",
            case.name,
            failures.len(),
            &failures[..failures.len().min(6)]
        );
    }
}
