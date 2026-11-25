//! A point-sampling evaluator for PBRT textures, ported from pbrt-r4 `src/textures/`
//! (211eb9e, 4.5.2), itself after pbrt-v4 `src/pbrt/textures.{h,cpp}`.
//!
//! Copyright (c) 1998-2020 Matt Pharr, Wenzel Jakob and Greg Humphreys.
//! Licensed under the Apache License, Version 2.0 (as pbrt-r4 and pbrt-ui).
//!
//! What is ported and what is not:
//! - point sampling only: no differentials, no MIP-maps, no closed-form
//!   checkerboard antialiasing (r4's `aamode none`); FBm-style noise gets
//!   zero footprints, which r4 turns into the full octave count;
//! - values are RGB triples (no spectral evaluation); a float texture is
//!   grey;
//! - images arrive decoded and linear (the caller applies the encoding).
//!
//! The caller (core) resolves parameters with the r4 defaults
//! and builds the [`Texture`] tree; this module only evaluates it.
pub mod noise;
pub mod spectra;

use std::sync::Arc;

pub type Rgb = [f32; 3];

/// A 4×4 row-major matrix mapping world points into texture space (the
/// inverse of the texture's declaration CTM).
pub type Matrix = [[f32; 4]; 4];

pub const IDENTITY: Matrix = [
    [1., 0., 0., 0.],
    [0., 1., 0., 0.],
    [0., 0., 1., 0.],
    [0., 0., 0., 1.],
];

fn transform_point(m: &Matrix, p: [f32; 3]) -> [f32; 3] {
    let row = |r: &[f32; 4]| r[0] * p[0] + r[1] * p[1] + r[2] * p[2] + r[3];
    let (x, y, z, w) = (row(&m[0]), row(&m[1]), row(&m[2]), row(&m[3]));
    if w == 1. || w == 0. {
        [x, y, z]
    } else {
        [x / w, y / w, z / w]
    }
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Where a texture is evaluated: the surface point, its normal and UV.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Context {
    pub p: [f32; 3],
    pub n: [f32; 3],
    pub uv: [f32; 2],
}

/// r4's `TextureMapping2D`.
#[derive(Clone, Debug, PartialEq)]
pub enum Mapping2D {
    Uv {
        su: f32,
        sv: f32,
        du: f32,
        dv: f32,
    },
    Spherical(Matrix),
    Cylindrical(Matrix),
    Planar {
        texture_from_world: Matrix,
        vs: [f32; 3],
        vt: [f32; 3],
        ds: f32,
        dt: f32,
    },
}

impl Mapping2D {
    pub fn map(&self, ctx: &Context) -> [f32; 2] {
        use std::f32::consts::PI;
        match self {
            Mapping2D::Uv { su, sv, du, dv } => [su * ctx.uv[0] + du, sv * ctx.uv[1] + dv],
            Mapping2D::Spherical(m) => {
                let pt = transform_point(m, ctx.p);
                let length = dot(pt, pt).sqrt();
                let v = if length > 0. {
                    [pt[0] / length, pt[1] / length, pt[2] / length]
                } else {
                    pt
                };
                let theta = v[2].clamp(-1., 1.).acos();
                let phi = {
                    let p = v[1].atan2(v[0]);
                    if p < 0. { p + 2. * PI } else { p }
                };
                [theta / PI, phi / (2. * PI)]
            }
            Mapping2D::Cylindrical(m) => {
                let pt = transform_point(m, ctx.p);
                [(PI + pt[1].atan2(pt[0])) / (2. * PI), pt[2]]
            }
            Mapping2D::Planar {
                texture_from_world,
                vs,
                vt,
                ds,
                dt,
            } => {
                let v = transform_point(texture_from_world, ctx.p);
                [ds + dot(v, *vs), dt + dot(v, *vt)]
            }
        }
    }
}

/// How an image lookup outside [0, 1) wraps (r4's `WrapMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wrap {
    Repeat,
    Black,
    Clamp,
}

/// A decoded image in linear RGB, rows top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Rgb>,
}

impl Image {
    fn texel(&self, x: i64, y: i64, wrap: Wrap) -> Rgb {
        let (w, h) = (self.width as i64, self.height as i64);
        let (x, y) = match wrap {
            Wrap::Repeat => (x.rem_euclid(w), y.rem_euclid(h)),
            Wrap::Clamp => (x.clamp(0, w - 1), y.clamp(0, h - 1)),
            Wrap::Black => {
                if x < 0 || y < 0 || x >= w || y >= h {
                    return [0.; 3];
                }
                (x, y)
            }
        };
        self.pixels[(y * w + x) as usize]
    }

    /// pbrt's bilinear lookup at `st` (texel centers at half-integers).
    fn bilerp(&self, st: [f32; 2], wrap: Wrap) -> Rgb {
        if self.width == 0 || self.height == 0 {
            return [0.; 3];
        }
        let x = st[0] * self.width as f32 - 0.5;
        let y = st[1] * self.height as f32 - 0.5;
        let (xi, yi) = (x.floor(), y.floor());
        let (dx, dy) = (x - xi, y - yi);
        let (xi, yi) = (xi as i64, yi as i64);
        let v00 = self.texel(xi, yi, wrap);
        let v10 = self.texel(xi + 1, yi, wrap);
        let v01 = self.texel(xi, yi + 1, wrap);
        let v11 = self.texel(xi + 1, yi + 1, wrap);
        std::array::from_fn(|c| {
            (1. - dx) * (1. - dy) * v00[c]
                + dx * (1. - dy) * v10[c]
                + (1. - dx) * dy * v01[c]
                + dx * dy * v11[c]
        })
    }
}

/// An operand: a constant, or another texture's value.
#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    Constant(Rgb),
    Texture(Arc<Texture>),
}

impl Operand {
    pub fn evaluate(&self, ctx: &Context) -> Rgb {
        match self {
            Operand::Constant(value) => *value,
            Operand::Texture(texture) => texture.evaluate(ctx),
        }
    }

    /// A float operand reads the first channel (float textures are grey).
    pub fn scalar(&self, ctx: &Context) -> f32 {
        self.evaluate(ctx)[0]
    }
}

/// The texture tree core builds from a resource and its upstream.
#[derive(Clone, Debug, PartialEq)]
pub enum Texture {
    Constant(Rgb),
    Scale {
        tex: Operand,
        scale: Operand,
    },
    Mix {
        tex1: Operand,
        tex2: Operand,
        amount: Operand,
    },
    /// `amount = |n · dir|` (dir already in world space).
    DirectionMix {
        tex1: Operand,
        tex2: Operand,
        dir: [f32; 3],
    },
    Checkerboard2D {
        mapping: Mapping2D,
        tex1: Operand,
        tex2: Operand,
    },
    Checkerboard3D {
        texture_from_world: Matrix,
        tex1: Operand,
        tex2: Operand,
    },
    Dots {
        mapping: Mapping2D,
        inside: Operand,
        outside: Operand,
    },
    Fbm {
        texture_from_world: Matrix,
        octaves: u32,
        omega: f32,
    },
    Wrinkled {
        texture_from_world: Matrix,
        octaves: u32,
        omega: f32,
    },
    Windy {
        texture_from_world: Matrix,
    },
    Marble {
        texture_from_world: Matrix,
        octaves: u32,
        omega: f32,
        scale: f32,
        variation: f32,
    },
    Bilerp {
        mapping: Mapping2D,
        v00: Rgb,
        v01: Rgb,
        v10: Rgb,
        v11: Rgb,
    },
    Image {
        mapping: Mapping2D,
        image: Arc<Image>,
        wrap: Wrap,
        scale: f32,
        invert: bool,
    },
}

fn grey(v: f32) -> Rgb {
    [v; 3]
}

fn lerp_rgb(t: f32, a: Rgb, b: Rgb) -> Rgb {
    std::array::from_fn(|c| (1. - t) * a[c] + t * b[c])
}

/// r4's marble spline control points.
const MARBLE: [Rgb; 9] = [
    [0.58, 0.58, 0.6],
    [0.58, 0.58, 0.6],
    [0.58, 0.58, 0.6],
    [0.5, 0.5, 0.5],
    [0.6, 0.59, 0.58],
    [0.58, 0.58, 0.6],
    [0.58, 0.58, 0.6],
    [0.2, 0.2, 0.33],
    [0.58, 0.58, 0.6],
];

impl Texture {
    pub fn evaluate(&self, ctx: &Context) -> Rgb {
        let zero = [0.; 3];
        match self {
            Texture::Constant(value) => *value,
            Texture::Scale { tex, scale } => {
                let (value, s) = (tex.evaluate(ctx), scale.scalar(ctx));
                value.map(|v| v * s)
            }
            Texture::Mix { tex1, tex2, amount } => {
                let t = amount.scalar(ctx);
                lerp_rgb(t, tex1.evaluate(ctx), tex2.evaluate(ctx))
            }
            Texture::DirectionMix { tex1, tex2, dir } => {
                let amount = dot(ctx.n, *dir).abs();
                let (t1, t2) = (tex1.evaluate(ctx), tex2.evaluate(ctx));
                std::array::from_fn(|c| amount * t1[c] + (1. - amount) * t2[c])
            }
            Texture::Checkerboard2D {
                mapping,
                tex1,
                tex2,
            } => {
                let st = mapping.map(ctx);
                // r4's `%` on i32: an odd negative sum is -1, so it is tex2.
                if (st[0].floor() as i32 + st[1].floor() as i32) % 2 == 0 {
                    tex1.evaluate(ctx)
                } else {
                    tex2.evaluate(ctx)
                }
            }
            Texture::Checkerboard3D {
                texture_from_world,
                tex1,
                tex2,
            } => {
                let p = transform_point(texture_from_world, ctx.p);
                if (p[0].floor() as i32 + p[1].floor() as i32 + p[2].floor() as i32) % 2 == 0 {
                    tex1.evaluate(ctx)
                } else {
                    tex2.evaluate(ctx)
                }
            }
            Texture::Dots {
                mapping,
                inside,
                outside,
            } => {
                let st = mapping.map(ctx);
                let s_cell = (st[0] + 0.5).floor();
                let t_cell = (st[1] + 0.5).floor();
                if noise::noise(s_cell + 0.5, t_cell + 0.5, 0.5) > 0. {
                    let radius = 0.35_f32;
                    let max_shift = 0.5 - radius;
                    let s_center =
                        s_cell + max_shift * noise::noise(s_cell + 1.5, t_cell + 2.8, 0.5);
                    let t_center =
                        t_cell + max_shift * noise::noise(s_cell + 4.5, t_cell + 9.8, 0.5);
                    let (ds, dt) = (st[0] - s_center, st[1] - t_center);
                    if ds * ds + dt * dt < radius * radius {
                        return inside.evaluate(ctx);
                    }
                }
                outside.evaluate(ctx)
            }
            Texture::Fbm {
                texture_from_world,
                octaves,
                omega,
            } => grey(noise::fbm(
                transform_point(texture_from_world, ctx.p),
                zero,
                zero,
                *omega,
                *octaves,
            )),
            Texture::Wrinkled {
                texture_from_world,
                octaves,
                omega,
            } => grey(noise::turbulence(
                transform_point(texture_from_world, ctx.p),
                zero,
                zero,
                *omega,
                *octaves,
            )),
            Texture::Windy { texture_from_world } => {
                let p = transform_point(texture_from_world, ctx.p);
                let wind_strength = noise::fbm(p.map(|v| 0.1 * v), zero, zero, 0.5, 3);
                let wave_height = noise::fbm(p, zero, zero, 0.5, 6);
                grey(wind_strength.abs() * wave_height)
            }
            Texture::Marble {
                texture_from_world,
                octaves,
                omega,
                scale,
                variation,
            } => {
                let p = transform_point(texture_from_world, ctx.p).map(|v| scale * v);
                let marble = p[1] + variation * noise::fbm(p, zero, zero, *omega, *octaves);
                let t = 0.5 + 0.5 * marble.sin();
                let segments = MARBLE.len() - 3;
                let first = ((t * segments as f32).floor() as usize).min(segments - 1);
                let t = t * segments as f32 - first as f32;
                let [c0, c1, c2, c3] = [0, 1, 2, 3].map(|i| MARBLE[first + i]);
                let (s0, s1, s2) = (
                    lerp_rgb(t, c0, c1),
                    lerp_rgb(t, c1, c2),
                    lerp_rgb(t, c2, c3),
                );
                let (s0, s1) = (lerp_rgb(t, s0, s1), lerp_rgb(t, s1, s2));
                // r4's extra 1.5 to spread the colors.
                lerp_rgb(t, s0, s1).map(|v| v * 1.5)
            }
            Texture::Bilerp {
                mapping,
                v00,
                v01,
                v10,
                v11,
            } => {
                let st = mapping.map(ctx);
                let (a, b) = ((1. - st[0]) * (1. - st[1]), (1. - st[0]) * st[1]);
                let (c, d) = (st[0] * (1. - st[1]), st[0] * st[1]);
                std::array::from_fn(|i| a * v00[i] + b * v01[i] + c * v10[i] + d * v11[i])
            }
            Texture::Image {
                mapping,
                image,
                wrap,
                scale,
                invert,
            } => {
                let mut st = mapping.map(ctx);
                // pbrt flips t: image rows run top to bottom.
                st[1] = 1. - st[1];
                let value = image.bilerp(st, *wrap).map(|v| v * scale);
                if *invert {
                    value.map(|v| (1. - v).max(0.))
                } else {
                    value
                }
            }
        }
    }
}

/// The preview's world-space mapping of a pixel: the plane
/// `p = (u, v, 0)`, `n = (0, 0, 1)`, `uv ∈ [0, 1]²` (row 0 at v = 1, so
/// the image reads like the UV square).
pub fn preview_context(x: u32, y: u32, size: u32) -> Context {
    let u = (x as f32 + 0.5) / size as f32;
    let v = 1. - (y as f32 + 0.5) / size as f32;
    Context {
        p: [u, v, 0.],
        n: [0., 0., 1.],
        uv: [u, v],
    }
}

/// Linear to 8-bit sRGB, clamped to [0, 1] first.
pub fn srgb8(linear: f32) -> u8 {
    let v = linear.clamp(0., 1.);
    let encoded = if v <= 0.003_130_8 {
        12.92 * v
    } else {
        1.055 * v.powf(1. / 2.4) - 0.055
    };
    (encoded * 255. + 0.5) as u8
}

/// An RGBA8 sRGB thumbnail of `texture`, `size` × `size`.
pub fn thumbnail(texture: &Texture, size: u32) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let value = texture.evaluate(&preview_context(x, y, size));
            rgba.extend(value.map(srgb8));
            rgba.push(255);
        }
    }
    rgba
}

#[cfg(test)]
mod tests;
