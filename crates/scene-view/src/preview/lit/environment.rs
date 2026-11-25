//! Image-based `infinite` lights. All of them bake into one world-space cube
//! map for the background and specular, and one set of spherical harmonics for
//! the diffuse irradiance, so that the shader has one environment to read.
use super::{LitNote, texture_vm};
use protocol::geometry::{GeometryManifest, LightRecord, PreviewParameter, TextureRecord};
use std::collections::BTreeMap;
use std::f64::consts::PI;

/// Equal-area maps need no solid-angle weights, so box filtering keeps the energy.
const MAX_IMAGE_SIZE: u32 = 1024;
const MAX_HARMONICS_SIZE: u32 = 256;
const HARMONICS_GRID: u32 = 256;

pub(crate) struct EnvironmentImage {
    pub width: u32,
    pub height: u32,
    pub texels: Vec<[f32; 4]>,
    /// Row-major, for the cube bake's direction lookup.
    pub light_from_world: [f64; 16],
    pub scale: f32,
}

pub(crate) struct Environment {
    pub images: Vec<EnvironmentImage>,
    /// Coefficients of the irradiance, the cosine convolution already applied.
    pub irradiance: [[f32; 4]; 9],
    /// Mean radiance.
    pub average: [f32; 3],
}

/// The blob name of the light at `index` in the manifest.
fn blob_name(index: usize) -> String {
    format!("light/{index}")
}

fn float(parameters: &[PreviewParameter], name: &str, default: f64) -> f64 {
    parameters
        .iter()
        .find(|p| p.matches(name))
        .and_then(|p| p.floats.first())
        .copied()
        .unwrap_or(default)
}

/// The lights that read an image, as r4 does: `filename`, or else
/// `mapname`. The manifest-side residency query needs the
/// same test, and `lit::environment` re-exports it.
pub(crate) fn image_name(record: &LightRecord) -> Option<&str> {
    ["filename", "mapname"].iter().find_map(|name| {
        record
            .parameters
            .iter()
            .find(|p| p.matches(name))?
            .texts
            .first()
            .map(String::as_str)
            .filter(|text| !text.is_empty())
    })
}

/// Everything `from_manifest` reads, one entry per image-based light: its
/// index, blob sha256 (None when absent), transform and parameters. Equal
/// inputs give an equal environment, so the renderer skips decode and bake.
pub(crate) fn inputs(manifest: &GeometryManifest) -> Vec<(usize, Option<String>, String)> {
    manifest
        .lights
        .iter()
        .enumerate()
        .filter(|(_, record)| {
            record.implementation.eq_ignore_ascii_case("infinite") && image_name(record).is_some()
        })
        .map(|(index, record)| {
            let sha256 = manifest
                .blobs
                .get(&blob_name(index))
                .map(|file| file.sha256.clone());
            let settings = format!("{:?}{:?}", record.world_from_light, record.parameters);
            (index, sha256, settings)
        })
        .collect()
}

/// The lights whose images were read, and the environment they make. Lights
/// left out stay uniform, with a note.
pub(crate) fn from_manifest(
    manifest: &GeometryManifest,
    blobs: &BTreeMap<String, Vec<u8>>,
    notes: &mut Vec<LitNote>,
) -> (Vec<usize>, Option<Environment>) {
    let mut used = Vec::new();
    let mut images = Vec::new();
    let mut note = |reason: String| {
        notes.push(LitNote {
            reason,
            material: None,
            draws: 0,
        });
    };
    for (index, record) in manifest.lights.iter().enumerate() {
        if !record.implementation.eq_ignore_ascii_case("infinite") {
            continue;
        }
        let Some(name) = image_name(record) else {
            continue;
        };
        let image = blobs
            .get(&blob_name(index))
            .ok_or_else(|| "its file was not found".to_string())
            .and_then(|bytes| decode(bytes))
            .and_then(|(width, height, texels)| {
                let light_from_world = texture_vm::invert(&record.world_from_light)
                    .ok_or_else(|| "its transform is singular".to_string())?;
                let illuminance = float(&record.parameters, "illuminance", -1.);
                let scale = float(&record.parameters, "scale", 1.)
                    * if illuminance > 0. {
                        illuminance / PI
                    } else {
                        1.
                    };
                Ok(EnvironmentImage {
                    width,
                    height,
                    texels,
                    light_from_world,
                    scale: scale as f32,
                })
            });
        match image {
            Ok(image) => {
                used.push(index);
                images.push(image);
                if record.parameters.iter().any(|p| p.matches("portal")) {
                    note("infinite light portal: ignored, the whole map lights the scene".into());
                }
            }
            Err(error) => {
                note(format!(
                    "infinite light map {name}: {error}; using uniform L"
                ));
            }
        }
    }
    if images.is_empty() {
        return (used, None);
    }
    note("image-based light: no shadows, only screen-space AO".into());
    let (irradiance, average) = harmonics(&images);
    let environment = Environment {
        images,
        irradiance,
        average,
    };
    (used, Some(environment))
}

/// As r4: EXR and other float formats are linear, the rest sRGB; negative
/// values clamp to zero. Larger images box down, which keeps their energy.
pub(crate) fn decode(bytes: &[u8]) -> Result<(u32, u32, Vec<[f32; 4]>), String> {
    let record = TextureRecord {
        name: String::new(),
        value_type: "spectrum".into(),
        implementation: "imagemap".into(),
        world_from_texture: std::array::from_fn(|i| if i % 5 == 0 { 1. } else { 0. }),
        source: None,
        parameters: vec![PreviewParameter {
            name: "encoding".into(),
            value_type: "string".into(),
            floats: vec![],
            texts: vec!["sRGB".into()],
            truncated: false,
        }],
    };
    let image = super::decode_texture(&record, bytes)?;
    let (width, height) = image.dimensions();
    let texels = image
        .pixels()
        .map(|p| [p[0].max(0.), p[1].max(0.), p[2].max(0.), 1.])
        .collect::<Vec<_>>();
    Ok(shrink(width, height, texels, MAX_IMAGE_SIZE))
}

/// Box-filter down to fit `limit`. Destination texels cover equal source areas,
/// with fractional coverage at their edges, so the map's mean stays exact.
fn shrink(width: u32, height: u32, texels: Vec<[f32; 4]>, limit: u32) -> (u32, u32, Vec<[f32; 4]>) {
    let factor = width.max(height).div_ceil(limit).max(1);
    if factor == 1 {
        return (width, height, texels);
    }
    let (small_width, small_height) = (width.div_ceil(factor), height.div_ceil(factor));
    // Per destination texel along an axis: the source texels it overlaps.
    let spans = |from: u32, to: u32| -> Vec<Vec<(usize, f64)>> {
        let step = from as f64 / to as f64;
        (0..to)
            .map(|i| {
                let (start, end) = (i as f64 * step, (i + 1) as f64 * step);
                (start.floor() as u32..(end.ceil() as u32).min(from))
                    .map(|s| {
                        let overlap = end.min(s as f64 + 1.) - start.max(s as f64);
                        (s as usize, overlap / step)
                    })
                    .collect()
            })
            .collect()
    };
    let (columns, rows) = (spans(width, small_width), spans(height, small_height));
    let mut small = Vec::with_capacity((small_width * small_height) as usize);
    for row in &rows {
        for column in &columns {
            let mut sum = [0f64; 3];
            for (y, row_weight) in row {
                for (x, column_weight) in column {
                    let texel = texels[y * width as usize + x];
                    for c in 0..3 {
                        sum[c] += texel[c] as f64 * row_weight * column_weight;
                    }
                }
            }
            small.push([sum[0] as f32, sum[1] as f32, sum[2] as f32, 1.]);
        }
    }
    (small_width, small_height, small)
}

/// pbrt's `EqualAreaSquareToSphere`.
pub(crate) fn square_to_sphere(u: f64, v: f64) -> [f64; 3] {
    let (u, v) = (2. * u - 1., 2. * v - 1.);
    let (up, vp) = (u.abs(), v.abs());
    let signed = 1. - (up + vp);
    let r = 1. - signed.abs();
    let phi = if r == 0. { 1. } else { (vp - up) / r + 1. } * (PI / 4.);
    let z = (1. - r * r).copysign(signed);
    let root = (2. - r * r).max(0.).sqrt();
    [
        phi.cos().copysign(u) * r * root,
        phi.sin().copysign(v) * r * root,
        z,
    ]
}

/// Real spherical harmonics through l = 2 at a unit direction.
fn basis([x, y, z]: [f64; 3]) -> [f64; 9] {
    [
        0.282095,
        0.488603 * y,
        0.488603 * z,
        0.488603 * x,
        1.092548 * x * y,
        1.092548 * y * z,
        0.315392 * (3. * z * z - 1.),
        1.092548 * x * z,
        0.546274 * (x * x - y * y),
    ]
}

/// Cosine-convolution factors A_l by band, per basis function above.
const COSINE: [f64; 9] = [
    PI,
    2. * PI / 3.,
    2. * PI / 3.,
    2. * PI / 3.,
    PI / 4.,
    PI / 4.,
    PI / 4.,
    PI / 4.,
    PI / 4.,
];

/// Every point of an equal-area map covers the same solid angle, so projecting
/// one is a plain sum over a uniform grid.
fn harmonics(images: &[EnvironmentImage]) -> ([[f32; 4]; 9], [f32; 3]) {
    let mut coefficients = [[0f64; 3]; 9];
    let mut average = [0f64; 3];
    for image in images {
        let (width, height, texels) = shrink(
            image.width,
            image.height,
            image.texels.clone(),
            MAX_HARMONICS_SIZE,
        );
        let world_from_light = texture_vm::invert(&image.light_from_world).unwrap_or(IDENTITY);
        let count = (width * height) as f64;
        for texel in &texels {
            for c in 0..3 {
                average[c] += texel[c] as f64 * image.scale as f64 / count;
            }
        }
        // A fixed grid, so that a map of few texels still integrates their whole
        // area rather than concentrating each at its center.
        let solid_angle = 4. * PI / (HARMONICS_GRID * HARMONICS_GRID) as f64;
        for y in 0..HARMONICS_GRID {
            let v = (y as f64 + 0.5) / HARMONICS_GRID as f64;
            let row = (v * height as f64) as u32;
            for x in 0..HARMONICS_GRID {
                let u = (x as f64 + 0.5) / HARMONICS_GRID as f64;
                let column = (u * width as f64) as u32;
                let world = rotate(&world_from_light, square_to_sphere(u, v));
                let weights = basis(world);
                let texel = texels[(row * width + column) as usize];
                for c in 0..3 {
                    let radiance = texel[c] as f64 * image.scale as f64;
                    for (coefficient, weight) in coefficients.iter_mut().zip(weights) {
                        coefficient[c] += radiance * weight * solid_angle;
                    }
                }
            }
        }
    }
    let irradiance = std::array::from_fn(|i| {
        let [r, g, b] = coefficients[i];
        [
            (r * COSINE[i]) as f32,
            (g * COSINE[i]) as f32,
            (b * COSINE[i]) as f32,
            0.,
        ]
    });
    (irradiance, average.map(|v| v as f32))
}

const IDENTITY: [f64; 16] = [
    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
];

fn rotate(m: &[f64; 16], v: [f64; 3]) -> [f64; 3] {
    let out: [f64; 3] = std::array::from_fn(|r| (0..3).map(|c| m[r * 4 + c] * v[c]).sum::<f64>());
    let length = out.iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-12);
    out.map(|x| x / length)
}

/// What the shader's `environment_irradiance` computes.
#[cfg(test)]
fn irradiance_at(irradiance: &[[f32; 4]; 9], n: [f64; 3]) -> [f64; 3] {
    let weights = basis(n);
    std::array::from_fn(|c| {
        (0..9)
            .map(|i| irradiance[i][c] as f64 * weights[i])
            .sum::<f64>()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(
        size: u32,
        texel: impl Fn(f64, f64, f64) -> f32,
        light_from_world: [f64; 16],
    ) -> EnvironmentImage {
        let texels = (0..size * size)
            .map(|i| {
                let d = square_to_sphere(
                    ((i % size) as f64 + 0.5) / size as f64,
                    ((i / size) as f64 + 0.5) / size as f64,
                );
                let v = texel(d[0], d[1], d[2]);
                [v, v, v, 1.]
            })
            .collect();
        EnvironmentImage {
            width: size,
            height: size,
            texels,
            light_from_world,
            scale: 1.,
        }
    }

    #[test]
    fn a_constant_map_gives_the_uniform_light_irradiance() {
        let (sh, average) = harmonics(&[image(64, |_, _, _| 0.5, IDENTITY)]);
        assert!((average[0] - 0.5).abs() < 1e-5);
        for n in [[0., 0., 1.], [1., 0., 0.], [0., -1., 0.]] {
            let e = irradiance_at(&sh, n)[0];
            assert!((e - 0.5 * PI).abs() < 1e-3, "{n:?}: {e}");
        }
    }

    /// A linear radiance L = max(z, 0) has an exact irradiance through band 2:
    /// E(+z) = 2π/3 at the pole, from ∫ cos²θ dω over the upper hemisphere.
    #[test]
    fn a_smooth_map_matches_the_analytic_irradiance() {
        let (sh, _) = harmonics(&[image(128, |_, _, z| z.max(0.) as f32, IDENTITY)]);
        let up = irradiance_at(&sh, [0., 0., 1.])[0];
        assert!((up - 2. * PI / 3.).abs() < 0.05, "{up}");
        let down = irradiance_at(&sh, [0., 0., -1.])[0];
        assert!(down.abs() < 0.08, "{down}");
    }

    #[test]
    fn the_light_transform_turns_the_irradiance() {
        // Light z lands on world x: a rotation by +90 degrees about y.
        let world_from_light = [
            0., 0., 1., 0., 0., 1., 0., 0., -1., 0., 0., 0., 0., 0., 0., 1.,
        ];
        let light_from_world = texture_vm::invert(&world_from_light).unwrap();
        let (sh, _) = harmonics(&[image(64, |_, _, z| z.max(0.) as f32, light_from_world)]);
        let lit = irradiance_at(&sh, [1., 0., 0.])[0];
        let dark = irradiance_at(&sh, [-1., 0., 0.])[0];
        assert!(lit > 1.8 && dark.abs() < 0.1, "{lit} {dark}");
    }

    #[test]
    fn shrinking_keeps_the_energy() {
        let texels: Vec<[f32; 4]> = (0..64u32).map(|i| [i as f32, 0., 0., 1.]).collect();
        let mean = texels.iter().map(|t| t[0]).sum::<f32>() / 64.;
        let (w, h, small) = shrink(8, 8, texels, 2);
        assert_eq!((w, h), (2, 2));
        let small_mean = small.iter().map(|t| t[0]).sum::<f32>() / 4.;
        assert_eq!(small_mean, mean);
    }

    /// Sizes that do not divide evenly keep the mean, whichever texel is bright.
    #[test]
    fn shrinking_uneven_sizes_keeps_a_corner_highlight_in_proportion() {
        for bright in [0usize, 257 * 257 - 1, 257 * 100 + 37] {
            let mut texels = vec![[0f32, 0., 0., 1.]; 257 * 257];
            texels[bright][0] = 1000.;
            let mean = 1000. / (257. * 257.);
            let (w, h, small) = shrink(257, 257, texels, 128);
            let small_mean = small.iter().map(|t| t[0]).sum::<f32>() / (w * h) as f32;
            assert!(
                (small_mean / mean - 1.).abs() < 1e-3,
                "{bright}: {small_mean} vs {mean}"
            );
        }
    }

    /// A low-resolution map covers its whole area, not its texel centers.
    #[test]
    fn low_resolution_maps_light_like_their_background() {
        for size in [1, 2, 3] {
            let texels = vec![[1f32, 1., 1., 1.]; size * size];
            let single = EnvironmentImage {
                width: size as u32,
                height: size as u32,
                texels,
                light_from_world: IDENTITY,
                scale: 1.,
            };
            let (sh, average) = harmonics(&[single]);
            assert!((average[0] - 1.).abs() < 1e-5);
            for n in [[0., 0., 1.], [0., 0., -1.], [1., 0., 0.], [0., 1., 0.]] {
                let e = irradiance_at(&sh, n)[0];
                assert!((e - PI).abs() < 0.03, "{size}: {n:?}: {e}");
            }
        }
    }

    #[test]
    fn image_lights_note_their_missing_shadows_and_portal() {
        let mut png = Vec::new();
        image::RgbImage::from_pixel(2, 2, image::Rgb([128, 128, 128]))
            .write_to(
                &mut std::io::Cursor::new(&mut png),
                image::ImageOutputFormat::Png,
            )
            .unwrap();
        let mut manifest = GeometryManifest::default();
        manifest.lights.push(LightRecord {
            implementation: "infinite".into(),
            world_from_light: IDENTITY,
            source: None,
            entity_id: None,
            parameters: vec![
                PreviewParameter {
                    name: "filename".into(),
                    value_type: "string".into(),
                    floats: vec![],
                    texts: vec!["env.png".into()],
                    truncated: false,
                },
                PreviewParameter {
                    name: "portal".into(),
                    value_type: "point3".into(),
                    floats: vec![0.; 12],
                    texts: vec![],
                    truncated: false,
                },
            ],
        });
        let blobs = BTreeMap::from([("light/0".to_string(), png)]);
        let mut notes = Vec::new();
        let (used, environment) = from_manifest(&manifest, &blobs, &mut notes);
        assert_eq!(used, [0]);
        assert!(environment.is_some());
        let reasons: Vec<_> = notes.iter().map(|n| n.reason.as_str()).collect();
        assert!(
            reasons.iter().any(|r| r.contains("no shadows")),
            "{reasons:?}"
        );
        assert!(reasons.iter().any(|r| r.contains("portal")), "{reasons:?}");
    }

    #[test]
    fn filename_wins_over_mapname_and_empty_names_read_nothing() {
        let text = |name: &str, value: &str| PreviewParameter {
            name: name.into(),
            value_type: "string".into(),
            floats: vec![],
            texts: vec![value.into()],
            truncated: false,
        };
        let light = |parameters| LightRecord {
            implementation: "infinite".into(),
            world_from_light: IDENTITY,
            source: None,
            entity_id: None,
            parameters,
        };
        assert_eq!(
            image_name(&light(vec![text("mapname", "a"), text("filename", "b")])),
            Some("b")
        );
        assert_eq!(image_name(&light(vec![text("mapname", "a")])), Some("a"));
        assert_eq!(image_name(&light(vec![text("filename", "")])), None);
    }
}
