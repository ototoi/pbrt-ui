//! Lit images stay on the GPU between updates. Same content must not
//! decode twice, a permuted slot order must render like a fresh renderer, a
//! failed decode stays demoted without re-decoding, and an unchanged
//! environment must not re-bake. The pixel comparisons are against uncached
//! renders of the same scene, never stored goldens.
use scene_view::{LitDebug, PreviewMode, PreviewRenderer, ViewerCamera};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tempfile::TempDir;

// GPU device creation is not safe across parallel test threads.
static GPU: Mutex<()> = Mutex::new(());

/// A PNG of three solid regions, so a sample from the wrong image layer or
/// uv half shows the wrong colour instead of near-matching noise.
fn quadrant_png(top_left: [u8; 3], top_right: [u8; 3], bottom: [u8; 3]) -> Vec<u8> {
    let mut image = image::RgbaImage::new(16, 16);
    for y in 0..16 {
        for x in 0..16 {
            let color = if y < 8 {
                if x < 8 { top_left } else { top_right }
            } else {
                bottom
            };
            image.put_pixel(x, y, image::Rgba([color[0], color[1], color[2], 255]));
        }
    }
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
}

/// Two textured quads side by side, each material with its own imagemap.
/// `right_file` is the right material's image, so two scenes can differ in
/// exactly one image.
fn two_quad_scene(right_file: &str) -> String {
    [
        "LookAt 0 0 5  0 0 0  0 1 0",
        "Camera \"perspective\" \"float fov\" [40]",
        "WorldBegin",
        "Texture \"left\" \"spectrum\" \"imagemap\" \"string filename\" \"a.png\" \"string encoding\" \"linear\"",
        &format!(
            "Texture \"right\" \"spectrum\" \"imagemap\" \"string filename\" \"{right_file}\" \"string encoding\" \"linear\""
        ),
        "AttributeBegin",
        "Translate -0.9 0 0",
        "Material \"diffuse\" \"texture reflectance\" \"left\"",
        "Shape \"trianglemesh\" \"point3 P\" [-0.8 -0.8 0  0.8 -0.8 0  0.8 0.8 0  -0.8 0.8 0] \"texcoord2 uv\" [0 0 1 0 1 1 0 1] \"integer indices\" [0 1 2 0 2 3]",
        "AttributeEnd",
        "AttributeBegin",
        "Translate 0.9 0 0",
        "Material \"diffuse\" \"texture reflectance\" \"right\"",
        "Shape \"trianglemesh\" \"point3 P\" [-0.8 -0.8 0  0.8 -0.8 0  0.8 0.8 0  -0.8 0.8 0] \"texcoord2 uv\" [0 0 1 0 1 1 0 1] \"integer indices\" [0 1 2 0 2 3]",
        "AttributeEnd",
    ]
    .join("\n")
}

fn parse_scene_text(dir: &Path, scene: &str, text: &str) -> scene_parse::PreviewGeometry {
    write(&dir.join(scene), text.as_bytes());
    scene_parse::parse_scene(dir.join(scene), scene_parse::ParseOptions::default()).unwrap()
}

const EDITIONS: BTreeMap<u64, u64> = BTreeMap::new();

fn albedo(renderer: &mut PreviewRenderer, geometry: &scene_parse::PreviewGeometry) -> Vec<u8> {
    render(renderer, geometry, LitDebug::Albedo)
}

fn render(
    renderer: &mut PreviewRenderer,
    geometry: &scene_parse::PreviewGeometry,
    debug: LitDebug,
) -> Vec<u8> {
    renderer
        .update_scene(&geometry.manifest, &geometry.blobs, &EDITIONS)
        .unwrap();
    pixels_of_debug(renderer, geometry, debug)
}

/// Render the already-installed scene (no `update_scene` first), so a
/// partial-read update's own stage is what gets compared.
fn pixels_of(renderer: &mut PreviewRenderer, geometry: &scene_parse::PreviewGeometry) -> Vec<u8> {
    pixels_of_debug(renderer, geometry, LitDebug::Albedo)
}
fn pixels_of_debug(
    renderer: &mut PreviewRenderer,
    geometry: &scene_parse::PreviewGeometry,
    debug: LitDebug,
) -> Vec<u8> {
    let time = geometry
        .manifest
        .supported_times
        .first()
        .copied()
        .unwrap_or(0.);
    renderer
        .render_with_options(
            ViewerCamera::from_manifest(&geometry.manifest, 1.),
            96,
            96,
            PreviewMode::Lit,
            time,
            debug,
        )
        .unwrap()
        .rgba
}

#[test]
#[ignore = "requires a GPU adapter"]
fn same_manifest_twice_decodes_images_only_the_first_time() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new().unwrap();
    let first = material_pair(dir.path()).0;
    for prefer_bindings in [false, true] {
        let mut renderer = PreviewRenderer::new_with_texture_arrays(prefer_bindings).unwrap();
        renderer
            .update_scene(&first.manifest, &first.blobs, &EDITIONS)
            .unwrap();
        // The stage defers, so the second update replaces the
        // pending work — the first lit render decodes each image once.
        renderer
            .update_scene(&first.manifest, &first.blobs, &EDITIONS)
            .unwrap();
        let _ = pixels_of_debug(&mut renderer, &first, LitDebug::Albedo);
        let decodes = renderer.stats().image_decodes;
        assert_eq!(decodes, 2, "both images decode on the first lit render");
        let _ = pixels_of_debug(&mut renderer, &first, LitDebug::Albedo);
        assert_eq!(
            renderer.stats().image_decodes,
            decodes,
            "a second lit render decodes nothing"
        );
        // A third update over committed textures keeps it that way.
        renderer
            .update_scene(&first.manifest, &first.blobs, &EDITIONS)
            .unwrap();
        let _ = pixels_of_debug(&mut renderer, &first, LitDebug::Albedo);
        assert_eq!(
            renderer.stats().image_decodes,
            decodes,
            "unchanged content never decodes twice"
        );
    }
}

/// Two scenes over one image set: `a.png` is byte-identical in both, the right
/// material reads a different image in `second`.
fn material_pair(dir: &Path) -> (scene_parse::PreviewGeometry, scene_parse::PreviewGeometry) {
    let root: PathBuf = dir.to_path_buf();
    write(
        &root.join("a.png"),
        &quadrant_png([10, 10, 200], [230, 30, 30], [30, 200, 30]),
    );
    write(
        &root.join("b.png"),
        &quadrant_png([200, 200, 10], [30, 230, 30], [200, 30, 30]),
    );
    let first = parse_scene_text(&root, "first.pbrt", &two_quad_scene("b.png"));
    write(
        &root.join("c.png"),
        &quadrant_png([255, 0, 255], [0, 120, 255], [255, 120, 0]),
    );
    let second = parse_scene_text(&root, "second.pbrt", &two_quad_scene("c.png"));
    (first, second)
}

#[test]
#[ignore = "requires a GPU adapter"]
fn changing_one_image_decodes_exactly_one() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new().unwrap();
    let (first, second) = material_pair(dir.path());
    for prefer_bindings in [false, true] {
        let mut renderer = PreviewRenderer::new_with_texture_arrays(prefer_bindings).unwrap();
        renderer
            .update_scene(&first.manifest, &first.blobs, &EDITIONS)
            .unwrap();
        // Stage the first package with its first lit render, then
        // swap in one changed image.
        let _ = pixels_of_debug(&mut renderer, &first, LitDebug::Albedo);
        let decodes = renderer.stats().image_decodes;
        renderer
            .update_scene(&second.manifest, &second.blobs, &EDITIONS)
            .unwrap();
        let _ = pixels_of_debug(&mut renderer, &second, LitDebug::Albedo);
        assert_eq!(
            renderer.stats().image_decodes,
            decodes + 1,
            "exactly the changed image decodes at the next lit render"
        );
    }
}

/// Swap the two materials (and their draws' ids) so the texture slot order
/// flips while the rendered image cannot change.
fn permute_materials(manifest: &mut protocol::geometry::GeometryManifest) {
    assert_eq!(manifest.materials.len(), 2, "the fixture has two materials");
    manifest.materials.swap(0, 1);
    manifest.textures.reverse();
    for draw in &mut manifest.draws {
        match draw.material_id {
            Some(0) => draw.material_id = Some(1),
            Some(1) => draw.material_id = Some(0),
            _ => panic!("only two materials: {:?}", draw.material_id),
        }
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn permuted_slot_order_renders_like_a_fresh_renderer() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new().unwrap();
    let first = material_pair(dir.path()).0;
    for prefer_bindings in [false, true] {
        let mut permuted = first.clone();
        permute_materials(&mut permuted.manifest);
        let mut reused = PreviewRenderer::new_with_texture_arrays(prefer_bindings).unwrap();
        let _plain = albedo(&mut reused, &first);
        let permuted_pixels = albedo(&mut reused, &permuted);
        let mut fresh = PreviewRenderer::new_with_texture_arrays(prefer_bindings).unwrap();
        let fresh_pixels = albedo(&mut fresh, &permuted);
        assert_eq!(
            permuted_pixels, fresh_pixels,
            "the reused array must shade from the same image bytes"
        );
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_failed_decode_stays_demoted_without_redecoding() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    // The left image is present but corrupt; the right one is a valid PNG, so
    // exactly one decode is attempted and it fails.
    write(&root.join("a.png"), b"not a png at all");
    write(
        &root.join("b.png"),
        &quadrant_png([200, 200, 10], [30, 230, 30], [200, 30, 30]),
    );
    let bad = parse_scene_text(&root, "bad.pbrt", &two_quad_scene("b.png"));
    for prefer_bindings in [false, true] {
        let mut renderer = PreviewRenderer::new_with_texture_arrays(prefer_bindings).unwrap();
        let first_pixels = albedo(&mut renderer, &bad);
        // Two images, each attempted once: the corrupt one fails, the valid
        // one uploads.
        let decodes = renderer.stats().image_decodes;
        assert_eq!(decodes, 2, "each image decodes exactly once");
        assert!(
            renderer
                .lit_notes()
                .iter()
                .any(|n| n.reason.contains("replaced by 0.5")),
            "{:?}",
            renderer.lit_notes()
        );
        let second_pixels = albedo(&mut renderer, &bad);
        assert_eq!(
            renderer.stats().image_decodes,
            decodes,
            "the failed image re-decodes nothing on the retry"
        );
        assert_eq!(
            first_pixels, second_pixels,
            "the demoted colour must not change between updates"
        );
    }
}

fn environment_scene(map: &str) -> String {
    [
        "LookAt 0 0 3.5  0 0 0  0 1 0",
        "Camera \"perspective\" \"float fov\" [45]",
        "WorldBegin",
        &format!("LightSource \"infinite\" \"string mapname\" \"{map}\" \"float scale\" [1]"),
        "Material \"diffuse\" \"rgb reflectance\" [0.1 0.5 0.9]",
        "Shape \"sphere\" \"float radius\" 0.9",
    ]
    .join("\n")
}

fn environment_maps(dir: &Path) {
    let root = dir.to_path_buf();
    write(
        &root.join("env-a.png"),
        &quadrant_png([255, 128, 64], [255, 128, 64], [16, 16, 64]),
    );
    write(
        &root.join("env-b.png"),
        &quadrant_png([64, 255, 128], [64, 255, 128], [32, 8, 16]),
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn an_unchanged_environment_image_does_not_rebake_or_redecode() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new().unwrap();
    environment_maps(dir.path());
    let first = parse_scene_text(dir.path(), "env1.pbrt", &environment_scene("env-a.png"));
    let second = parse_scene_text(dir.path(), "env2.pbrt", &environment_scene("env-b.png"));
    let mut renderer = PreviewRenderer::new().unwrap();
    renderer
        .update_scene(&first.manifest, &first.blobs, &EDITIONS)
        .unwrap();
    let (decodes, bakes) = (
        renderer.stats().image_decodes,
        renderer.stats().environment_bakes,
    );
    assert_eq!(decodes, 1, "the environment map decodes once");
    assert_eq!(bakes, 1, "and bakes once");
    renderer
        .update_scene(&first.manifest, &first.blobs, &EDITIONS)
        .unwrap();
    assert_eq!(renderer.stats().image_decodes, decodes);
    assert_eq!(renderer.stats().environment_bakes, bakes);
    let reused = render(&mut renderer, &first, LitDebug::Off);
    let fresh = render(&mut PreviewRenderer::new().unwrap(), &first, LitDebug::Off);
    assert!(
        reused == fresh,
        "a reused environment renders like a fresh bake"
    );
    renderer
        .update_scene(&second.manifest, &second.blobs, &EDITIONS)
        .unwrap();
    assert_eq!(renderer.stats().image_decodes, decodes + 1);
    assert_eq!(renderer.stats().environment_bakes, bakes + 1);
}

/// The install reads only the blobs the renderer does not
/// already hold. The partial read an unchanged-content install performs
/// is an EMPTY image set; an edit is fed exactly the one changed image.
/// Both must render byte-identically to the same updates fed whole
/// packages.
#[test]
#[ignore = "requires a GPU adapter"]
fn partial_reads_render_like_full_packages() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new().unwrap();
    let (first, second) = material_pair(dir.path());
    for prefer_bindings in [false, true] {
        // The desktop's own read decision, from the renderer's residency
        // answers: unchanged content reads nothing at all. (Geometry is
        // the editions' answer, never resident_blob's.)
        let mut partial = PreviewRenderer::new_with_texture_arrays(prefer_bindings).unwrap();
        partial
            .update_scene(&first.manifest, &first.blobs, &EDITIONS)
            .unwrap();
        let unchanged_read: std::collections::BTreeSet<String> = first
            .manifest
            .blobs
            .keys()
            .filter(|name| name.starts_with("texture/") || name.starts_with("light/"))
            .filter(|name| !partial.resident_blob(name, &first.manifest))
            .cloned()
            .collect();
        assert!(
            unchanged_read.is_empty(),
            "an unchanged update needs no image bytes, got {unchanged_read:?}"
        );
        let empty = BTreeMap::new();
        assert!(
            partial
                .update_scene(&first.manifest, &empty, &EDITIONS)
                .is_ok(),
            "meshes are cache hits, so an empty map installs cleanly"
        );

        // One image changed: the read set is exactly its blob.
        let changed_read: std::collections::BTreeSet<String> = second
            .manifest
            .blobs
            .keys()
            .filter(|name| name.starts_with("texture/") || name.starts_with("light/"))
            .filter(|name| !partial.resident_blob(name, &second.manifest))
            .cloned()
            .collect();
        assert_eq!(changed_read.len(), 1, "only the changed image is read");
        let changed_blobs: BTreeMap<String, Vec<u8>> = second
            .blobs
            .iter()
            .filter(|(name, _)| changed_read.contains(*name))
            .map(|(name, bytes)| (name.clone(), bytes.clone()))
            .collect();
        partial
            .update_scene(&second.manifest, &changed_blobs, &EDITIONS)
            .unwrap();
        let partial_pixels = pixels_of(&mut partial, &second);
        let full_pixels = {
            let mut full = PreviewRenderer::new_with_texture_arrays(prefer_bindings).unwrap();
            full.update_scene(&first.manifest, &first.blobs, &EDITIONS)
                .unwrap();
            full.update_scene(&second.manifest, &second.blobs, &EDITIONS)
                .unwrap();
            pixels_of(&mut full, &second)
        };
        assert_eq!(
            partial_pixels, full_pixels,
            "the partial read renders like the full package"
        );
    }
}

/// Unchanged environment inputs reuse the committed environment
/// whole, so their map blob stays unread and the empty read still renders
/// like a full reload.
#[test]
#[ignore = "requires a GPU adapter"]
fn unchanged_environment_inputs_skip_the_maps_bytes_entirely() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new().unwrap();
    environment_maps(dir.path());
    let scene = parse_scene_text(dir.path(), "env.pbrt", &environment_scene("env-a.png"));
    let mut renderer = PreviewRenderer::new().unwrap();
    renderer
        .update_scene(&scene.manifest, &scene.blobs, &EDITIONS)
        .unwrap();
    let read: Vec<String> = scene
        .manifest
        .blobs
        .keys()
        .filter(|name| name.starts_with("texture/") || name.starts_with("light/"))
        .filter(|name| !renderer.resident_blob(name, &scene.manifest))
        .cloned()
        .collect();
    assert!(read.is_empty(), "the environment map is resident: {read:?}");
    let partial = BTreeMap::new();
    let mut reused = renderer;
    let partial_render = {
        reused
            .update_scene(&scene.manifest, &partial, &EDITIONS)
            .unwrap();
        pixels_of_debug(&mut reused, &scene, LitDebug::Off)
    };
    let fresh = render(&mut PreviewRenderer::new().unwrap(), &scene, LitDebug::Off);
    assert_eq!(
        partial_render, fresh,
        "the byte-less update renders like a fresh full package"
    );
}
