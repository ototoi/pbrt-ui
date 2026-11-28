//! Bounded reader for the small part of RGL tensor files needed to give
//! measured materials a representative preview color.
use super::ParseOptions;
use crate::error::Diagnostic;
use protocol::geometry::{MaterialRecord, PreviewParameter};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
struct Field {
    name: String,
    dtype: u8,
    offset: u64,
    shape: Vec<u64>,
    bytes: u64,
}

/// Add preview-only reflectance parameters to measured records. The cache is
/// scoped to one analysis, so replacing a file is visible on the next parse.
pub(super) fn complete_records(
    records: &mut [MaterialRecord],
    root: &Path,
    options: &ParseOptions,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut cache = HashMap::<PathBuf, Result<[f32; 3], String>>::new();
    let mut diagnosed = HashSet::<PathBuf>::new();
    for record in records {
        if record.implementation != "measured"
            || record.parameters.iter().any(|p| p.matches("reflectance"))
        {
            continue;
        }
        let filename = record
            .parameters
            .iter()
            .find(|p| p.matches("filename"))
            .and_then(|p| p.texts.first())
            .filter(|s| !s.is_empty());
        let resolved = filename.map(|name| {
            super::program::resolve_referenced_asset(name, record.source.as_ref(), root)
                .map_err(|error| error.to_string())
        });
        let path = resolved.as_ref().and_then(|result| result.as_ref().ok());
        let result = match &resolved {
            Some(Ok(path)) => Some(cached_color(&mut cache, path, || {
                read_color(path, options.max_asset_bytes)
            })),
            Some(Err(error)) => Some(Err(error.clone())),
            None => None,
        };
        match result.as_ref() {
            Some(Ok(color)) => record.parameters.push(rgb_parameter(*color)),
            _ => {
                let key = path
                    .map(|path| protocol::path::canonicalize(path).unwrap_or_else(|_| path.clone()))
                    .unwrap_or_else(|| {
                        let filename = filename.map_or("<missing filename>", String::as_str);
                        record
                            .source
                            .as_ref()
                            .and_then(|source| Path::new(&source.path).parent())
                            .unwrap_or(Path::new("."))
                            .join(filename)
                    });
                if diagnosed.insert(key) {
                    let why = result
                        .as_ref()
                        .and_then(|r| r.as_ref().err())
                        .cloned()
                        .unwrap_or_else(|| "filename is missing".into());
                    let warning = Diagnostic::warning(
                        "measured_color_failed",
                        format!("measured BRDF could not be read: {why}"),
                    );
                    diagnostics.push(if let Some(source) = &record.source {
                        warning.at(&source.path, source.line)
                    } else {
                        warning
                    });
                }
            }
        }
    }
}

fn rgb_parameter(rgb: [f32; 3]) -> PreviewParameter {
    PreviewParameter {
        name: "reflectance".into(),
        value_type: "rgb".into(),
        floats: rgb.into_iter().map(f64::from).collect(),
        texts: Vec::new(),
        truncated: false,
    }
}

fn cached_color(
    cache: &mut HashMap<PathBuf, Result<[f32; 3], String>>,
    path: &Path,
    read: impl FnOnce() -> Result<[f32; 3], String>,
) -> Result<[f32; 3], String> {
    let key = protocol::path::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    cache.entry(key).or_insert_with(read).clone()
}

fn read_color(path: &Path, max_bytes: usize) -> Result<[f32; 3], String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    if size > max_bytes as u64 {
        return Err(format!("file exceeds the {max_bytes} byte asset limit"));
    }
    read_tensor_color(&mut file, size)
}

fn read_tensor_color(file: &mut File, size: u64) -> Result<[f32; 3], String> {
    if size < 18 {
        return Err("truncated tensor header".into());
    }
    let mut header = [0; 18];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    if &header[..11] != b"tensor_file" || header[11] != 0 || header[12..14] != [1, 0] {
        return Err("invalid tensor magic or version".into());
    }
    let count = u32::from_le_bytes(header[14..18].try_into().unwrap()) as usize;
    if count > 256 {
        return Err("tensor field count exceeds 256".into());
    }
    let mut fields = Vec::with_capacity(count);
    for _ in 0..count {
        let name_len = read_u16(file)? as usize;
        if name_len > 1024 {
            return Err("tensor field name exceeds 1024 bytes".into());
        }
        let mut name = vec![0; name_len];
        file.read_exact(&mut name).map_err(|e| e.to_string())?;
        let name = String::from_utf8(name).map_err(|_| "tensor field name is not UTF-8")?;
        let ndim = read_u16(file)? as usize;
        if ndim > 16 {
            return Err("tensor field has more than 16 dimensions".into());
        }
        let mut dtype = [0];
        file.read_exact(&mut dtype).map_err(|e| e.to_string())?;
        let offset = read_u64(file)?;
        let mut shape = Vec::with_capacity(ndim);
        let mut elements = 1u64;
        for _ in 0..ndim {
            let dim = read_u64(file)?;
            if dim == 0 {
                return Err("tensor dimensions must be positive".into());
            }
            elements = elements.checked_mul(dim).ok_or("tensor shape overflow")?;
            shape.push(dim);
        }
        let bytes = elements
            .checked_mul(type_size(dtype[0])?)
            .ok_or("tensor byte size overflow")?;
        let end = offset.checked_add(bytes).ok_or("tensor offset overflow")?;
        if offset < 18 || end > size {
            return Err("tensor field lies outside file".into());
        }
        fields.push(Field {
            name,
            dtype: dtype[0],
            offset,
            shape,
            bytes,
        });
    }
    let table_end = file.stream_position().map_err(|e| e.to_string())?;
    if fields.iter().any(|f| f.offset < table_end) {
        return Err("tensor payload overlaps field table".into());
    }
    let mut intervals = fields
        .iter()
        .map(|f| (f.offset, f.offset + f.bytes))
        .collect::<Vec<_>>();
    intervals.sort_unstable();
    if intervals.windows(2).any(|w| w[0].1 > w[1].0) {
        return Err("tensor fields overlap".into());
    }
    let find = |name: &str| -> Result<&Field, String> {
        let mut found = fields.iter().filter(|f| f.name == name);
        let one = found
            .next()
            .ok_or_else(|| format!("missing {name} field"))?;
        if found.next().is_some() {
            return Err(format!("duplicate {name} field"));
        }
        Ok(one)
    };
    let wavelengths = find("wavelengths")?;
    let spectra = find("spectra")?;
    if wavelengths.dtype != 10 || wavelengths.shape.len() != 1 {
        return Err("wavelengths must be 1D Float32".into());
    }
    if spectra.dtype != 10 || spectra.shape.len() != 5 {
        return Err("spectra must be 5D Float32".into());
    }
    let [p, t, l, n, n2] = spectra.shape.as_slice() else {
        unreachable!()
    };
    if n != n2 || *l != wavelengths.shape[0] {
        return Err("spectra shape does not match wavelengths".into());
    }
    let mut wl_bytes =
        vec![0; usize::try_from(wavelengths.bytes).map_err(|_| "wavelength data too large")?];
    file.seek(SeekFrom::Start(wavelengths.offset))
        .map_err(|e| e.to_string())?;
    file.read_exact(&mut wl_bytes).map_err(|e| e.to_string())?;
    let wl = floats(&wl_bytes)?;
    if wl.iter().any(|v| !v.is_finite() || *v <= 0.) || wl.windows(2).any(|w| w[0] >= w[1]) {
        return Err("wavelengths must be finite, positive, and strictly ascending".into());
    }
    let denominator = (*p as f64) * (*t as f64) * (*n as f64) * (*n as f64);
    let mut sums = vec![0f64; wl.len()];
    let chunk = 64 * 1024usize;
    let mut bytes = vec![0; chunk];
    file.seek(SeekFrom::Start(spectra.offset))
        .map_err(|e| e.to_string())?;
    let mut position = 0u64;
    while position < spectra.bytes {
        let take = (spectra.bytes - position).min(chunk as u64) as usize;
        file.read_exact(&mut bytes[..take])
            .map_err(|e| e.to_string())?;
        for (i, raw) in bytes[..take].as_chunks::<4>().0.iter().enumerate() {
            let value = f32::from_le_bytes(*raw);
            if !value.is_finite() || value < 0. {
                return Err("spectra contain a non-finite or negative value".into());
            }
            let sample = (position as usize / 4) + i;
            let wavelength = (sample / (*n as usize * *n as usize)) % wl.len();
            sums[wavelength] += f64::from(value);
        }
        position += take as u64;
    }
    let mut pairs = Vec::with_capacity(wl.len() * 2);
    for (lambda, value) in wl.into_iter().zip(sums) {
        pairs.extend([lambda, (value / denominator) as f32]);
    }
    let mut rgb =
        crate::texture_eval::spectra::sampled(&pairs).ok_or("invalid averaged spectrum")?;
    rgb.iter_mut().for_each(|v| *v = v.clamp(0., 1.));
    Ok(rgb)
}

fn type_size(dtype: u8) -> Result<u64, String> {
    match dtype {
        1 | 2 => Ok(1),
        3 | 4 | 9 => Ok(2),
        5 | 6 | 10 => Ok(4),
        7 | 8 | 11 => Ok(8),
        _ => Err("invalid tensor field type".into()),
    }
}
fn read_u16(file: &mut File) -> Result<u16, String> {
    let mut b = [0; 2];
    file.read_exact(&mut b).map_err(|e| e.to_string())?;
    Ok(u16::from_le_bytes(b))
}
fn read_u64(file: &mut File) -> Result<u64, String> {
    let mut b = [0; 8];
    file.read_exact(&mut b).map_err(|e| e.to_string())?;
    Ok(u64::from_le_bytes(b))
}
fn floats(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if !bytes.len().is_multiple_of(4) {
        return Err("invalid float payload size".into());
    }
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[derive(Clone)]
    struct TestField {
        name: String,
        dtype: u8,
        shape: Vec<u64>,
        data: Vec<u8>,
    }

    fn f32_field(name: &str, shape: &[u64], values: &[f32]) -> TestField {
        TestField {
            name: name.into(),
            dtype: 10,
            shape: shape.into(),
            data: values.iter().flat_map(|v| v.to_le_bytes()).collect(),
        }
    }

    fn fixture_fields(wavelengths: &[f32], shape: &[u64], spectra: &[f32]) -> Vec<TestField> {
        vec![
            f32_field("wavelengths", &[wavelengths.len() as u64], wavelengths),
            f32_field("spectra", shape, spectra),
        ]
    }

    fn tensor_fields(fields: &[TestField]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"tensor_file\0");
        bytes.extend([1, 0]);
        bytes.extend((fields.len() as u32).to_le_bytes());
        let mut offsets = Vec::new();
        for field in fields {
            bytes.extend((field.name.len() as u16).to_le_bytes());
            bytes.extend(field.name.as_bytes());
            bytes.extend((field.shape.len() as u16).to_le_bytes());
            bytes.push(field.dtype);
            offsets.push(bytes.len());
            bytes.extend([0; 8]);
            for dim in &field.shape {
                bytes.extend(dim.to_le_bytes());
            }
        }
        let mut offset = bytes.len() as u64;
        for (field, offset_at) in fields.iter().zip(offsets) {
            bytes[offset_at..offset_at + 8].copy_from_slice(&offset.to_le_bytes());
            offset += field.data.len() as u64;
        }
        for field in fields {
            bytes.extend(&field.data);
        }
        bytes
    }

    #[derive(Clone, Copy)]
    struct FieldLocation {
        dtype: usize,
        offset: usize,
        shape: usize,
        payload: usize,
    }

    fn locate_fields(bytes: &[u8]) -> Vec<(String, FieldLocation)> {
        let count = u32::from_le_bytes(bytes[14..18].try_into().unwrap()) as usize;
        let mut cursor = 18;
        let mut fields = Vec::new();
        for _ in 0..count {
            let name_len =
                u16::from_le_bytes(bytes[cursor..cursor + 2].try_into().unwrap()) as usize;
            cursor += 2;
            let name = String::from_utf8(bytes[cursor..cursor + name_len].to_vec()).unwrap();
            cursor += name_len;
            let ndim = u16::from_le_bytes(bytes[cursor..cursor + 2].try_into().unwrap()) as usize;
            cursor += 2;
            let dtype = cursor;
            cursor += 1;
            let offset = cursor;
            let payload =
                u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap()) as usize;
            cursor += 8;
            let shape = cursor;
            cursor += ndim * 8;
            fields.push((
                name,
                FieldLocation {
                    dtype,
                    offset,
                    shape,
                    payload,
                },
            ));
        }
        fields
    }

    fn tensor() -> Vec<u8> {
        // Stored layout is [P,T,L,N,N], with distinct averaged wavelength
        // values to catch using the wrong flattened axis.
        let wavelengths = [465., 532., 630.];
        let shape = [2, 1, 3, 2, 2];
        let mut spectra = Vec::new();
        for values in [[0., 0.4, 0.8], [0.2, 0.6, 1.0]] {
            for value in values {
                spectra.extend([value; 4]);
            }
        }
        tensor_fields(&fixture_fields(&wavelengths, &shape, &spectra))
    }

    fn temp_file(bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.bsdf");
        File::create(&path).unwrap().write_all(bytes).unwrap();
        (dir, path)
    }

    fn parameter(name: &str, value_type: &str, floats: &[f64], texts: &[&str]) -> PreviewParameter {
        PreviewParameter {
            name: name.into(),
            value_type: value_type.into(),
            floats: floats.into(),
            texts: texts.iter().map(|s| (*s).into()).collect(),
            truncated: false,
        }
    }

    fn measured_record(filename: &str, source: Option<&str>) -> MaterialRecord {
        MaterialRecord {
            implementation: "measured".into(),
            resource_id: None,
            name: Some("m".into()),
            source: source.map(|path| protocol::package::SourceLocation {
                path: path.into(),
                line: 1,
                column: 1,
            }),
            parameters: vec![parameter("filename", "string", &[], &[filename])],
        }
    }

    #[test]
    fn tensor_reader_averages_non_wavelength_axes_in_storage_order() {
        let (_dir, path) = temp_file(&tensor());
        assert_eq!(read_color(&path, 4096).unwrap(), [0.9, 0.5, 0.1]);
    }

    #[test]
    fn tensor_reader_interpolates_wavelengths_and_returns_black_and_clamped_values() {
        let interpolate = tensor_fields(&fixture_fields(
            &[400., 500., 600., 700.],
            &[1, 1, 4, 1, 1],
            &[0., 0.5, 1., 0.25],
        ));
        let (_dir, path) = temp_file(&interpolate);
        let got = read_color(&path, 4096).unwrap();
        for (actual, expected) in got.into_iter().zip([0.775, 0.66, 0.325]) {
            assert!((f64::from(actual) - expected).abs() < 1e-5, "{got:?}");
        }

        for values in [[0.; 4], [2.; 4]] {
            let bytes = tensor_fields(&fixture_fields(
                &[400., 500., 600., 700.],
                &[1, 1, 4, 1, 1],
                &values,
            ));
            let (_dir, path) = temp_file(&bytes);
            let color = read_color(&path, 4096).unwrap();
            assert_eq!(color, if values[0] == 0. { [0.; 3] } else { [1.; 3] });
        }
    }

    #[test]
    fn tensor_reader_rejects_truncation_offset_and_shape_overflow() {
        let bytes = tensor();
        let (_dir, path) = temp_file(&bytes[..24]);
        assert!(read_color(&path, 4096).is_err());

        for mutate in ["offset", "shape"] {
            let mut bytes = tensor();
            let (_, location) = locate_fields(&bytes)
                .into_iter()
                .find(|(name, _)| name == "spectra")
                .unwrap();
            if mutate == "offset" {
                bytes[location.offset..location.offset + 8]
                    .copy_from_slice(&u64::MAX.to_le_bytes());
            } else {
                bytes[location.shape..location.shape + 8].copy_from_slice(&u64::MAX.to_le_bytes());
            }
            let (_dir, path) = temp_file(&bytes);
            assert!(read_color(&path, 4096).is_err(), "{mutate}");
        }
    }

    #[test]
    fn tensor_reader_rejects_required_field_errors_types_shapes_and_invalid_values() {
        let base = fixture_fields(&[465., 532., 630.], &[1, 1, 3, 1, 1], &[0.2, 0.4, 0.6]);
        let mut cases = Vec::new();
        cases.push(tensor_fields(&base[1..])); // missing wavelengths
        let mut duplicate = base.clone();
        duplicate.push(base[0].clone());
        cases.push(tensor_fields(&duplicate));

        let mut wrong_type = tensor();
        let (_, location) = locate_fields(&wrong_type)
            .into_iter()
            .find(|(name, _)| name == "spectra")
            .unwrap();
        wrong_type[location.dtype] = 5;
        cases.push(wrong_type);

        let mut wrong_shape = tensor();
        let (_, location) = locate_fields(&wrong_shape)
            .into_iter()
            .find(|(name, _)| name == "spectra")
            .unwrap();
        wrong_shape[location.shape + 2 * 8..location.shape + 3 * 8]
            .copy_from_slice(&2u64.to_le_bytes());
        cases.push(wrong_shape);

        for value in [f32::NAN, -0.1] {
            let mut invalid_value = tensor();
            let (_, location) = locate_fields(&invalid_value)
                .into_iter()
                .find(|(name, _)| name == "spectra")
                .unwrap();
            invalid_value[location.payload..location.payload + 4]
                .copy_from_slice(&value.to_le_bytes());
            cases.push(invalid_value);
        }
        for bytes in cases {
            let (_dir, path) = temp_file(&bytes);
            assert!(read_color(&path, 4096).is_err());
        }
    }

    #[test]
    fn tensor_reader_honors_asset_byte_budget() {
        let bytes = tensor();
        let (_dir, path) = temp_file(&bytes);
        assert!(read_color(&path, bytes.len() - 1).is_err());
    }

    #[test]
    fn canonical_asset_aliases_share_one_analysis_cache_entry() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let path = dir.path().join("sample.bsdf");
        std::fs::write(&path, tensor()).unwrap();
        let alias = dir.path().join("sub/../sample.bsdf");
        let mut cache = HashMap::new();
        let mut reads = 0;
        let first = cached_color(&mut cache, &path, || {
            reads += 1;
            read_color(&path, 4096)
        })
        .unwrap();
        let second = cached_color(&mut cache, &alias, || {
            reads += 1;
            read_color(&alias, 4096)
        })
        .unwrap();
        assert_eq!(first, second);
        assert_eq!(reads, 1);
    }

    #[test]
    fn tensor_reader_rejects_nonfinite_wavelengths() {
        let bytes = tensor_fields(&fixture_fields(
            &[465., f32::INFINITY, 630.],
            &[1, 1, 3, 1, 1],
            &[0.2, 0.4, 0.6],
        ));
        let (_dir, path) = temp_file(&bytes);
        assert!(read_color(&path, 4096).is_err());
    }

    #[test]
    #[ignore = "requires the sibling pbrt-r4 test fixture"]
    fn tensor_reader_accepts_the_real_r4_bsdf_fixture() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../pbrt-r4/tests/bsdfs/paper_white_spec.bsdf");
        let color = read_color(&path, 128 * 1024 * 1024).unwrap();
        assert!(
            color
                .into_iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(&v))
        );
    }

    #[test]
    fn inline_and_named_measured_materials_resolve_include_relative_assets() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("sub")).unwrap();
        std::fs::write(root.path().join("sample.bsdf"), tensor()).unwrap();
        std::fs::write(
            root.path().join("sub/sample.bsdf"),
            tensor_fields(&fixture_fields(
                &[465., 532., 630.],
                &[1, 1, 3, 1, 1],
                &[0.8, 0.3, 0.1],
            )),
        )
        .unwrap();
        std::fs::write(
            root.path().join("sub/material.pbrt"),
            "Material \"measured\" \"string filename\" \"sample.bsdf\"\nShape \"sphere\"\n",
        )
        .unwrap();
        let scene = root.path().join("scene.pbrt");
        std::fs::write(
            &scene,
            "WorldBegin\nInclude \"sub/material.pbrt\"\nMakeNamedMaterial \"named\" \"string type\" \"measured\" \"string filename\" \"sample.bsdf\"\nAttributeBegin\nNamedMaterial \"named\"\nShape \"disk\"\nAttributeEnd\n",
        )
        .unwrap();
        let preview = crate::scene::parse_scene(&scene, ParseOptions::default()).unwrap();
        assert_eq!(preview.manifest.materials.len(), 2);
        for material in &preview.manifest.materials {
            let expected = if material.name.is_some() {
                [0.9, 0.5, 0.1]
            } else {
                [0.1, 0.3, 0.8]
            };
            assert!(
                material.parameters.iter().any(|p| {
                    p.matches("reflectance")
                        && p.floats
                            .iter()
                            .zip(expected)
                            .all(|(actual, expected)| (actual - expected).abs() < 1e-6)
                }),
                "{}: {:?}",
                material.name.as_deref().unwrap_or("inline"),
                material.parameters
            );
        }
        assert!(preview.diagnostics.is_empty());
    }

    #[test]
    fn named_measured_mix_child_is_completed_and_explicit_reflectance_wins() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("sample.bsdf"), tensor()).unwrap();
        let scene = root.path().join("scene.pbrt");
        std::fs::write(
            &scene,
            "WorldBegin\nMakeNamedMaterial \"leaf\" \"string type\" \"measured\" \"string filename\" \"sample.bsdf\"\nMaterial \"mix\" \"string materials\" [\"leaf\" \"leaf\"] \"float amount\" [0.5]\nShape \"sphere\"\n",
        ).unwrap();
        let preview = crate::scene::parse_scene(&scene, ParseOptions::default()).unwrap();
        let leaf = preview
            .manifest
            .materials
            .iter()
            .find(|m| m.name.as_deref() == Some("leaf"))
            .unwrap();
        assert!(
            leaf.parameters
                .iter()
                .any(|p| p.matches("reflectance") && p.floats[0] > 0.8)
        );
        assert!(preview.diagnostics.is_empty());

        let mut explicit = measured_record("does-not-exist.bsdf", None);
        explicit
            .parameters
            .push(parameter("reflectance", "rgb", &[0.1, 0.2, 0.3], &[]));
        let mut diagnostics = Vec::new();
        complete_records(
            std::slice::from_mut(&mut explicit),
            root.path(),
            &ParseOptions::default(),
            &mut diagnostics,
        );
        assert_eq!(
            explicit
                .parameters
                .iter()
                .filter(|p| p.matches("reflectance"))
                .count(),
            1
        );
        assert_eq!(
            explicit
                .parameters
                .iter()
                .find(|p| p.matches("reflectance"))
                .unwrap()
                .floats,
            [0.1, 0.2, 0.3]
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn failed_asset_diagnostics_are_deduplicated_per_resolved_source_asset() {
        let root = tempfile::tempdir().unwrap();
        let mut records = [
            measured_record("same.bsdf", Some("/scene-a/a.pbrt")),
            measured_record("same.bsdf", Some("/scene-b/b.pbrt")),
            measured_record("same.bsdf", Some("/scene-a/a.pbrt")),
        ];
        let mut diagnostics = Vec::new();
        complete_records(
            &mut records,
            root.path(),
            &ParseOptions::default(),
            &mut diagnostics,
        );
        assert_eq!(
            diagnostics.len(),
            2,
            "same source/asset deduplicates, distinct directories do not"
        );
    }

    #[test]
    fn a_new_parse_observes_replaced_bsdf_bytes() {
        let root = tempfile::tempdir().unwrap();
        let bsdf = root.path().join("sample.bsdf");
        let scene = root.path().join("scene.pbrt");
        std::fs::write(&bsdf, tensor()).unwrap();
        std::fs::write(&scene, "WorldBegin\nMaterial \"measured\" \"string filename\" \"sample.bsdf\"\nShape \"sphere\"\n").unwrap();
        let color = || {
            let preview = crate::scene::parse_scene(&scene, ParseOptions::default()).unwrap();
            preview.manifest.materials[0]
                .parameters
                .iter()
                .find(|p| p.matches("reflectance"))
                .unwrap()
                .floats
                .clone()
        };
        let first = color();
        let replacement = tensor_fields(&fixture_fields(
            &[465., 532., 630.],
            &[1, 1, 3, 1, 1],
            &[0.2, 0.2, 0.2],
        ));
        std::fs::write(&bsdf, replacement).unwrap();
        let second = color();
        assert_ne!(first, second);
        assert!(second.iter().all(|v| (*v - 0.2).abs() < 1e-6));
    }

    #[test]
    fn missing_measured_asset_leaves_gray_fallback_and_one_diagnostic() {
        let root = tempfile::tempdir().unwrap();
        let mut record = MaterialRecord {
            implementation: "measured".into(),
            resource_id: None,
            name: Some("m".into()),
            source: None,
            parameters: vec![PreviewParameter {
                name: "filename".into(),
                value_type: "string".into(),
                floats: vec![],
                texts: vec!["missing.bsdf".into()],
                truncated: false,
            }],
        };
        let mut diagnostics = Vec::new();
        complete_records(
            std::slice::from_mut(&mut record),
            root.path(),
            &ParseOptions::default(),
            &mut diagnostics,
        );
        assert!(!record.parameters.iter().any(|p| p.matches("reflectance")));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "measured_color_failed");
    }
}
