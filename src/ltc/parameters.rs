#![allow(non_snake_case)]

use std::f32::consts::PI;

// Size of precomputed table (theta, alpha)
pub const N: usize = 64;

// Number of samples used to compute the error during fitting
pub const NSAMPLE: usize = 32;

// Minimal roughness (avoid singularities)
pub const MIN_ALPHA: f32 = 0.00001;

// Pi constant
pub const PI_F32: f32 = PI;
