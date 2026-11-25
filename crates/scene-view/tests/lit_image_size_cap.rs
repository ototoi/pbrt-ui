//! The binding path reads image headers before decoding
//! — storage caps at 2048 on the longest edge, the byte budget grew to
//! 256 MiB, and an image the header rejects is never decoded.
use scene_view::{LitDebug, PreviewMode, PreviewRenderer, ViewerCamera};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use tempfile::TempDir;

// GPU device creation is not safe across parallel test threads.
static GPU: Mutex<()> = Mutex::new(());

const EDITIONS: BTreeMap<u64, u64> = BTreeMap::new();

/// The GPU byte cost of a texture: texels per mip level times the format's
/// texel bytes.
fn storage_bytes(width: u32, height: u32, texel_bytes: u64) -> u64 {
    (0..width.max(height).ilog2() + 1)
        .map(|mip| {
            u64::from((width >> mip).max(1)) * u64::from((height >> mip).max(1)) * texel_bytes
        })
        .sum()
}

fn parse_scene_text(dir: &Path, scene: &str, text: &str) -> scene_parse::PreviewGeometry {
    std::fs::write(dir.join(scene), text.as_bytes()).unwrap();
    scene_parse::parse_scene(dir.join(scene), scene_parse::ParseOptions::default()).unwrap()
}

fn pixel(frame: &scene_view::PreviewFrame, x: u32, y: u32) -> [u8; 3] {
    let offset = ((y * 128 + x) * 4) as usize;
    frame.rgba[offset..offset + 3].try_into().unwrap()
}

fn camera() -> ViewerCamera {
    ViewerCamera {
        eye: [0., 0., 5.],
        target: [0.; 3],
        up: [0., 1., 0.],
        fov_y_degrees: 45.,
        near: 0.1,
        far: 100.,
        mirror: false,
    }
}

/// Render the installed scene's albedo (the deferred lit texture
/// stage runs at this render), leaving the stats and notes to the caller.
fn lit_albedo(renderer: &mut PreviewRenderer, geometry: &scene_parse::PreviewGeometry) {
    let time = geometry
        .manifest
        .supported_times
        .first()
        .copied()
        .unwrap_or(0.);
    renderer
        .render_with_options(camera(), 128, 128, PreviewMode::Lit, time, LitDebug::Albedo)
        .unwrap();
}

#[test]
#[ignore = "requires a GPU adapter"]
fn oversized_images_are_stored_downscaled_to_the_cap() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new().unwrap();
    // Four solid quadrants: the 4096×1028 header stores as 2048×512
    // RGBA8UnormSrgb.
    let image = image::RgbaImage::from_fn(4096, 1024, |x, y| {
        let color = if x < 2048 {
            if y < 512 {
                [200, 40, 40]
            } else {
                [40, 200, 40]
            }
        } else if y < 512 {
            [40, 40, 200]
        } else {
            [200, 200, 40]
        };
        image::Rgba([color[0], color[1], color[2], 255])
    });
    image.save(dir.path().join("boards.png")).unwrap();
    let scene = [
        "LookAt 0 0 5  0 0 0  0 1 0",
        "Camera \"perspective\" \"float fov\" [45]",
        "WorldBegin",
        "Texture \"boards\" \"spectrum\" \"imagemap\" \"string filename\" \"boards.png\"",
        "Material \"diffuse\" \"texture reflectance\" \"boards\"",
        "Shape \"trianglemesh\" \"point3 P\" [-1 -1 0 1 -1 0 1 1 0 -1 1 0] \"texcoord2 uv\" [0 0 1 0 1 1 0 1] \"integer indices\" [0 1 2 0 2 3]",
    ]
    .join("\n");
    let geometry = parse_scene_text(dir.path(), "scene.pbrt", &scene);
    let mut renderer = PreviewRenderer::new_with_texture_arrays(true).unwrap();
    renderer
        .update_scene(&geometry.manifest, &geometry.blobs, &EDITIONS)
        .unwrap();
    let frame = renderer
        .render_with_options(
            camera(),
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
    // Level sizes 2048×512 of RGBA8UnormSrgb; the bytes named the stored
    // shape rather than the 4096×1024 source. The stage deferred
    // to the lit render above, so its byte count lands with it.
    assert_eq!(
        renderer.stats().lit_image_bytes,
        storage_bytes(2048, 512, 4),
        "a 4096×1024 image stores 2048×512 RGBA8UnormSrgb"
    );
    // One sample per quadrant interior survives the cap. The viewport's
    // camera convention mirrors u horizontally (the same convention the
    // strip-image test asserts), so screen-left shows the image's right
    // half.
    for (x, y, expected) in [
        (48u32, 48u32, [40u8, 40, 200]),
        (80, 48, [200, 40, 40]),
        (48, 80, [200, 200, 40]),
        (80, 80, [40, 200, 40]),
    ] {
        assert!(
            pixel(&frame, x, y)
                .iter()
                .zip(expected)
                .all(|(actual, expected)| (i16::from(*actual) - i16::from(expected)).abs() <= 4),
            "quadrant at ({x},{y}): {:?} != {expected:?}",
            pixel(&frame, x, y)
        );
    }
}

/// `count` distinct 16-bit 2048² images, one quad each: every stored image
/// costs its full RGBA16Float mip chain, so six fit the 256 MiB budget and
/// the seventh's header rejects it.
fn budget_overflow_scene(dir: &Path, count: u8) -> scene_parse::PreviewGeometry {
    for index in 0..count {
        let image = image::ImageBuffer::<image::Luma<u16>, Vec<u16>>::from_pixel(2048, 2048, {
            let value = 65535 - index as u16 * 4000;
            image::Luma([value])
        });
        image.save(dir.join(format!("gray{index}.png"))).unwrap();
    }
    let mut source = String::from(
        "LookAt 0 0 5  0 0 0  0 1 0\nCamera \"perspective\" \"float fov\" [45]\nWorldBegin\n",
    );
    for index in 0..count {
        let x = -0.9 + index as f64 * 1.8 / count as f64;
        source.push_str(&format!(
            "Texture \"g{index}\" \"spectrum\" \"imagemap\" \"string filename\" \"gray{index}.png\"\n\
             AttributeBegin\nTranslate {x} 0 0\nMaterial \"diffuse\" \"texture reflectance\" \"g{index}\"\n\
             Shape \"trianglemesh\" \"point3 P\" [-0.5 -0.5 0 0.5 -0.5 0 0.5 0.5 0 -0.5 0.5 0] \"texcoord2 uv\" [0 0 1 0 1 1 0 1] \"integer indices\" [0 1 2 0 2 3]\nAttributeEnd\n"
        ));
    }
    let path = dir.join("scene.pbrt");
    std::fs::write(&path, source).unwrap();
    scene_parse::parse_scene(path, scene_parse::ParseOptions::default()).unwrap()
}

#[test]
#[ignore = "requires a GPU adapter"]
fn an_image_its_header_rejects_is_never_decoded() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new().unwrap();
    let geometry = budget_overflow_scene(dir.path(), 7);
    let mut renderer = PreviewRenderer::new_with_texture_arrays(true).unwrap();
    renderer
        .update_scene(&geometry.manifest, &geometry.blobs, &EDITIONS)
        .unwrap();
    // The stage defers; render lit once to run it. Six decode and
    // upload; the seventh's header rejects it against the byte budget
    // before any pixel decode.
    lit_albedo(&mut renderer, &geometry);
    assert_eq!(
        renderer.stats().image_decodes,
        6,
        "the rejected image never decodes"
    );
    assert_eq!(
        renderer.stats().lit_image_bytes,
        storage_bytes(2048, 2048, 8) * 6,
        "the six accepted images' whole RGBA16Float mip chains"
    );
    assert!(
        renderer
            .lit_notes()
            .iter()
            .any(|note| note.reason.contains("image byte budget exceeded")),
        "{:?}",
        renderer.lit_notes()
    );
    // The reject memo demotes again after a repeat without another decode.
    renderer
        .update_scene(&geometry.manifest, &geometry.blobs, &EDITIONS)
        .unwrap();
    lit_albedo(&mut renderer, &geometry);
    assert_eq!(
        renderer.stats().image_decodes,
        6,
        "the memoised reject stays undecoded"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_source_over_twice_the_cap_keeps_its_average() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new().unwrap();
    // Columns white, black, black, white: a single bilinear tap per output
    // texel at a 4× reduction lands between the two black columns, so the
    // stored image must be prefiltered to keep the 0.5 linear average.
    image::RgbaImage::from_fn(8192, 64, |x, _| {
        image::Rgba(if matches!(x % 4, 0 | 3) {
            [255; 4]
        } else {
            [0, 0, 0, 255]
        })
    })
    .save(dir.path().join("stripes.png"))
    .unwrap();
    let scene = [
        "LookAt 0 0 5  0 0 0  0 1 0",
        "Camera \"perspective\" \"float fov\" [45]",
        "WorldBegin",
        "Texture \"stripes\" \"spectrum\" \"imagemap\" \"string filename\" \"stripes.png\"",
        "Material \"diffuse\" \"texture reflectance\" \"stripes\"",
        "Shape \"trianglemesh\" \"point3 P\" [-1 -1 0 1 -1 0 1 1 0 -1 1 0] \"texcoord2 uv\" [0 0 1 0 1 1 0 1] \"integer indices\" [0 1 2 0 2 3]",
    ]
    .join("\n");
    let geometry = parse_scene_text(dir.path(), "scene.pbrt", &scene);
    let mut renderer = PreviewRenderer::new_with_texture_arrays(true).unwrap();
    renderer
        .update_scene(&geometry.manifest, &geometry.blobs, &EDITIONS)
        .unwrap();
    let frame = renderer
        .render_with_options(camera(), 128, 128, PreviewMode::Lit, 0., LitDebug::Albedo)
        .unwrap();
    // Linear 0.5 written as sRGB.
    let expected = 188u8;
    let actual = pixel(&frame, 64, 64);
    assert!(
        actual.iter().all(|channel| channel.abs_diff(expected) <= 6),
        "{actual:?} != {expected} (the stripes' linear average)"
    );
}
