#![allow(non_snake_case)]

use super::brdf::Brdf;
use super::math::{abs_cos_theta, cos_phi, sin_phi, sin_theta};

fn safe_div(a: f32, b: f32) -> f32 {
    if b.abs() < 1e-6 {
        1.0
    } else {
        (a / b).min(1.0)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BrdfOrenNayar;

impl BrdfOrenNayar {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Brdf for BrdfOrenNayar {
    fn eval(&self, V: &glam::Vec3, L: &glam::Vec3, alpha: f32) -> (f32, f32) {
        if V.z <= 0.0 || L.z <= 0.0 {
            return (0.0, 0.0);
        }

        let wi = L;
        let wo = V;

        // Oren-Nayar BRDF calculation
        // sigma = alpha * PI * 0.5
        let sigma = alpha * std::f32::consts::PI * 0.5;
        let sigma2 = sigma * sigma;

        // Precompute coefficients
        let A = 1.0 - 0.5 * sigma2 / (sigma2 + 0.33);
        let B = 0.45 * sigma2 / (sigma2 + 0.09);

        let sin_theta_i = sin_theta(wi);
        let sin_theta_o = sin_theta(wo);

        // Compute cosine term of Oren-Nayar model
        let mut max_cos = 0.0;
        if sin_theta_i > 1e-6 && sin_theta_o > 1e-6 {
            let sin_phi_i = sin_phi(wi);
            let cos_phi_i = cos_phi(wi);
            let sin_phi_o = sin_phi(wo);
            let cos_phi_o = cos_phi(wo);
            let d_cos = cos_phi_i * cos_phi_o + sin_phi_i * sin_phi_o;
            max_cos = d_cos.max(0.0);
        }

        // Compute sine and tangent terms of Oren-Nayar model

        //let (sin_alpha, tan_beta) = if abs_cos_theta(wi) > abs_cos_theta(wo) {
        //    (sin_theta_o, safe_div(sin_theta_i, abs_cos_theta(wi)))
        //} else {
        //    (sin_theta_i, safe_div(sin_theta_o, abs_cos_theta(wo)))
        //};
        let sin_alpha_tan_beta = if abs_cos_theta(wi) > abs_cos_theta(wo) {
            safe_div(sin_theta_o * sin_theta_i, abs_cos_theta(wi))
        } else {
            safe_div(sin_theta_i * sin_theta_o, abs_cos_theta(wo))
        };

        let value = (A + B * max_cos * sin_alpha_tan_beta) / std::f32::consts::PI;
        let pdf = L.z / std::f32::consts::PI;
        (value, pdf)
    }

    fn sample(&self, _V: &glam::Vec3, _alpha: f32, U1: f32, U2: f32) -> glam::Vec3 {
        // Cosine-weighted hemisphere sampling (Lambertian)
        let r = U1.sqrt();
        let phi = 2.0 * std::f32::consts::PI * U2;
        let L = glam::Vec3::new(r * phi.cos(), r * phi.sin(), (1.0 - r * r).max(0.0).sqrt());
        L
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_brdf_oren_nayar_eval() {
        let brdf = BrdfOrenNayar::new();
        let V = glam::Vec3::new(0.0, 0.0, 1.0);
        let L = glam::Vec3::new(0.0, 0.0, 1.0);
        let alpha = 0.5;
        let (value, pdf) = brdf.eval(&V, &L, alpha);
        assert!(value > 0.0);
        assert!(pdf > 0.0);
    }

    #[test]
    fn test_brdf_oren_nayar_sample() {
        let brdf = BrdfOrenNayar::new();
        let V = glam::Vec3::new(0.0, 0.0, 1.0);
        let L = brdf.sample(&V, 0.5, 0.5, 0.5);
        // Sample should produce a valid direction
        assert!(L.length() > 0.0);
        // Should be in upper hemisphere
        assert!(L.z >= 0.0);
    }

    #[test]
    fn test_brdf_oren_nayar_eval_grazing() {
        let brdf = BrdfOrenNayar::new();
        let V = glam::Vec3::new(0.9, 0.0, 0.1).normalize();
        let L = glam::Vec3::new(0.0, 0.0, 1.0);
        let alpha = 0.3;
        let (value, pdf) = brdf.eval(&V, &L, alpha);
        assert!(value > 0.0);
        assert!(pdf > 0.0);
    }

    #[test]
    fn test_brdf_oren_nayar_eval_invalid_v() {
        let brdf = BrdfOrenNayar::new();
        let V = glam::Vec3::new(0.0, 0.0, -1.0);
        let L = glam::Vec3::new(0.0, 0.0, 1.0);
        let alpha = 0.5;
        let (value, pdf) = brdf.eval(&V, &L, alpha);
        assert_eq!(value, 0.0);
        assert_eq!(pdf, 0.0);
    }

    #[test]
    fn test_brdf_oren_nayar_eval_invalid_l() {
        let brdf = BrdfOrenNayar::new();
        let V = glam::Vec3::new(0.0, 0.0, 1.0);
        let L = glam::Vec3::new(0.0, 0.0, -1.0);
        let alpha = 0.5;
        let (value, pdf) = brdf.eval(&V, &L, alpha);
        assert_eq!(value, 0.0);
        assert_eq!(pdf, 0.0);
    }

    #[test]
    fn test_brdf_oren_nayar_roughness_effect() {
        let brdf = BrdfOrenNayar::new();
        let V = glam::Vec3::new(0.5, 0.0, 0.866).normalize();
        let L = glam::Vec3::new(0.0, 0.0, 1.0);

        // Test with different roughness values
        let (value1, _) = brdf.eval(&V, &L, 0.1);
        let (value2, _) = brdf.eval(&V, &L, 0.9);

        // Both should be positive
        assert!(value1 > 0.0);
        assert!(value2 > 0.0);
    }

    #[test]
    fn test_brdf_oren_nayar_smooth_limit() {
        let brdf = BrdfOrenNayar::new();
        let V = glam::Vec3::new(0.0, 0.0, 1.0);
        let L = glam::Vec3::new(0.0, 0.0, 1.0);

        // With alpha = 0 (smooth surface), should approach Lambertian
        let (value, pdf) = brdf.eval(&V, &L, 0.0);
        let lambertian_value = L.z / std::f32::consts::PI;

        assert!((value - lambertian_value).abs() < 1e-5);
        assert!(pdf > 0.0);
    }
}
