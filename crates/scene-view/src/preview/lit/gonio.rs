//! CPU construction of goniometric and projection lights: point lights whose
//! intensity turns with the direction to the surface (r4 `lights/goniometric.rs`,
//! `lights/projection.rs`). Their control images decode like the environment's
//! maps and ride in shared texture-VM image slots; the per-light light frame —
//! the light_from_world rotation plus slot/resolution/screen data — appends to
//! the emitter vertex buffer, referenced by `kind.z` (the polygon lights'
//! scheme). Without a slot the light turns uniform: its mean modulation is
//! folded into `color` (goniometric) or returned from the frame (projection).
use super::lights::{intensity_color, scalar};
use super::texture_vm;
use super::{GpuLight, LIGHT_GONIOMETRIC, LIGHT_PROJECTION, LitNote, ProgramLinker};
use protocol::geometry::LightRecord;

/// `kind.w` word count for a goniometric light with a slotted image:
/// three matrix rows (slot/resolution in w) plus the mean-Y word.
pub(crate) const GONIOMETRIC_WORDS: usize = 4;
/// `kind.w` word count for a projection light with a slotted image:
/// three matrix rows (slot/resolution in w), the screen bounds,
/// [tan(fov/2), A], and the mean colour for the slotless fallback.
pub(crate) const PROJECTION_WORDS: usize = 6;
/// Marker slot for a decoded image that did not get an image slot.
const NO_IMAGE: u32 = u32::MAX;

/// v4 swaps the light's y and z axes so +y is up in the diagram.
const SWAP_YZ: [f64; 16] = [
    1., 0., 0., 0., //
    0., 0., 1., 0., //
    0., 1., 0., 0., //
    0., 0., 0., 1.,
];
/// The projection flips y: images have +y up, pbrt light space +z forward.
const FLIP_Y: [f64; 16] = [
    1., 0., 0., 0., //
    0., -1., 0., 0., //
    0., 0., 1., 0., //
    0., 0., 0., 1.,
];

/// The identity settings of a goniometric control map — its decode
/// reads no parameters beyond the bytes.
const GONIOMETRIC_IDENTITY: &str = "goniometric";
/// A projection control map's weighted mean depends on the fov, so its
/// identity carries the fov's bits.
fn projection_identity(fov: f64) -> String {
    format!("projection:{}", fov.to_bits())
}

/// The settings of the summary memo a control-map light's evaluate
/// side uses — the same strings the memo tables are keyed by, so the
/// renderer's residency query and evaluate agree on which bytes are
/// still needed (a projection's fov included).
pub(crate) fn control_summary_identity(record: &LightRecord) -> Option<String> {
    let params = &record.parameters;
    match record.implementation.to_ascii_lowercase().as_str() {
        "goniometric" if super::environment::image_name(record).is_some() => {
            Some(GONIOMETRIC_IDENTITY.into())
        }
        "projection" if super::environment::image_name(record).is_some() => {
            Some(projection_identity(scalar(params, "fov", 90.)))
        }
        _ => None,
    }
}

pub(crate) struct LightFrame {
    pub light: GpuLight,
    /// World position (world_from_light x origin), in f64 for the shadow
    /// frame like every positional light's.
    pub position: [f64; 3],
    /// The frame's vec4s: zero when the light has no image, else a
    /// `GONIOMETRIC_WORDS`/`PROJECTION_WORDS` run.
    pub words: Vec<[f32; 4]>,
}

fn mul(a: [f64; 16], b: [f64; 16]) -> [f64; 16] {
    let mut out = [0f64; 16];
    for row in 0..4 {
        for column in 0..4 {
            out[row * 4 + column] = (0..4).map(|k| a[row * 4 + k] * b[k * 4 + column]).sum();
        }
    }
    out
}

fn transform_point(m: [f64; 16], p: [f64; 3]) -> [f64; 3] {
    [
        m[0] * p[0] + m[1] * p[1] + m[2] * p[2] + m[3],
        m[4] * p[0] + m[5] * p[1] + m[6] * p[2] + m[7],
        m[8] * p[0] + m[9] * p[1] + m[10] * p[2] + m[11],
    ]
}

/// pbrt-v4's `SphereToSquare` (r4 `util/geometry/misc.rs`), the exact inverse
/// of `environment::square_to_sphere`. The shading port lives in WGSL; this
/// CPU mirror round-trips the formula in tests.
#[cfg(test)]
pub(crate) fn sphere_to_square(d: [f64; 3]) -> (f64, f64) {
    let (x, y, z) = (d[0].abs(), d[1].abs(), d[2].abs());
    let r = (1. - z).max(0.).sqrt();
    let a = x.max(y);
    let b = if a == 0. { 0. } else { x.min(y) / a };
    // 6th-degree minimax approximation of atan(b/a) * 2/pi (Clarberg),
    // r4's constants in r4's Horner order (4.190_388_180_291_658e-2 is
    // r4's 0.419_038_818_029_165_75e-1 at its shortest round-trip f64).
    let bb = b;
    let phi = (((((-0.251_390_972_343_483_53e-1 * bb + 4.190_388_180_291_658e-2) * bb
        + 0.881_770_664_775_316_3e-1)
        * bb
        - 0.247_333_733_281_268_94)
        * bb
        + 0.615_720_178_982_802_2e-2)
        * bb
        + 0.636_226_545_274_016_1)
        * bb
        + 0.406_758_566_246_788_5e-5;
    let phi = if x < y { 1. - phi } else { phi };
    let mut v = phi * r;
    let mut u = r - v;
    if d[2] < 0. {
        std::mem::swap(&mut u, &mut v);
        u = 1. - u;
        v = 1. - v;
    }
    (0.5 * (u.copysign(d[0]) + 1.), 0.5 * (v.copysign(d[1]) + 1.))
}

/// r4 `RGBColorSpace::luminance_vector` for sRGB.
fn luminance(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

/// r4 lights.cpp `GoniometricLight::Create`: scale *= power / (4pi * mean Y).
fn goniometric_power_mean(mean_y: f64, power: f64) -> f64 {
    power / (4. * std::f64::consts::PI * mean_y)
}

/// The screen of a projection light: extent from the image aspect and `A`, the
/// projected image area factor (r4 `projection_frame`).
fn projection_screen(width: u32, height: u32, fov: f64) -> ([f32; 4], f64, f64) {
    let aspect = width as f64 / height as f64;
    let bounds = if aspect > 1. {
        [-aspect, -1., aspect, 1.]
    } else {
        [-1., -1. / aspect, 1., 1. / aspect]
    };
    let tan_half = (0.5 * fov.to_radians()).tan();
    let a = 4. * tan_half * tan_half * aspect.max(1. / aspect);
    (
        [
            bounds[0] as f32,
            bounds[1] as f32,
            bounds[2] as f32,
            bounds[3] as f32,
        ],
        a,
        tan_half,
    )
}

/// cos^3 of the screen point's direction, r4 `screen_dwd_a`: the pinhole maps
/// (ps.x, ps.y) onto the ray (ps.x, ps.y, 1/tanHalf) in light space.
fn cos_cubed(ps: (f64, f64), tan_half: f64) -> f64 {
    (1. + tan_half * tan_half * (ps.0 * ps.0 + ps.1 * ps.1)).powf(-1.5)
}

fn image_name(record: &LightRecord) -> Option<String> {
    ["filename", "mapname"].iter().find_map(|name| {
        record
            .parameters
            .iter()
            .find(|p| p.matches(name))?
            .texts
            .first()
            .filter(|text| !text.is_empty())
            .cloned()
    })
}

fn note(notes: &mut Vec<LitNote>, reason: String) {
    notes.push(LitNote {
        reason,
        material: None,
        draws: 0,
    });
}

/// The scalars a control map's decode yields: `ImageMemos` stores them
/// per identity, never the decoded image itself. Projection maps pass the
/// fov — their cos-weighted mean depends on it — and reject empty or NaN
/// images; goniometric maps only need their means.
pub(crate) fn summarize_control_map(
    width: u32,
    height: u32,
    texels: &[[f32; 4]],
    fov: Option<f64>,
) -> Result<super::ControlMapSummary, super::ControlMapError> {
    if fov.is_some()
        && (width == 0
            || height == 0
            || texels
                .iter()
                .any(|t| t[0].is_nan() || t[1].is_nan() || t[2].is_nan()))
    {
        return Err(super::ControlMapError::EmptyOrNan);
    }
    let mut cos_weighted = 0f64;
    if let Some(fov) = fov {
        let (bounds, _a, tan_half) = projection_screen(width, height, fov);
        // r4 `projection_frame` + the phi integral: sum lum(rgb) cos^3.
        let mut pixel_sum = 0f64;
        for y in 0..height {
            for x in 0..width {
                let ps = (
                    bounds[0] as f64
                        + (bounds[2] - bounds[0]) as f64 * (x as f64 + 0.5) / width as f64,
                    bounds[1] as f64
                        + (bounds[3] - bounds[1]) as f64 * (y as f64 + 0.5) / height as f64,
                );
                let rgb = texels[(y * width + x) as usize];
                pixel_sum += luminance([rgb[0], rgb[1], rgb[2]]) as f64 * cos_cubed(ps, tan_half);
            }
        }
        cos_weighted = pixel_sum / (width as f64 * height as f64);
    }
    let mut sum_rgb = [0f64; 3];
    let mut sum_y = 0f64;
    for texel in texels {
        sum_y += (texel[0] + texel[1] + texel[2]) as f64 / 3.;
        for (channel, value) in texel.iter().take(3).enumerate() {
            sum_rgb[channel] += *value as f64;
        }
    }
    let count = texels.len() as f64;
    Ok(super::ControlMapSummary {
        width,
        height,
        mean_y: sum_y / count,
        mean_rgb: sum_rgb.map(|value| value / count),
        cos_weighted,
    })
}

/// A goniometric light: I(w) = scale * I * Y(equalAreaSphereToSquare(w_light))
/// with light space = world_from_light x swapYZ.
pub(crate) fn goniometric(
    record: &LightRecord,
    slot_key: &str,
    linker: &mut ProgramLinker<'_>,
    notes: &mut Vec<LitNote>,
) -> Option<LightFrame> {
    let params = &record.parameters;
    let mut scale = scalar(params, "scale", 1.);
    let rgb = intensity_color(record, notes);
    let mut mean_y = 1f64;
    let mut words = Vec::new();
    let name = image_name(record).unwrap_or_default();
    // The map's means come from the identity-keyed memo, so an unchanged
    // map never re-decodes inside evaluate.
    let mut decoded = None;
    if let Some(name) = (!name.is_empty()).then_some(name.clone()) {
        match linker.memos.control_map(
            slot_key,
            linker.manifest,
            linker.blobs,
            GONIOMETRIC_IDENTITY,
            None,
        ) {
            Ok(summary) => {
                if summary.width != summary.height {
                    note(
                        notes,
                        format!(
                            "goniometric light map {name}: non-square, which r4 rejects; using uniform I"
                        ),
                    );
                } else {
                    decoded = Some(summary);
                }
            }
            Err(error) => {
                note(
                    notes,
                    format!("goniometric light map {name}: {error}; using uniform I"),
                );
            }
        }
    }
    if let Some(summary) = &decoded {
        mean_y = summary.mean_y;
        let width = summary.width;
        let height = summary.height;
        let light_from_world = match texture_vm::invert(&mul(record.world_from_light, SWAP_YZ)) {
            Some(inverse) => inverse,
            None => {
                note(
                    notes,
                    "goniometric light transform is singular; using the identity rotation".into(),
                );
                IDENTITY
            }
        };
        match linker.link_light_image(slot_key) {
            Some(slot) => {
                for row in 0..3 {
                    let mut word = [
                        light_from_world[row * 4] as f32,
                        light_from_world[row * 4 + 1] as f32,
                        light_from_world[row * 4 + 2] as f32,
                        0.,
                    ];
                    word[3] = match row {
                        0 => slot as f32,
                        1 => width as f32,
                        _ => height as f32,
                    };
                    words.push(word);
                }
                words.push([mean_y as f32, 0., 0., 0.]);
                debug_assert_eq!(words.len(), GONIOMETRIC_WORDS);
            }
            None => {
                note(
                    notes,
                    format!(
                        "image slot budget exceeded: light control map {slot_key} replaced by its average"
                    ),
                );
            }
        }
    }
    let power = scalar(params, "power", 0.);
    if power > 0. {
        scale *= goniometric_power_mean(mean_y, power);
    }
    // Slot overflow: Y collapses to its mean; fold that into the intensity
    // and drop the words, leaving a plain positional light.
    let color = if decoded.is_some() && words.is_empty() {
        rgb.map(|v| v * scale * mean_y)
    } else {
        rgb.map(|v| v * scale)
    };
    let position = transform_point(record.world_from_light, [0.; 3]);
    Some(LightFrame {
        light: GpuLight {
            position: [
                position[0] as f32,
                position[1] as f32,
                position[2] as f32,
                0.,
            ],
            direction: [0., 0., 0., mean_y as f32],
            color: [color[0] as f32, color[1] as f32, color[2] as f32, 0.],
            kind: [LIGHT_GONIOMETRIC, 0, 0, words.len() as u32],
        },
        position,
        words,
    })
}

/// A projection light: I(w) = scale * rgb(nearest image texel at the screen
/// point the direction projects onto), light space = world_from_light x
/// Scale(1, -1, 1). Without a readable image r4 errors, so Lit notes and does
/// not draw the light.
pub(crate) fn projection(
    record: &LightRecord,
    slot_key: &str,
    linker: &mut ProgramLinker<'_>,
    notes: &mut Vec<LitNote>,
) -> Option<LightFrame> {
    let params = &record.parameters;
    let mut scale = scalar(params, "scale", 1.);
    let power = scalar(params, "power", 0.);
    let fov = scalar(params, "fov", 90.);
    let name = match image_name(record) {
        Some(name) => name,
        None => {
            note(
                notes,
                "projection light without a filename: the light is not drawn".into(),
            );
            return None;
        }
    };
    // The map's means come from the identity-keyed memo (fov included),
    // so an unchanged map never re-decodes inside evaluate.
    let summary = match linker.memos.control_map(
        slot_key,
        linker.manifest,
        linker.blobs,
        &projection_identity(fov),
        Some(fov),
    ) {
        Ok(summary) => summary,
        Err(super::ControlMapError::EmptyOrNan) => {
            note(
                notes,
                format!("projection light map {name}: empty or NaN; the light is not drawn"),
            );
            return None;
        }
        Err(error) => {
            note(
                notes,
                format!("projection light map {name}: {error}; the light is not drawn"),
            );
            return None;
        }
    };
    let (width, height) = (summary.width, summary.height);
    let (bounds, a, tan_half) = projection_screen(width, height, fov);
    let mean_cos_weighted = summary.cos_weighted;
    if power > 0. {
        scale *= power / (a * mean_cos_weighted);
    }
    let mean_rgb = summary.mean_rgb;
    let mut slot = NO_IMAGE;
    let mut warn_slot = false;
    match linker.link_light_image(slot_key) {
        Some(allocated) => slot = allocated,
        None => {
            note(
                notes,
                format!(
                    "image slot budget exceeded: light control map {slot_key} replaced by its average"
                ),
            );
            warn_slot = true;
        }
    }
    let light_from_world = match texture_vm::invert(&mul(record.world_from_light, FLIP_Y)) {
        Some(inverse) => inverse,
        None => {
            note(
                notes,
                "projection light transform is singular; the light is not drawn".into(),
            );
            return None;
        }
    };
    // A slotless projection flows like a mapless one: the mean folds into
    // the intensity and the frame words drop, so kind.w 0 shades uniform
    // instead of reading a slot the array does not hold.
    let mut words = Vec::with_capacity(PROJECTION_WORDS);
    if !warn_slot {
        for row in 0..3 {
            let mut word = [
                light_from_world[row * 4] as f32,
                light_from_world[row * 4 + 1] as f32,
                light_from_world[row * 4 + 2] as f32,
                0.,
            ];
            word[3] = match row {
                0 => slot as f32,
                1 => width as f32,
                _ => height as f32,
            };
            words.push(word);
        }
        words.push(bounds);
        words.push([tan_half as f32, a as f32, 0., 0.]);
        words.push([
            mean_rgb[0] as f32,
            mean_rgb[1] as f32,
            mean_rgb[2] as f32,
            0.,
        ]);
    }
    // Slot overflow folds the mean into the intensity and drops the map's
    // own colours, exactly like goniometric carries its mean Y.
    let color = if warn_slot {
        mean_rgb.map(|v| v * scale)
    } else {
        [scale, scale, scale]
    };
    let position = transform_point(record.world_from_light, [0.; 3]);
    Some(LightFrame {
        light: GpuLight {
            position: [
                position[0] as f32,
                position[1] as f32,
                position[2] as f32,
                0.,
            ],
            // A * mean(lum * cos^3) rides direction.w for choose_shadowed.
            direction: [0., 0., 0., (a * mean_cos_weighted) as f32],
            color: [color[0] as f32, color[1] as f32, color[2] as f32, 0.],
            kind: [LIGHT_PROJECTION, 0, 0, words.len() as u32],
        },
        position,
        words,
    })
}

const IDENTITY: [f64; 16] = [
    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
];

/// Fold the lights whose control map rode image slot `slot` back to the
/// uniform average (the renderer's device-budget fallback, the same
/// treatment a texture's average-color fallback gives materials):
/// goniometric multiplies by its mean Y, projection by its channel means,
/// and both drop their frame words — `kind.w` 0 shades them as plain
/// positional lights. Returns false when no framed light uses the slot.
pub(crate) fn demote_slot(lit: &mut super::LitScene, slot: u32, reason: &str) -> bool {
    let fold = |light: &mut GpuLight, light_frames: &[[f32; 4]], vertices: u32| -> bool {
        if !matches!(light.kind[0], LIGHT_GONIOMETRIC | LIGHT_PROJECTION) || light.kind[3] == 0 {
            return false;
        }
        let frame = light.kind[2].saturating_sub(vertices) as usize;
        let Some(first) = light_frames.get(frame) else {
            return false;
        };
        if (first[3] as u32) != slot {
            return false;
        }
        match light.kind[0] {
            LIGHT_GONIOMETRIC => {
                // Mean Y rides direction.w: uniform I = scale * I * meanY.
                let mean_y = light.direction[3];
                for channel in 0..3 {
                    light.color[channel] *= mean_y;
                }
            }
            _ => {
                // The projection frame ends with its channel means.
                let Some(word) = light_frames.get(frame + PROJECTION_WORDS - 1) else {
                    return false;
                };
                for (channel, mean) in light.color[..3].iter_mut().zip(&word[..3]) {
                    *channel *= mean;
                }
            }
        }
        light.kind[3] = 0;
        true
    };
    // `analytic` indexes `light_frames` directly; only the live `lights`
    // carry rebind's shift past the emitter vertices.
    let demoted_kind = lit.analytic.iter_mut().find_map(|light| {
        fold(light, &lit.light_frames, 0).then(|| {
            if light.kind[0] == LIGHT_GONIOMETRIC {
                "goniometric"
            } else {
                "projection"
            }
        })
    });
    // A later rebind copies `analytic` into `lights`, so both take the
    // fold; the note goes out once.
    for light in &mut lit.lights {
        fold(light, &lit.light_frames, lit.emitter_vertices.len() as u32);
    }
    match demoted_kind {
        Some(name) => {
            note(
                &mut lit.notes,
                format!("{reason}: {name} light map replaced by its average"),
            );
            true
        }
        // analytic can be empty in mid-construction states; lights alone
        // still count (and note through the demoted light's own kind).
        None => {
            let demoted = lit.lights.iter_mut().find_map(|light| {
                fold(light, &lit.light_frames, lit.emitter_vertices.len() as u32).then(|| {
                    if light.kind[0] == LIGHT_GONIOMETRIC {
                        "goniometric"
                    } else {
                        "projection"
                    }
                })
            });
            match demoted {
                Some(name) => {
                    note(
                        &mut lit.notes,
                        format!("{reason}: {name} light map replaced by its average"),
                    );
                    true
                }
                None => false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::lit::{LIGHT_GONIOMETRIC, LIGHT_PROJECTION};
    use protocol::geometry::LightRecord;
    use std::collections::BTreeMap;

    const IDENTITY: [f64; 16] = [
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ];

    fn light_record(
        implementation: &str,
        parameters: Vec<protocol::geometry::PreviewParameter>,
    ) -> LightRecord {
        LightRecord {
            implementation: implementation.into(),
            world_from_light: IDENTITY,
            source: None,
            entity_id: None,
            parameters,
        }
    }

    fn text(name: &str, value: &str) -> protocol::geometry::PreviewParameter {
        protocol::geometry::PreviewParameter {
            name: name.into(),
            value_type: "string".into(),
            floats: vec![],
            texts: vec![value.into()],
            truncated: false,
        }
    }

    fn float(name: &str, value: f64) -> protocol::geometry::PreviewParameter {
        protocol::geometry::PreviewParameter {
            name: name.into(),
            value_type: "float".into(),
            floats: vec![value],
            texts: vec![],
            truncated: false,
        }
    }

    fn png(rows: [[u8; 3]; 4]) -> Vec<u8> {
        // 2x2 sRGB png, rows top-to-bottom as given.
        let mut bytes = Vec::new();
        let image =
            image::RgbImage::from_fn(2, 2, |x, y| image::Rgb(rows[y as usize * 2 + x as usize]));
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageOutputFormat::Png,
            )
            .unwrap();
        bytes
    }

    fn srgb(byte: u8) -> f64 {
        let v = byte as f64 / 255.;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }

    fn linker(blobs: &BTreeMap<String, Vec<u8>>) -> crate::preview::lit::ProgramLinker<'_> {
        crate::preview::lit::ProgramLinker::for_tests(blobs)
    }

    /// Mean over pixel centres of cos^3 of their direction (r4's `phi`
    /// integrand for a constant map), mirroring r4's formula for the test
    /// below.
    fn cos_cubed_mean(width: u32, height: u32, tan_half: f64, bounds: [f32; 4]) -> f64 {
        let mut sum = 0.;
        for y in 0..height {
            for x in 0..width {
                let ps = (
                    bounds[0] as f64
                        + (bounds[2] - bounds[0]) as f64 * (x as f64 + 0.5) / width as f64,
                    bounds[1] as f64
                        + (bounds[3] - bounds[1]) as f64 * (y as f64 + 0.5) / height as f64,
                );
                sum += cos_cubed(ps, tan_half);
            }
        }
        sum / (width as f64 * height as f64)
    }

    fn linker_with_capacity(
        blobs: &BTreeMap<String, Vec<u8>>,
        capacity: usize,
    ) -> crate::preview::lit::ProgramLinker<'_> {
        crate::preview::lit::ProgramLinker::for_tests_with_capacity(blobs, capacity)
    }

    /// r4 `EqualAreaSphereToSquare` inverts pbrt's `EqualAreaSquareToSphere`
    /// to within the minimax polynomial's own error: |uv| ≤ 0.5 · 4.07e-6
    /// (the atan approximation at b/a → 1), since r4 uses the same
    /// polynomial. The points include the b/a → 1 ridge, the worst case.
    #[test]
    fn sphere_to_square_round_trips_pbrt_s_square() {
        for (u, v) in [
            (0.5, 0.5),
            (0.1, 0.9),
            (0.75, 0.25),
            (0.02, 0.5),
            (0.5, 0.99),
            (0.25, 0.1),
        ] {
            let direction = crate::preview::lit::environment::square_to_sphere(u, v);
            let (back_u, back_v) = sphere_to_square(direction);
            assert!(
                (back_u - u).abs() < 3e-6 && (back_v - v).abs() < 3e-6,
                "({u}, {v}) -> {direction:?} -> ({back_u}, {back_v})"
            );
        }
        // + z maps back to the square's centre, - z to a corner fold.
        let (u, v) = sphere_to_square([0., 0., 1.]);
        assert!((u - 0.5).abs() < 1e-9 && (v - 0.5).abs() < 1e-9);
    }

    /// The goniometric frame carries the world_from_light x swapYZ rotation
    /// (r4 v4 swaps y and z so image +y is up), the slot and the resolution,
    /// plus the image's mean Y for shadows and the power conversion.
    #[test]
    fn goniometric_frame_swaps_yz_and_carries_mean_y() {
        let mut blobs: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        blobs.insert(
            "light/0".to_string(),
            png([[10, 10, 10], [10, 10, 10], [200, 200, 200], [200, 200, 200]]),
        );
        let record = light_record(
            "goniometric",
            vec![text("filename", "map.png"), float("scale", 2.)],
        );
        let mut notes = Vec::new();
        let frame = goniometric(&record, "light/0", &mut linker(&blobs), &mut notes).unwrap();
        assert_eq!(frame.light.kind[0], LIGHT_GONIOMETRIC);
        assert!(frame.light.kind[3] == GONIOMETRIC_WORDS as u32);
        assert_eq!(frame.position, [0.; 3]);
        let mean_y = 0.5 * (srgb(10) as f32 + srgb(200) as f32);
        assert!(
            (frame.light.direction[3] - mean_y).abs() < 1e-4,
            "mean Y rides direction.w for choose_shadowed: {}",
            frame.light.direction[3]
        );
        assert!((frame.light.color[0] - 2.).abs() < 1e-6);
        // The inverse YZ swap sends world +y to light +z and world +z to light +y.
        let rows = [
            [frame.words[0][0], frame.words[0][1], frame.words[0][2]],
            [frame.words[1][0], frame.words[1][1], frame.words[1][2]],
            [frame.words[2][0], frame.words[2][1], frame.words[2][2]],
        ];
        let world_y: [f32; 3] = [0., 1., 0.];
        let out = [
            rows[0].iter().zip(world_y).map(|(a, b)| a * b).sum::<f32>(),
            rows[1].iter().zip(world_y).map(|(a, b)| a * b).sum::<f32>(),
            rows[2].iter().zip(world_y).map(|(a, b)| a * b).sum::<f32>(),
        ];
        assert!((out[0]).abs() < 1e-6 && (out[1]).abs() < 1e-6 && (out[2] - 1.).abs() < 1e-6);
        // Slot, width and height ride the rows' w components.
        assert_eq!(frame.words[0][3] as u32, 0);
        assert_eq!(frame.words[1][3] as u32, 2);
        assert_eq!(frame.words[2][3] as u32, 2);
        assert!(notes.is_empty(), "{notes:?}");
    }

    /// `power` rescales the intensity: scale *= power / (4pi * mean Y) — for a
    /// uniform Y of 0.5 and power 12, an I of 1 doubles.
    #[test]
    fn goniometric_power_converts_over_four_pi_mean_y() {
        let mut blobs: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        blobs.insert("light/0".to_string(), png([[128, 128, 128]; 4]));
        // sRGB 128 does not decode to 0.5; compute the true mean.
        let mean = srgb(128);
        let record = light_record(
            "goniometric",
            vec![text("filename", "map.png"), float("power", 12.)],
        );
        let frame = goniometric(&record, "light/0", &mut linker(&blobs), &mut Vec::new()).unwrap();
        let expected = 12. / (4. * std::f64::consts::PI * mean) as f32;
        assert!((frame.light.color[0] - expected).abs() < 1e-4);
    }

    /// Without an image the light stays uniform with Y = 1 (r4 warns and
    /// renders white); a non-square map is refused with a note, as r4 errors.
    #[test]
    fn goniometric_without_image_is_uniform_and_non_square_is_noted() {
        let blobs: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let record = light_record("goniometric", vec![float("scale", 3.)]);
        let mut notes = Vec::new();
        let frame = goniometric(&record, "light/0", &mut linker(&blobs), &mut notes).unwrap();
        assert_eq!(frame.light.kind[3], 0, "no frame words without a map");
        assert!((frame.light.color[0] - 3.).abs() < 1e-6);
        assert_eq!(frame.light.direction[3], 1., "uniform Y of 1");
        assert!(notes.is_empty(), "pbrt accepts a mapless goniometric light");

        let mut wide = BTreeMap::new();
        let image = image::RgbImage::from_fn(4, 2, |_, _| image::Rgb([100, 100, 100]));
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageOutputFormat::Png,
            )
            .unwrap();
        wide.insert("light/0".to_string(), bytes);
        let record = light_record("goniometric", vec![text("filename", "wide.png")]);
        let mut notes = Vec::new();
        let frame = goniometric(&record, "light/0", &mut linker(&wide), &mut notes).unwrap();
        assert_eq!(frame.light.kind[3], 0, "a non-square map is dropped");
        assert!(
            notes.iter().any(|n| n.reason.contains("non-square")),
            "{notes:?}"
        );
    }

    /// The projection frame holds the flipped-Y rotation, screen bounds from
    /// the image aspect and fov, tan(fov/2), A and the cos^3-weighted rgb
    /// mean for shadows and the power conversion.
    #[test]
    fn projection_frame_flips_y_and_reports_screen_data() {
        let mut blobs: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        // 2x1 wide image: aspect 2 -> screen [-2, 2] x [-1, 1].
        let image = image::RgbImage::from_fn(2, 1, |_, _| image::Rgb([255, 0, 0]));
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageOutputFormat::Png,
            )
            .unwrap();
        blobs.insert("light/0".to_string(), bytes);
        let record = light_record(
            "projection",
            vec![
                text("filename", "map.png"),
                float("scale", 2.),
                float("fov", 60.),
            ],
        );
        let mut notes = Vec::new();
        let frame = projection(&record, "light/0", &mut linker(&blobs), &mut notes).unwrap();
        assert_eq!(frame.light.kind[0], LIGHT_PROJECTION);
        assert!(frame.light.kind[3] == PROJECTION_WORDS as u32);
        assert_eq!(frame.position, [0.; 3]);
        // The inverse Y scale negates light-space y: world +y maps to light -y,
        // while x stays unchanged.
        let rows = [
            [frame.words[0][0], frame.words[0][1], frame.words[0][2]],
            [frame.words[1][0], frame.words[1][1], frame.words[1][2]],
        ];
        let light_x = rows[0]
            .iter()
            .zip([1f32, 0., 0.])
            .map(|(a, b)| a * b)
            .sum::<f32>();
        let light_y = rows[1]
            .iter()
            .zip([0f32, 1., 0.])
            .map(|(a, b)| a * b)
            .sum::<f32>();
        let light_z = rows[1]
            .iter()
            .zip([0f32, 0., 1.])
            .map(|(a, b)| a * b)
            .sum::<f32>();
        assert!((light_x - 1.).abs() < 1e-6, "x preserved");
        assert!((light_y + 1.).abs() < 1e-6, "y flips");
        assert!(light_z.abs() < 1e-6);
        assert_eq!(frame.words[0][3] as u32, 0, "slot");
        assert_eq!(frame.words[1][3] as u32, 2, "width");
        assert_eq!(frame.words[2][3] as u32, 1, "height");
        let bounds = frame.words[3];
        assert!((bounds[0] + 2.).abs() < 1e-5 && (bounds[1] + 1.).abs() < 1e-5);
        assert!((bounds[2] - 2.).abs() < 1e-5 && (bounds[3] - 1.).abs() < 1e-5);
        let tan_half = (60f64 / 2.).to_radians().tan() as f32;
        assert!((frame.words[4][0] - tan_half).abs() < 1e-5);
        let a = 4. * tan_half * tan_half * 2.;
        assert!((frame.words[4][1] - a).abs() < 1e-5);
        // direction.w carries A * mean(lum * cos^3) for choose_shadowed.
        assert!(frame.light.direction[3] > 0.);
        assert!((frame.light.color[0] - 2.).abs() < 1e-6, "scale only");
        assert!(notes.is_empty(), "{notes:?}");
    }

    /// r4's power conversion: scale *= power / (A * mean(rgb.lum * cos^3)),
    /// the cosine from each pixel centre's direction.
    #[test]
    fn projection_power_converts_over_a_times_cos_weighted_mean() {
        let mut blobs: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let image = image::RgbImage::from_fn(2, 1, |_, _| image::Rgb([255, 255, 255]));
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageOutputFormat::Png,
            )
            .unwrap();
        blobs.insert("light/0".to_string(), bytes);
        let record = light_record(
            "projection",
            vec![text("filename", "map.png"), float("power", 8.)],
        );
        let frame = projection(&record, "light/0", &mut linker(&blobs), &mut Vec::new()).unwrap();
        // fov defaults to 90 -> tan(45) = 1; A = 4 * tan^2 * max(aspect,1/aspect) = 8.
        let a = 4. * 1. * 2f64;
        // Uniform white pixels: mean(lum*cos^3) is cos^3 alone, over a
        // 2x1 image (screen [-2, 2] x [-1, 1] at fov 90).
        let mean = cos_cubed_mean(2, 1, 1., [-2., -1., 2., 1.]);
        let expected = 8. / (a * mean);
        assert!(
            (frame.light.color[0] as f64 - expected).abs() < 1e-3,
            "color {} vs expected {expected:.5} (A={a}, mean={mean:.5})",
            frame.light.color[0]
        );
    }

    /// Without a filename r4 errors; Lit notes and does not draw the light.
    #[test]
    fn projection_without_a_filename_is_noted_and_skipped() {
        let blobs: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let record = light_record("projection", vec![]);
        let mut notes = Vec::new();
        assert!(projection(&record, "light/0", &mut linker(&blobs), &mut notes).is_none());
        assert!(
            notes.iter().any(|n| n.reason.contains("filename")),
            "{notes:?}"
        );
    }

    /// A parsed projection light keeps its frame: the rotated inverse, the
    /// slot, the bounds and the mean rgb survive evaluate — the audit
    /// scene's half-turn about x turns r4's flip into a z flip.
    #[test]
    fn a_parsed_projection_light_keeps_its_frame() {
        let dir = tempfile::tempdir().unwrap();
        let gray = image::GrayImage::from_pixel(64, 64, image::Luma([125]));
        gray.save(dir.path().join("slide.png")).unwrap();
        let path = dir.path().join("probe.pbrt");
        std::fs::write(
            &path,
            "WorldBegin\nAttributeBegin\nTranslate 0 0 4\nRotate 180 1 0 0\nLightSource \"projection\" \"string filename\" \"slide.png\" \"float scale\" [100]\nAttributeEnd\n",
        )
        .unwrap();
        let scene = scene_parse::parse_scene(path, scene_parse::ParseOptions::default()).unwrap();
        assert!(scene.blobs.contains_key("light/0"));
        let lit = crate::preview::lit::evaluate(&scene.manifest, &scene.blobs);
        assert!(lit.notes.is_empty(), "framed, not demoted: {:?}", lit.notes);
        let light = &lit.lights[0];
        assert_eq!(light.kind[0], LIGHT_PROJECTION);
        assert_eq!(light.kind[3], PROJECTION_WORDS as u32);
        assert_eq!(light.kind[2], 0, "frame at the vertex-word origin");
        assert_eq!(light.position, [0., 0., 4., 0.]);
        assert_eq!(lit.light_frames[0][3] as u32, 0, "slot");
        assert_eq!(lit.light_frames[1][3] as u32, 64, "width");
        assert_eq!(lit.light_frames[2][3] as u32, 64, "height");
        // light_from_world = inverse(Translate Rotate_x(180) Flip(1,-1,1)):
        // the half-turn and the flip cancel along y, keep the z flip.
        let [x, y, z] = [
            lit.light_frames[0][0],
            lit.light_frames[1][1],
            lit.light_frames[2][2],
        ];
        assert!((x - 1.).abs() < 1e-6 && (y - 1.).abs() < 1e-6 && (z + 1.).abs() < 1e-6);
        assert_eq!(lit.light_frames[3], [-1., -1., 1., 1.], "screen bounds");
        assert!((lit.light_frames[4][0] - 1.).abs() < 1e-6, "tan(fov/2)");
        assert!((lit.light_frames[4][1] - 4.).abs() < 1e-5, "A");
        // The gray map's mean, for the slotless fallback: sRGB 125.
        let mean = lit.light_frames[5][0] as f64;
        let expected = ((125. / 255. + 0.055) / 1.055f64).powf(2.4);
        assert!(
            (mean - expected).abs() < 1e-4,
            "mean {mean} vs sRGB {expected}"
        );
        assert!(
            (light.color[0] - 100.).abs() < 1e-4,
            "slotted colors hold only the scale"
        );
    }

    /// The renderer's device-budget fallback folds a framed light to its
    /// uniform average in both the live list and `analytic` (so a rebind
    /// cannot resurrect the frame), with one note.
    #[test]
    fn demote_slot_folds_both_copies_and_notes_once() {
        let mut blobs: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        blobs.insert(
            "light/0".to_string(),
            png([[10, 10, 10], [100, 100, 100], [250, 250, 250], [37, 37, 37]]),
        );
        let mut manifest = protocol::geometry::GeometryManifest::default();
        manifest.lights.push(light_record(
            "goniometric",
            vec![text("filename", "map.png"), float("scale", 1.)],
        ));
        let mut lit = crate::preview::lit::evaluate(&manifest, &blobs);
        assert_eq!(lit.lights[0].kind[3], GONIOMETRIC_WORDS as u32);
        let mean_y: f32 =
            (srgb(10) as f32 + srgb(100) as f32 + srgb(250) as f32 + srgb(37) as f32) / 4.;
        assert!(demote_slot(&mut lit, 0, "test budget"), "slot 0 folds");
        assert_eq!(lit.lights[0].kind[3], 0, "live list uniform");
        assert_eq!(lit.analytic[0].kind[3], 0, "analytic uniform");
        // The fold multiplies into the intensity; scale was 1.
        assert!(
            (lit.lights[0].color[0] - mean_y).abs() < 1e-5,
            "{} vs {mean_y}",
            lit.lights[0].color[0]
        );
        assert_eq!(
            lit.notes
                .iter()
                .filter(|n| n.reason.contains("replaced by its average"))
                .count(),
            1,
            "{:?}",
            lit.notes
        );
        assert!(
            lit.notes
                .iter()
                .any(|n| n.reason.contains("goniometric light map"))
        );
        // Unknown slots and empty scenes demote nothing.
        assert!(!demote_slot(&mut lit, 7, "test budget"));
    }

    /// With polygon-emitter vertices in the buffer only the live lights carry
    /// rebind's shift; the analytic copy must still find its own frame.
    #[test]
    fn demote_slot_finds_the_analytic_frame_past_emitter_vertices() {
        let mut blobs: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for index in 0..2 {
            blobs.insert(
                format!("light/{index}"),
                png([[10, 10, 10], [100, 100, 100], [250, 250, 250], [37, 37, 37]]),
            );
        }
        let mut manifest = protocol::geometry::GeometryManifest::default();
        for _ in 0..2 {
            manifest.lights.push(light_record(
                "goniometric",
                vec![text("filename", "map.png")],
            ));
        }
        let mut lit = crate::preview::lit::evaluate(&manifest, &blobs);
        // What rebind does with one triangle emitter.
        lit.emitter_vertices = vec![[0.; 3]; 3];
        for light in &mut lit.lights {
            light.kind[2] += 3;
        }
        assert!(demote_slot(&mut lit, 1, "test budget"));
        assert_eq!(lit.analytic[1].kind[3], 0, "analytic copy demoted");
        assert_eq!(lit.lights[1].kind[3], 0, "live copy demoted");
        assert_eq!(lit.analytic[0].kind[3], GONIOMETRIC_WORDS as u32);
    }

    /// A projection light that misses the slot budget folds its mean into
    /// the intensity and drops its frame words, exactly like a mapless
    /// light: `kind.w` 0 shades it as a uniform positional light, never a
    /// frame whose slot word would read past the image array.
    #[test]
    fn projection_slot_overflow_folds_the_mean_and_drops_the_frame() {
        let mut blobs: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        blobs.insert(
            "light/0".to_string(),
            png([[10, 10, 10], [100, 100, 100], [250, 250, 250], [37, 37, 37]]),
        );
        let record = light_record(
            "projection",
            vec![text("filename", "map.png"), float("scale", 2.)],
        );
        let mut notes = Vec::new();
        let frame = projection(
            &record,
            "light/0",
            &mut linker_with_capacity(&blobs, 0),
            &mut notes,
        )
        .unwrap();
        assert_eq!(frame.light.kind[3], 0, "no frame words without a slot");
        assert!(frame.words.is_empty(), "the frame is dropped, not parked");
        // The four texels' grays: their sRGB mean, per channel, times scale.
        let grays = [10u8, 100, 250, 37];
        let mean = grays.iter().map(|v| srgb(*v)).sum::<f64>() / grays.len() as f64;
        let expected = (mean * 2.) as f32;
        for channel in 0..3 {
            assert!(
                (frame.light.color[channel] - expected).abs() < 1e-3,
                "channel {channel}: {} vs {expected}",
                frame.light.color[channel]
            );
        }
        assert!(
            notes.iter().any(|n| n.reason.contains("image slot")),
            "{notes:?}"
        );
    }

    /// A slot budget failure leaves the light uniform and notes it, the same
    /// fallback as the material textures' average colour.
    #[test]
    fn slot_overflow_folds_the_mean_and_notes() {
        let mut blobs: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        blobs.insert(
            "light/0".to_string(),
            png([[64, 64, 64], [192, 192, 192], [64, 64, 64], [192, 192, 192]]),
        );
        let record = light_record(
            "goniometric",
            vec![text("filename", "map.png"), float("scale", 1.)],
        );
        let mut notes = Vec::new();
        let frame = goniometric(
            &record,
            "light/0",
            &mut linker_with_capacity(&blobs, 0),
            &mut notes,
        )
        .unwrap();
        assert!(frame.light.kind[3] == 0, "no frame words without a slot");
        let mean = (srgb(64) + srgb(192)) / 2.;
        assert!(
            (frame.light.color[0] as f64 - mean).abs() < 1e-4,
            "mean Y folds into color: {} vs {mean}",
            frame.light.color[0]
        );
        assert!(
            notes.iter().any(|n| n.reason.contains("image slot")),
            "{notes:?}"
        );
        // direction.w is the f32 rounding of the fold's mean Y.
        assert!((frame.light.direction[3] as f64 - mean).abs() < 1e-6);
    }
}
