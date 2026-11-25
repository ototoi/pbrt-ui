//! An approximate material preview: a unit
//! sphere under a fixed studio light, shaded on the CPU so core can serve
//! it on the wire. The look follows each material's
//! parameters monotonically; it is not a PBRT render.
//!
//! - The sphere is pbrt's (radius 1, z up), seen orthographically from
//!   -y: `p`, `n` and `uv` are what pbrt's sphere hands its textures, so
//!   texture inputs evaluate with [`crate::texture_eval`].
//! - The geometry per pixel depends only on the size and is computed once
//!   ([`gbuffer`]); a preview only evaluates textures and shades.
//! - Fresnel is pbrt-r4's `fr_dielectric` / `fr_complex` per RGB channel.
//! - Shading per material is a small approximation: Lambert,
//!   a normalized Blinn-Phong highlight for the key light and a sky
//!   gradient for reflections, blurred toward its mean by roughness;
//!   dielectrics refract through the sphere onto a checker backdrop.
//! - Colours are clamped sRGB, as in texture previews.
use crate::texture_eval::{Context, Operand, Rgb, srgb8};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

/// A conductor's colour: complex IOR, or r4's reflectance form (eta 1,
/// k from the reflectance).
#[derive(Clone, Debug, PartialEq)]
pub enum Tint {
    Fresnel { eta: Operand, k: Operand },
    Reflectance(Operand),
}

/// A microfacet roughness input: the value and r4's `remaproughness`.
#[derive(Clone, Debug, PartialEq)]
pub struct Roughness {
    pub value: Operand,
    pub remap: bool,
}

impl Roughness {
    pub fn smooth() -> Self {
        Self {
            value: Operand::Constant([0.; 3]),
            remap: true,
        }
    }

    /// r4's Trowbridge-Reitz `roughness_to_alpha` when remapped.
    fn alpha(&self, ctx: &Context) -> f32 {
        let r = self.value.scalar(ctx).max(0.);
        if self.remap { r.sqrt() } else { r }
    }
}

/// The tree core builds from a material resource and its upstream.
#[derive(Clone, Debug, PartialEq)]
pub enum Material {
    Diffuse {
        reflectance: Operand,
    },
    CoatedDiffuse {
        reflectance: Operand,
        roughness: Roughness,
        eta: f32,
    },
    Conductor {
        tint: Tint,
        roughness: Roughness,
    },
    CoatedConductor {
        interface_roughness: Roughness,
        interface_eta: f32,
        tint: Tint,
        roughness: Roughness,
    },
    Dielectric {
        eta: f32,
        roughness: Roughness,
        thin: bool,
    },
    DiffuseTransmission {
        reflectance: Operand,
        transmittance: Operand,
        scale: f32,
    },
    /// `amount` 0 is the first material, 1 the second (r4 picks one
    /// stochastically with that probability: the mean is the blend).
    Mix {
        materials: [Arc<Material>; 2],
        amount: Operand,
    },
    /// pbrt's `interface`: no surface, only the medium boundary.
    Interface,
    /// A material the preview does not model (measured, hair,
    /// subsurface): drawn as a Lambert stand-in of this colour.
    Fallback(Rgb),
}

/// One pixel's view of the sphere.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// What the sphere's textures see (object space = world space).
    pub ctx: Context,
    /// `n · v`, with `v` toward the eye.
    pub cos_view: f32,
    /// The pixel's width in sphere radii (transmission supersamples it).
    pub footprint: f32,
}

/// The eye looks along +y; `v` points back toward it.
const VIEW: [f32; 3] = [0., -1., 0.];
/// The half-width of the view, in sphere radii (a small margin).
const EXTENT: f32 = 1.15;
/// The checker backdrop's plane (behind the sphere) and cell size.
const BACKDROP_Y: f32 = 3.;
const CHECKER: f32 = 0.25;
/// Key light radiance and its direction (upper left, toward the eye).
const KEY: f32 = 0.9;

fn key_direction() -> [f32; 3] {
    normalize([-0.5, -0.7, 0.6])
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = dot(v, v).sqrt();
    if length > 0. {
        v.map(|c| c / length)
    } else {
        v
    }
}

fn add(a: Rgb, b: Rgb) -> Rgb {
    std::array::from_fn(|c| a[c] + b[c])
}

fn mul(a: Rgb, b: Rgb) -> Rgb {
    std::array::from_fn(|c| a[c] * b[c])
}

fn scaled(s: f32, a: Rgb) -> Rgb {
    a.map(|v| s * v)
}

fn lerp(t: f32, a: Rgb, b: Rgb) -> Rgb {
    std::array::from_fn(|c| (1. - t) * a[c] + t * b[c])
}

/// The screen point of pixel `(x, y)`: x right, y up, in sphere radii.
fn screen(x: u32, y: u32, size: u32) -> [f32; 2] {
    let s = |i: u32| ((i as f32 + 0.5) / size as f32 * 2. - 1.) * EXTENT;
    [s(x), -s(y)]
}

/// pbrt's sphere parameterization: u = φ / 2π, v = 1 − θ / π.
fn sphere_uv(p: [f32; 3]) -> [f32; 2] {
    use std::f32::consts::PI;
    let phi = {
        let phi = p[1].atan2(p[0]);
        if phi < 0. { phi + 2. * PI } else { phi }
    };
    [phi / (2. * PI), 1. - p[2].clamp(-1., 1.).acos() / PI]
}

fn hit_at(sx: f32, sy: f32, footprint: f32) -> Option<Hit> {
    let d2 = 1. - sx * sx - sy * sy;
    if d2 < 0. {
        return None;
    }
    let p = [sx, -d2.sqrt(), sy];
    Some(Hit {
        ctx: Context {
            p,
            n: p,
            uv: sphere_uv(p),
        },
        cos_view: dot(p, VIEW),
        footprint,
    })
}

/// The sphere's pixels at `size`, rows from the top; `None` is backdrop.
/// Computed once per size and shared, within the cache's budget.
pub fn gbuffer(size: u32) -> Arc<Vec<Option<Hit>>> {
    static CACHE: OnceLock<Mutex<Geometry>> = OnceLock::new();
    CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .get(size)
}

/// What one geometry buffer costs at `size`, in bytes.
fn geometry_bytes(size: u32) -> usize {
    size as usize * size as usize * std::mem::size_of::<Option<Hit>>()
}

/// The retained sphere geometry, most recently used last. The budget is
/// bytes, not size count, so a few large previews stay resident while an
/// agent sweeping every supported size cannot retain one buffer per
/// size — all 505 sizes sum to about 1.8 GiB. What falls out is rebuilt
/// on return; a preview holding an `Arc` keeps its memory to the end.
struct Geometry {
    order: std::collections::VecDeque<u32>,
    map: BTreeMap<u32, Arc<Vec<Option<Hit>>>>,
    bytes: usize,
}

/// Roughly three full-resolution (512 × 512) buffers.
const GEOMETRY_BUDGET: usize = 32 * 1024 * 1024;

impl Default for Geometry {
    fn default() -> Self {
        Self {
            order: Default::default(),
            map: Default::default(),
            bytes: 0,
        }
    }
}

impl Geometry {
    fn get(&mut self, size: u32) -> Arc<Vec<Option<Hit>>> {
        if let Some(buffer) = self.map.get(&size) {
            self.order.retain(|occupied| *occupied != size);
            self.order.push_back(size);
            return buffer.clone();
        }
        let buffer: Arc<Vec<Option<Hit>>> = Arc::new(
            (0..size * size)
                .map(|i| {
                    let [sx, sy] = screen(i % size, i / size, size);
                    hit_at(sx, sy, 2. * EXTENT / size as f32)
                })
                .collect(),
        );
        self.map.insert(size, buffer.clone());
        self.order.push_back(size);
        self.bytes += geometry_bytes(size);
        while self.bytes > GEOMETRY_BUDGET
            && let Some(evicted) = self.order.pop_front()
        {
            self.map.remove(&evicted);
            self.bytes = self.bytes.saturating_sub(geometry_bytes(evicted));
        }
        buffer
    }
}

/// The checker backdrop at plane point `(x, z)`.
fn checker(x: f32, z: f32) -> Rgb {
    let cell = (x / CHECKER).floor() as i64 + (z / CHECKER).floor() as i64;
    if cell.rem_euclid(2) == 0 {
        [0.3; 3]
    } else {
        [0.12; 3]
    }
}

/// The backdrop's mean, for rough transmission.
const CHECKER_MEAN: Rgb = [0.21; 3];

/// The sky gradient reflections see: dark ground, bright horizon, a
/// softer zenith.
fn sky(dir: [f32; 3]) -> Rgb {
    let z = dir[2];
    if z < 0. {
        lerp((-z).min(1.), [0.18; 3], [0.06; 3])
    } else {
        lerp(z.min(1.), [0.95; 3], [0.5, 0.52, 0.56])
    }
}

/// The sky's mean, for rough reflections.
const SKY_MEAN: Rgb = [0.42, 0.43, 0.45];

/// What a ray leaving in `dir` from `from` sees: the backdrop, or the sky.
fn environment(from: [f32; 3], dir: [f32; 3]) -> Rgb {
    if dir[1] > 1e-4 {
        let t = (BACKDROP_Y - from[1]) / dir[1];
        checker(from[0] + t * dir[0], from[2] + t * dir[2])
    } else {
        sky(dir)
    }
}

fn reflect(v: [f32; 3], n: [f32; 3]) -> [f32; 3] {
    let d = 2. * dot(n, v);
    std::array::from_fn(|c| d * n[c] - v[c])
}

/// pbrt's `Refract`: `wi` toward the surface side `n` faces; `eta` is
/// the inside over the outside IOR. None on total internal reflection.
fn refract(wi: [f32; 3], n: [f32; 3], eta: f32) -> Option<[f32; 3]> {
    let cos_i = dot(n, wi);
    let sin2_t = (1. - cos_i * cos_i).max(0.) / (eta * eta);
    if sin2_t >= 1. {
        return None;
    }
    let cos_t = (1. - sin2_t).sqrt();
    Some(std::array::from_fn(|c| {
        -wi[c] / eta + (cos_i / eta - cos_t) * n[c]
    }))
}

/// pbrt-r4 `fr_dielectric` (pbrt-v4 `FrDielectric`).
pub fn fr_dielectric(cos_theta_i: f32, eta: f32) -> f32 {
    let (mut cos_theta_i, mut eta) = (cos_theta_i.clamp(-1., 1.), eta);
    if cos_theta_i < 0. {
        eta = 1. / eta;
        cos_theta_i = -cos_theta_i;
    }
    let sin2_theta_t = (1. - cos_theta_i * cos_theta_i) / (eta * eta);
    if sin2_theta_t >= 1. {
        return 1.;
    }
    let cos_theta_t = (1. - sin2_theta_t).max(0.).sqrt();
    let r_parl = (eta * cos_theta_i - cos_theta_t) / (eta * cos_theta_i + cos_theta_t);
    let r_perp = (cos_theta_i - eta * cos_theta_t) / (cos_theta_i + eta * cos_theta_t);
    (r_parl * r_parl + r_perp * r_perp) / 2.
}

/// pbrt-r4 `fr_complex_scalar` (pbrt-v4 `FrComplex` for one wavelength).
pub fn fr_complex(cos_theta_i: f32, eta_r: f32, eta_i: f32) -> f32 {
    let cos_theta_i = cos_theta_i.clamp(0., 1.);
    let sin2_theta_i = 1. - cos_theta_i * cos_theta_i;
    let eta2_r = eta_r * eta_r - eta_i * eta_i;
    let eta2_i = 2. * eta_r * eta_i;
    let denom = eta2_r * eta2_r + eta2_i * eta2_i;
    let sin2_theta_t_r = sin2_theta_i * eta2_r / denom;
    let sin2_theta_t_i = -sin2_theta_i * eta2_i / denom;
    let (cos_t_r, cos_t_i) = complex_sqrt(1. - sin2_theta_t_r, -sin2_theta_t_i);
    let (eta_cos_r, eta_cos_i) = (eta_r * cos_theta_i, eta_i * cos_theta_i);
    let r_parl = norm_of_div(
        eta_cos_r - cos_t_r,
        eta_cos_i - cos_t_i,
        eta_cos_r + cos_t_r,
        eta_cos_i + cos_t_i,
    );
    let eta_ct_r = eta_r * cos_t_r - eta_i * cos_t_i;
    let eta_ct_i = eta_r * cos_t_i + eta_i * cos_t_r;
    let r_perp = norm_of_div(
        cos_theta_i - eta_ct_r,
        -eta_ct_i,
        cos_theta_i + eta_ct_r,
        eta_ct_i,
    );
    (r_parl + r_perp) / 2.
}

fn complex_sqrt(re: f32, im: f32) -> (f32, f32) {
    let r = (re * re + im * im).sqrt();
    let a = (0.5 * (r + re)).max(0.).sqrt();
    let b = (0.5 * (r - re)).max(0.).sqrt();
    (a, if im >= 0. { b } else { -b })
}

fn norm_of_div(num_r: f32, num_i: f32, den_r: f32, den_i: f32) -> f32 {
    let denom = den_r * den_r + den_i * den_i;
    if denom == 0. {
        return 0.;
    }
    let q_r = (num_r * den_r + num_i * den_i) / denom;
    let q_i = (num_i * den_r - num_r * den_i) / denom;
    q_r * q_r + q_i * q_i
}

/// Diffuse irradiance at normal `n`: the key light plus a hemispheric
/// ambient (ground below, sky above).
fn irradiance(n: [f32; 3]) -> f32 {
    KEY * dot(n, key_direction()).max(0.) + 0.06 + 0.29 * (n[2] * 0.5 + 0.5)
}

/// The key light's normalized Blinn-Phong highlight at `alpha`, and the
/// cosine its Fresnel term reads (`v · h`).
fn highlight(n: [f32; 3], alpha: f32) -> (f32, f32) {
    let l = key_direction();
    let h = normalize(add(l, VIEW));
    // r4 puts no ceiling on roughness (roughness_to_alpha is a bare
    // sqrt), so alpha arrives past 1; the exponent would go negative
    // there and 0^-x · 0 goes NaN black on the rim. The preview's lobe
    // stops broadening at 1 instead.
    let alpha = alpha.clamp(0.03, 1.);
    let exponent = 2. / (alpha * alpha) - 2.;
    let lobe = (exponent + 8.) / (8. * std::f32::consts::PI)
        * dot(n, h).max(0.).powf(exponent)
        * dot(n, l).max(0.);
    (KEY * lobe, dot(VIEW, h).max(0.))
}

/// The reflected sky, blurred toward its mean as `alpha` grows. Only
/// transmission sees the backdrop: reflected, its checker aliases at
/// the rim.
fn reflection(hit: &Hit, alpha: f32) -> Rgb {
    let r = reflect(VIEW, hit.ctx.n);
    lerp((alpha * 1.5).min(1.), sky(r), SKY_MEAN)
}

/// The conductor's eta and k at the hit, both relative to `outside`.
fn conductor_ior(tint: &Tint, ctx: &Context, outside: f32) -> (Rgb, Rgb) {
    match tint {
        Tint::Fresnel { eta, k } => (
            eta.evaluate(ctx).map(|v| v / outside),
            k.evaluate(ctx).map(|v| v / outside),
        ),
        // r4: eta 1, k = 2 √r / √(1 − r), r clamped to [0, 0.9999].
        Tint::Reflectance(reflectance) => (
            [1.; 3],
            reflectance.evaluate(ctx).map(|r| {
                let r = r.clamp(0., 0.9999);
                2. * r.sqrt() / (1. - r).max(0.).sqrt()
            }),
        ),
    }
}

fn conductor(hit: &Hit, tint: &Tint, alpha: f32, outside: f32) -> Rgb {
    let (eta, k) = conductor_ior(tint, &hit.ctx, outside);
    let fresnel = |cos: f32| -> Rgb { std::array::from_fn(|c| fr_complex(cos, eta[c], k[c])) };
    let (lobe, cos_h) = highlight(hit.ctx.n, alpha);
    add(
        mul(fresnel(hit.cos_view), reflection(hit, alpha)),
        scaled(lobe, fresnel(cos_h)),
    )
}

/// A dielectric coat's reflection (environment and highlight) and the
/// fraction it passes to what is beneath.
fn coat(hit: &Hit, eta: f32, alpha: f32) -> (Rgb, f32) {
    let f = fr_dielectric(hit.cos_view, eta);
    let (lobe, cos_h) = highlight(hit.ctx.n, alpha);
    let reflected = add(
        scaled(f, reflection(hit, alpha)),
        [lobe * fr_dielectric(cos_h, eta); 3],
    );
    (reflected, 1. - f)
}

/// What passes through the sphere, averaged over 3 × 3 rays across the
/// pixel: refraction minifies the backdrop, and one ray aliases it.
fn transmitted(hit: &Hit, eta: f32, thin: bool) -> Rgb {
    const N: usize = 3;
    let (sx, sy) = (hit.ctx.p[0], hit.ctx.p[2]);
    let mut sum = [0.; 3];
    let mut count = 0;
    for i in 0..N * N {
        let offset = |j: usize| ((j as f32 + 0.5) / N as f32 - 0.5) * hit.footprint;
        // Sub-rays past the silhouette keep the pixel's own hit.
        let sub = hit_at(sx + offset(i % N), sy + offset(i / N), hit.footprint).unwrap_or(*hit);
        sum = add(sum, transmitted_ray(&sub, eta, thin));
        count += 1;
    }
    scaled(1. / count as f32, sum)
}

/// One ray through the sphere: refracted in and out (the far side's
/// reflection under total internal reflection), or straight for a thin
/// sheet.
fn transmitted_ray(hit: &Hit, eta: f32, thin: bool) -> Rgb {
    let p = hit.ctx.p;
    let incoming = [0., 1., 0.];
    if thin || eta == 1. {
        return environment(p, incoming);
    }
    let Some(inside) = refract(VIEW, hit.ctx.n, eta) else {
        return sky(reflect(VIEW, hit.ctx.n));
    };
    // The far intersection of the unit sphere along `inside`.
    let t = -2. * dot(p, inside);
    let exit: [f32; 3] = std::array::from_fn(|c| p[c] + t * inside[c]);
    let inward = exit.map(|v| -v);
    let back = inside.map(|v| -v);
    match refract(back, inward, 1. / eta) {
        Some(out) => environment(exit, out),
        None => sky(reflect(back, inward)),
    }
}

fn dielectric(hit: &Hit, eta: f32, alpha: f32, thin: bool) -> Rgb {
    let mut f = fr_dielectric(hit.cos_view, eta);
    if thin && f < 1. {
        // pbrt's thin dielectric: inter-reflection inside the sheet.
        f += (1. - f) * (1. - f) * f / (1. - f * f);
    }
    let (lobe, cos_h) = highlight(hit.ctx.n, alpha);
    let through = lerp(
        (alpha * 1.5).min(1.),
        transmitted(hit, eta, thin),
        CHECKER_MEAN,
    );
    add(
        add(scaled(f, reflection(hit, alpha)), scaled(1. - f, through)),
        [lobe * fr_dielectric(cos_h, eta); 3],
    )
}

impl Material {
    /// The approximate radiance toward the eye at `hit`.
    pub fn shade(&self, hit: &Hit) -> Rgb {
        let ctx = &hit.ctx;
        let n = ctx.n;
        match self {
            Material::Diffuse { reflectance } => scaled(irradiance(n), reflectance.evaluate(ctx)),
            Material::Fallback(albedo) => scaled(irradiance(n), *albedo),
            Material::CoatedDiffuse {
                reflectance,
                roughness,
                eta,
            } => {
                let (reflected, passed) = coat(hit, *eta, roughness.alpha(ctx));
                let base = scaled(irradiance(n), reflectance.evaluate(ctx));
                add(reflected, scaled(passed, base))
            }
            Material::Conductor { tint, roughness } => {
                conductor(hit, tint, roughness.alpha(ctx), 1.)
            }
            Material::CoatedConductor {
                interface_roughness,
                interface_eta,
                tint,
                roughness,
            } => {
                let (reflected, passed) = coat(hit, *interface_eta, interface_roughness.alpha(ctx));
                let base = conductor(hit, tint, roughness.alpha(ctx), *interface_eta);
                add(reflected, scaled(passed, base))
            }
            Material::Dielectric {
                eta,
                roughness,
                thin,
            } => dielectric(hit, *eta, roughness.alpha(ctx), *thin),
            Material::DiffuseTransmission {
                reflectance,
                transmittance,
                scale,
            } => {
                let back = n.map(|v| -v);
                add(
                    scaled(scale * irradiance(n), reflectance.evaluate(ctx)),
                    scaled(scale * irradiance(back), transmittance.evaluate(ctx)),
                )
            }
            Material::Mix { materials, amount } => {
                let t = amount.scalar(ctx).clamp(0., 1.);
                lerp(t, materials[0].shade(hit), materials[1].shade(hit))
            }
            Material::Interface => environment(ctx.p, [0., 1., 0.]),
        }
    }
}

/// The backdrop seen past the sphere at pixel `(x, y)`.
pub fn backdrop(x: u32, y: u32, size: u32) -> Rgb {
    let [sx, sy] = screen(x, y, size);
    checker(sx, sy)
}

/// An RGBA8 sRGB thumbnail of `material` on the sphere, `size` × `size`.
pub fn thumbnail(material: &Material, size: u32) -> Vec<u8> {
    let pixels = gbuffer(size);
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for (i, hit) in pixels.iter().enumerate() {
        let i = i as u32;
        let value = match hit {
            Some(hit) => material.shade(hit),
            None => backdrop(i % size, i / size, size),
        };
        rgba.extend(value.map(srgb8));
        rgba.push(255);
    }
    rgba
}

#[cfg(test)]
mod tests;
