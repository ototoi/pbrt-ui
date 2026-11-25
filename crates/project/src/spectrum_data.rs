//! Spectrum data input and its open-time migration: pbrt-r4's spd float-file
//! form parsed once for both the pbrt-io
//! importer and the conversion of legacy spd Asset references on open.
use crate::*;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

/// The largest spd file opened for conversion. A file holding more pairs
/// than [`crate::MAX_SPECTRUM_SAMPLES`] is already oversized in content, so
/// this bounds the read itself.
pub const MAX_SPD_FILE_BYTES: u64 = 1024 * 1024;

/// One spd file's samples: wavelengths in nm and their values, ascending.
#[derive(Clone, Debug, PartialEq)]
pub struct SpdSamples {
    pub wavelengths: Vec<f64>,
    pub values: Vec<f64>,
}

/// Parse r4's spd form — `read_float_file`'s rules (pbrt-r4
/// util/misc/float_file.rs): whitespace-separated numbers, any line that
/// contains `#` is a comment. Strict mode rejects cases that pbrt-r4 accepts:
/// file must hold an even, finite, positive-λ, non-descending list of at
/// most [`crate::MAX_SPECTRUM_SAMPLES`] pairs; the caller reports the
/// reason and skips the input.
pub fn parse_spd(bytes: &[u8]) -> std::result::Result<SpdSamples, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "the file is not UTF-8".to_owned())?;
    let mut numbers = Vec::new();
    for line in text.lines() {
        if line.contains('#') {
            continue;
        }
        for token in line.split_ascii_whitespace() {
            let number = token
                .parse::<f64>()
                .map_err(|_| format!("non-numeric sample {token:?}"))?;
            if !number.is_finite() {
                return Err(format!("non-finite sample {token:?}"));
            }
            numbers.push(number);
        }
    }
    if numbers.is_empty() {
        return Err("the file holds no samples".to_owned());
    }
    if numbers.len() % 2 != 0 {
        return Err(format!(
            "odd sample count {}: a spectrum file holds wavelength/value pairs",
            numbers.len()
        ));
    }
    if numbers.len() / 2 > crate::MAX_SPECTRUM_SAMPLES {
        return Err(format!(
            "more than {} sample pairs",
            crate::MAX_SPECTRUM_SAMPLES
        ));
    }
    let samples = SpdSamples {
        wavelengths: numbers.chunks_exact(2).map(|pair| pair[0]).collect(),
        values: numbers.chunks_exact(2).map(|pair| pair[1]).collect(),
    };
    for pair in samples.wavelengths.windows(2) {
        if pair[1] < pair[0] {
            return Err("descending wavelengths".to_owned());
        }
    }
    for wavelength in &samples.wavelengths {
        if *wavelength <= 0. {
            return Err(format!("non-positive wavelength {wavelength}"));
        }
    }
    Ok(samples)
}

/// A legacy spd asset: the text/plain `.spd` files import used to stage
/// under `Spectra/` and reference them with `spectrum` parameters.
fn is_legacy_spd(asset: &Asset) -> bool {
    Path::new(&asset.location)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("spd"))
        && asset.media_type.eq_ignore_ascii_case("text/plain")
}

/// while reading a project, `spectrum` parameters that reference
/// legacy spd assets become SpectrumData references (one shared resource
/// per file), the converted assets leave the document so the next save
/// prunes their files, and files that cannot be read stay behind with a
/// warning diagnostic on the import record that owns them. The transform
/// is a read: the project's format and saved revision do not move, so an
/// unconverted disk state converts the same way again next open.
pub(crate) fn convert_legacy_spd_references(
    document: &mut ProjectDocument,
    read: impl Fn(&Asset) -> Result<Vec<u8>>,
) -> bool {
    let candidates: BTreeMap<AssetId, Asset> = document
        .assets
        .values()
        .filter(|asset| is_legacy_spd(asset))
        .map(|asset| (asset.id, asset.clone()))
        .collect();
    // Only files some spectrum parameter references ever convert.
    let mut referenced = BTreeSet::new();
    each_parameter(document, &mut |parameter| {
        if is_spectrum_assets(parameter) {
            for id in asset_ids(parameter) {
                if candidates.contains_key(id) {
                    referenced.insert(*id);
                }
            }
        }
    });
    if referenced.is_empty() {
        return false;
    }
    let mut parsed = BTreeMap::new();
    for id in &referenced {
        let asset = &candidates[id];
        let result = read(asset).and_then(|bytes| {
            verify(asset, &bytes)?;
            parse_spd(&bytes).map_err(invalid)
        });
        parsed.insert(*id, result.map_err(|error| error.to_string()));
    }
    // One resource per convertible file, placed by the shared rules.
    let mut resources = BTreeMap::new();
    let mut namer = crate::layout::LocationNamer::new();
    for location in document
        .scenes
        .values()
        .map(|scene| scene.location.as_str())
        .chain(
            document
                .resources
                .values()
                .map(|resource| resource.location.as_str()),
        )
        .chain(
            document
                .assets
                .values()
                .map(|asset| asset.location.as_str()),
        )
    {
        namer.reserve(location);
    }
    for (id, result) in &parsed {
        let Ok(samples) = result else {
            continue;
        };
        let asset = &candidates[id];
        let label = Path::new(&asset.location)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("spectrum")
            .to_owned();
        let parent = Path::new(&asset.location)
            .parent()
            .and_then(|parent| parent.as_os_str().to_str())
            .unwrap_or_default()
            .to_owned();
        let resource_id = ResourceId::new();
        let floats = |values: &[f64]| {
            ParameterValues::Floats(
                values
                    .iter()
                    .map(|value| FloatValue::Finite(*value))
                    .collect(),
            )
        };
        document.resources.insert(
            resource_id,
            Resource {
                id: resource_id,
                label: label.clone(),
                location: namer.take(
                    &parent,
                    &label,
                    crate::layout::kind_extension(ResourceKind::SpectrumData),
                ),
                edition: Revision::initial(),
                kind: ResourceKind::SpectrumData,
                texture_value_type: None,
                implementation: "piecewise".into(),
                parameters: vec![
                    Parameter {
                        value_type: "float".into(),
                        name: "wavelengths".into(),
                        color_space: ColorSpaceBinding::Inherit,
                        values: floats(&samples.wavelengths),
                    },
                    Parameter {
                        value_type: "float".into(),
                        name: "values".into(),
                        color_space: ColorSpaceBinding::Inherit,
                        values: floats(&samples.values),
                    },
                ],
                definition_scope: None,
                declared_in: None,
                origin: asset
                    .origin
                    .as_ref()
                    .and_then(|origin| origin.origin.clone()),
            },
        );
        resources.insert(*id, resource_id);
    }
    // Convert each parameter whose every spd file parsed; a partial
    // failure keeps the whole parameter on its Asset references.
    each_parameter_mut(document, &mut |parameter| {
        if !is_spectrum_assets(parameter) {
            return;
        }
        let ParameterValues::Assets(ids) = &parameter.values else {
            return;
        };
        if ids.is_empty() {
            return;
        }
        let Some(converted) = ids
            .iter()
            .map(|id| resources.get(id))
            .collect::<Option<Vec<_>>>()
        else {
            return;
        };
        parameter.values = ParameterValues::Resources {
            kind: ResourceKind::SpectrumData,
            ids: converted.into_iter().cloned().collect(),
        };
    });
    // Converted assets nothing else still references leave the document,
    // so the next save prunes them; unreadable files warn on their import.
    let mut changed = !resources.is_empty();
    for (id, result) in parsed {
        let asset = &candidates[&id];
        let Err(reason) = &result else {
            if !still_referenced(document, id) {
                document.assets.remove(&id);
            }
            continue;
        };
        changed |= warn_about(document, &format!("kept {}: {}", asset.location, reason));
    }
    changed
}

/// The bytes on disk are the model's asset: size and SHA-256 must agree
/// before a legacy file becomes a resource.
fn verify(asset: &Asset, bytes: &[u8]) -> Result<()> {
    if bytes.len() as u64 != asset.size || format!("{:x}", Sha256::digest(bytes)) != asset.sha256 {
        return Err(invalid("the file on disk does not match the project model"));
    }
    Ok(())
}

fn is_spectrum_assets(parameter: &Parameter) -> bool {
    parameter.value_type == "spectrum" && matches!(parameter.values, ParameterValues::Assets(_))
}

fn asset_ids(parameter: &Parameter) -> &[AssetId] {
    match &parameter.values {
        ParameterValues::Assets(ids) => ids,
        _ => &[],
    }
}

/// A disk-resident asset keeps no durable link to the import that brought
/// it (the sidecar note survives as its `import` identity only in older
/// writings; occurrence and span are session state). A legacy project is
/// one import, so its record carries the report; anything else is left to
/// the conversion itself, which keeps the file working either way.
fn warn_about(document: &mut ProjectDocument, message: &str) -> bool {
    if document.imports.len() != 1 {
        return false;
    }
    let Some(record) = document.imports.values_mut().next() else {
        return false;
    };
    if record
        .diagnostics
        .iter()
        .any(|known| known.message == message)
    {
        return false;
    }
    record.diagnostics.push(ImportDiagnostic {
        code: "spectrum_migration".into(),
        message: message.to_owned(),
        severity: Some(ImportSeverity::Warning),
        origin: None,
    });
    true
}

/// Whether any parameter still references `asset`, of any type.
fn still_referenced(document: &ProjectDocument, asset: AssetId) -> bool {
    let mut referenced = false;
    each_parameter(document, &mut |parameter| {
        if asset_ids(parameter).contains(&asset)
            || matches!(&parameter.values, ParameterValues::Array { asset: id, .. } if *id == asset)
        {
            referenced = true;
        }
    });
    referenced
}

/// Every mutable parameter table of the document.
fn each_parameter_mut(document: &mut ProjectDocument, visit: &mut impl FnMut(&mut Parameter)) {
    for scene in document.scenes.values_mut() {
        let scene = std::sync::Arc::make_mut(scene);
        for node in scene.nodes.values_mut() {
            match &mut node.data {
                NodeKind::Entity { parameters, .. }
                | NodeKind::Configuration { parameters, .. } => {
                    for parameter in parameters.iter_mut() {
                        visit(parameter);
                    }
                }
                _ => {}
            }
        }
    }
    for node in document.definition_nodes.values_mut() {
        match &mut node.data {
            NodeKind::Entity { parameters, .. } | NodeKind::Configuration { parameters, .. } => {
                for parameter in parameters.iter_mut() {
                    visit(parameter);
                }
            }
            _ => {}
        }
    }
    for resource in document.resources.values_mut() {
        for parameter in resource.parameters.iter_mut() {
            visit(parameter);
        }
    }
}

/// The same walk, read-only.
fn each_parameter(document: &ProjectDocument, visit: &mut impl FnMut(&Parameter)) {
    for scene in document.scenes.values() {
        for node in scene.nodes.values() {
            match &node.data {
                NodeKind::Entity { parameters, .. }
                | NodeKind::Configuration { parameters, .. } => {
                    for parameter in parameters {
                        visit(parameter);
                    }
                }
                _ => {}
            }
        }
    }
    for node in document.definition_nodes.values() {
        match &node.data {
            NodeKind::Entity { parameters, .. } | NodeKind::Configuration { parameters, .. } => {
                for parameter in parameters {
                    visit(parameter);
                }
            }
            _ => {}
        }
    }
    for resource in document.resources.values() {
        for parameter in &resource.parameters {
            visit(parameter);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spd_parsing_follows_r4_and_rejects_what_the_plan_rejects() {
        let good = parse_spd(b"298.757050 1.920375\n# a comment 5 5\n\n302.400421 1.92\n").unwrap();
        assert_eq!(good.wavelengths, [298.757050, 302.400421]);
        assert_eq!(good.values, [1.920375, 1.92]);
        // Duplicate wavelengths are not descending: r4 dedups them, the
        // only rejects descending order.
        parse_spd(b"400 1 400 2\n").unwrap();
        assert!(parse_spd(b"400 1 500 2\x80\x80\n").is_err(), "non-UTF-8");
        assert!(parse_spd(b"400 abc\n").unwrap_err().contains("non-numeric"));
        assert!(parse_spd(b"400 inf\n").unwrap_err().contains("non-finite"));
        assert!(parse_spd(b"400 1 500\n").unwrap_err().contains("odd"));
        assert!(
            parse_spd(b"500 1 400 2\n")
                .unwrap_err()
                .contains("descending")
        );
        assert!(parse_spd(b"0 1\n").unwrap_err().contains("positive"));
        assert!(parse_spd(b"# only\n").unwrap_err().contains("no samples"));
        let mut giant = Vec::new();
        for index in 0..=crate::MAX_SPECTRUM_SAMPLES {
            giant.extend_from_slice(format!("{} 1\n", 400. + index as f64).as_bytes());
        }
        assert!(parse_spd(&giant).unwrap_err().contains("sample pairs"));
    }
}
