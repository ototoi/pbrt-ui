//! `project.spectrum_color`: one parameter's spectrum-ish value as
//! the linear RGB a swatch paints, resolved through the same `spectrum`
//! crate Lit shades with, plus the human-readable `source` a tooltip
//! shows ("blackbody 6500 K", a builtin name, "SpectrumData: Au.k", rgb
//! triples or sample pairs).
//!
//! The role the colour is resolved for (albedo / unbounded / illuminant)
//! comes from the request when it says one, else from the schema's
//! interpretation classes, else from the parameter's name; the
//! chromaticity role shows the curve's colour with the magnitude
//! dropped, and the conductor role resolves a conductor material's
//! Metal colour (F0) from its eta/k or reflectance. SpectrumData
//! resources asked directly default to the chromaticity read, the
//! curve's own colour.

// The wire's error payload is what every failure here returns; boxed
// smallness is not worth re-typing the whole handler chain (the savers
// carry the same allowance).
#![allow(clippy::result_large_err)]

use super::*;
use protocol::project as dto;

/// One resolved colour: the rgb and the human `source`.
struct Resolved {
    rgb: [f64; 3],
    source: String,
}

/// The chromaticity role's normalizing read of an rgb answer (the
/// blackbody and rgb rows): negatives clamped to 0, then the biggest
/// channel scaled to 1\. Pairs, names and SpectrumData resolve
/// chromaticity inside `pairs_rgb` / `chromaticity` themselves.
fn for_role(rgb: [f64; 3], role: spectrum::Role) -> [f64; 3] {
    match role {
        spectrum::Role::Chromaticity => spectrum::chromaticity_of_rgb(rgb),
        _ => rgb,
    }
}

/// One metal row's declared value, held owned so a SpectrumData curve
/// can bring its own samples, with the human phrase that names how it
/// was given.
enum Side {
    Rgb([f64; 3]),
    Constant(f64),
    Named(String),
    Pairs(Vec<f64>),
    /// A texture connection: never evaluated, answered grey.
    Texture,
}

impl Side {
    /// The crate value of the side, if it is one the crate can read.
    fn value(&self) -> Option<spectrum::Value<'_>> {
        match self {
            Self::Rgb(rgb) => Some(spectrum::Value::Rgb(*rgb)),
            Self::Constant(value) => Some(spectrum::Value::Constant(*value)),
            Self::Named(name) => Some(spectrum::Value::Named(name)),
            Self::Pairs(pairs) => Some(spectrum::Value::Pairs(pairs)),
            Self::Texture => None,
        }
    }
}

/// The `SpectrumData: …` source half: one curve's samples read off the
/// document, the same read `curve_of` colours with.
fn curve_pairs(
    active: &ActiveProject,
    resource: &project::Resource,
) -> Result<Vec<f64>, ErrorPayload> {
    let read = |asset| -> pbrt_io::Result<std::path::PathBuf> {
        let root = active.asset_root.as_deref().ok_or_else(|| {
            pbrt_io::Error::Invalid("this project keeps no data assets on disk".into())
        })?;
        active
            .session
            .snapshot()
            .folder_asset_path(std::path::Path::new(root), asset)
            .map_err(pbrt_io::Error::from)
    };
    let (wavelengths, values) =
        pbrt_io::spectrum_samples(resource, read).map_err(|error| invalid(error.to_string()))?;
    let mut pairs = Vec::with_capacity(wavelengths.len() * 2);
    for (wavelength, value) in wavelengths.into_iter().zip(values) {
        pairs.push(wavelength);
        pairs.push(value);
    }
    Ok(pairs)
}

/// One metal row's declared value and its source phrase: rgb, a scalar,
/// a builtin name, inline pairs, a SpectrumData reference, or a texture
/// connection (never evaluated). Anything else — a blackbody on eta, a
/// number that is none of these — is refused.
fn side_of(
    name: &str,
    parameter: &project::Parameter,
    doc: &project::ProjectDocument,
    active: &ActiveProject,
) -> Result<(Side, String), ErrorPayload> {
    let floats = |values: &[project::FloatValue]| -> Result<Vec<f64>, ErrorPayload> {
        values
            .iter()
            .map(|value| match value {
                project::FloatValue::Finite(number) if number.is_finite() => Ok(*number),
                _ => Err(invalid("the parameter holds non-finite values")),
            })
            .collect()
    };
    let kind = parameter.value_type.to_ascii_lowercase();
    match &parameter.values {
        project::ParameterValues::BuiltinSpectra(names) => {
            let name_of = names
                .first()
                .ok_or_else(|| invalid("the parameter names no spectrum"))?;
            Ok((Side::Named(name_of.clone()), format!("{name}: {name_of}")))
        }
        project::ParameterValues::Strings(names) => {
            let name_of = names
                .first()
                .ok_or_else(|| invalid("the parameter names no spectrum"))?;
            Ok((Side::Named(name_of.clone()), format!("{name}: {name_of}")))
        }
        project::ParameterValues::Floats(values) => match kind.as_str() {
            "rgb" => {
                let values = floats(values)?;
                let first = values
                    .first()
                    .copied()
                    .ok_or_else(|| invalid("the parameter holds no values"))?;
                let second = values.get(1).copied().unwrap_or(first);
                let third = values.get(2).copied().unwrap_or(second);
                Ok((
                    Side::Rgb([first, second, third]),
                    format!(
                        "{name}: rgb {} {} {}",
                        number(first),
                        number(second),
                        number(third)
                    ),
                ))
            }
            "float" => {
                let value = floats(values)?
                    .first()
                    .copied()
                    .ok_or_else(|| invalid("the parameter holds no values"))?;
                Ok((Side::Constant(value), format!("{name}: {}", number(value))))
            }
            "spectrum" => {
                let pairs = floats(values)?;
                if pairs.is_empty() || !pairs.len().is_multiple_of(2) {
                    return Err(invalid(
                        "spectrum samples must be λ value pairs (an even, non-empty count)",
                    ));
                }
                let source = format!("{name}: spectrum samples ({} pairs)", pairs.len() / 2);
                Ok((Side::Pairs(pairs), source))
            }
            other => Err(invalid(format!("{name} as {other} is no metal curve"))),
        },
        project::ParameterValues::Resources {
            kind: project::ResourceKind::SpectrumData,
            ids,
        } => {
            let resource = doc
                .resources
                .get(
                    ids.first()
                        .ok_or_else(|| invalid("the parameter references no spectrum"))?,
                )
                .ok_or_else(|| ErrorPayload::new(ErrorCode::NotFound, "spectrum data not found"))?;
            let label = resource.label.clone();
            Ok((
                Side::Pairs(curve_pairs(active, resource)?),
                format!("{name}: SpectrumData: {label}"),
            ))
        }
        project::ParameterValues::Resources {
            kind: project::ResourceKind::Texture,
            ..
        } => Ok((Side::Texture, format!("{name}: texture (not evaluated)"))),
        _ => Err(invalid(format!(
            "{name} carries a value the Metal colour cannot read"
        ))),
    }
}

/// The conductor role: a conductor material's Metal colour —
/// the normal-incidence F0 of its eta/k (a missing side filled with
/// r4's copper), or its reflectance's own colour, under the
/// coatedconductor's interface index. r4 refuses a reflectance beside
/// eta/k, so the answer warns and shows no colour; a texture side is
/// not evaluated, and answers grey the same way. A name with neither a
/// curve nor channel samples is not a refractive index:
/// the grey with a source saying so, never an error. Whichever metal
/// row asked, the answer is the same.
fn conductor_colour(
    doc: &project::ProjectDocument,
    active: &ActiveProject,
    p: &dto::SpectrumColorParams,
) -> Result<Value, ErrorPayload> {
    const GREY: [f64; 3] = [0.5; 3];
    const WARN: &str = "r4 rejects reflectance together with eta/k";
    if !matches!(
        p.parameter.as_str(),
        "eta" | "k" | "reflectance" | "conductor.eta" | "conductor.k"
    ) {
        return Err(invalid(
            "parameter must be eta, k, reflectance, conductor.eta or conductor.k",
        ));
    }
    let dto::ParameterTarget::Resource(id) = &p.target else {
        return Err(invalid("the conductor role needs a material resource"));
    };
    let resource = doc
        .resources
        .get(&project::ResourceId(*id))
        .ok_or_else(|| ErrorPayload::new(ErrorCode::NotFound, "resource not found"))?;
    if resource.kind != project::ResourceKind::Material {
        return Err(invalid("the conductor role needs a material resource"));
    }
    let prefix = match resource.implementation.as_str() {
        "conductor" => "",
        "coatedconductor" => "conductor.",
        _ => {
            return Err(invalid(
                "the conductor colour needs a conductor or coatedconductor material",
            ));
        }
    };
    let declared = |name: &str| {
        resource
            .parameters
            .iter()
            .find(|parameter| parameter.name == name)
    };
    let reflectance = declared("reflectance");
    let eta = declared(&format!("{prefix}eta"));
    let k = declared(&format!("{prefix}k"));
    // r4 refuses the combination (conductor.rs): warn, show no colour.
    if reflectance.is_some() && (eta.is_some() || k.is_some()) {
        return Ok(answer(
            &p.project,
            &Resolved {
                rgb: GREY,
                source: WARN.into(),
            },
            spectrum::Role::Conductor,
        ));
    }
    // The coat's index: the interface eta, at one wavelength when
    // spectral (r4 samples one), 1.5 by default, 1 in place of a
    // non-positive read (r4 replaces a zero sample with one).
    let medium = if prefix.is_empty() {
        1.
    } else {
        match declared("interface.eta") {
            Some(parameter) => side_of("interface.eta", parameter, doc, active)
                .ok()
                .and_then(|(side, _)| {
                    side.value()
                        .and_then(|value| spectrum::curve_point(value, 550.))
                })
                .map(|eta| if eta > 0. { eta } else { 1. })
                .unwrap_or(1.5),
            None => 1.5,
        }
    };
    // The one reflectance form: the metal is its reflectance's colour.
    if let Some(reflectance) = reflectance
        && eta.is_none()
        && k.is_none()
    {
        let (side, source) = side_of("reflectance", reflectance, doc, active)?;
        if matches!(side, Side::Texture) {
            return Ok(answer(
                &p.project,
                &Resolved { rgb: GREY, source },
                spectrum::Role::Conductor,
            ));
        }
        let rgb = spectrum::reflectance_f0(
            side.value()
                .ok_or_else(|| invalid("the reflectance is not a colour"))?,
            medium,
        )
        .ok_or_else(|| invalid("the declared reflectance is not a colour"))?;
        return Ok(answer(
            &p.project,
            &Resolved { rgb, source },
            spectrum::Role::Conductor,
        ));
    }
    // eta/k: each named side resolves, a missing side fills with r4's
    // copper default; a texture side is not evaluated and answers grey.
    let side_or_fill = |name: &str,
                        parameter: Option<&project::Parameter>|
     -> Result<(Side, String), ErrorPayload> {
        // r4's conductor default for the missing side.
        let Some(parameter) = parameter else {
            let base = name.strip_prefix("conductor.").unwrap_or(name);
            return Ok((
                Side::Named(format!("metal-Cu-{base}")),
                format!("{name}: default (Cu)"),
            ));
        };
        side_of(name, parameter, doc, active)
    };
    let eta = side_or_fill(&format!("{prefix}eta"), eta)?;
    let k = side_or_fill(&format!("{prefix}k"), k)?;
    let eta_row = format!("{prefix}eta");
    let k_row = format!("{prefix}k");
    let mut sources = [eta.1, k.1];
    if matches!(eta.0, Side::Texture) || matches!(k.0, Side::Texture) {
        return Ok(answer(
            &p.project,
            &Resolved {
                rgb: GREY,
                source: sources.join(" · "),
            },
            spectrum::Role::Conductor,
        ));
    }
    // a name with neither a curve nor channel samples is
    // not a refractive index (an illuminant such as stdillum-D65, or a
    // typo): the neutral grey with a source saying so, never an error —
    // and Lit and the thumbnail shade the same grey, so all three agree.
    let mut grey = false;
    for (index, (side, row)) in [(&eta.0, &eta_row), (&k.0, &k_row)].into_iter().enumerate() {
        if let Side::Named(name) = side
            && spectrum::named_is_not_a_refractive_index(name)
        {
            sources[index] = format!("{row}: {name} is not a refractive index (not evaluated)");
            grey = true;
        }
    }
    if grey {
        return Ok(answer(
            &p.project,
            &Resolved {
                rgb: GREY,
                source: sources.join(" · "),
            },
            spectrum::Role::Conductor,
        ));
    }
    let rgb = spectrum::conductor_f0(
        eta.0
            .value()
            .ok_or_else(|| invalid("the declared eta is not a colour"))?,
        k.0.value()
            .ok_or_else(|| invalid("the declared k is not a colour"))?,
        medium,
    )
    .ok_or_else(|| invalid("the declared eta/k curves could not be read"))?;
    Ok(answer(
        &p.project,
        &Resolved {
            rgb,
            source: sources.join(" · "),
        },
        spectrum::Role::Conductor,
    ))
}

/// The `project.spectrum_color` answer for one pinned project state.
pub(super) fn resolve(
    doc: &project::ProjectDocument,
    active: &ActiveProject,
    p: &dto::SpectrumColorParams,
) -> Result<Value, ErrorPayload> {
    let role = match p.role.as_deref() {
        Some(text) => spectrum::Role::from_wire_str(text).ok_or_else(|| {
            invalid("role must be albedo, unbounded, illuminant, chromaticity or conductor")
        })?,
        None => default_role(doc, active, p)?,
    };
    // the conductor role is the material's Metal colour — its own
    // resolution, whichever metal row asks.
    if role == spectrum::Role::Conductor {
        return conductor_colour(doc, active, p);
    }
    match target_parameters(doc, active, &p.target)? {
        // A SpectrumData resource is the spectrum itself: its two tables
        // (either name) resolve the whole curve it holds.
        Target::SpectrumData(resource) => {
            if !matches!(p.parameter.as_str(), "values" | "wavelengths") {
                return Err(ErrorPayload::new(
                    ErrorCode::NotFound,
                    "parameter not found",
                ));
            }
            let resolved = curve_of(active, resource, role)?;
            Ok(answer(&p.project, &resolved, role))
        }
        Target::Parameters { parameters, .. } => {
            let parameter = parameters
                .iter()
                .find(|parameter| parameter.name == p.parameter)
                .ok_or_else(|| ErrorPayload::new(ErrorCode::NotFound, "parameter not found"))?;
            let resolved = resolved(parameter, doc, active, role)?;
            Ok(answer(&p.project, &resolved, role))
        }
    }
}

/// What a request's target resolves to on the document.
#[allow(clippy::large_enum_variant)]
enum Target<'a> {
    /// The target resource's parameters, with what the schema needs to
    /// know about them.
    Parameters {
        directive: &'static str,
        implementation: String,
        texture_value_type: Option<project::TextureValueType>,
        parameters: &'a [project::Parameter],
    },
    /// The target is the spectrum itself.
    SpectrumData(&'a project::Resource),
}

fn target_parameters<'a>(
    doc: &'a project::ProjectDocument,
    active: &ActiveProject,
    target: &dto::ParameterTarget,
) -> Result<Target<'a>, ErrorPayload> {
    use project::{ResourceKind as R, TextureValueType as T};
    let not_found = |what| ErrorPayload::new(ErrorCode::NotFound, format!("{what} not found"));
    match target {
        dto::ParameterTarget::Resource(id) => {
            let resource = doc
                .resources
                .get(&project::ResourceId(*id))
                .ok_or_else(|| not_found("resource"))?;
            if resource.kind == R::SpectrumData {
                return Ok(Target::SpectrumData(resource));
            }
            let directive = match resource.kind {
                R::Material => "MakeNamedMaterial",
                R::Texture => "Texture",
                R::Medium => "MakeNamedMedium",
                _ => return Err(invalid("this resource kind has no parameters")),
            };
            Ok(Target::Parameters {
                directive,
                implementation: resource.implementation.clone(),
                texture_value_type: resource.texture_value_type.map(|texture| match texture {
                    T::Float => project::TextureValueType::Float,
                    T::Spectrum => project::TextureValueType::Spectrum,
                }),
                parameters: &resource.parameters,
            })
        }
        dto::ParameterTarget::Node(id) => {
            use project::{ConfigurationKind as C, EntityKind as E, NodeKind};
            let view = doc
                .scene_view(active.session.active_scene())
                .map_err(|error| invalid(error.to_string()))?;
            let node = &view
                .node(project::NodeId(*id))
                .ok_or_else(|| not_found("node"))?
                .data;
            Ok(match node {
                NodeKind::Entity {
                    entity,
                    implementation,
                    parameters,
                    ..
                } => Target::Parameters {
                    directive: match entity {
                        E::Shape => "Shape",
                        E::Light => "LightSource",
                        E::AreaLight => "AreaLightSource",
                        E::Camera => "Camera",
                    },
                    implementation: implementation.clone(),
                    texture_value_type: None,
                    parameters,
                },
                NodeKind::Configuration {
                    configuration,
                    implementation,
                    parameters,
                } => Target::Parameters {
                    directive: match configuration {
                        C::Film => "Film",
                        C::Sampler => "Sampler",
                        C::Integrator => "Integrator",
                        C::PixelFilter => "PixelFilter",
                        C::Accelerator => "Accelerator",
                    },
                    implementation: implementation.clone(),
                    texture_value_type: None,
                    parameters,
                },
                _ => return Err(invalid("node does not declare parameters")),
            })
        }
    }
}

/// The role the schema gives the parameter, before the name heuristic.
pub(super) fn role_for_parameter(
    directive: &str,
    implementation: &str,
    texture_value_type: Option<project::TextureValueType>,
    parameter: &str,
) -> spectrum::Role {
    use pbrt_io::parameter_schema::{self as schema, DefaultValue, TextureInput};
    if let Some(spec) = schema::schema(directive)
        .and_then(|table| schema::implementation(table, implementation))
        .and_then(|found| found.parameters(texture_value_type))
        .and_then(|rows| {
            rows.iter()
                .find(|row| row.name == parameter || row.aliases.contains(&parameter))
        })
    {
        match spec.texture {
            Some(TextureInput::SpectrumAlbedo) => return spectrum::Role::Albedo,
            Some(TextureInput::SpectrumUnbounded) => return spectrum::Role::Unbounded,
            Some(TextureInput::SpectrumIlluminant) => return spectrum::Role::Illuminant,
            _ => {}
        }
        if matches!(spec.default, Some(DefaultValue::Illuminant)) {
            return spectrum::Role::Illuminant;
        }
    }
    matches!(
        parameter.to_ascii_lowercase().as_str(),
        "i" | "l" | "emission"
    )
    .then_some(spectrum::Role::Illuminant)
    .unwrap_or(spectrum::Role::Albedo)
}

/// The role a request left unstated: the schema's answer for a
/// parameter target, the chromaticity for a spectrum asked directly —
/// the curve's colour, whatever its values' magnitude.
fn default_role(
    doc: &project::ProjectDocument,
    active: &ActiveProject,
    p: &dto::SpectrumColorParams,
) -> Result<spectrum::Role, ErrorPayload> {
    Ok(match target_parameters(doc, active, &p.target)? {
        Target::SpectrumData(_) => spectrum::Role::Chromaticity,
        Target::Parameters {
            directive,
            implementation,
            texture_value_type,
            ..
        } => role_for_parameter(directive, &implementation, texture_value_type, &p.parameter),
    })
}

/// One parameter's current value as the swatch rgb and its source.
fn resolved(
    parameter: &project::Parameter,
    doc: &project::ProjectDocument,
    active: &ActiveProject,
    role: spectrum::Role,
) -> Result<Resolved, ErrorPayload> {
    let floats = |values: &[project::FloatValue]| -> Result<Vec<f64>, ErrorPayload> {
        values
            .iter()
            .map(|value| match value {
                project::FloatValue::Finite(number) if number.is_finite() => Ok(*number),
                _ => Err(invalid("the parameter holds non-finite values")),
            })
            .collect()
    };
    let kind = parameter.value_type.to_ascii_lowercase();
    match (&parameter.values, kind.as_str()) {
        // A blackbody is a temperature: the luminance-normalized colour,
        // spelled with its kelvin.
        (project::ParameterValues::Floats(values), "blackbody") => {
            let kelvin = floats(values)?
                .first()
                .copied()
                .ok_or_else(|| invalid("blackbody needs a temperature"))?;
            Ok(Resolved {
                rgb: for_role(spectrum::blackbody_rgb(kelvin), role),
                source: format!("blackbody {} K", number(kelvin)),
            })
        }
        // λ v pairs: the role's own read, the form spelled out.
        (project::ParameterValues::Floats(values), "spectrum") => {
            let pairs = floats(values)?;
            match spectrum::pairs_rgb(&pairs, role) {
                Some(rgb) => Ok(Resolved {
                    rgb,
                    source: format!("spectrum samples ({} pairs)", pairs.len() / 2),
                }),
                None => Err(invalid(
                    "spectrum samples must be λ value pairs (an even, non-empty count)",
                )),
            }
        }
        // A builtin name: the r4 table's values, or white like Lit when
        // the table has none for it; the chromaticity role reads the
        // name's own curve.
        (project::ParameterValues::BuiltinSpectra(names), _) => {
            let name = names
                .first()
                .ok_or_else(|| invalid("the parameter names no spectrum"))?;
            let rgb = if role == spectrum::Role::Chromaticity {
                spectrum::chromaticity(spectrum::Value::Named(name)).unwrap_or([1.; 3])
            } else {
                spectrum::named(name).unwrap_or([1.; 3])
            };
            Ok(Resolved {
                rgb,
                source: name.clone(),
            })
        }
        // A SpectrumData reference: the curve it shares, named by its
        // label.
        (
            project::ParameterValues::Resources {
                kind: project::ResourceKind::SpectrumData,
                ids,
            },
            _,
        ) => {
            let resource = doc
                .resources
                .get(
                    ids.first()
                        .ok_or_else(|| invalid("the parameter references no spectrum"))?,
                )
                .ok_or_else(|| ErrorPayload::new(ErrorCode::NotFound, "spectrum data not found"))?;
            curve_of(active, resource, role)
        }
        // A legacy spd asset (kept when migration could not read it):
        // white like Lit, saying the file it points to.
        (project::ParameterValues::Assets(assets), "spectrum") => {
            let location = assets
                .first()
                .and_then(|asset| doc.assets.get(asset))
                .map(|asset| asset.location.clone())
                .unwrap_or_else(|| "spectrum file".into());
            Ok(Resolved {
                rgb: [1.; 3],
                source: location,
            })
        }
        // Any other floats (an rgb triple, a stretched scalar): read as
        // the RGB the swatch shows.
        (project::ParameterValues::Floats(values), _) => {
            let values = floats(values)?;
            let first = values
                .first()
                .copied()
                .ok_or_else(|| invalid("the parameter holds no values"))?;
            let second = values.get(1).copied().unwrap_or(first);
            let third = values.get(2).copied().unwrap_or(second);
            // One value is a number (a dielectric's `float eta`), not a colour.
            let source = if values.len() == 1 {
                format!("float {}", number(first))
            } else {
                format!("rgb {} {} {}", number(first), number(second), number(third))
            };
            Ok(Resolved {
                rgb: for_role([first, second, third], role),
                source,
            })
        }
        _ => Err(invalid(
            "the parameter's value is not a spectrum, rgb or blackbody",
        )),
    }
}

/// One SpectrumData resource's curve as its rgb, its label the source.
fn curve_of(
    active: &ActiveProject,
    resource: &project::Resource,
    role: spectrum::Role,
) -> Result<Resolved, ErrorPayload> {
    let read = |asset| -> pbrt_io::Result<std::path::PathBuf> {
        let root = active.asset_root.as_deref().ok_or_else(|| {
            pbrt_io::Error::Invalid("this project keeps no data assets on disk".into())
        })?;
        active
            .session
            .snapshot()
            .folder_asset_path(std::path::Path::new(root), asset)
            .map_err(pbrt_io::Error::from)
    };
    let (wavelengths, values) =
        pbrt_io::spectrum_samples(resource, read).map_err(|error| invalid(error.to_string()))?;
    // One interleaved λ v list, the form pairs_rgb reads.
    let mut pairs = Vec::with_capacity(wavelengths.len() * 2);
    for (wavelength, value) in wavelengths.into_iter().zip(values) {
        pairs.push(wavelength);
        pairs.push(value);
    }
    let rgb = spectrum::pairs_rgb(&pairs, role)
        .ok_or_else(|| invalid("spectrum data holds no samples"))?;
    Ok(Resolved {
        rgb,
        source: format!("SpectrumData: {}", resource.label),
    })
}

/// A float a human reads: whole numbers without their trailing ".0".
pub(super) fn number(value: f64) -> String {
    if value == value.trunc() && value.is_finite() && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        value.to_string()
    }
}

/// The wire reply: the rgb, the source, the role, and the project echo
/// every project query answers with.
fn answer(project: &dto::ProjectRef, resolved: &Resolved, role: spectrum::Role) -> Value {
    json!({
        "project": project,
        "rgb": [resolved.rgb[0], resolved.rgb[1], resolved.rgb[2]],
        "source": resolved.source,
        "role": role.as_wire_str(),
    })
}
