//! A material resource, its mix inputs, and their textures as
//! the approximate sphere preview `project.preview_material` caches.
//! Parameters resolve as texture previews do (the declaration, else the
//! table's default); materials the table does not cover read their
//! declarations with r4's defaults written here.
use super::texture_preview::{
    Builder, MAX_DEPTH, MAX_NODES, Parameters, builder, cache_key, encode_png, expanded,
};
use project::{ParameterValues, ProjectDocument, Resource, ResourceId, ResourceKind};
use scene_parse::material_preview::{Material, Roughness, Tint, thumbnail};
use scene_parse::texture_eval::{Operand, spectra};
use std::sync::Arc;

/// Bumped whenever the shading, the defaults it reads or the PNG change.
/// glass-name etas and names that are not refractive
/// indices now shade their own colours (4).
const PREVIEW_VERSION: &str = "material-preview 4";

/// r4's conductor defaults as the old per-channel operands kept them
/// (the copper channel samples), fallbacks for a texture-connected side
/// that shades per hit.
fn copper() -> (Operand, Operand) {
    let named = |name| Operand::Constant(spectra::named(name).unwrap_or([1.; 3]));
    (named("metal-Cu-eta"), named("metal-Cu-k"))
}

/// r4's conductor default as an F0 colour: copper's own curves, in a
/// medium or out.
fn copper_f0(medium: f64) -> [f64; 3] {
    spectrum::conductor_f0(
        spectrum::Value::Named("metal-Cu-eta"),
        spectrum::Value::Named("metal-Cu-k"),
        medium,
    )
    .unwrap_or([0.5; 3])
}

/// One metal side's own value, held owned so a Spectrum Data
/// reference can bring its curve's samples: the shape the spectrum crate
/// reads. Textures keep their own per-hit path, not this one.
enum MetalSide {
    Rgb([f64; 3]),
    Constant(f64),
    Named(String),
    Pairs(Vec<f64>),
}

impl MetalSide {
    fn value(&self) -> spectrum::Value<'_> {
        match self {
            Self::Rgb(rgb) => spectrum::Value::Rgb(*rgb),
            Self::Constant(value) => spectrum::Value::Constant(*value),
            Self::Named(name) => spectrum::Value::Named(name),
            Self::Pairs(pairs) => spectrum::Value::Pairs(pairs),
        }
    }
}

impl Builder<'_> {
    pub(super) fn material(
        &mut self,
        id: ResourceId,
        depth: usize,
    ) -> Result<Arc<Material>, String> {
        if depth > MAX_DEPTH {
            return Err("material chain is too deep".into());
        }
        if let Some(material) = self.materials.get(&id) {
            return Ok(material.clone());
        }
        let resource = self
            .document
            .resources
            .get(&id)
            .filter(|resource| resource.kind == ResourceKind::Material)
            .ok_or("not a material")?;
        self.inputs.push(format!(
            "material {} {}",
            id.0,
            serde_json::json!({
                "implementation": resource.implementation,
                "parameters": resource.parameters,
            })
        ));
        let material = Arc::new(self.build_material(resource, depth)?);
        self.materials.insert(id, material.clone());
        Ok(material)
    }

    /// `<prefix>roughness`, or `<prefix>uroughness` when declared (the
    /// preview's lobes are isotropic), with `remaproughness`.
    fn roughness(
        &mut self,
        p: &Parameters,
        prefix: &str,
        depth: usize,
    ) -> Result<Roughness, String> {
        let anisotropic = format!("{prefix}uroughness");
        let name = if p.declared(&anisotropic).is_some() {
            anisotropic
        } else {
            format!("{prefix}roughness")
        };
        Ok(Roughness {
            value: self.operand(p, &name, [0.; 3], depth)?,
            remap: p.declared("remaproughness").is_none() || p.boolean("remaproughness"),
        })
    }

    /// A scalar IOR: a float, or a spectrum read at the green wavelength.
    /// Textures and invalid values fall back to 1.5, as r4 warns and does.
    fn eta(&mut self, p: &Parameters, name: &str, depth: usize) -> Result<f32, String> {
        Ok(match self.operand(p, name, [1.5; 3], depth)? {
            Operand::Constant(value) if value[1] > 0. => value[1],
            _ => 1.5,
        })
    }

    /// A conductor's tint is its metal colour, matching the F0 Lit
    /// and the Metal colour row show — eta/k curves (inline λ/value
    /// pairs, builtin names, a Spectrum Data reference's samples, which
    /// the old per-channel read answered with copper) resolve through
    /// the spectrum crate, a missing side fills with r4's copper, and a
    /// declared reflectance keeps its own colour (r4 refuses the
    /// reflectance-beside-eta/k pair; like Lit, the reflectance draws).
    /// A name that is not a refractive index, such as an illuminant, shades the row's neutral grey. A texture-connected side
    /// cannot resolve to a colour: it keeps the per-channel operands,
    /// shading per hit (the copper channel samples as the other side's
    /// fallback). `medium` is the index around the metal (the coat's
    /// interface eta for a coatedconductor).
    fn tint(
        &mut self,
        p: &Parameters,
        prefix: &str,
        medium: f32,
        depth: usize,
    ) -> Result<Tint, String> {
        let (eta_name, k_name) = (format!("{prefix}eta"), format!("{prefix}k"));
        if p.texture("reflectance").is_some() {
            return Ok(Tint::Reflectance(self.operand(
                p,
                "reflectance",
                [0.5; 3],
                depth,
            )?));
        }
        if p.texture(&eta_name).is_some() || p.texture(&k_name).is_some() {
            let (eta, k) = copper();
            let fallback = |operand: &Operand| match operand {
                Operand::Constant(value) => *value,
                Operand::Texture(_) => [1.; 3],
            };
            return Ok(Tint::Fresnel {
                eta: self.operand(p, &eta_name, fallback(&eta), depth)?,
                k: self.operand(p, &k_name, fallback(&k), depth)?,
            });
        }
        let medium = medium as f64;
        let colour = |rgb: [f64; 3]| Operand::Constant(rgb.map(|v| v as f32));
        if let Some(reflectance) = self.metal_side(p, "reflectance")? {
            return match spectrum::reflectance_f0(reflectance.value(), medium) {
                Some(rgb) => Ok(Tint::Reflectance(colour(rgb))),
                None => Ok(Tint::Reflectance(colour(copper_f0(medium)))),
            };
        }
        let eta = self.metal_side(p, &eta_name)?;
        let k = self.metal_side(p, &k_name)?;
        // a name with neither a curve nor channel samples
        // is not a refractive index (an illuminant, a typo): the neutral
        // grey — the same grey the Metal colour row answers, so the
        // sphere and the row agree. Lit shades it too.
        if [eta.as_ref(), k.as_ref()]
            .into_iter()
            .flatten()
            .any(|side| {
                matches!(side, MetalSide::Named(name)
                    if spectrum::named_is_not_a_refractive_index(name))
            })
        {
            return Ok(Tint::Reflectance(colour([0.5; 3])));
        }
        let f0 = spectrum::conductor_f0(
            eta.unwrap_or(MetalSide::Named("metal-Cu-eta".into()))
                .value(),
            k.unwrap_or(MetalSide::Named("metal-Cu-k".into())).value(),
            medium,
        )
        // Curves the crate cannot read (an odd pair count) shade as
        // r4's own default copper.
        .unwrap_or_else(|| copper_f0(medium));
        Ok(Tint::Reflectance(colour(f0)))
    }

    /// One metal side's declared value, or None when it is undeclared or
    /// a shape no curve reads (a texture rides per hit instead). A
    /// Spectrum Data reference reads its sample tables and records the
    /// curve's content in the cache key's inputs, so an edited curve
    /// re-shades the thumbnail.
    fn metal_side(&mut self, p: &Parameters, name: &str) -> Result<Option<MetalSide>, String> {
        let Some(parameter) = p.declared(name) else {
            return Ok(None);
        };
        let side = match &parameter.values {
            ParameterValues::BuiltinSpectra(names) | ParameterValues::Strings(names) => {
                names.first().map(|name| MetalSide::Named(name.clone()))
            }
            ParameterValues::Floats(values) => {
                let floats = values
                    .iter()
                    .map(|value| match value {
                        project::FloatValue::Finite(number) if number.is_finite() => Ok(*number),
                        _ => Err(format!("{name} holds non-finite samples")),
                    })
                    .collect::<Result<Vec<f64>, String>>()?;
                match parameter.value_type.as_str() {
                    "rgb" => floats.first().map(|first| {
                        let second = floats.get(1).copied().unwrap_or(*first);
                        MetalSide::Rgb([*first, second, floats.get(2).copied().unwrap_or(second)])
                    }),
                    "float" => floats.first().copied().map(MetalSide::Constant),
                    "spectrum" if floats.len().is_multiple_of(2) && !floats.is_empty() => {
                        Some(MetalSide::Pairs(floats))
                    }
                    _ => None,
                }
            }
            ParameterValues::Resources {
                kind: ResourceKind::SpectrumData,
                ids,
            } => match ids.first().and_then(|id| self.document.resources.get(id)) {
                Some(resource) => {
                    let (curve, remark) = self.spectrum_curve(resource)?;
                    self.inputs.push(remark);
                    Some(MetalSide::Pairs(curve))
                }
                None => None,
            },
            _ => None,
        };
        Ok(side)
    }

    /// A Spectrum Data reference's samples and cache key: the curve is read through the snapshot's staged assets — a
    /// lease's payload, the project folder's file — or inline floats
    /// straight from the tables. `.meta` sample tables never read a
    /// file; a `.array` table's content lands in the remark's SHA-256,
    /// so the key follows it.
    fn spectrum_curve(&mut self, resource: &Resource) -> Result<(Vec<f64>, String), String> {
        let read = |asset: project::AssetId| -> pbrt_io::Result<std::path::PathBuf> {
            match self.snapshot {
                Some(snapshot) => {
                    if let Some(lease) = snapshot.payload_by(asset) {
                        return Ok(lease.path().to_path_buf());
                    }
                    let root = self
                        .asset_root
                        .ok_or_else(|| pbrt_io::Error::Invalid("no asset folder".into()))?;
                    snapshot
                        .folder_asset_path(std::path::Path::new(root), asset)
                        .map_err(pbrt_io::Error::from)
                }
                None => {
                    let root = self
                        .asset_root
                        .ok_or_else(|| pbrt_io::Error::Invalid("no asset folder".into()))?;
                    let asset = self
                        .document
                        .assets
                        .get(&asset)
                        .ok_or_else(|| pbrt_io::Error::Invalid("missing asset".into()))?;
                    Ok(std::path::Path::new(root).join(&asset.location))
                }
            }
        };
        let (wavelengths, values) =
            pbrt_io::spectrum_samples(resource, read).map_err(|error| error.to_string())?;
        let mut pairs = Vec::with_capacity(wavelengths.len() * 2);
        let mut hasher = sha2::Sha256::new();
        use sha2::Digest;
        for (wavelength, value) in wavelengths.into_iter().zip(values) {
            hasher.update(wavelength.to_le_bytes());
            hasher.update(value.to_le_bytes());
            pairs.push(wavelength);
            pairs.push(value);
        }
        Ok((
            pairs,
            format!("spectrum {} {:x}", resource.label, hasher.finalize()),
        ))
    }

    fn build_material(&mut self, resource: &Resource, depth: usize) -> Result<Material, String> {
        let rows = pbrt_io::parameter_schema::schema("Material")
            .and_then(|table| {
                pbrt_io::parameter_schema::implementation(table, &resource.implementation)
            })
            .and_then(|found| found.parameters(None))
            .unwrap_or(&[]);
        let parameters = Parameters { resource, rows };
        let p = &parameters;
        Ok(match resource.implementation.as_str() {
            "diffuse" => Material::Diffuse {
                reflectance: self.operand(p, "reflectance", [0.5; 3], depth)?,
            },
            "coateddiffuse" => Material::CoatedDiffuse {
                reflectance: self.operand(p, "reflectance", [0.5; 3], depth)?,
                roughness: self.roughness(p, "", depth)?,
                eta: self.eta(p, "eta", depth)?,
            },
            "conductor" => Material::Conductor {
                tint: self.tint(p, "", 1., depth)?,
                roughness: self.roughness(p, "", depth)?,
            },
            "coatedconductor" => {
                let interface_eta = self.eta(p, "interface.eta", depth)?;
                Material::CoatedConductor {
                    interface_roughness: self.roughness(p, "interface.", depth)?,
                    interface_eta,
                    tint: self.tint(p, "conductor.", interface_eta, depth)?,
                    roughness: self.roughness(p, "conductor.", depth)?,
                }
            }
            "dielectric" => Material::Dielectric {
                eta: self.eta(p, "eta", depth)?,
                roughness: self.roughness(p, "", depth)?,
                thin: false,
            },
            // r4's thin dielectric is always smooth.
            "thindielectric" => Material::Dielectric {
                eta: self.eta(p, "eta", depth)?,
                roughness: Roughness::smooth(),
                thin: true,
            },
            "diffusetransmission" => Material::DiffuseTransmission {
                reflectance: self.operand(p, "reflectance", [0.25; 3], depth)?,
                transmittance: self.operand(p, "transmittance", [0.25; 3], depth)?,
                scale: p.float("scale", 1.),
            },
            "mix" => {
                let ids = match p.declared("materials").map(|p| p.values) {
                    Some(ParameterValues::Resources {
                        kind: ResourceKind::Material,
                        ids,
                    }) if ids.len() == 2 => ids,
                    _ => return Err("mix needs two materials".into()),
                };
                Material::Mix {
                    materials: [
                        self.material(ids[0], depth + 1)?,
                        self.material(ids[1], depth + 1)?,
                    ],
                    amount: self.operand(p, "amount", [0.5; 3], depth)?,
                }
            }
            "interface" => Material::Interface,
            // Not modelled: a neutral Lambert stand-in.
            "measured" | "hair" | "subsurface" => Material::Fallback([0.5; 3]),
            other => return Err(format!("no preview for {other} materials")),
        })
    }
}

/// Whether `material` (with its textures) expands to at most `budget`
/// nodes.
fn material_expanded(material: &Material, budget: &mut usize) -> bool {
    if *budget == 0 {
        return false;
    }
    *budget -= 1;
    let operand = |operand: &Operand, budget: &mut usize| match operand {
        Operand::Texture(texture) => expanded(texture, budget),
        Operand::Constant(_) => true,
    };
    let tint = |tint: &Tint, budget: &mut usize| match tint {
        Tint::Fresnel { eta, k } => operand(eta, budget) && operand(k, budget),
        Tint::Reflectance(reflectance) => operand(reflectance, budget),
    };
    match material {
        Material::Diffuse { reflectance } => operand(reflectance, budget),
        Material::CoatedDiffuse {
            reflectance,
            roughness,
            ..
        } => operand(reflectance, budget) && operand(&roughness.value, budget),
        Material::Conductor { tint: t, roughness } => {
            tint(t, budget) && operand(&roughness.value, budget)
        }
        Material::CoatedConductor {
            interface_roughness,
            tint: t,
            roughness,
            ..
        } => {
            operand(&interface_roughness.value, budget)
                && tint(t, budget)
                && operand(&roughness.value, budget)
        }
        Material::Dielectric { roughness, .. } => operand(&roughness.value, budget),
        Material::DiffuseTransmission {
            reflectance,
            transmittance,
            ..
        } => operand(reflectance, budget) && operand(transmittance, budget),
        Material::Mix { materials, amount } => {
            material_expanded(&materials[0], budget)
                && material_expanded(&materials[1], budget)
                && operand(amount, budget)
        }
        Material::Interface | Material::Fallback(_) => true,
    }
}

/// The cache key of `material`'s preview at `size`, without decoding any
/// image: it changes whenever the material, a mix input, an upstream
/// texture, an image, a referenced Spectrum Data curve or the shading
/// version changes. `snapshot` rides along so a lease-staged data asset
/// reads the same bytes the shading pass reads (None for the tests'
/// bare documents).
pub(super) fn key(
    document: &ProjectDocument,
    snapshot: Option<&project::ProjectSnapshot>,
    material: ResourceId,
    asset_root: Option<&str>,
    size: u32,
) -> Result<String, String> {
    let mut builder = builder(document, asset_root, false);
    builder.snapshot = snapshot;
    let tree = builder.material(material, 0)?;
    if !material_expanded(&tree, &mut MAX_NODES.clone()) {
        return Err("the material graph is too large to preview".into());
    }
    Ok(cache_key(
        "material",
        PREVIEW_VERSION,
        size,
        &builder.inputs,
    ))
}

/// The material's tree, images decoded (a cache miss).
#[cfg(test)]
pub(super) fn tree(
    document: &ProjectDocument,
    material: ResourceId,
    asset_root: Option<&str>,
) -> Result<Arc<Material>, String> {
    builder(document, asset_root, true).material(material, 0)
}

pub(super) fn tree_from_snapshot<'a>(
    snapshot: &'a project::ProjectSnapshot,
    material: ResourceId,
    asset_root: Option<&'a str>,
) -> Result<Arc<Material>, String> {
    let mut builder = builder(snapshot.document(), asset_root, true);
    builder.snapshot = Some(snapshot);
    builder.material(material, 0)
}

/// The preview as PNG bytes.
pub(super) fn png(tree: &Material, size: u32) -> Result<Vec<u8>, String> {
    encode_png(thumbnail(tree, size), size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use project::{ColorSpaceBinding, FloatValue, Parameter, Revision};

    fn resource(
        document: &mut ProjectDocument,
        kind: ResourceKind,
        implementation: &str,
        parameters: Vec<Parameter>,
    ) -> ResourceId {
        let resource = Resource {
            id: ResourceId::new(),
            label: implementation.into(),
            location: format!("Materials/{implementation}.material"),
            edition: Revision::initial(),
            kind,
            texture_value_type: (kind == ResourceKind::Texture)
                .then_some(project::TextureValueType::Spectrum),
            implementation: implementation.into(),
            parameters,
            definition_scope: None,
            declared_in: None,
            origin: None,
        };
        let id = resource.id;
        document.resources.insert(id, resource);
        id
    }

    fn parameter(name: &str, value_type: &str, values: ParameterValues) -> Parameter {
        Parameter {
            name: name.into(),
            value_type: value_type.into(),
            color_space: ColorSpaceBinding::Inherit,
            values,
        }
    }

    fn floats(values: &[f64]) -> ParameterValues {
        ParameterValues::Floats(values.iter().map(|v| FloatValue::Finite(*v)).collect())
    }

    /// The fixtures' Au spd curves as interleaved λ/value pairs — the
    /// samples a Spectrum Data reference to them holds.
    fn read_au_spd(name: &str) -> Vec<f64> {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/spectra")
                .join(name),
        )
        .unwrap()
        .lines()
        .filter_map(|line| line.split_once(' '))
        .flat_map(|(a, b)| [a.parse::<f64>().unwrap(), b.trim().parse::<f64>().unwrap()])
        .collect()
    }

    fn build(document: &ProjectDocument, id: ResourceId) -> Material {
        (*tree(document, id, None).unwrap()).clone()
    }

    #[test]
    fn parameters_resolve_with_r4_defaults_and_spectra() {
        let mut document = ProjectDocument::new("Materials");
        // A bare conductor is r4's copper — now as its F0 colour, the
        // number the Metal colour row shows, not the channel operands.
        let bare = resource(&mut document, ResourceKind::Material, "conductor", vec![]);
        let Material::Conductor { tint, roughness } = build(&document, bare) else {
            panic!();
        };
        let copper = spectrum::conductor_f0(
            spectrum::Value::Named("metal-Cu-eta"),
            spectrum::Value::Named("metal-Cu-k"),
            1.,
        )
        .unwrap()
        .map(|v| v as f32);
        assert_eq!(
            tint,
            Tint::Reflectance(Operand::Constant(copper)),
            "r4 fills copper, {tint:?}"
        );
        assert!(roughness.remap, "remaproughness defaults to true");
        // A declared builtin eta joins r4's copper k — the mix, not
        // eta's metal alone.
        let gold = resource(
            &mut document,
            ResourceKind::Material,
            "conductor",
            vec![parameter(
                "eta",
                "spectrum",
                ParameterValues::BuiltinSpectra(vec!["metal-Au-eta".into()]),
            )],
        );
        let Material::Conductor { tint, .. } = build(&document, gold) else {
            panic!();
        };
        let mixed = spectrum::conductor_f0(
            spectrum::Value::Named("metal-Au-eta"),
            spectrum::Value::Named("metal-Cu-k"),
            1.,
        )
        .unwrap()
        .map(|v| v as f32);
        assert_eq!(
            tint,
            Tint::Reflectance(Operand::Constant(mixed)),
            "{tint:?}"
        );
        let tinted = resource(
            &mut document,
            ResourceKind::Material,
            "conductor",
            vec![parameter("reflectance", "rgb", floats(&[0.9, 0.5, 0.2]))],
        );
        assert!(matches!(
            build(&document, tinted),
            Material::Conductor {
                tint: Tint::Reflectance(Operand::Constant([0.9, 0.5, 0.2])),
                ..
            }
        ));
        // Glass reads a named IOR at the green wavelength.
        let glass = resource(
            &mut document,
            ResourceKind::Material,
            "dielectric",
            vec![parameter(
                "eta",
                "spectrum",
                ParameterValues::BuiltinSpectra(vec!["glass-BK7".into()]),
            )],
        );
        assert!(matches!(
            build(&document, glass),
            Material::Dielectric { eta, thin: false, .. }
                if (eta - spectrum::named("glass-BK7").unwrap()[1] as f32).abs() < 1e-6
        ));
    }

    /// An eta/k pair referenced through Spectrum Data draws the metal with its
    /// curves name — the F0 of the shared curves, where the thumbnail
    /// used to fall back to copper for every curve (and inline pairs
    /// read as their first numbers).
    #[test]
    fn conductor_spectrum_data_shades_its_own_metal_not_copper() {
        let mut document = ProjectDocument::new("Materials");
        // Au's spd curves as Spectrum Data resources' sample tables.
        let eta_pairs = read_au_spd("Au.eta.spd");
        let k_pairs = read_au_spd("Au.k.spd");
        let curve_resource = |document: &mut ProjectDocument, pairs: &[f64]| {
            resource(
                document,
                ResourceKind::SpectrumData,
                "piecewise",
                vec![
                    parameter(
                        "wavelengths",
                        "float",
                        floats(&pairs.iter().step_by(2).copied().collect::<Vec<_>>()),
                    ),
                    parameter(
                        "values",
                        "float",
                        floats(&pairs.iter().skip(1).step_by(2).copied().collect::<Vec<_>>()),
                    ),
                ],
            )
        };
        let eta_curve = curve_resource(&mut document, &eta_pairs);
        let k_curve = curve_resource(&mut document, &k_pairs);
        let gold = resource(
            &mut document,
            ResourceKind::Material,
            "conductor",
            vec![
                parameter(
                    "eta",
                    "spectrum",
                    ParameterValues::Resources {
                        kind: ResourceKind::SpectrumData,
                        ids: vec![eta_curve],
                    },
                ),
                parameter(
                    "k",
                    "spectrum",
                    ParameterValues::Resources {
                        kind: ResourceKind::SpectrumData,
                        ids: vec![k_curve],
                    },
                ),
            ],
        );
        let Material::Conductor { tint, .. } = build(&document, gold) else {
            panic!();
        };
        let au = spectrum::conductor_f0(
            spectrum::Value::Pairs(&eta_pairs),
            spectrum::Value::Pairs(&k_pairs),
            1.,
        )
        .unwrap()
        .map(|v| v as f32);
        let copper = spectrum::conductor_f0(
            spectrum::Value::Named("metal-Cu-eta"),
            spectrum::Value::Named("metal-Cu-k"),
            1.,
        )
        .unwrap()
        .map(|v| v as f32);
        assert_ne!(au, copper, "Au is not copper");
        let Tint::Reflectance(Operand::Constant(got)) = tint else {
            panic!("the curve-sourced metal shades as its colour: {tint:?}");
        };
        for (got, want) in got.into_iter().zip(au) {
            assert!((got - want).abs() < 1e-3, "{got} vs {want}");
        }
    }

    /// the thumbnail shows what the Metal colour row
    /// shows for a non-metal named eta — a glass reads its channel
    /// samples, an illuminant is no refractive index and shades the
    /// row's neutral grey, copper never.
    #[test]
    fn a_glass_eta_shades_like_the_row_and_illuminants_go_grey() {
        let mut document = ProjectDocument::new("Materials");
        let glass = resource(
            &mut document,
            ResourceKind::Material,
            "conductor",
            vec![parameter(
                "eta",
                "spectrum",
                ParameterValues::BuiltinSpectra(vec!["glass-BK7".into()]),
            )],
        );
        let Material::Conductor { tint, .. } = build(&document, glass) else {
            panic!();
        };
        let expected = spectrum::conductor_f0(
            spectrum::Value::Named("glass-BK7"),
            spectrum::Value::Named("metal-Cu-k"),
            1.,
        )
        .unwrap()
        .map(|v| v as f32);
        let Tint::Reflectance(Operand::Constant(got)) = tint else {
            panic!("the glass eta shades its channels: {tint:?}");
        };
        for (got, want) in got.into_iter().zip(expected) {
            assert!((got - want).abs() < 1e-6, "{got} vs {want}");
        }
        // An illuminant is not a refractive index: the row answers the
        // neutral grey, and so does the sphere.
        let warm = resource(
            &mut document,
            ResourceKind::Material,
            "conductor",
            vec![parameter(
                "eta",
                "spectrum",
                ParameterValues::BuiltinSpectra(vec!["stdillum-D65".into()]),
            )],
        );
        let Material::Conductor { tint, .. } = build(&document, warm) else {
            panic!();
        };
        assert_eq!(
            tint,
            Tint::Reflectance(Operand::Constant([0.5; 3])),
            "{tint:?}"
        );
    }

    /// A coated conductor's F0 divides by the interface index,
    /// the coat table's answer under 1.5.
    #[test]
    fn coatedconductor_shades_under_the_interface_index() {
        let mut document = ProjectDocument::new("Materials");
        let coated = resource(
            &mut document,
            ResourceKind::Material,
            "coatedconductor",
            vec![
                parameter(
                    "conductor.eta",
                    "spectrum",
                    ParameterValues::BuiltinSpectra(vec!["metal-Au-eta".into()]),
                ),
                parameter(
                    "conductor.k",
                    "spectrum",
                    ParameterValues::BuiltinSpectra(vec!["metal-Au-k".into()]),
                ),
            ],
        );
        let Material::CoatedConductor {
            tint,
            interface_eta,
            ..
        } = build(&document, coated)
        else {
            panic!();
        };
        assert_eq!(interface_eta, 1.5);
        let au_coat = spectrum::conductor_f0(
            spectrum::Value::Named("metal-Au-eta"),
            spectrum::Value::Named("metal-Au-k"),
            1.5,
        )
        .unwrap()
        .map(|v| v as f32);
        let Tint::Reflectance(Operand::Constant(got)) = tint else {
            panic!("{tint:?}");
        };
        for (got, want) in got.into_iter().zip(au_coat) {
            assert!((got - want).abs() < 1e-3, "{got} vs {want}");
        }
    }

    #[test]
    fn mix_reads_both_materials_and_the_key_follows_them() {
        let mut document = ProjectDocument::new("Mix");
        let red = resource(
            &mut document,
            ResourceKind::Material,
            "diffuse",
            vec![parameter("reflectance", "rgb", floats(&[0.8, 0.1, 0.1]))],
        );
        let glass = resource(&mut document, ResourceKind::Material, "dielectric", vec![]);
        let fbm = resource(&mut document, ResourceKind::Texture, "fbm", vec![]);
        let mix = resource(
            &mut document,
            ResourceKind::Material,
            "mix",
            vec![
                parameter(
                    "materials",
                    "string",
                    ParameterValues::Resources {
                        kind: ResourceKind::Material,
                        ids: vec![red, glass],
                    },
                ),
                parameter(
                    "amount",
                    "texture",
                    ParameterValues::Resources {
                        kind: ResourceKind::Texture,
                        ids: vec![fbm],
                    },
                ),
            ],
        );
        let Material::Mix { materials, amount } = build(&document, mix) else {
            panic!();
        };
        assert!(matches!(*materials[0], Material::Diffuse { .. }));
        assert!(matches!(*materials[1], Material::Dielectric { .. }));
        assert!(matches!(amount, Operand::Texture(_)));
        // Editing an input material, or the texture, changes the key.
        let first = key(&document, None, mix, None, 64).unwrap();
        assert!(first.starts_with("material-"), "{first}");
        document.resources.get_mut(&red).unwrap().parameters =
            vec![parameter("reflectance", "rgb", floats(&[0.1, 0.8, 0.1]))];
        let second = key(&document, None, mix, None, 64).unwrap();
        assert_ne!(first, second);
        document.resources.get_mut(&fbm).unwrap().parameters =
            vec![parameter("roughness", "float", floats(&[0.7]))];
        assert_ne!(key(&document, None, mix, None, 64).unwrap(), second);
    }

    #[test]
    fn unmodelled_materials_get_a_stand_in_and_textures_are_refused() {
        let mut document = ProjectDocument::new("Others");
        let hair = resource(&mut document, ResourceKind::Material, "hair", vec![]);
        assert_eq!(build(&document, hair), Material::Fallback([0.5; 3]));
        let thin = resource(
            &mut document,
            ResourceKind::Material,
            "thindielectric",
            vec![],
        );
        assert!(matches!(
            build(&document, thin),
            Material::Dielectric { thin: true, .. }
        ));
        let texture = resource(&mut document, ResourceKind::Texture, "fbm", vec![]);
        assert_eq!(
            tree(&document, texture, None).unwrap_err(),
            "not a material"
        );
        let broken = resource(&mut document, ResourceKind::Material, "mix", vec![]);
        assert!(tree(&document, broken, None).is_err());
    }

    #[test]
    fn previews_are_pngs() {
        let mut document = ProjectDocument::new("Png");
        let id = resource(
            &mut document,
            ResourceKind::Material,
            "coateddiffuse",
            vec![],
        );
        let png = png(&build(&document, id), 32).unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }
}
