use protocol::geometry::GeometryManifest;
use scene_view::{
    LitDebug, PreviewMode, PreviewRenderer, ViewerCamera, capture_server,
    render_preview_at_time_with_options,
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

fn main() {
    if let Err(error) = run() {
        eprintln!("pbrt-ui-capture: {error}");
        std::process::exit(2);
    }
}

/// Resident mode: one device/renderer serves NDJSON requests on stdin/stdout,
/// reusing the GPU scene while the geometry key is unchanged.
fn server() -> Result<(), Box<dyn std::error::Error>> {
    let mut renderer = PreviewRenderer::new()?;
    let stdin = std::io::stdin().lock();
    let stdout = std::io::stdout();
    capture_server::serve(
        stdin,
        stdout,
        Box::new(move |request, geometry| {
            if let Some(geometry) = geometry {
                let editions = mesh_editions(&geometry.manifest, &geometry.blobs)?;
                renderer.update_scene(&geometry.manifest, &geometry.blobs, &editions)?;
            }
            let camera = camera_of(request);
            let frame = renderer.render_with_options(
                camera,
                request.spec.size[0],
                request.spec.size[1],
                mode_of(request),
                request.spec.time,
                debug_of(&request.spec)?,
            )?;
            Ok((frame, renderer.lit_notes().to_vec()))
        }),
    )
    .map_err(|error| -> Box<dyn std::error::Error> { error.into() })
}

fn debug_of(spec: &protocol::jobs::CaptureSpec) -> Result<LitDebug, String> {
    match (spec.debug_albedo, spec.debug_ao) {
        (true, true) => Err("debug_albedo and debug_ao are exclusive".into()),
        (true, false) => Ok(LitDebug::Albedo),
        (false, true) => Ok(LitDebug::Ao),
        (false, false) => Ok(LitDebug::Off),
    }
}

fn mode_of(request: &capture_server::CaptureRequest) -> PreviewMode {
    match request.spec.mode {
        protocol::view::DisplayMode::Solid => PreviewMode::Solid,
        protocol::view::DisplayMode::Normal => PreviewMode::Normal,
        protocol::view::DisplayMode::Wireframe => PreviewMode::Wireframe,
        protocol::view::DisplayMode::Lit => PreviewMode::Lit,
    }
}

fn camera_of(request: &capture_server::CaptureRequest) -> ViewerCamera {
    let c = &request.spec.camera;
    ViewerCamera {
        eye: c.eye,
        target: c.target,
        up: c.up,
        fov_y_degrees: c.fov_y_degrees,
        near: c.near,
        far: c.far,
        mirror: c.mirror,
    }
}

/// Mesh editions for the renderer: content digests of each mesh's blob, so
/// equal content keeps its GPU buffers across requests. The whole blob is
/// hashed; sampling its head would let same-length edits reuse stale buffers.
fn mesh_editions(
    manifest: &GeometryManifest,
    blobs: &BTreeMap<String, Vec<u8>>,
) -> Result<BTreeMap<u64, u64>, String> {
    capture_server::mesh_editions(manifest, blobs)
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let first = args.next().ok_or(
        "usage: pbrt-ui-capture GEOMETRY.json OUTPUT.png [WIDTH HEIGHT] [solid|normal|wireframe|lit|debug_albedo|debug_ao]",
    )?;
    if first == "--server" {
        return server();
    }
    if first == "--convert" || first == "--convert-f32" {
        let input = args.next().ok_or("missing input image")?;
        let output = args.next().ok_or("missing output PNG")?;
        let image = if first == "--convert-f32" {
            let bytes = std::fs::read(input)?;
            if bytes.len() < 8 {
                return Err("truncated RGB snapshot".into());
            }
            let width = u32::from_le_bytes(bytes[0..4].try_into()?);
            let height = u32::from_le_bytes(bytes[4..8].try_into()?);
            if width == 0
                || height == 0
                || width > 8192
                || height > 8192
                || width as u64 * height as u64 > 16_777_216
                || bytes.len() as u64 != 8 + width as u64 * height as u64 * 12
            {
                return Err("invalid RGB snapshot dimensions".into());
            }
            let values = bytes[8..]
                .chunks_exact(4)
                .map(|value| f32::from_le_bytes(value.try_into().unwrap()))
                .collect();
            image::Rgb32FImage::from_raw(width, height, values).ok_or("invalid RGB snapshot")?
        } else {
            // Radiance HDR renders keep their float light (image 0.24's
            // DynamicImage path would clamp them before the tone map).
            let bytes = std::fs::read(&input)?;
            if scene_parse::hdr::is_hdr(&bytes, &input) {
                scene_parse::hdr::decode(&bytes)?.to_rgb32f()
            } else {
                image::open(&input)?.to_rgb32f()
            }
        };
        // Use the same conversion as the Image Viewer when opening EXR/HDR renders.
        scene_view::tonemap::reinhard_srgb(&image).save(output)?;
        return Ok(());
    }
    let spec: Option<protocol::jobs::CaptureSpec> = if first == "--spec" {
        Some(serde_json::from_slice(&std::fs::read(
            args.next().ok_or("missing capture spec")?,
        )?)?)
    } else {
        None
    };
    let manifest_path = spec
        .as_ref()
        .map(|s| std::ffi::OsString::from(&s.geometry_path))
        .unwrap_or(first);
    let output = PathBuf::from(args.next().ok_or("missing output PNG path")?);
    let mut width = args
        .next()
        .map(|v| v.to_string_lossy().parse::<u32>())
        .transpose()?
        .unwrap_or(1280);
    let mut height = args
        .next()
        .map(|v| v.to_string_lossy().parse::<u32>())
        .transpose()?
        .unwrap_or(720);
    let mut debug = LitDebug::Off;
    let mut mode = match args
        .next()
        .map(|v| v.to_string_lossy().to_ascii_lowercase())
    {
        None => PreviewMode::Solid,
        Some(v) if v == "solid" => PreviewMode::Solid,
        Some(v) if v == "normal" => PreviewMode::Normal,
        Some(v) if v == "wireframe" => PreviewMode::Wireframe,
        Some(v) if v == "lit" => PreviewMode::Lit,
        Some(v) if v == "debug_albedo" => {
            debug = LitDebug::Albedo;
            PreviewMode::Lit
        }
        Some(v) if v == "debug_ao" => {
            debug = LitDebug::Ao;
            PreviewMode::Lit
        }
        Some(v) => return Err(format!("unknown preview mode {v:?}").into()),
    };
    if let Some(spec) = &spec {
        [width, height] = spec.size;
        mode = match spec.mode {
            protocol::view::DisplayMode::Solid => PreviewMode::Solid,
            protocol::view::DisplayMode::Normal => PreviewMode::Normal,
            protocol::view::DisplayMode::Wireframe => PreviewMode::Wireframe,
            protocol::view::DisplayMode::Lit => PreviewMode::Lit,
        };
        if debug == LitDebug::Off {
            debug = debug_of(spec)?;
        }
    }
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || width as u64 * height as u64 > 16_777_216
    {
        return Err("capture dimensions exceed limit".into());
    }
    if args.next().is_some() {
        return Err("too many arguments".into());
    }
    let manifest_path = PathBuf::from(manifest_path).canonicalize()?;
    let directory = manifest_path
        .parent()
        .ok_or("geometry manifest has no parent")?;
    let manifest: GeometryManifest = serde_json::from_slice(&std::fs::read(&manifest_path)?)?;
    let mut blobs = BTreeMap::new();
    for name in manifest.blobs.keys() {
        let rel = Path::new(name);
        if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
            return Err(format!("unsafe blob path {name}").into());
        }
        let path = directory.join(rel).canonicalize()?;
        if !path.starts_with(directory) {
            return Err(format!("blob escapes geometry directory: {name}").into());
        }
        blobs.insert(name.clone(), std::fs::read(path)?);
    }
    let camera = if let Some(spec) = &spec {
        let c = &spec.camera;
        ViewerCamera {
            eye: c.eye,
            target: c.target,
            up: c.up,
            fov_y_degrees: c.fov_y_degrees,
            near: c.near,
            far: c.far,
            mirror: c.mirror,
        }
    } else {
        ViewerCamera::from_manifest(&manifest, width as f64 / height as f64)
    };
    let time = spec.as_ref().map_or_else(
        || manifest.supported_times[manifest.supported_times.len() / 2],
        |spec| spec.time,
    );
    let frame = render_preview_at_time_with_options(
        &manifest, &blobs, camera, width, height, mode, time, debug,
    )?;
    image::RgbaImage::from_raw(width, height, frame.rgba.clone())
        .ok_or("invalid RGBA frame size")?
        .save(&output)?;
    if spec.is_some() {
        std::fs::write(
            output.with_extension("pick"),
            frame
                .pick_ids
                .iter()
                .flat_map(|id| id.to_le_bytes())
                .collect::<Vec<_>>(),
        )?;
    }
    let mut coverage = BTreeMap::<u32, u64>::new();
    for id in &frame.pick_ids {
        if *id != 0 {
            *coverage.entry(*id).or_default() += 1;
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "output": output.canonicalize()?.display().to_string(), "width": width, "height": height,
            "mode": format!("{mode:?}").to_ascii_lowercase(), "debug_albedo": debug == LitDebug::Albedo, "debug_ao": debug == LitDebug::Ao, "backend": frame.backend,
            "draw_count": manifest.draws.len(), "picked_draws": coverage.len(), "covered_pixels": coverage.values().sum::<u64>()
        }))?
    );
    Ok(())
}
