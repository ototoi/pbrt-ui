//! The Lit texture stage defers until a render needs it. A Solid
//! update — the desktop's default display — prepares meshes, evaluates the
//! material program and bakes the environment but decodes no images; the
//! first Lit render runs the stage exactly as an in-update upload would
//! have (one decode per fresh image, the same demote notes), further Lit
//! renders pay nothing, and a second update before any Lit render
//! replaces the pending work instead of doubling it.
use scene_view::{LitDebug, PreviewMode, PreviewRenderer, ViewerCamera};
use std::collections::BTreeMap;
use std::path::Path;
use tempfile::TempDir;

// GPU device creation is not safe across parallel test threads.
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

const EDITIONS: BTreeMap<u64, u64> = BTreeMap::new();

/// A solid-color PNG: any sample point inside its quad shows the color, so
/// the test is immune to the viewport's mirroring and uv conventions while
/// a slot mix-up or an unmapped instruction still shows the wrong color.
fn flat_png(color: [u8; 3]) -> Vec<u8> {
    let image =
        image::RgbaImage::from_pixel(16, 16, image::Rgba([color[0], color[1], color[2], 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

/// Two textured quads side by side, each material imaging its own file so
/// a deferred stage that samples the wrong layer fails visibly. The PNGs
/// are untagged sRGB, so the stored texels round-trip through the albedo
/// debug's linear→sRGB output (the convention lit_image_size_cap asserts).
fn two_quad_scene() -> String {
    [
        "LookAt 0 0 5  0 0 0  0 1 0",
        "Camera \"perspective\" \"float fov\" [40]",
        "WorldBegin",
        "Texture \"left\" \"spectrum\" \"imagemap\" \"string filename\" \"left.png\"",
        "Texture \"right\" \"spectrum\" \"imagemap\" \"string filename\" \"right.png\"",
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

/// The two quad centers on a 96px viewport (fov 40 at distance 5). The
/// viewport's camera convention mirrors horizontally, so the world +x quad
/// (right.png) appears on screen-left; the pairs below name quads by world
/// x, never by screen side, so a convention change fails loudly.
const WORLD_MINUS_X_AT: u32 = 72;
const WORLD_PLUS_X_AT: u32 = 24;
/// left.png — the image of the world −x quad's material (10, 30, 200).
const MINUS_X_IMAGE: [u8; 3] = [10, 30, 200];
/// right.png — the image of the world +x quad's material (220, 40, 30).
const PLUS_X_IMAGE: [u8; 3] = [220, 40, 30];
/// The 0.5 linear fallback as the albedo debug writes it back out (sRGB).
const FALLBACK_ALBEDO: [u8; 3] = [188, 188, 188];

fn parse(dir: &Path, text: &str) -> scene_parse::PreviewGeometry {
    let path = dir.join("scene.pbrt");
    std::fs::write(&path, text).unwrap();
    scene_parse::parse_scene(&path, scene_parse::ParseOptions::default()).unwrap()
}

fn fixture(dir: &Path) -> scene_parse::PreviewGeometry {
    std::fs::write(dir.join("left.png"), flat_png(MINUS_X_IMAGE)).unwrap();
    std::fs::write(dir.join("right.png"), flat_png(PLUS_X_IMAGE)).unwrap();
    parse(dir, &two_quad_scene())
}

fn time_of(geometry: &scene_parse::PreviewGeometry) -> f64 {
    geometry
        .manifest
        .supported_times
        .first()
        .copied()
        .unwrap_or(0.)
}

fn render(
    renderer: &mut PreviewRenderer,
    geometry: &scene_parse::PreviewGeometry,
    mode: PreviewMode,
    debug: LitDebug,
) -> Vec<u8> {
    renderer
        .render_with_options(
            ViewerCamera::from_manifest(&geometry.manifest, 1.),
            96,
            96,
            mode,
            time_of(geometry),
            debug,
        )
        .unwrap()
        .rgba
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let at = ((y * 96 + x) * 4) as usize;
    [pixels[at], pixels[at + 1], pixels[at + 2]]
}

fn assert_pixel_close(pixels: &[u8], x: u32, y: u32, expected: [u8; 3], what: &str) {
    let got = pixel(pixels, x, y);
    assert!(
        got.iter()
            .zip(expected)
            .all(|(a, e)| (i16::from(*a) - i16::from(e)).abs() <= 4),
        "{what} at ({x},{y}): {got:?} != {expected:?}"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn solid_updates_and_renders_decode_no_lit_images_until_lit_needs_them() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new().unwrap();
    let first = fixture(dir.path());
    for prefer_bindings in [false, true] {
        let mut renderer = PreviewRenderer::new_with_texture_arrays(prefer_bindings).unwrap();
        renderer
            .update_scene(&first.manifest, &first.blobs, &EDITIONS)
            .unwrap();
        render(&mut renderer, &first, PreviewMode::Solid, LitDebug::Off);
        assert_eq!(
            renderer.stats().image_decodes,
            0,
            "a solid update and a solid render decode no lit images (P-c C4)"
        );
        assert!(
            renderer.take_rendered_lit_textures().is_none(),
            "a solid render runs no lit texture stage"
        );
        // The images sample correctly through the deferred stage: each
        // quad shows its own flat image, so a wrong layer or a stale bind
        // group shows the other quad's color.
        let albedo = render(&mut renderer, &first, PreviewMode::Lit, LitDebug::Albedo);
        assert_eq!(
            renderer.stats().image_decodes,
            2,
            "the first lit render decodes each image exactly once"
        );
        let stage = renderer
            .take_rendered_lit_textures()
            .expect("the first lit render runs the deferred stage");
        assert!(
            stage.timings.lit_textures > std::time::Duration::ZERO,
            "the stage reports its own duration when it runs: {:?}",
            stage.timings
        );
        assert!(
            renderer.take_rendered_lit_textures().is_none(),
            "the report is taken once"
        );
        assert_pixel_close(
            &albedo,
            WORLD_PLUS_X_AT,
            48,
            PLUS_X_IMAGE,
            "the world +x quad's image",
        );
        assert_pixel_close(
            &albedo,
            WORLD_MINUS_X_AT,
            48,
            MINUS_X_IMAGE,
            "the world -x quad's image",
        );
        let again = render(&mut renderer, &first, PreviewMode::Lit, LitDebug::Albedo);
        assert_eq!(
            renderer.stats().image_decodes,
            2,
            "the second lit render decodes nothing"
        );
        assert!(
            renderer.take_rendered_lit_textures().is_none(),
            "the second lit render runs no stage"
        );
        assert_eq!(
            again, albedo,
            "the lit image does not change between renders"
        );
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn a_second_update_before_any_lit_render_decodes_each_image_only_once() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new().unwrap();
    let first = fixture(dir.path());
    for prefer_bindings in [false, true] {
        let mut renderer = PreviewRenderer::new_with_texture_arrays(prefer_bindings).unwrap();
        renderer
            .update_scene(&first.manifest, &first.blobs, &EDITIONS)
            .unwrap();
        // The second update replaces the pending stage (no double decode).
        renderer
            .update_scene(&first.manifest, &first.blobs, &EDITIONS)
            .unwrap();
        let albedo = render(&mut renderer, &first, PreviewMode::Lit, LitDebug::Albedo);
        assert_eq!(
            renderer.stats().image_decodes,
            2,
            "two updates before the first lit render still decode each image once"
        );
        assert_pixel_close(
            &albedo,
            WORLD_PLUS_X_AT,
            48,
            PLUS_X_IMAGE,
            "the world +x quad's image",
        );
        assert_pixel_close(
            &albedo,
            WORLD_MINUS_X_AT,
            48,
            MINUS_X_IMAGE,
            "the world -x quad's image",
        );
    }
}

/// The stage's own approximation notes fold when it runs, so the Console
/// sees them only after the first switch to Lit.
#[test]
#[ignore = "requires a GPU adapter"]
fn texture_stage_notes_appear_only_after_the_first_lit_render() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    std::fs::write(root.join("left.png"), b"not a png at all").unwrap();
    std::fs::write(root.join("right.png"), flat_png(PLUS_X_IMAGE)).unwrap();
    let broken = parse(&root, &two_quad_scene());
    for prefer_bindings in [false, true] {
        let mut renderer = PreviewRenderer::new_with_texture_arrays(prefer_bindings).unwrap();
        renderer
            .update_scene(&broken.manifest, &broken.blobs, &EDITIONS)
            .unwrap();
        render(&mut renderer, &broken, PreviewMode::Solid, LitDebug::Off);
        assert!(
            renderer
                .lit_notes()
                .iter()
                .all(|note| !note.reason.contains("replaced by 0.5")),
            "the fold note waits for the stage to run: {:?}",
            renderer.lit_notes()
        );
        let albedo = render(&mut renderer, &broken, PreviewMode::Lit, LitDebug::Albedo);
        assert!(
            renderer
                .lit_notes()
                .iter()
                .any(|note| note.reason.contains("replaced by 0.5")),
            "the first lit render folds the failed image: {:?}",
            renderer.lit_notes()
        );
        let stage = renderer
            .take_rendered_lit_textures()
            .expect("the stage ran at the lit render");
        assert!(
            stage
                .notes
                .iter()
                .any(|note| note.reason.contains("replaced by 0.5")),
            "the stage's report carries the folded note: {:?}",
            stage.notes
        );
        assert_eq!(
            renderer.stats().image_decodes,
            2,
            "each image is attempted exactly once"
        );
        // The corrupt image's quad shows the 0.5 linear fallback as the
        // albedo debug encodes it, not the intact image's color.
        assert_pixel_close(
            &albedo,
            WORLD_MINUS_X_AT,
            48,
            FALLBACK_ALBEDO,
            "the failed image's fallback",
        );
        assert_pixel_close(
            &albedo,
            WORLD_PLUS_X_AT,
            48,
            PLUS_X_IMAGE,
            "the intact quad's image",
        );
    }
}

/// A texture renamed during Solid updates keeps its image: the desktop
/// skips bytes the renderer calls resident, so the pending stage must
/// recover them by content, not by the old blob name.
#[test]
#[ignore = "requires a GPU adapter"]
fn a_texture_renamed_before_the_first_lit_render_keeps_its_image() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new().unwrap();
    let first = fixture(dir.path());
    let renamed = parse(
        dir.path(),
        &two_quad_scene().replace("\"right\"", "\"right_renamed\""),
    );
    for prefer_bindings in [false, true] {
        let mut renderer = PreviewRenderer::new_with_texture_arrays(prefer_bindings).unwrap();
        renderer
            .update_scene(&first.manifest, &first.blobs, &EDITIONS)
            .unwrap();
        // The desktop's read: only bytes the renderer does not hold.
        let unread: BTreeMap<String, Vec<u8>> = renamed
            .blobs
            .iter()
            .filter(|(key, _)| !renderer.resident_blob(key, &renamed.manifest))
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect();
        renderer
            .update_scene(&renamed.manifest, &unread, &EDITIONS)
            .unwrap();
        let albedo = render(&mut renderer, &renamed, PreviewMode::Lit, LitDebug::Albedo);
        assert_pixel_close(
            &albedo,
            WORLD_PLUS_X_AT,
            48,
            PLUS_X_IMAGE,
            "the renamed texture's image",
        );
    }
}
