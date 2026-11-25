#![allow(non_snake_case)]

// Size of precomputed table (theta, alpha)
pub const N: usize = 64;

// Minimal roughness (avoid singularities)
pub const MIN_ALPHA: f32 = 0.00001;

// Epsilon for numerical stability
pub const EPSILON: f32 = 1e-7;
