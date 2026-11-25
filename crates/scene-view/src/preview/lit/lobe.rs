use super::KIND_LAMBERT;
use protocol::geometry::{MaterialRecord, PreviewParameter};

pub(crate) const MIN_ALPHA: f32 = 0.00001;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct GpuLobe {
    pub diffuse: [f32; 3],
    pub kind: u32,
    pub spec_f0: [f32; 3],
    pub spec_alpha: f32,
    pub spec_f90: [f32; 3],
    pub transmission: f32,
    pub coat: [f32; 4],
    pub texture: [f32; 4],
    /// Packed texture programs (0 = none): reflectance, spec u, spec v, coat u.
    pub programs_a: [u32; 4],
    /// Coat v, mix amount, then a mix's two child lobes (indices, not programs).
    pub programs_b: [u32; 4],
    /// x: `LOBE_*` bits.
    pub flags: [u32; 4],
}

/// The reflectance program gives a metal's F0 rather than the diffuse color.
pub(crate) const LOBE_REFLECTANCE_IS_F0: u32 = 1;
/// Roughness programs are remapped to alpha with `sqrt`.
pub(crate) const LOBE_REMAP_ROUGHNESS: u32 = 2;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct LitNote {
    pub reason: String,
    pub material: Option<String>,
    pub draws: usize,
}

pub(crate) fn dielectric_f0(eta: f64) -> f64 {
    ((eta - 1.0) / (eta + 1.0)).powi(2)
}

/// pbrt-v3 material names that r4 renders only after `--upgrade`, which
/// rewrites them to v4 materials (r4 `parser/upgrade.rs::upgrade_material`).
/// Lit shows the upgraded material, as r4 would after the upgrade.
pub(crate) fn upgrade_v3(record: &MaterialRecord) -> Option<MaterialRecord> {
    let name = record.implementation.to_ascii_lowercase();
    let mut out = record.clone();
    let rename = |out: &mut MaterialRecord, from: &str, to: &str| {
        for parameter in &mut out.parameters {
            if parameter.name == from {
                parameter.name = to.into();
            }
        }
    };
    let remove = |out: &mut MaterialRecord, names: &[&str]| {
        out.parameters.retain(|p| !names.contains(&p.name.as_str()));
    };
    let ks_is_zero = rgb_of(&record.parameters, "Ks", [0.25; 3]) == [0.; 3];
    if name == "glass" || name == "uber" {
        rename(&mut out, "index", "eta");
    }
    rename(&mut out, "bumpmap", "displacement");
    out.implementation = match name.as_str() {
        "matte" => {
            rename(&mut out, "Kd", "reflectance");
            remove(&mut out, &["sigma"]);
            "diffuse"
        }
        "kdsubsurface" => {
            rename(&mut out, "Kd", "reflectance");
            "subsurface"
        }
        "disney" => {
            rename(&mut out, "color", "reflectance");
            "diffuse"
        }
        "substrate" => {
            remove(&mut out, &["Ks"]);
            rename(&mut out, "Kd", "reflectance");
            "coateddiffuse"
        }
        "plastic" | "uber" => {
            remove(&mut out, &["Ks", "Kr", "Kt"]);
            rename(&mut out, "Kd", "reflectance");
            if ks_is_zero {
                remove(&mut out, &["roughness", "eta"]);
                "diffuse"
            } else {
                "coateddiffuse"
            }
        }
        "glass" => {
            remove(&mut out, &["Kr", "Kt"]);
            "dielectric"
        }
        "metal" => {
            remove(&mut out, &["Kr"]);
            "conductor"
        }
        "translucent" => {
            rename(&mut out, "Kd", "transmittance");
            remove(&mut out, &["reflect", "transmit", "Ks", "roughness"]);
            "diffusetransmission"
        }
        "mirror" => {
            remove(&mut out, &["Kr"]);
            out.parameters.push(text_parameter("eta", "metal-Ag-eta"));
            out.parameters.push(text_parameter("k", "metal-Ag-k"));
            "conductor"
        }
        _ => return None,
    }
    .into();
    Some(out)
}

fn text_parameter(name: &str, value: &str) -> PreviewParameter {
    PreviewParameter {
        name: name.into(),
        value_type: "spectrum".into(),
        floats: vec![],
        texts: vec![value.into()],
        truncated: false,
    }
}

fn remaps_roughness(p: &[PreviewParameter]) -> bool {
    find(p, "remaproughness")
        .and_then(|v| {
            v.texts
                .first()
                .map(|t| t == "true")
                .or(v.floats.first().map(|f| *f != 0.))
        })
        .unwrap_or(true)
}

/// GGX alpha from a pbrt roughness parameter: `sqrt(roughness)` when
/// `remaproughness` is true (pbrt's default), the value itself otherwise.
/// Anisotropic u/v roughness is averaged.
fn alpha_of(p: &[PreviewParameter], prefix: &str, default: f64) -> f32 {
    let remap = remaps_roughness(p);
    let base = float(p, &format!("{prefix}roughness")).unwrap_or(default);
    let u = float(p, &format!("{prefix}uroughness"))
        .unwrap_or(base)
        .max(0.0);
    let v = float(p, &format!("{prefix}vroughness"))
        .unwrap_or(base)
        .max(0.0);
    let to_alpha = |r: f64| if remap { r.sqrt() } else { r };
    ((to_alpha(u) + to_alpha(v)) * 0.5).max(MIN_ALPHA as f64) as f32
}

/// One spectrum-ish preview parameter as the `spectrum` crate's `Value`,
/// told apart by the declared type, not by the floats' shape: inline
/// λ/value pairs and builtin names carry `spectrum`, a colour `rgb`, a
/// scalar `float`. Spectrum Data expands into pairs, which the old
/// leading-three-numbers read mistook for a colour.
fn value_of(parameter: &PreviewParameter) -> Option<spectrum::Value<'_>> {
    match parameter.value_type.as_str() {
        "spectrum" if !parameter.floats.is_empty() => {
            // A well-formed pair list is the curve; an odd count (a
            // scalar spectrum[X] survived as one number) reads as a
            // constant what a 3-less shape it is: nothing.
            parameter
                .floats
                .len()
                .is_multiple_of(2)
                .then(|| spectrum::Value::Pairs(&parameter.floats))
        }
        "spectrum" if !parameter.texts.is_empty() => parameter
            .texts
            .first()
            .map(|name| spectrum::Value::Named(name)),
        "rgb" if !parameter.floats.is_empty() => {
            let first = parameter.floats[0];
            let second = parameter.floats.get(1).copied().unwrap_or(first);
            Some(spectrum::Value::Rgb([
                first,
                second,
                parameter.floats.get(2).copied().unwrap_or(second),
            ]))
        }
        "float" | "integer" if !parameter.floats.is_empty() => parameter
            .floats
            .first()
            .copied()
            .map(spectrum::Value::Constant),
        _ => None,
    }
}

/// Copper's F0 — r4's conductor default, in vacuum or under a coat.
fn copper_f0(medium: f64) -> [f64; 3] {
    spectrum::conductor_f0(
        spectrum::Value::Named("metal-Cu-eta"),
        spectrum::Value::Named("metal-Cu-k"),
        medium,
    )
    .unwrap_or([0.5; 3])
}

/// The neutral grey a name that is not a refractive index shades
/// — the same grey the Metal colour row answers, so Lit,
/// the thumbnail and the row agree.
const GREY_NOT_AN_INDEX: [f64; 3] = [0.5; 3];

/// Copper with the note an unreadable declared value earns: the reason
/// the preview refused to draw the declared metal ("metal spectrum
/// unknown: copper used", a malformed sample list — r4's own default
/// stands in). A name the crate cannot read is a different refusal —
/// the grey of [`GREY_NOT_AN_INDEX`] with its own note.
fn copper_with_note(
    spectrum: Option<&PreviewParameter>,
    medium: f64,
    notes: &mut Vec<LitNote>,
    material: Option<&str>,
) -> [f64; 3] {
    let text = spectrum
        .and_then(|v| v.texts.first())
        .map_or("unknown".to_owned(), String::clone);
    notes.push(LitNote {
        reason: format!("metal spectrum {text}: copper used"),
        material: material.map(str::to_owned),
        draws: 0,
    });
    copper_f0(medium)
}

/// Conductor F0 in metallic terms, the way `project.spectrum_color`'s
/// conductor role and the Metal colour row compute it: a
/// `reflectance` reads its own colour (r4's eta-1 form); declared
/// `eta`/`k` read as curves — λ/value pairs, builtin names (a glass
/// as its channel samples) or rgb — and a missing side fills with
/// r4's copper default; a declared SHAPE no curve can read (a blackbody
/// typed on eta, an odd sample count) falls back to copper with a
/// note, while a name with neither a curve nor channel samples (an
/// illuminant, a typo, a Viewer's `spds/…` string) shades
/// the row's neutral grey with its note. `medium` is the index around
/// the metal: 1 for a bare conductor, the interface eta under a coat,
/// where pbrt-v4 divides the metal's eta and k by it (r4
/// materials/coated_conductor.rs).
fn conductor_lobe_f0(
    p: &[PreviewParameter],
    reflectance_key: &str,
    prefix: &str,
    medium: f64,
    notes: &mut Vec<LitNote>,
    material: Option<&str>,
) -> [f64; 3] {
    let key = |n: &str| format!("{prefix}{n}");
    let eta = find(p, &key("eta"));
    let k = find(p, &key("k"));
    if (eta.is_some() || k.is_some()) && find(p, reflectance_key).is_some() {
        notes.push(LitNote {
            reason: format!(
                "reflectance beside {prefix}eta/k: r4 refuses both, colour from reflectance"
            ),
            material: material.map(str::to_owned),
            draws: 0,
        });
    }
    if let Some(declared) = find(p, reflectance_key) {
        // r4 turns reflectance r into eta 1, k = 2√r/√(1−r): the F0 the
        // reflectance's own colour shows, divided by the coat's index.
        if let Some(value) = value_of(declared)
            && let Some(f0) = spectrum::reflectance_f0(value, medium)
        {
            return f0;
        }
        return copper_with_note(Some(declared), medium, notes, material);
    }
    if eta.is_none() && k.is_none() {
        return copper_f0(medium);
    }
    // r4 fills an undeclared side with copper (conductor.rs), reading
    // the given side as it stands — a default, no note.
    let eta_value = match (eta, eta.and_then(value_of)) {
        (None, _) => Some(spectrum::Value::Named("metal-Cu-eta")),
        (Some(_), None) => return copper_with_note(eta, medium, notes, material),
        (Some(_), value) => value,
    };
    let k_value = match (k, k.and_then(value_of)) {
        (None, _) => Some(spectrum::Value::Named("metal-Cu-k")),
        (Some(_), None) => return copper_with_note(k, medium, notes, material),
        (Some(_), value) => value,
    };
    let (Some(eta_value), Some(k_value)) = (eta_value, k_value) else {
        return copper_f0(medium);
    };
    match spectrum::conductor_f0(eta_value, k_value, medium) {
        Some(f0) => f0,
        // A name with neither a curve nor channel samples (an
        // illuminant, a typo, a Viewer's `spds/…` string) cannot be
        // evaluated here: the neutral grey the Metal colour row answers,
        // with a note. The note says the value is
        // unread, not that it is no index — a Viewer's file name may
        // well name a curve r4 can render. Any other unreadable SHAPE
        // (a malformed list) keeps the copper fallback and its note.
        None => {
            let not_an_index = |value: spectrum::Value| match value {
                spectrum::Value::Named(name) if spectrum::named_is_not_a_refractive_index(name) => {
                    Some(name.to_owned())
                }
                _ => None,
            };
            let named: Vec<String> = [eta_value, k_value]
                .into_iter()
                .filter_map(not_an_index)
                .collect();
            if named.is_empty() {
                copper_with_note(eta.or(k), medium, notes, material)
            } else {
                notes.push(LitNote {
                    reason: format!(
                        "metal spectrum {}: unread as a curve, grey shown",
                        named.join(" · ")
                    ),
                    material: material.map(str::to_owned),
                    draws: 0,
                });
                GREY_NOT_AN_INDEX
            }
        }
    }
}

/// A coatedconductor's interface index: the `interface.eta` float, or
/// one 550 nm read of a spectral declaration (pairs, a name, an rgb);
/// 1.5 when undeclared, and 1 in place of a non-positive read (r4
/// replaces a zero sample with one).
fn interface_medium(p: &[PreviewParameter]) -> f64 {
    find(p, "interface.eta")
        .and_then(|declared| match declared.value_type.as_str() {
            "float" => declared.floats.first().copied(),
            _ => value_of(declared).and_then(|value| spectrum::curve_point(value, 550.)),
        })
        .map(|eta| if eta > 0. { eta } else { 1. })
        .unwrap_or(1.5)
}

/// Unpolarized Fresnel reflectance of a dielectric of relative index `eta`
/// for light arriving at `cos_i` from the outside.
fn fresnel_dielectric(cos_i: f64, eta: f64) -> f64 {
    let sin_t2 = (1. - cos_i * cos_i) / (eta * eta);
    if sin_t2 >= 1. {
        return 1.;
    }
    let cos_t = (1. - sin_t2).sqrt();
    let parallel = (eta * cos_i - cos_t) / (eta * cos_i + cos_t);
    let perpendicular = (cos_i - eta * cos_t) / (cos_i + eta * cos_t);
    0.5 * (parallel * parallel + perpendicular * perpendicular)
}

/// Fraction of diffuse light inside a smooth dielectric of index `eta` that
/// its surface reflects back in: 1 − (1 − r_e)/η², with r_e the outside
/// hemispherical reflectance ∫ F(cos) 2 cos dcos (≈ 0.596 for η = 1.5).
pub(crate) fn internal_reflectance(eta: f64) -> f64 {
    let steps = 512;
    let outside = (0..steps)
        .map(|i| {
            let cos = (i as f64 + 0.5) / steps as f64;
            fresnel_dielectric(cos, eta) * 2. * cos / steps as f64
        })
        .sum::<f64>();
    1. - (1. - outside) / (eta * eta)
}

/// pbrt simulates coated materials with a stochastic layered BxDF; Lit uses
/// closed forms that land within about 25% of r4.
fn layered_note(name: &str, record: &MaterialRecord, notes: &mut Vec<LitNote>) {
    notes.push(LitNote {
        reason: format!("{name}: layered coat approximated"),
        material: record.name.clone(),
        draws: 0,
    });
}

/// A smooth-or-rough dielectric coat: [F0, α, eta, internal reflectance].
fn coat(p: &[PreviewParameter], eta: f64, prefix: &str) -> [f32; 4] {
    [
        dielectric_f0(eta) as f32,
        alpha_of(p, prefix, 0.0),
        eta as f32,
        internal_reflectance(eta) as f32,
    ]
}

/// A subsurface medium's drawn albedo: its `reflectance`, or the
/// per-channel single-scattering albedo σs/(σs+σa) of `sigma_s` over
/// `sigma_a`, or gray when neither is given.
fn subsurface_albedo(p: &[PreviewParameter]) -> [f64; 3] {
    if find(p, "reflectance").is_some() {
        return rgb_of(p, "reflectance", [0.5; 3]);
    }
    if find(p, "sigma_s").is_some() && find(p, "sigma_a").is_some() {
        let sigma_s = rgb_of(p, "sigma_s", [0.; 3]);
        let sigma_a = rgb_of(p, "sigma_a", [0.; 3]);
        return std::array::from_fn(|channel| {
            let total = sigma_s[channel] + sigma_a[channel];
            if total > 0. {
                sigma_s[channel] / total
            } else {
                0.5
            }
        });
    }
    [0.5; 3]
}

/// A hair's drawn color: `reflectance`, then `color`, else the eumelanin
/// brown of pbrt's default pigmentation.
fn hair_color(p: &[PreviewParameter]) -> [f64; 3] {
    for key in ["reflectance", "color"] {
        if find(p, key).is_some() {
            return rgb_of(p, key, [0.17, 0.10, 0.05]);
        }
    }
    [0.17, 0.10, 0.05]
}

/// One material's lobe from its literal values. Texture inputs and mix
/// children are linked afterwards by `lit::evaluate`.
pub(crate) fn material_lobe(record: &MaterialRecord, notes: &mut Vec<LitNote>) -> GpuLobe {
    if let Some(upgraded) = upgrade_v3(record) {
        notes.push(LitNote {
            reason: format!(
                "{}: pbrt-v3 material, shown as r4's --upgrade makes it ({})",
                record.implementation, upgraded.implementation
            ),
            material: record.name.clone(),
            draws: 0,
        });
        return material_lobe(&upgraded, notes);
    }
    let p = &record.parameters;
    let name = record.implementation.to_ascii_lowercase();
    let color = |key, default| rgb_of(p, key, default);
    let mut out = GpuLobe {
        kind: KIND_LAMBERT,
        spec_f90: [1.; 3],
        texture: [-1., 1., 0., 0.],
        ..Default::default()
    };
    if remaps_roughness(p) {
        out.flags[0] |= LOBE_REMAP_ROUGHNESS;
    }
    match name.as_str() {
        "diffuse" => out.diffuse = color("reflectance", [0.5; 3]).map(|x| x as f32),
        // Lit has no transmission through diffusers: only the reflected part.
        "diffusetransmission" => out.diffuse = color("reflectance", [0.25; 3]).map(|x| x as f32),
        "conductor" => {
            let f0 = conductor_lobe_f0(p, "reflectance", "", 1., notes, record.name.as_deref());
            out.spec_f0 = f0.map(|x| x as f32);
            out.flags[0] |= LOBE_REFLECTANCE_IS_F0;
            out.spec_alpha = alpha_of(p, "", 0.0);
        }
        "dielectric" | "thindielectric" => {
            let f = dielectric_f0(float(p, "eta").unwrap_or(1.5)) as f32;
            out.spec_f0 = [f; 3];
            out.spec_alpha = alpha_of(p, "", 0.0);
            out.transmission = 1.0 - f;
        }
        "coateddiffuse" => {
            layered_note(&name, record, notes);
            out.diffuse = color("reflectance", [0.5; 3]).map(|x| x as f32);
            out.coat = coat(p, float(p, "eta").unwrap_or(1.5), "");
        }
        "coatedconductor" => {
            layered_note(&name, record, notes);
            let eta = interface_medium(p);
            // pbrt-v4 takes `reflectance` unprefixed; eta, k and roughness
            // carry `conductor.`.
            let f0 = conductor_lobe_f0(
                p,
                "reflectance",
                "conductor.",
                eta,
                notes,
                record.name.as_deref(),
            );
            out.spec_f0 = f0.map(|x| x as f32);
            out.flags[0] |= LOBE_REFLECTANCE_IS_F0;
            out.spec_alpha = alpha_of(p, "conductor.", 0.0);
            out.coat = coat(p, eta, "interface.");
        }
        // No subsurface scattering in Lit: a coated diffuse of the albedo.
        "subsurface" => {
            out.diffuse = subsurface_albedo(p).map(|x| x as f32);
            out.coat = coat(p, float(p, "eta").unwrap_or(1.33), "");
            notes.push(LitNote {
                reason: "subsurface: no scattering in Lit, shown as coated diffuse".into(),
                material: record.name.clone(),
                draws: 0,
            });
        }
        // No fibre BSDF in Lit: a plain diffuse of the hair's color.
        "hair" => {
            out.diffuse = hair_color(p).map(|x| x as f32);
            notes.push(LitNote {
                reason: "hair: no fibre model in Lit, shown as diffuse".into(),
                material: record.name.clone(),
                draws: 0,
            });
        }
        "measured" => {
            out.diffuse = color("reflectance", [0.5; 3]).map(|x| x as f32);
            let loaded = find(p, "reflectance").is_some();
            notes.push(LitNote {
                reason: if loaded {
                    "measured: shown as representative-color diffuse in Lit".into()
                } else {
                    "measured: file unread, shown as gray diffuse".into()
                },
                material: record.name.clone(),
                draws: 0,
            });
        }
        "interface" => out.kind = super::KIND_INVISIBLE,
        // Gray until `lit::evaluate` links the children.
        "mix" => {
            out.kind = super::KIND_MIX;
            out.diffuse = [0.5; 3];
        }
        _ => {
            let c = color("reflectance", [0.5; 3]);
            out.diffuse = c.map(|x| x as f32);
            notes.push(LitNote {
                reason: format!("{name}: shown as gray diffuse"),
                material: record.name.clone(),
                draws: 0,
            });
        }
    }
    out
}

fn find<'a>(p: &'a [PreviewParameter], n: &str) -> Option<&'a PreviewParameter> {
    p.iter().find(|v| v.matches(n))
}
fn float(p: &[PreviewParameter], n: &str) -> Option<f64> {
    p.iter().find_map(|v| v.float(n))
}
pub(super) fn rgb_of(p: &[PreviewParameter], n: &str, d: [f64; 3]) -> [f64; 3] {
    p.iter()
        .find(|v| v.matches(n) && !v.floats.is_empty())
        .map_or(d, |v| {
            [
                v.floats[0],
                *v.floats.get(1).unwrap_or(&v.floats[0]),
                *v.floats
                    .get(2)
                    .unwrap_or(v.floats.get(1).unwrap_or(&v.floats[0])),
            ]
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn material(implementation: &str, values: &[(&str, &[f64])]) -> MaterialRecord {
        MaterialRecord {
            implementation: implementation.into(),
            resource_id: None,
            name: Some(implementation.into()),
            source: None,
            parameters: values
                .iter()
                .map(|(name, floats)| PreviewParameter {
                    name: (*name).into(),
                    value_type: "float".into(),
                    floats: floats.to_vec(),
                    texts: vec![],
                    truncated: false,
                })
                .collect(),
        }
    }
    fn convert(m: MaterialRecord) -> (GpuLobe, Vec<LitNote>) {
        let mut notes = vec![];
        let l = material_lobe(&m, &mut notes);
        (l, notes)
    }
    #[test]
    fn diffuse_keeps_its_reflectance_and_has_no_specular() {
        let (l, _) = convert(material("diffuse", &[]));
        assert_eq!(l.diffuse, [0.5; 3]);
        assert_eq!(l.spec_alpha, 0.0);
    }
    /// Lit's answers within `tolerance` of the named-metal tables (the
    /// 10 nm grid Lit's crate integration reads against the tables' 1 nm
    /// one; the tolerance is 2e-3).
    fn close(got: [f32; 3], want: [f32; 3], tolerance: f32) {
        for (got, want) in got.into_iter().zip(want) {
            assert!(
                (got - want).abs() < tolerance,
                "{got:?} vs the table's {want:?}"
            );
        }
    }

    /// A named-text or λ/value-pair spectrum parameter.
    fn spectrum(name: &str, texts: &[&str], floats: &[f64]) -> PreviewParameter {
        PreviewParameter {
            name: name.into(),
            value_type: "spectrum".into(),
            floats: floats.to_vec(),
            texts: texts.iter().map(|t| (*t).into()).collect(),
            truncated: false,
        }
    }

    /// An rgb-typed colour parameter.
    fn rgb(name: &str, floats: &[f64]) -> PreviewParameter {
        PreviewParameter {
            name: name.into(),
            value_type: "rgb".into(),
            floats: floats.to_vec(),
            texts: vec![],
            truncated: false,
        }
    }

    /// The test fixtures' Au spd curves (56 pairs each), the shape
    /// Spectrum Data expands into the preview's parameters.
    fn au_spd() -> (Vec<f64>, Vec<f64>) {
        let read = |file: &str| {
            std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../tests/fixtures/spectra")
                    .join(file),
            )
            .unwrap()
            .lines()
            .filter_map(|line| line.split_once(' '))
            .flat_map(|(a, b)| [a.parse::<f64>().unwrap(), b.trim().parse::<f64>().unwrap()])
            .collect::<Vec<f64>>()
        };
        (read("Au.eta.spd"), read("Au.k.spd"))
    }

    #[test]
    fn conductor_defaults_to_copper_with_sqrt_roughness() {
        let (l, notes) = convert(material("conductor", &[("roughness", &[0.04])]));
        assert_eq!(l.spec_alpha, 0.2);
        close(
            l.spec_f0,
            super::super::named_metals::NAMED_METALS[0].1,
            2e-3,
        );
        assert!(notes.is_empty(), "the r4 default is no fallback note");
    }
    #[test]
    fn conductor_reflectance_overrides_eta_and_k() {
        let mut m = material("conductor", &[]);
        m.parameters.push(rgb("reflectance", &[0.9, 0.6, 0.3]));
        let (l, _) = convert(m);
        assert_eq!(l.spec_f0, [0.9, 0.6, 0.3]);
    }
    #[test]
    fn coateddiffuse_has_a_dielectric_coat_over_its_reflectance() {
        let (l, _) = convert(material("coateddiffuse", &[]));
        assert_eq!(l.coat[0], dielectric_f0(1.5) as f32);
        assert_eq!(l.diffuse, [0.5; 3]);
    }
    #[test]
    fn dielectric_reflects_with_eta_and_transmits_the_rest() {
        let (l, _) = convert(material("dielectric", &[]));
        assert_eq!(l.spec_f0, [0.04; 3]);
        assert_eq!(l.transmission, 0.96);
    }
    #[test]
    fn thindielectric_uses_the_smooth_dielectric_lobe() {
        let (l, _) = convert(material("thindielectric", &[]));
        assert_eq!(l.spec_f0, [0.04; 3]);
        assert_eq!(l.transmission, 0.96);
        assert_eq!(l.spec_alpha, MIN_ALPHA);
    }
    /// v3 names follow r4's `--upgrade` (parser/upgrade.rs): r4 refuses
    /// them otherwise, so Lit shows what the upgrade renders, with a note.
    #[test]
    fn v3_plastic_upgrades_to_coateddiffuse_with_v4_defaults() {
        let (l, notes) = convert(material("plastic", &[("Kd", &[0.2, 0.3, 0.4])]));
        assert_eq!(l.diffuse, [0.2, 0.3, 0.4]);
        assert_eq!(l.coat[0], dielectric_f0(1.5) as f32);
        assert_eq!(l.coat[1], MIN_ALPHA); // coateddiffuse roughness 0
        assert!(notes[0].reason.contains("--upgrade"), "{notes:?}");
        let (plain, _) = convert(material("plastic", &[("Ks", &[0., 0., 0.])]));
        assert_eq!(plain.coat[0], 0.0, "Ks 0 upgrades to plain diffuse");
    }
    #[test]
    fn v3_mirror_upgrades_to_smooth_silver() {
        let (l, _) = convert(material("mirror", &[("Kr", &[0.5; 3])]));
        close(
            l.spec_f0,
            super::super::named_metals::NAMED_METALS[2].1,
            2e-3,
        );
        assert_eq!(l.spec_alpha, MIN_ALPHA);
    }
    #[test]
    fn v3_matte_and_glass_rename_their_parameters() {
        assert_eq!(
            convert(material("matte", &[("Kd", &[0.7; 3])])).0.diffuse,
            [0.7; 3]
        );
        let (glass, _) = convert(material("glass", &[("index", &[2.0])]));
        assert_eq!(glass.spec_f0, [dielectric_f0(2.0) as f32; 3]);
    }
    #[test]
    fn remaproughness_false_uses_roughness_as_alpha() {
        let mut m = material("conductor", &[("roughness", &[0.04])]);
        m.parameters.push(PreviewParameter {
            name: "remaproughness".into(),
            value_type: "bool".into(),
            floats: vec![],
            texts: vec!["false".into()],
            truncated: false,
        });
        assert_eq!(convert(m).0.spec_alpha, 0.04);
    }
    #[test]
    fn coatedconductor_reads_its_conductor_parameters() {
        let mut m = material("coatedconductor", &[("conductor.roughness", &[0.09])]);
        m.parameters.push(rgb("reflectance", &[0.9, 0.5, 0.1]));
        let (l, _) = convert(m);
        // r4 builds eta 1, k = 2·sqrt(r)/sqrt(1 − r) from the reflectance
        // and divides both by the interface eta 1.5, per display
        // channel — the construction the r4 reference scenes pin.
        let interface = 1. / 1.5;
        let expected = [0.9f64, 0.5, 0.1].map(|r| {
            let k = 2. * r.sqrt() / (1. - r).sqrt() / 1.5;
            ((interface - 1.) * (interface - 1.) + k * k)
                / ((interface + 1.) * (interface + 1.) + k * k)
        });
        close(l.spec_f0, expected.map(|v| v as f32), 1e-6);
        assert!((l.spec_alpha - 0.3).abs() < 1e-6);
        assert_eq!(l.coat[0], dielectric_f0(1.5) as f32);
    }
    /// An unknown named spectrum (a typo, an illuminant)
    /// is not a refractive index — the neutral grey the Metal colour
    /// row answers, with the note, copper never. A declared shape no
    /// curve can read at all (an odd sample count) keeps r4's own
    /// copper stand-in and its note.
    #[test]
    fn unknown_names_go_grey_and_unreadable_shapes_keep_copper() {
        let mut m = material("conductor", &[]);
        m.parameters.push(spectrum("eta", &["metal-nope-eta"], &[]));
        let (l, n) = convert(m);
        close(l.spec_f0, [0.5; 3], 1e-6);
        assert_eq!(
            n[0].reason,
            "metal spectrum metal-nope-eta: unread as a curve, grey shown"
        );
        let mut odd = material("conductor", &[]);
        odd.parameters.push(spectrum("eta", &[], &[400., 1., 500.]));
        let (l, n) = convert(odd);
        close(
            l.spec_f0,
            super::super::named_metals::NAMED_METALS[0].1,
            2e-3,
        );
        assert_eq!(n[0].reason, "metal spectrum unknown: copper used");
    }
    #[test]
    fn interface_material_draws_nothing() {
        let (l, _) = convert(material("interface", &[]));
        assert_eq!(l.kind, super::super::KIND_INVISIBLE);
    }
    #[test]
    fn unsupported_materials_are_gray_with_a_note() {
        let (l, n) = convert(material("bogus", &[]));
        assert_eq!(l.diffuse, [0.5; 3]);
        assert_eq!(n.len(), 1);
    }
    /// Lit has no subsurface scattering (PR-A): the albedo is drawn as a
    /// diffuse under a dielectric coat, eta defaulting to 1.33.
    #[test]
    fn subsurface_reflectance_draws_as_a_coated_diffuse() {
        let (l, n) = convert(material("subsurface", &[("reflectance", &[0.2, 0.4, 0.6])]));
        assert_eq!(l.diffuse, [0.2, 0.4, 0.6]);
        assert_eq!(l.coat[0], dielectric_f0(1.33) as f32);
        assert_eq!(n.len(), 1);
        assert!(n[0].reason.starts_with("subsurface"), "{n:?}");
    }
    /// Without a reflectance, the medium's single-scattering albedo
    /// σs/(σs+σa) per channel stands in.
    #[test]
    fn subsurface_derives_its_albedo_from_sigma_s_and_sigma_a() {
        let (l, _) = convert(material(
            "subsurface",
            &[("sigma_s", &[3., 1., 1.]), ("sigma_a", &[1., 1., 3.])],
        ));
        assert_eq!(l.diffuse, [0.75, 0.5, 0.25]);
    }
    #[test]
    fn subsurface_without_optics_stays_a_coated_gray() {
        let (l, n) = convert(material("subsurface", &[]));
        assert_eq!(l.diffuse, [0.5; 3]);
        assert!(l.coat[0] > 0., "still under a coat");
        assert_eq!(n.len(), 1);
    }
    /// Lit has no fibre BSDF (PR-A): hair draws as the diffuse of its
    /// `reflectance`, then its `color`, else a eumelanin brown.
    #[test]
    fn hair_reads_reflectance_then_color_and_defaults_to_brown() {
        let (both, n) = convert(material(
            "hair",
            &[
                ("reflectance", &[0.9, 0.1, 0.1]),
                ("color", &[0.1, 0.9, 0.1]),
            ],
        ));
        assert_eq!(both.diffuse, [0.9, 0.1, 0.1], "reflectance wins");
        let (color, _) = convert(material("hair", &[("color", &[0.1, 0.9, 0.1])]));
        assert_eq!(color.diffuse, [0.1, 0.9, 0.1]);
        let (plain, _) = convert(material("hair", &[]));
        assert_eq!(plain.diffuse, [0.17, 0.10, 0.05]);
        assert_eq!(n.len(), 1);
        assert!(n[0].reason.starts_with("hair"), "{n:?}");
    }

    #[test]
    fn measured_uses_preview_reflectance_or_gray_with_a_note() {
        let (loaded, loaded_notes) =
            convert(material("measured", &[("reflectance", &[0.1, 0.3, 0.8])]));
        assert_eq!(loaded.diffuse, [0.1, 0.3, 0.8]);
        assert!(loaded_notes[0].reason.contains("representative-color"));

        let (missing, missing_notes) = convert(material("measured", &[]));
        assert_eq!(missing.diffuse, [0.5; 3]);
        assert!(missing_notes[0].reason.contains("file unread"));
    }
    #[test]
    fn named_metal_table_covers_the_r4_metals() {
        let metals = super::super::named_metals::NAMED_METALS;
        assert_eq!(
            metals.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
            ["Cu", "Au", "Ag", "Al"]
        );
        assert!(metals[0].1[0] > metals[0].1[2]);
    }
    #[test]
    fn internal_reflectance_of_glass_is_about_six_tenths() {
        assert!((internal_reflectance(1.5) - 0.596).abs() < 0.005);
        assert!(internal_reflectance(1.0).abs() < 1e-9);
    }
    /// pbrt-v4 divides the coated metal's eta and k by the interface eta,
    /// which lowers its reflectance (r4 materials/coated_conductor.rs).
    #[test]
    fn coated_metals_reflect_less_than_bare_ones() {
        let bare = convert(material("conductor", &[])).0.spec_f0;
        let coated = convert(material("coatedconductor", &[])).0.spec_f0;
        close(
            coated,
            super::super::named_metals::NAMED_METALS_IN_COAT[0].1,
            2e-3,
        );
        assert!(coated[1] < bare[1]);
        // reflectance r under no coat is r back.
        let mut m = material("conductor", &[]);
        m.parameters.push(rgb("reflectance", &[0.5; 3]));
        let (bare_r, _) = convert(m);
        assert!((bare_r.spec_f0[0] - 0.5).abs() < 1e-6);
    }
    /// pbrt-v4's coatedconductor takes `reflectance` unprefixed; only eta, k
    /// and roughness carry `conductor.` (review #265).
    #[test]
    fn coatedconductor_reads_unprefixed_reflectance() {
        let mut m = material("coatedconductor", &[]);
        m.parameters.push(rgb("reflectance", &[0.9, 0.5, 0.1]));
        let (l, _) = convert(m);
        // r4 builds eta 1, k = 2·sqrt(r)/sqrt(1 − r) from the reflectance
        // and divides both by the interface eta 1.5 — per display
        // channel, the construction itself the reference scenes pin.
        let expected = [0.9f64, 0.5, 0.1].map(|r| {
            let interface = 1. / 1.5;
            let k = 2. * r.sqrt() / (1. - r).sqrt() / 1.5;
            ((interface - 1.) * (interface - 1.) + k * k)
                / ((interface + 1.) * (interface + 1.) + k * k)
        });
        close(l.spec_f0, expected.map(|v| v as f32), 1e-6);
    }

    #[test]
    fn a_named_conductor_reads_both_sides_own_curves() {
        // eta of one metal and k of another mix (r4 reads both), where
        // the old preview showed eta's metal alone.
        let mut m = material("conductor", &[]);
        m.parameters.push(spectrum("eta", &["metal-Au-eta"], &[]));
        m.parameters.push(spectrum("k", &["metal-Cu-k"], &[]));
        let (mixed, notes) = convert(m);
        let mut all_au = material("conductor", &[]);
        all_au
            .parameters
            .push(spectrum("eta", &["metal-Au-eta"], &[]));
        all_au.parameters.push(spectrum("k", &["metal-Au-k"], &[]));
        let all_au = convert(all_au).0.spec_f0;
        let all_copper = convert(material("conductor", &[])).0.spec_f0;
        assert_ne!(mixed.spec_f0, all_au, "the k side must count");
        assert_ne!(mixed.spec_f0, all_copper, "the eta side must count");
        close(
            mixed.spec_f0,
            spectrum::conductor_f0(
                spectrum::Value::Named("metal-Au-eta"),
                spectrum::Value::Named("metal-Cu-k"),
                1.,
            )
            .unwrap()
            .map(|v| v as f32),
            1e-6,
        );
        assert!(notes.is_empty(), "r4 reads both sides without a note");
    }

    /// A non-metal curve on eta shades what the Metal
    /// colour row shows — a glass reads its channel samples (the rgb
    /// way), and a name with nothing to read (an illuminant) shades the
    /// row's neutral grey with a note, copper never.
    #[test]
    fn a_glass_eta_shades_like_the_row_and_a_nameless_curve_goes_grey() {
        let mut m = material("conductor", &[]);
        m.parameters.push(spectrum("eta", &["glass-BK7"], &[]));
        let (l, notes) = convert(m);
        close(
            l.spec_f0,
            spectrum::conductor_f0(
                spectrum::Value::Named("glass-BK7"),
                spectrum::Value::Named("metal-Cu-k"),
                1.,
            )
            .unwrap()
            .map(|v| v as f32),
            1e-6,
        );
        assert!(
            !notes.iter().any(|n| n.reason.contains("copper")),
            "{notes:?}"
        );
        // An illuminant such as D65 is not a refractive index: the
        // neutral grey the Metal colour row answers.
        let mut m = material("conductor", &[]);
        m.parameters.push(spectrum("eta", &["stdillum-D65"], &[]));
        let (l, notes) = convert(m);
        close(l.spec_f0, [0.5; 3], 1e-6);
        assert!(
            notes
                .iter()
                .any(|n| n.reason.contains("stdillum-D65: unread as a curve")),
            "{notes:?}"
        );
        assert!(
            !notes.iter().any(|n| n.reason.contains("copper")),
            "{notes:?}"
        );
        // Both sides unread: the note names both, the way the
        // Metal colour row's source does.
        let mut m = material("conductor", &[]);
        m.parameters.push(spectrum("eta", &["stdillum-D65"], &[]));
        m.parameters.push(spectrum("k", &["illum-D50"], &[]));
        let (_, notes) = convert(m);
        assert!(
            notes.iter().any(|n| n
                .reason
                .contains("stdillum-D65 · illum-D50: unread as a curve")),
            "{notes:?}"
        );
    }

    /// A declared side joins r4's default copper for the missing one —
    /// a default fill, not a fallback note.
    #[test]
    fn a_one_sided_conductor_keeps_its_given_side_over_copper_default() {
        let mut eta_only = material("conductor", &[]);
        eta_only
            .parameters
            .push(spectrum("eta", &[], &[465., 1.37, 532., 0.46, 630., 0.18]));
        let (l, notes) = convert(eta_only);
        close(
            l.spec_f0,
            spectrum::conductor_f0(
                spectrum::Value::Pairs(&[465., 1.37, 532., 0.46, 630., 0.18]),
                spectrum::Value::Named("metal-Cu-k"),
                1.,
            )
            .unwrap()
            .map(|v| v as f32),
            1e-6,
        );
        assert!(notes.is_empty(), "r4 fills the missing side without a note");
        let mut k_only = material("conductor", &[]);
        k_only.parameters.push(spectrum("k", &["metal-Au-k"], &[]));
        let (l, notes) = convert(k_only);
        close(
            l.spec_f0,
            spectrum::conductor_f0(
                spectrum::Value::Named("metal-Cu-eta"),
                spectrum::Value::Named("metal-Au-k"),
                1.,
            )
            .unwrap()
            .map(|v| v as f32),
            1e-6,
        );
        assert!(notes.is_empty());
    }

    /// Spectrum Data (and inline pairs) arrive as `spectrum` λ/value
    /// floats, which the old read mistook for an rgb triple.
    #[test]
    fn spectrum_pairs_of_eta_and_k_read_as_curves() {
        let (eta, k) = au_spd();
        let mut m = material("conductor", &[]);
        m.parameters.push(spectrum("eta", &[], &eta));
        m.parameters.push(spectrum("k", &[], &k));
        let (l, notes) = convert(m);
        // The spd pairs are r4's own Au curves: the named table's gold.
        close(
            l.spec_f0,
            super::super::named_metals::NAMED_METALS[1].1,
            2e-3,
        );
        assert!(
            !notes.iter().any(|n| n.reason.contains("copper")),
            "{notes:?}"
        );
    }

    /// A spectral `interface.eta` is read at one wavelength, not its
    /// first number: the pairs' leading 400 is the wavelength, not the
    /// coat's index.
    #[test]
    fn spectral_interface_eta_reads_one_wavelength_like_r4() {
        let mut m = material("coatedconductor", &[]);
        m.parameters
            .push(spectrum("interface.eta", &[], &[400., 2., 700., 2.]));
        m.parameters
            .push(spectrum("conductor.eta", &["metal-Au-eta"], &[]));
        m.parameters
            .push(spectrum("conductor.k", &["metal-Au-k"], &[]));
        let (l, _) = convert(m);
        let medium =
            spectrum::curve_point(spectrum::Value::Pairs(&[400., 2., 700., 2.]), 550.).unwrap();
        close(
            l.spec_f0,
            spectrum::conductor_f0(
                spectrum::Value::Named("metal-Au-eta"),
                spectrum::Value::Named("metal-Au-k"),
                medium,
            )
            .unwrap()
            .map(|v| v as f32),
            1e-6,
        );
        assert!(
            l.spec_f0[2] > 0.,
            "a coat of 2 drops the blue below the 1.5 table"
        );
    }

    /// A reflectance declared as λ/value pairs reads its curve's colour,
    /// not the first three numbers of the list.
    #[test]
    fn spectral_reflectance_reads_the_curve_not_the_leading_numbers() {
        let mut m = material("conductor", &[]);
        m.parameters
            .push(spectrum("reflectance", &[], &[380., 0.2, 780., 0.8]));
        let (l, notes) = convert(m);
        close(
            l.spec_f0,
            spectrum::reflectance_f0(spectrum::Value::Pairs(&[380., 0.2, 780., 0.8]), 1.)
                .unwrap()
                .map(|v| v as f32),
            1e-6,
        );
        assert!(
            l.spec_f0[0] > l.spec_f0[2] && l.spec_f0[0] > 0.25 && l.spec_f0[0] < 0.8,
            "the rising curve reads red-dominant, {:?}",
            l.spec_f0
        );
        assert!(
            !notes.iter().any(|n| n.reason.contains("copper")),
            "{notes:?}"
        );
    }
}
