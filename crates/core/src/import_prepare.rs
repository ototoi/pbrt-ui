//! Worker-only preparation and bounded validation of standalone resources.
use project::*;
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};

pub(crate) fn staging_base(project_folder: Option<&Path>, cache_root: &Path) -> std::path::PathBuf {
    project_folder
        .map(|folder| folder.join("Library/Staging"))
        .unwrap_or_else(|| cache_root.to_path_buf())
}

const MAX_ASSET: u64 = 4 * 1024 * 1024 * 1024;
const MAX_PIXELS: u64 = 67_108_864;
const MAX_DECODE: u64 = 1024 * 1024 * 1024;

fn is_gzip_ply(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("gz"))
        && path
            .file_stem()
            .and_then(|stem| Path::new(stem).extension())
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("ply"))
}

#[cfg(test)]
fn resource(source: &Path, cancelled: impl Fn() -> bool) -> Result<PreparedImport> {
    let base = source.parent().unwrap_or(Path::new("."));
    resource_in(source, base, cancelled)
}

pub(crate) fn resource_in(
    source: &Path,
    staging_base: &Path,
    cancelled: impl Fn() -> bool,
) -> Result<PreparedImport> {
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if extension == "spd" {
        return spectrum_data_import(source, &cancelled);
    }
    let media = match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "tga" => "image/x-tga",
        "hdr" => "image/vnd.radiance",
        "exr" => "image/x-exr",
        "ply" => "application/x-ply",
        "gz" if is_gzip_ply(source) => "application/x-ply",
        _ => {
            return Err(ProjectError::Invalid(
                "unsupported resource format; choose PNG, JPEG, TGA, HDR, EXR, PLY, PLY.GZ or SPD"
                    .into(),
            ));
        }
    };
    let before = std::fs::metadata(source)?;
    if !before.is_file() {
        return Err(ProjectError::Invalid(
            "resource source must be a file".into(),
        ));
    }
    if before.len() > MAX_ASSET {
        return Err(ProjectError::ResourceLimit("asset exceeds 4 GiB".into()));
    }
    let mut input = File::open(source)?;
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0; 64 * 1024];
    loop {
        if cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size = size
            .checked_add(count as u64)
            .ok_or_else(|| ProjectError::ResourceLimit("asset byte count overflow".into()))?;
        if size > MAX_ASSET {
            return Err(ProjectError::ResourceLimit("asset exceeds 4 GiB".into()));
        }
        hash.update(&buffer[..count]);
    }
    let filename = source
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| ProjectError::Invalid("source filename is not UTF-8".into()))?;
    let mut document = ProjectDocument::new_empty("Resource import");
    // A provisional folder only: placement binds the caller's folder.
    let folder = if media == "application/x-ply" {
        "Meshes"
    } else {
        "Images"
    };
    let asset = Asset {
        id: AssetId::new(),
        location: format!(
            "{folder}/{}.{}",
            project::layout::sanitize(
                source
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or(filename),
                "resource"
            ),
            extension
        ),
        media_type: media.into(),
        sha256: format!("{:x}", hash.finalize()),
        size,
        // No AssetOrigin: sidecar meta/2 keeps provenance only for assets a
        // PBRT ImportRecord introduced (its origin requires an import id), so
        // a logical-path-only origin cannot survive save/read-back - and the
        // dirty check compares the live document with the saved read-back.
        origin: None,
    };
    let mut staging = ImportStaging::new_in(staging_base)?;
    let lease = staging.stage_file_cancellable(&asset, source, &cancelled)?;
    let after = std::fs::metadata(source)?;
    if before.len() != after.len() || before.modified()? != after.modified()? {
        return Err(ProjectError::SourceChanged(source.display().to_string()));
    }
    validate_asset(&asset, lease.path(), &cancelled)?;
    document.assets.insert(asset.id, asset);
    PreparedImport::new(document, vec![lease])
}

/// a standalone.spd file
/// imports as one SpectrumData resource — the samples parsed with the
/// same `read_float_file` rules the scene importer uses, no asset and no
/// staged payload. Placement re-folds the label into the caller's folder
/// or the kind's, like every resource an import carries.
fn spectrum_data_import(source: &Path, cancelled: &impl Fn() -> bool) -> Result<PreparedImport> {
    let before = std::fs::metadata(source)?;
    if !before.is_file() {
        return Err(ProjectError::Invalid(
            "resource source must be a file".into(),
        ));
    }
    if before.len() > MAX_SPD_FILE_BYTES {
        return Err(ProjectError::ResourceLimit("spd file exceeds 1 MiB".into()));
    }
    let mut input = File::open(source)?;
    let mut bytes = Vec::with_capacity(before.len() as usize);
    loop {
        if cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let mut buffer = [0; 64 * 1024];
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.len() as u64 > MAX_SPD_FILE_BYTES {
            return Err(ProjectError::ResourceLimit("spd file exceeds 1 MiB".into()));
        }
    }
    let after = std::fs::metadata(source)?;
    if before.len() != after.len() || before.modified()? != after.modified()? {
        return Err(ProjectError::SourceChanged(source.display().to_string()));
    }
    let filename = source
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| ProjectError::Invalid("source filename is not UTF-8".into()))?;
    let samples = parse_spd(&bytes)
        .map_err(|reason| ProjectError::Invalid(format!("spectrum file {filename}: {reason}")))?;
    let label = project::layout::sanitize(
        source
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(filename),
        "spectrum",
    );
    let table = |values: Vec<f64>| {
        ParameterValues::Floats(
            values
                .into_iter()
                .map(FloatValue::Finite)
                .collect::<Vec<FloatValue>>(),
        )
    };
    let folder = project::layout::kind_folder(ResourceKind::SpectrumData);
    let id = ResourceId::new();
    let location = format!(
        "{folder}/{label}.{}",
        project::layout::kind_extension(ResourceKind::SpectrumData)
    );
    let mut document = ProjectDocument::new_empty("Resource import");
    document.folders.insert(folder.into());
    document.resources.insert(
        id,
        Resource {
            id,
            label,
            location,
            edition: Revision::initial(),
            kind: ResourceKind::SpectrumData,
            texture_value_type: None,
            implementation: "piecewise".into(),
            parameters: vec![
                Parameter {
                    value_type: "float".into(),
                    name: "wavelengths".into(),
                    color_space: ColorSpaceBinding::Inherit,
                    values: table(samples.wavelengths),
                },
                Parameter {
                    value_type: "float".into(),
                    name: "values".into(),
                    color_space: ColorSpaceBinding::Inherit,
                    values: table(samples.values),
                },
            ],
            definition_scope: None,
            declared_in: None,
            origin: None,
        },
    );
    PreparedImport::new(document, Vec::new())
}

pub(crate) fn validate_batch(batch: &PreparedImport, cancelled: impl Fn() -> bool) -> Result<()> {
    for asset in batch.document().assets.values() {
        if cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let lease = batch
            .payload(asset.id)
            .ok_or_else(|| ProjectError::Invalid("missing prepared payload".into()))?;
        validate_asset(asset, lease.path(), &cancelled)?;
    }
    Ok(())
}

fn validate_asset(asset: &Asset, path: &Path, cancelled: &impl Fn() -> bool) -> Result<()> {
    if cancelled() {
        return Err(ProjectError::Cancelled);
    }
    if asset.media_type == "application/x-ply" {
        use scene_parse::ply::{PlyValidationError as E, PlyValidationLimits, validate_ply_file};
        // The staged file is named by its asset ID, and location names may
        // be disambiguated on placement. Detect compression from its bytes.
        let result = validate_ply_file(path, PlyValidationLimits::default(), cancelled);
        result.map_err(|error| match error {
            E::Cancelled => ProjectError::Cancelled,
            E::Limit(limit) => ProjectError::ResourceLimit(format!("PLY {limit}")),
            E::Invalid(error) => ProjectError::Invalid(error.to_string()),
        })?;
    } else if asset.media_type.starts_with("image/") {
        let expected = image::ImageFormat::from_path(&asset.location).map_err(image_error)?;
        let open = || -> Result<image::io::Reader<std::io::BufReader<File>>> {
            // Keep the expected format first: content guessing overwrites it
            // when magic bytes identify another format (the mismatch case),
            // but formats without magic — TGA — keep the extension's format.
            let mut reader = image::io::Reader::open(path)?;
            reader.set_format(expected);
            let mut reader = reader.with_guessed_format()?;
            if reader.format() != Some(expected) {
                return Err(ProjectError::Invalid(
                    "image content does not match its extension".into(),
                ));
            }
            let mut limits = image::io::Limits::default();
            limits.max_image_width = Some(16_384);
            limits.max_image_height = Some(16_384);
            limits.max_alloc = Some(MAX_DECODE);
            reader.limits(limits);
            Ok(reader)
        };
        let (width, height) = open()?.into_dimensions().map_err(image_error)?;
        let pixels = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or_else(|| ProjectError::ResourceLimit("image dimension overflow".into()))?;
        if width == 0
            || height == 0
            || width > 16_384
            || height > 16_384
            || pixels > MAX_PIXELS
            || pixels.checked_mul(16).is_none_or(|size| size > MAX_DECODE)
        {
            return Err(ProjectError::ResourceLimit(
                "decoded image exceeds import limits".into(),
            ));
        }
        if cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let decoded = open()?.decode().map_err(image_error)?;
        if decoded.as_bytes().len() as u64 > MAX_DECODE {
            return Err(ProjectError::ResourceLimit(
                "decoded image exceeds 1 GiB".into(),
            ));
        }
    }
    if cancelled() {
        return Err(ProjectError::Cancelled);
    }
    Ok(())
}

fn image_error(error: image::ImageError) -> ProjectError {
    match error {
        image::ImageError::Limits(error) => ProjectError::ResourceLimit(error.to_string()),
        error => ProjectError::Invalid(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_staging_uses_project_library_or_cache_base() {
        let cache = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        assert_eq!(
            staging_base(Some(project.path()), cache.path()),
            project.path().join("Library/Staging")
        );
        assert_eq!(staging_base(None, cache.path()), cache.path());
    }
    use project::ProjectError;
    use std::path::PathBuf;

    fn directory() -> PathBuf {
        tempfile::tempdir().unwrap().keep()
    }
    fn png(path: &Path) {
        image::RgbImage::from_raw(2, 2, vec![0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255])
            .unwrap()
            .save(path)
            .unwrap();
    }
    fn tiny_ply() -> &'static [u8] {
        b"ply\nformat ascii 1.0\nelement vertex 4\nproperty float x\nproperty float y\n\
          property float z\nelement face 4\nproperty list uchar int vertex_indices\n\
          end_header\n0 0 0\n1 0 0\n0 1 0\n0 0 1\n3 0 1 2\n3 0 1 3\n3 1 2 3\n3 2 0 3\n"
    }
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                let carry = crc & 1;
                crc >>= 1;
                if carry != 0 {
                    crc ^= 0xEDB8_8320;
                }
            }
        }
        !crc
    }
    /// A PNG whose header declares `width`x`height` but whose body is a
    /// zero-length IDAT: header-only fixtures never reach a decoder, so the
    /// reject has to come from the dimension checks, not a decode failure.
    fn png_header_only(path: &Path, width: u32, height: u32) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']);
        let mut data = Vec::new();
        data.extend_from_slice(&width.to_be_bytes());
        data.extend_from_slice(&height.to_be_bytes());
        data.extend_from_slice(&[8, 0, 0, 0, 0]);
        let mut chunk = Vec::new();
        chunk.extend_from_slice(b"IHDR");
        chunk.extend_from_slice(&data);
        bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&chunk);
        bytes.extend_from_slice(&crc32(&chunk).to_be_bytes());
        // A well-formed zero-length IDAT keeps header-only readers happy.
        let idat = b"IDAT".to_vec();
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&idat);
        bytes.extend_from_slice(&crc32(&idat).to_be_bytes());
        std::fs::write(path, &bytes).unwrap();
    }

    #[test]
    fn standalone_images_become_an_image_asset_without_a_texture() {
        let root = directory();
        let source = root.join("wood.png");
        png(&source);
        let batch = resource(&source, || false).unwrap();
        let document = batch.document();
        assert_eq!(document.scenes.len(), 0);
        assert_eq!(document.resources.len(), 0, "images do not create textures");
        assert_eq!(document.assets.len(), 1);
        // No ImportRecord may be fabricated: the ImportedSource bookkeeping
        // is the PBRT parser's, and this input was never parsed as PBRT. No
        // AssetOrigin either - meta/2 provenance requires an import id, so
        // a logical-path-only origin would not survive save/read-back and
        // would keep the project dirty after every save.
        assert!(document.imports.is_empty());
        let asset = document.assets.values().next().unwrap();
        assert_eq!(asset.media_type, "image/png");
        assert_eq!(asset.location, "Images/wood.png");
        assert_eq!(asset.origin, None);
        let file_bytes = std::fs::read(&source).unwrap();
        assert_eq!(asset.size, file_bytes.len() as u64);
        assert_eq!(asset.sha256, format!("{:x}", Sha256::digest(&file_bytes)));
        // validate_batch re-checks the staged copy: same bytes, still decodes.
        validate_batch(&batch, || false).unwrap();
        let lease = batch.payload(asset.id).unwrap();
        assert_eq!(std::fs::read(lease.path()).unwrap(), file_bytes);
    }

    #[test]
    fn standalone_ply_becomes_a_mesh_asset_without_a_resource() {
        let root = directory();
        let source = root.join("toy.ply");
        std::fs::write(&source, tiny_ply()).unwrap();
        let batch = resource(&source, || false).unwrap();
        let document = batch.document();
        assert_eq!(document.resources.len(), 0);
        assert_eq!(document.assets.len(), 1);
        assert!(document.imports.is_empty());
        let asset = document.assets.values().next().unwrap();
        assert_eq!(asset.media_type, "application/x-ply");
        assert_eq!(asset.location, "Meshes/toy.ply");
        assert_eq!(asset.origin, None);
        validate_batch(&batch, || false).unwrap();
        let broken = root.join("broken.ply");
        std::fs::write(&broken, b"ply\nformat ascii 1.0\nelement vertex 1\n").unwrap();
        assert!(resource(&broken, || false).is_err());
    }

    #[test]
    fn standalone_gzip_ply_is_validated_and_kept_as_a_compressed_mesh_asset() {
        use std::io::Write as _;
        let root = directory();
        let source = root.join("toy.ply.gz");
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(tiny_ply()).unwrap();
        let compressed = encoder.finish().unwrap();
        std::fs::write(&source, &compressed).unwrap();

        let batch = resource(&source, || false).unwrap();
        let asset = batch.document().assets.values().next().unwrap();
        assert_eq!(asset.media_type, "application/x-ply");
        assert_eq!(asset.location, "Meshes/toy.ply.gz");
        validate_batch(&batch, || false).unwrap();
        assert_eq!(
            std::fs::read(batch.payload(asset.id).unwrap().path()).unwrap(),
            compressed
        );

        let unrelated = root.join("notes.gz");
        std::fs::write(&unrelated, compressed).unwrap();
        assert!(matches!(
            resource(&unrelated, || false),
            Err(ProjectError::Invalid(_))
        ));

        let broken = root.join("broken.ply.gz");
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(b"not a PLY").unwrap();
        std::fs::write(&broken, encoder.finish().unwrap()).unwrap();
        assert!(resource(&broken, || false).is_err());
    }

    #[test]
    fn scene_import_validates_same_basename_gzip_meshes_after_location_disambiguation() {
        use std::io::Write as _;
        let root = tempfile::tempdir().unwrap();
        for (folder, x) in [("left", 1), ("right", 2)] {
            let dir = root.path().join(folder);
            std::fs::create_dir(&dir).unwrap();
            let ply = format!(
                "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n{x} 0 0\n0 1 0\n0 0 1\n3 0 1 2\n"
            );
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(ply.as_bytes()).unwrap();
            std::fs::write(dir.join("mesh.ply.gz"), encoder.finish().unwrap()).unwrap();
        }
        let scene = root.path().join("scene.pbrt");
        std::fs::write(
            &scene,
            "WorldBegin\nShape \"plymesh\" \"string filename\" [\"left/mesh.ply.gz\"]\nShape \"plymesh\" \"string filename\" [\"right/mesh.ply.gz\"]\n",
        )
        .unwrap();

        let (batch, report) =
            pbrt_io::prepare_scene(&scene, &Default::default(), || false).unwrap();
        assert!(report.diagnostics.is_empty(), "{report:?}");
        let assets = batch.document().assets.values().collect::<Vec<_>>();
        assert_eq!(assets.len(), 2);
        assert_ne!(assets[0].sha256, assets[1].sha256);
        assert_ne!(assets[0].location, assets[1].location);
        assert!(
            assets
                .iter()
                .all(|asset| asset.media_type == "application/x-ply")
        );
        validate_batch(&batch, || false).unwrap();
    }

    /// a standalone.spd file
    /// imports as one SpectrumData resource — data in the resource, no
    /// asset, no lease — parsing with the same rules the scene importer
    /// uses, placed by the kind's folder and extension.
    #[test]
    fn standalone_spd_becomes_a_spectrum_data_resource() {
        let root = directory();
        let source = root.join("Au.k.spd");
        std::fs::write(&source, "# gold\n300.0 1.0\n500.0 2.0\n700.5 3.5\n").unwrap();
        let batch =
            resource(&source, || false).expect("a standalone spd file imports as spectrum data");
        let document = batch.document();
        assert_eq!(
            document.assets.len(),
            0,
            "no asset: the data lives in the resource"
        );
        assert_eq!(document.resources.len(), 1);
        let spectrum = document.resources.values().next().unwrap();
        assert_eq!(spectrum.kind, ResourceKind::SpectrumData);
        assert_eq!(spectrum.label, "Au.k");
        assert_eq!(spectrum.implementation, "piecewise");
        assert_eq!(spectrum.location, "Spectra/Au.k.spectrum");
        let floats = |name: &str| {
            spectrum
                .parameters
                .iter()
                .find(|parameter| parameter.name == name)
                .map(|parameter| &parameter.values)
        };
        assert_eq!(
            floats("wavelengths"),
            Some(&ParameterValues::Floats(vec![
                FloatValue::Finite(300.),
                FloatValue::Finite(500.),
                FloatValue::Finite(700.5),
            ]))
        );
        assert_eq!(
            floats("values"),
            Some(&ParameterValues::Floats(vec![
                FloatValue::Finite(1.),
                FloatValue::Finite(2.),
                FloatValue::Finite(3.5),
            ]))
        );
        // The caller's folder wins, like every single-resource import.
        let mut target = ProjectDocument::new_empty("target");
        target.folders.insert("Spectra".into());
        let placed = batch.placed_for(&target, Some("Spectra")).unwrap();
        assert_eq!(
            placed
                .document()
                .resources
                .values()
                .next()
                .unwrap()
                .location,
            "Spectra/Au.k.spectrum"
        );
        // The parse rules match the scene importer's: odd counts refuse.
        let broken = root.join("broken.spd");
        std::fs::write(&broken, b"1 2 3\n").unwrap();
        assert!(resource(&broken, || false).is_err());
    }

    #[test]
    fn unsupported_formats_and_content_mismatches_are_rejected() {
        let root = directory();
        let plain = root.join("notes.xyz");
        std::fs::write(&plain, b"data").unwrap();
        match resource(&plain, || false) {
            Err(ProjectError::Invalid(_)) => {}
            other => panic!("expected rejection, got {other:?}"),
        }
        // Real PNG bytes under a.jpg name: the detected content must win.
        // (image::save writes JPEG when the path says.jpg, so copy.)
        let real = root.join("photo.png");
        png(&real);
        let mismatch = root.join("photo.jpg");
        std::fs::copy(&real, &mismatch).unwrap();
        let outcome = resource(&mismatch, || false);
        assert!(outcome.is_err(), "mismatched content accepted: {outcome:?}");
        match &outcome {
            Err(ProjectError::Invalid(message)) => {
                assert!(message.contains("does not match"), "{message}")
            }
            _ => panic!("wrong rejection: {outcome:?}"),
        }
    }

    #[test]
    fn image_limits_reject_from_headers_before_decoding() {
        let root = directory();
        // A few hundred bytes declaring 10,000x8,000 pixels: over the
        // pixel budget without anything being decoded.
        let huge = root.join("huge.png");
        png_header_only(&huge, 10_000, 8_000);
        match resource(&huge, || false) {
            Err(ProjectError::ResourceLimit(_)) => {}
            other => panic!("expected limit rejection, got {other:?}"),
        }
        // 16,385 on one side trips the per-axis cap before the product one.
        let tall = root.join("tall.png");
        png_header_only(&tall, 16_385, 1);
        match resource(&tall, || false) {
            Err(ProjectError::ResourceLimit(_)) => {}
            other => panic!("expected limit rejection, got {other:?}"),
        }
        // 8192x8193 is one pixel row over the exact pixel budget.
        let boundary = root.join("boundary.png");
        png_header_only(&boundary, 8_192, 8_193);
        match resource(&boundary, || false) {
            Err(ProjectError::ResourceLimit(_)) => {}
            other => panic!("expected limit rejection, got {other:?}"),
        }
        let zero = root.join("zero.png");
        png_header_only(&zero, 0, 0);
        assert!(resource(&zero, || false).is_err());
    }

    #[test]
    fn cancelled_preparation_changes_nothing() {
        let root = directory();
        let source = root.join("wood.png");
        png(&source);
        assert!(matches!(
            resource(&source, || true),
            Err(ProjectError::Cancelled)
        ));
        let batch = resource(&source, || false).unwrap();
        assert!(matches!(
            validate_batch(&batch, || true),
            Err(ProjectError::Cancelled)
        ));
    }

    /// Filenames that sanitization rewrites ('?' is a legal character on
    /// Linux) still import, under the sanitized location.
    #[cfg(unix)]
    #[test]
    fn sanitized_stems_import_under_a_valid_location() {
        let root = directory();
        let real = root.join("ok.png");
        png(&real);
        let source = root.join("wood?.png");
        std::fs::copy(&real, &source).unwrap();
        let batch = resource(&source, || false).expect("a sanitizable filename still imports");
        let document = batch.document();
        let asset = document.assets.values().next().unwrap();
        assert_eq!(asset.location, "Images/wood_.png");
        // A stem that sanitizes to nothing takes the fallback name.
        let source = root.join(" .png");
        std::fs::copy(&real, &source).unwrap();
        let batch = resource(&source, || false).unwrap();
        let asset = batch.document().assets.values().next().unwrap();
        assert_eq!(asset.location, "Images/resource.png");
    }

    /// Uncompressed 24-bit true-color TGA: the one supported image format
    /// without magic bytes, so guessing cannot overwrite a wrong extension
    /// nor "confirm" a right one. It must import through the expected-format
    /// path instead.
    #[test]
    fn extension_only_image_formats_import_by_expected_path() {
        let root = directory();
        let source = root.join("tag.tga");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 0, 24, 32]);
        for pixel in [[0, 0, 255], [0, 255, 0], [255, 0, 0], [255, 255, 255]] {
            bytes.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
        }
        std::fs::write(&source, &bytes).unwrap();
        let batch = resource(&source, || false)
            .expect("TGA has no magic: the extension's format must be trusted");
        let document = batch.document();
        let asset = document.assets.values().next().unwrap();
        assert_eq!(asset.media_type, "image/x-tga");
        assert_eq!(asset.location, "Images/tag.tga");
        validate_batch(&batch, || false).unwrap();
    }
}
