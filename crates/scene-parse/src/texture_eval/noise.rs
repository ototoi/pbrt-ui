//! Perlin noise, FBm and turbulence, ported from pbrt-r4
//! `src/textures/noise.rs` (pbrt-r4 16b0aab), itself after pbrt-v4
//! `src/pbrt/util/noise.cpp`.
//!
//! Copyright (c) 1998-2020 Matt Pharr, Wenzel Jakob and Greg Humphreys.
//! Licensed under the Apache License, Version 2.0 (pbrt-v4 and pbrt-r4 are
//! Apache-2.0, as is pbrt-ui). The permutation table is Ken Perlin's
//! "Improved Noise" reference table (SIGGRAPH 2002).
//!
//! `f32` throughout, as r4's default `Float`, so preview values follow r4.
//! The derivative arguments of `fbm` / `turbulence` stay: point sampling
//! passes zero, which r4 turns into the full octave count.

fn smooth_step(min: f32, max: f32, value: f32) -> f32 {
    let v = ((value - min) / (max - min)).clamp(0., 1.);
    v * v * (-2. * v + 3.)
}

fn lerp(t: f32, a: f32, b: f32) -> f32 {
    (1. - t) * a + t * b
}

fn grad(x: u32, y: u32, z: u32, dx: f32, dy: f32, dz: f32) -> f32 {
    let (x, y, z) = (x as usize, y as usize, z as usize);
    let h = NOISEPERM[NOISEPERM[NOISEPERM[x] as usize + y] as usize + z] & 15;
    let u = if h < 8 || h == 12 || h == 13 { dx } else { dy };
    let v = if h < 4 || h == 12 || h == 13 { dy } else { dz };
    (if h & 1 != 0 { -u } else { u }) + (if h & 2 != 0 { -v } else { v })
}

fn noise_weight(t: f32) -> f32 {
    let t3 = t * t * t;
    let t4 = t3 * t;
    6. * t4 * t - 15. * t4 + 10. * t3
}

const NOISEPERMSIZE: u32 = 256;
#[rustfmt::skip]
const NOISEPERM: [u32; 512] = [
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69,
    142, 8, 99, 37, 240, 21, 10, 23, 190, 6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219,
    203, 117, 35, 11, 32, 57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175,
    74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230,
    220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54, 65, 25, 63, 161, 1, 216, 80, 73, 209,
    76, 132, 187, 208, 89, 18, 169, 200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198,
    173, 186, 3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212,
    207, 206, 59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213, 119, 248, 152, 2, 44,
    154, 163, 70, 221, 153, 101, 155, 167, 43, 172, 9, 129, 22, 39, 253, 19, 98, 108, 110, 79,
    113, 224, 232, 178, 185, 112, 104, 218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12,
    191, 179, 162, 241, 81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157,
    184, 84, 204, 176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29,
    24, 72, 243, 141, 128, 195, 78, 66, 215, 61, 156, 180,
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69,
    142, 8, 99, 37, 240, 21, 10, 23, 190, 6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219,
    203, 117, 35, 11, 32, 57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175,
    74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230,
    220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54, 65, 25, 63, 161, 1, 216, 80, 73, 209,
    76, 132, 187, 208, 89, 18, 169, 200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198,
    173, 186, 3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212,
    207, 206, 59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213, 119, 248, 152, 2, 44,
    154, 163, 70, 221, 153, 101, 155, 167, 43, 172, 9, 129, 22, 39, 253, 19, 98, 108, 110, 79,
    113, 224, 232, 178, 185, 112, 104, 218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12,
    191, 179, 162, 241, 81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157,
    184, 84, 204, 176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29,
    24, 72, 243, 141, 128, 195, 78, 66, 215, 61, 156, 180,
];

pub fn noise(x: f32, y: f32, z: f32) -> f32 {
    // v4's fmod(x, 1 << 30) guard against overflow in the cell index.
    let period = (1_i32 << 30) as f32;
    let (x, y, z) = (x % period, y % period, z % period);
    let (ix, iy, iz) = (x.floor() as i32, y.floor() as i32, z.floor() as i32);
    let (dx, dy, dz) = (x - ix as f32, y - iy as f32, z - iz as f32);
    let (ix, iy, iz) = (
        (ix as u32) & (NOISEPERMSIZE - 1),
        (iy as u32) & (NOISEPERMSIZE - 1),
        (iz as u32) & (NOISEPERMSIZE - 1),
    );
    let w000 = grad(ix, iy, iz, dx, dy, dz);
    let w100 = grad(ix + 1, iy, iz, dx - 1., dy, dz);
    let w010 = grad(ix, iy + 1, iz, dx, dy - 1., dz);
    let w110 = grad(ix + 1, iy + 1, iz, dx - 1., dy - 1., dz);
    let w001 = grad(ix, iy, iz + 1, dx, dy, dz - 1.);
    let w101 = grad(ix + 1, iy, iz + 1, dx - 1., dy, dz - 1.);
    let w011 = grad(ix, iy + 1, iz + 1, dx, dy - 1., dz - 1.);
    let w111 = grad(ix + 1, iy + 1, iz + 1, dx - 1., dy - 1., dz - 1.);
    let (wx, wy, wz) = (noise_weight(dx), noise_weight(dy), noise_weight(dz));
    let x00 = lerp(wx, w000, w100);
    let x10 = lerp(wx, w010, w110);
    let x01 = lerp(wx, w001, w101);
    let x11 = lerp(wx, w011, w111);
    let y0 = lerp(wy, x00, x10);
    let y1 = lerp(wy, x01, x11);
    lerp(wz, y0, y1)
}

fn length_squared(v: [f32; 3]) -> f32 {
    v[0] * v[0] + v[1] * v[1] + v[2] * v[2]
}

fn scaled(s: f32, p: [f32; 3]) -> [f32; 3] {
    [s * p[0], s * p[1], s * p[2]]
}

/// The (fractional) octave count r4 derives from the footprint.
fn octaves(dpdx: [f32; 3], dpdy: [f32; 3], max_octaves: u32) -> f32 {
    let len2 = length_squared(dpdx).max(length_squared(dpdy));
    (-1. - 0.5 * len2.log2()).clamp(0., max_octaves as f32)
}

pub fn fbm(p: [f32; 3], dpdx: [f32; 3], dpdy: [f32; 3], omega: f32, max_octaves: u32) -> f32 {
    let n = octaves(dpdx, dpdy, max_octaves);
    let n_int = n.floor() as usize;
    let (mut sum, mut lambda, mut o) = (0_f32, 1_f32, 1_f32);
    for _ in 0..n_int {
        let lp = scaled(lambda, p);
        sum += o * noise(lp[0], lp[1], lp[2]);
        lambda *= 1.99;
        o *= omega;
    }
    let n_partial = n - n_int as f32;
    let lp = scaled(lambda, p);
    sum + o * smooth_step(0.3, 0.7, n_partial) * noise(lp[0], lp[1], lp[2])
}

pub fn turbulence(
    p: [f32; 3],
    dpdx: [f32; 3],
    dpdy: [f32; 3],
    omega: f32,
    max_octaves: u32,
) -> f32 {
    let n = octaves(dpdx, dpdy, max_octaves);
    let n_int = n.floor() as usize;
    let (mut sum, mut lambda, mut o) = (0_f32, 1_f32, 1_f32);
    for _ in 0..n_int {
        let lp = scaled(lambda, p);
        sum += o * noise(lp[0], lp[1], lp[2]).abs();
        lambda *= 1.99;
        o *= omega;
    }
    let n_partial = n - n_int as f32;
    let lp = scaled(lambda, p);
    // The partial octave blends from 0.2 to the absolute noise, and each
    // octave that did not fit counts as 0.2 (pbrt-v4, r4 since 0878994).
    let fade = smooth_step(0.3, 0.7, n_partial);
    sum += o * ((1. - fade) * 0.2 + fade * noise(lp[0], lp[1], lp[2]).abs());
    for _ in n_int..max_octaves as usize {
        sum += o * 0.2;
        o *= omega;
    }
    sum
}
