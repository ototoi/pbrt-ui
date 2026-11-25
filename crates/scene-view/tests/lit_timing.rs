//! Measurement harness: runs `update_scene` over an
//! existing preview package (the same layout `pbrt-ui-capture` reads) and
//! prints the per-stage durations, decode counts and lit texture byte
//! accounting. The lit texture stage is deferred until the first Lit
//! render, so each run renders once lit to trigger it and reads the stage
//! record from the render. Opt-in and inert without the environment
//! variables, so it never runs in the normal gate passes.
use scene_view::{LitDebug, PreviewMode, PreviewRenderer, ViewerCamera};
use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};

/// Empty editions: every mesh re-prepares each run; the lit stages are
/// unaffected (they key by image identity).
const EDITIONS: BTreeMap<u64, u64> = BTreeMap::new();

fn load_package(
    dir: &Path,
) -> Result<
    (
        protocol::geometry::GeometryManifest,
        BTreeMap<String, Vec<u8>>,
    ),
    String,
> {
    let manifest_path = dir
        .join("geometry.json")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let manifest: protocol::geometry::GeometryManifest =
        serde_json::from_slice(&std::fs::read(&manifest_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let directory = manifest_path.parent().ok_or("no parent")?;
    let mut blobs = BTreeMap::new();
    for name in manifest.blobs.keys() {
        let rel = Path::new(name);
        if rel
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(format!("unsafe blob path {name}"));
        }
        let path = directory
            .join(rel)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !path.starts_with(directory) {
            return Err(format!("blob escapes geometry directory: {name}"));
        }
        blobs.insert(
            name.clone(),
            std::fs::read(path).map_err(|e| e.to_string())?,
        );
    }
    Ok((manifest, blobs))
}

/// Header dimensions of an image blob without decoding its pixels.
fn header_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    image::io::Reader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

/// Today's accounting for the package's lit images: one entry per distinct
/// blob content per value use (the renderer's image identities), each
/// RGBA16Float at its native size with all mip levels — the per-image byte
/// count before any budget rejection.
fn legacy_accounting(
    manifest: &protocol::geometry::GeometryManifest,
    blobs: &BTreeMap<String, Vec<u8>>,
) -> (u64, usize) {
    let mut identities: BTreeMap<String, (&str, &Vec<u8>)> = BTreeMap::new();
    for (key, file) in &manifest.blobs {
        if !key.starts_with("texture/") {
            continue;
        }
        let use_kind = if key.starts_with("texture/float/") {
            "float"
        } else {
            "spectrum"
        };
        identities.insert(
            format!("{use_kind}:{}", file.sha256),
            (use_kind, blobs.get(key).expect("blob key loaded")),
        );
    }
    let mut legacy = 0u64;
    let mut counted = 0usize;
    for (identity, (use_kind, bytes)) in identities {
        let Some((width, height)) = header_dimensions(bytes) else {
            println!("[lit-timing] legacy {use_kind} image {identity}: header unreadable");
            continue;
        };
        let mips = width.max(height).ilog2() + 1;
        let bytes = (0..mips)
            .map(|mip| u64::from((width >> mip).max(1)) * u64::from((height >> mip).max(1)) * 8)
            .sum::<u64>();
        println!(
            "[lit-timing] legacy {use_kind} image {identity}: {width}x{height} → {bytes} bytes"
        );
        legacy += bytes;
        counted += 1;
    }
    (legacy, counted)
}

#[test]
#[ignore = "requires a GPU adapter and PBRT_UI_LIT_TIMING_PACKAGE or PBRT_UI_LIT_TIMING_SCENE"]
fn package_timing_report() {
    // PBRT_UI_LIT_TIMING_SCENE builds a scratch
    // package from a PBRT scene once (the same layout core writes), so the
    // harness runs without a pre-made slot; an explicit package path wins.
    let package_path = std::env::var("PBRT_UI_LIT_TIMING_PACKAGE")
        .ok()
        .or_else(|| {
            let scene = std::env::var("PBRT_UI_LIT_TIMING_SCENE").ok()?;
            let name = Path::new(&scene)
                .file_stem()?
                .to_str()?
                .to_ascii_lowercase();
            let out = Path::new("/tmp/opencode")
                .join("pbrt-ui-lit-timing")
                .join(name);
            std::fs::create_dir_all(&out).ok()?;
            let built = scene_parse::ParseOptions::default();
            let preview =
                scene_parse::parse_scene(Path::new(&scene), built).expect("parse the timing scene");
            scene_parse::write_preview(&preview, &out, false).expect("write the timing package");
            Some(out.display().to_string())
        });
    let Some(package) = package_path else {
        return;
    };
    let runs: usize = std::env::var("PBRT_UI_LIT_TIMING_RUNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(5);
    let root = PathBuf::from(package);
    let (manifest, blobs) = load_package(&root).unwrap();
    let (legacy, counted) = legacy_accounting(&manifest, &blobs);
    println!("[lit-timing] legacy accounting over {counted} images: {legacy} bytes total");
    let mut renderer = PreviewRenderer::new().unwrap();
    let camera = ViewerCamera::from_manifest(&manifest, 1.);
    let time = manifest.supported_times.first().copied().unwrap_or(0.);
    let mut cold: Option<std::time::Duration> = None;
    let mut lit_textures_samples = Vec::new();
    let mut decodes_before = 0u64;
    for run in 0..runs {
        let started = std::time::Instant::now();
        renderer.update_scene(&manifest, &blobs, &EDITIONS).unwrap();
        let total = started.elapsed();
        let timings = renderer.last_update_timings();
        // The update deferred the lit texture stage; the first Lit
        // render runs it, so its durations, decode counts and byte
        // accounting land with that render.
        renderer
            .render_with_options(camera, 64, 64, PreviewMode::Lit, time, LitDebug::Off)
            .unwrap();
        let stage = renderer
            .take_rendered_lit_textures()
            .expect("the first lit render of each run runs the deferred stage");
        let decodes = renderer.stats().image_decodes - decodes_before;
        decodes_before = renderer.stats().image_decodes;
        println!(
            "[lit-timing] run {run}: total={:.1?} mesh={:.1?} lit_evaluate={:.1?} lit_textures={:.1?} (decode {:.1?} + gpu {:.1?}) environment={:.1?} decodes={decodes} lit_image_bytes={}",
            total,
            timings.mesh,
            timings.lit_evaluate,
            stage.timings.lit_textures,
            stage.timings.lit_textures_decode,
            stage.timings.lit_textures_gpu,
            timings.environment,
            renderer.stats().lit_image_bytes,
        );
        lit_textures_samples.push(stage.timings.lit_textures);
        if run == 0 {
            cold = Some(stage.timings.lit_textures);
        }
    }
    lit_textures_samples.sort();
    let median = lit_textures_samples[runs / 2];
    println!(
        "[lit-timing] SUMMARY lit_textures: first-run (cold) {:?}, median {:?} over {runs} runs",
        cold.unwrap_or_default(),
        median,
    );
}
