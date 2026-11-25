use super::{
    GpuLight, LIGHT_DISTANT, LIGHT_ENVIRONMENT, LIGHT_INFINITE, LIGHT_POINT, LIGHT_SPOT, LitNote,
};
use protocol::geometry::{LightRecord, PreviewParameter};
use spectrum::Role;
use std::f64::consts::PI;

// The value conversions this module resolves with (blackbody, named
// spectra, λ/v pairs) live in the `spectrum` crate, so the Lit
// preview and the UI's colour boxes never disagree. The PreviewParameter
// plumbing — which parameter names a light spells its colour as — stays
// here.
pub use spectrum::blackbody_rgb;

/// World position of a positional light in f64. `GpuLight::position` is
/// f32 and loses precision far from the origin; the renderer subtracts the
/// eye from this value instead.
pub(crate) fn light_world_position(record: &LightRecord) -> [f64; 3] {
    match record.implementation.to_ascii_lowercase().as_str() {
        "point" | "spot" => transform_point(
            record.world_from_light,
            color(&record.parameters, "from", [0.; 3]),
        ),
        _ => [0.; 3],
    }
}

/// All image-based lights as one: `color` is their mean radiance.
pub(crate) fn environment_entry(average: [f32; 3]) -> GpuLight {
    GpuLight {
        position: [0.; 4],
        direction: [0.; 4],
        color: [average[0], average[1], average[2], 0.],
        kind: [LIGHT_ENVIRONMENT, 0, 0, 0],
    }
}

/// One light colour parameter (`L` on distant, infinite and area lights,
/// `I` on the rest) resolved to the linear RGB the GPU path shades with,
/// plus the reason of any approximation.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct LightColor {
    pub rgb: [f64; 3],
    pub degraded: Option<String>,
}

/// A light colour resolved as `light_entry` treats analytic lights: a
/// `blackbody` parameter is a temperature whose colour `blackbody_rgb`
/// gives at luminance 1; a `spectrum` parameter runs through the
/// `spectrum` crate's illuminant read (a builtin name, or λ v pairs read
/// at the channel wavelengths, as material spectra are); anything else
/// keeps reading its floats as RGB. A spectrum this build cannot resolve
/// — an spd file, a name without RGB values — becomes white and says so
/// in `degraded` for a LitNote; spd and curve data files stay white.
pub(super) fn light_color(
    params: &[PreviewParameter],
    names: &[&str],
    default: [f64; 3],
) -> LightColor {
    if let Some(kelvin) = blackbody_kelvin(params) {
        return LightColor {
            rgb: blackbody_rgb(kelvin),
            degraded: None,
        };
    }
    for name in names {
        if let Some(parameter) = params
            .iter()
            .find(|p| p.matches(name) && p.value_type.eq_ignore_ascii_case("spectrum"))
        {
            return spectrum_color(parameter);
        }
    }
    LightColor {
        rgb: color_at(params, names, default),
        degraded: None,
    }
}

/// The temperature a `blackbody` parameter carries, however the scene
/// spells it: the parameter's type, or a parameter named for it.
fn blackbody_kelvin(params: &[PreviewParameter]) -> Option<f64> {
    params
        .iter()
        .find(|p| {
            p.value_type.eq_ignore_ascii_case("blackbody")
                || p.name.to_ascii_lowercase().contains("blackbody")
        })
        .and_then(|p| p.floats.first())
        .copied()
}

/// A `spectrum` parameter at RGB: λ v pairs first, as pbrt orders them,
/// then a builtin name. What neither path resolves is white, with the
/// reason naming the spectrum for the LitNote.
fn spectrum_color(parameter: &PreviewParameter) -> LightColor {
    // A light is an illuminant: the channel
    // read the GPU path shades with, level as written.
    if let Some(rgb) = spectrum::pairs_rgb(&parameter.floats, Role::Illuminant) {
        return LightColor {
            rgb,
            degraded: None,
        };
    }
    if let Some(rgb) = parameter
        .texts
        .first()
        .and_then(|text| spectrum::named(text))
    {
        return LightColor {
            rgb,
            degraded: None,
        };
    }
    LightColor {
        rgb: [1.; 3],
        degraded: Some(format!(
            "light spectrum {}: approximated as white",
            parameter
                .texts
                .first()
                .map(String::as_str)
                .unwrap_or(&parameter.name),
        )),
    }
}

/// The floats of the first colour parameter carrying any, read as RGB as
/// the pre-spectral code read them.
fn color_at(params: &[PreviewParameter], names: &[&str], default: [f64; 3]) -> [f64; 3] {
    params
        .iter()
        .find(|p| names.iter().any(|n| p.matches(n)) && !p.floats.is_empty())
        .map_or(default, |v| {
            [
                v.floats[0],
                *v.floats.get(1).unwrap_or(&v.floats[0]),
                *v.floats
                    .get(2)
                    .unwrap_or(v.floats.get(1).unwrap_or(&v.floats[0])),
            ]
        })
}

/// The `I`/`L` colour of an analytic light: its colour parameter resolved
/// through `light_color`, with any degradation noted.
pub(super) fn intensity_color(record: &LightRecord, notes: &mut Vec<LitNote>) -> [f64; 3] {
    let resolved = light_color(&record.parameters, &["I"], [1.; 3]);
    if let Some(reason) = resolved.degraded {
        notes.push(LitNote {
            reason,
            material: None,
            draws: 0,
        });
    }
    resolved.rgb
}

pub(crate) fn light_entry(record: &LightRecord, notes: &mut Vec<LitNote>) -> Option<GpuLight> {
    let params = &record.parameters;
    let scale = scalar(params, "scale", 1.);
    let name = record.implementation.to_ascii_lowercase();
    let color_name = if name == "distant" || name == "infinite" {
        "L"
    } else {
        "I"
    };
    let resolved = light_color(params, &[color_name], [1.; 3]);
    if let Some(reason) = resolved.degraded {
        notes.push(LitNote {
            reason,
            material: None,
            draws: 0,
        });
    }
    let mut rgb = resolved.rgb;
    let power = scalar(params, "power", 0.);
    let mut position = [0.; 3];
    let mut direction = [0.; 3];
    let kind;
    match name.as_str() {
        "point" => {
            position = transform_point(record.world_from_light, color(params, "from", [0.; 3]));
            if power > 0. {
                rgb = rgb.map(|v| v * power / (4. * PI));
            }
            kind = LIGHT_POINT;
        }
        "spot" => {
            position = transform_point(record.world_from_light, color(params, "from", [0.; 3]));
            let cone = scalar(params, "coneangle", 30.).to_radians();
            let delta = scalar(params, "conedelta", 5.).to_radians();
            let cos_end = cone.cos();
            let cos_start = (cone - delta).cos();
            let ke = 2. * PI * ((1. - cos_start) + (cos_start - cos_end) / 2.);
            if power > 0. && ke > 0. {
                rgb = rgb.map(|v| v * power / ke);
            }
            let from = color(params, "from", [0.; 3]);
            let to = color(params, "to", [0., 0., 1.]);
            let axis = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
            direction = normalize(transform_vector(record.world_from_light, axis));
            kind = LIGHT_SPOT;
            rgb = rgb.map(|v| v * scale);
            return Some(GpuLight {
                position: [
                    position[0] as f32,
                    position[1] as f32,
                    position[2] as f32,
                    0.,
                ],
                direction: [
                    direction[0] as f32,
                    direction[1] as f32,
                    direction[2] as f32,
                    cos_start as f32,
                ],
                color: [rgb[0] as f32, rgb[1] as f32, rgb[2] as f32, cos_end as f32],
                kind: [kind, 0, 0, 0],
            });
        }
        "distant" => {
            let from = color(params, "from", [0.; 3]);
            let to = color(params, "to", [0., 0., 1.]);
            direction = normalize(transform_vector(
                record.world_from_light,
                [from[0] - to[0], from[1] - to[1], from[2] - to[2]],
            ));
            let illum = scalar(params, "illuminance", 0.);
            if illum > 0. {
                rgb = rgb.map(|v| v * illum);
            }
            kind = LIGHT_DISTANT;
        }
        "infinite" => {
            let illum = scalar(params, "illuminance", 0.);
            if illum > 0. {
                rgb = rgb.map(|v| v * illum / PI);
            }
            kind = LIGHT_INFINITE;
        }
        _ => return None,
    }
    rgb = rgb.map(|v| v * scale);
    Some(GpuLight {
        position: [
            position[0] as f32,
            position[1] as f32,
            position[2] as f32,
            0.,
        ],
        direction: [
            direction[0] as f32,
            direction[1] as f32,
            direction[2] as f32,
            0.,
        ],
        color: [rgb[0] as f32, rgb[1] as f32, rgb[2] as f32, 0.],
        kind: [kind, 0, 0, 0],
    })
}

pub(super) fn scalar(p: &[PreviewParameter], name: &str, default: f64) -> f64 {
    p.iter()
        .find(|v| v.matches(name))
        .and_then(|v| v.floats.first())
        .copied()
        .unwrap_or(default)
}
pub(super) fn color(p: &[PreviewParameter], name: &str, default: [f64; 3]) -> [f64; 3] {
    for v in p {
        if v.matches(name) && !v.floats.is_empty() {
            return [
                v.floats[0],
                *v.floats.get(1).unwrap_or(&v.floats[0]),
                *v.floats
                    .get(2)
                    .unwrap_or(v.floats.get(1).unwrap_or(&v.floats[0])),
            ];
        }
    }
    default
}
fn normalize(v: [f64; 3]) -> [f64; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 1e-20 {
        v.map(|x| x / l)
    } else {
        [0., 0., 1.]
    }
}
fn transform_point(m: [f64; 16], p: [f64; 3]) -> [f64; 3] {
    [
        m[0] * p[0] + m[1] * p[1] + m[2] * p[2] + m[3],
        m[4] * p[0] + m[5] * p[1] + m[6] * p[2] + m[7],
        m[8] * p[0] + m[9] * p[1] + m[10] * p[2] + m[11],
    ]
}
fn transform_vector(m: [f64; 16], p: [f64; 3]) -> [f64; 3] {
    [
        m[0] * p[0] + m[1] * p[1] + m[2] * p[2],
        m[4] * p[0] + m[5] * p[1] + m[6] * p[2],
        m[8] * p[0] + m[9] * p[1] + m[10] * p[2],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param(name: &str, kind: &str, floats: &[f64], texts: &[&str]) -> PreviewParameter {
        PreviewParameter {
            name: name.into(),
            value_type: kind.into(),
            floats: floats.to_vec(),
            texts: texts.iter().map(|t| (*t).into()).collect(),
            truncated: false,
        }
    }

    fn light_record(implementation: &str, parameters: Vec<PreviewParameter>) -> LightRecord {
        LightRecord {
            implementation: implementation.into(),
            world_from_light: [
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
            ],
            source: None,
            entity_id: None,
            parameters,
        }
    }

    #[test]
    fn blackbody_l_resolves_to_the_luminance_normalised_rgb() {
        let resolved = light_color(&[param("L", "blackbody", &[5500.], &[])], &["L"], [1.; 3]);
        assert_eq!(resolved.rgb, blackbody_rgb(5500.));
        assert_eq!(resolved.degraded, None);
    }

    #[test]
    fn rgb_l_floats_stay_as_they_were() {
        let resolved = light_color(
            &[param("L", "rgb", &[0.25, 0.5, 0.75], &[])],
            &["L"],
            [1.; 3],
        );
        assert_eq!(resolved.rgb, [0.25, 0.5, 0.75]);
        assert_eq!(resolved.degraded, None);
    }

    /// `scale` and `power` are the callers' business (frame_emitters,
    /// light_entry); resolving must not bake them in.
    #[test]
    fn scale_and_power_leave_the_resolved_colour_alone() {
        let resolved = light_color(
            &[
                param("L", "blackbody", &[5500.], &[]),
                param("scale", "float", &[0.1], &[]),
                param("power", "float", &[100.], &[]),
            ],
            &["L"],
            [1.; 3],
        );
        assert_eq!(resolved.rgb, blackbody_rgb(5500.));
    }

    /// An area light spells its fallback parameter `emission`, after `L`.
    #[test]
    fn the_emission_parameter_answers_when_l_is_absent() {
        let resolved = light_color(
            &[param("emission", "rgb", &[0.2, 0.4, 0.8], &[])],
            &["L", "emission"],
            [1.; 3],
        );
        assert_eq!(resolved.rgb, [0.2, 0.4, 0.8]);
        assert_eq!(resolved.degraded, None);
    }

    #[test]
    fn unresolvable_named_spectra_go_white_and_say_so() {
        let resolved = light_color(
            &[param("L", "spectrum", &[], &["stdillum-D65"])],
            &["L"],
            [1.; 3],
        );
        assert_eq!(resolved.rgb, [1.; 3]);
        assert_eq!(
            resolved.degraded.as_deref(),
            Some("light spectrum stdillum-D65: approximated as white")
        );
    }

    /// A spectrum typed with an odd number of floats is malformed (pbrt
    /// requires ascending λ v pairs): white, and named by its parameter.
    #[test]
    fn malformed_spectrum_floats_go_white_and_name_the_parameter() {
        let resolved = light_color(
            &[param("L", "spectrum", &[1., 2., 3.], &[])],
            &["L"],
            [1.; 3],
        );
        assert_eq!(resolved.rgb, [1.; 3]);
        assert_eq!(
            resolved.degraded.as_deref(),
            Some("light spectrum L: approximated as white")
        );
    }

    #[test]
    fn intensity_color_carries_the_degradation_into_the_notes() {
        let record = light_record("point", vec![param("I", "spectrum", &[], &["stdillum-F2"])]);
        let mut notes = Vec::new();
        let rgb = intensity_color(&record, &mut notes);
        assert_eq!(rgb, [1.; 3]);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].material, None);
        assert_eq!(notes[0].draws, 0);
        assert_eq!(
            notes[0].reason,
            "light spectrum stdillum-F2: approximated as white"
        );
    }

    #[test]
    fn intensity_color_keeps_resolving_blackbody() {
        let record = light_record("point", vec![param("I", "blackbody", &[4000.], &[])]);
        let mut notes = Vec::new();
        let rgb = intensity_color(&record, &mut notes);
        assert_eq!(rgb, blackbody_rgb(4000.));
        assert!(notes.is_empty());
    }
}
