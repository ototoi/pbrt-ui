#![allow(non_snake_case)]

use super::brdf::Brdf;

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

        // PDF for cosine-weighted hemisphere sampling
        let pdf = L.z / std::f32::consts::PI;

        // Oren-Nayar BRDF calculation
        // sigma = alpha * PI * 0.5
        let sigma = alpha * std::f32::consts::PI * 0.5;
        let sigma2 = sigma * sigma;

        // Precompute coefficients
        let A = 1.0 - 0.5 * sigma2 / (sigma2 + 0.33);
        let B = 0.45 * sigma2 / (sigma2 + 0.09);

        // Angles
        let NdotL = L.z;
        let NdotV = V.z;

        // Compute theta_i and theta_r (angles from normal)
        let theta_i = NdotL.acos();
        let theta_r = NdotV.acos();

        // Determine alpha and beta (larger and smaller angles)
        let (sin_alpha, tan_beta) = if theta_i > theta_r {
            (theta_i.sin(), theta_r.tan())
        } else {
            (theta_r.sin(), theta_i.tan())
        };

        // Compute azimuthal difference
        // Project V and L onto the tangent plane and compute the angle between them
        let V_tangent = glam::Vec3::new(V.x, V.y, 0.0);
        let L_tangent = glam::Vec3::new(L.x, L.y, 0.0);

        let cos_phi_diff = if V_tangent.length() > 1e-6 && L_tangent.length() > 1e-6 {
            let V_norm = V_tangent.normalize();
            let L_norm = L_tangent.normalize();
            V_norm.dot(L_norm).clamp(-1.0, 1.0).max(0.0)
        } else {
            1.0
        };

        // Oren-Nayar formula
        let value = (A + B * cos_phi_diff * sin_alpha * tan_beta) * NdotL / std::f32::consts::PI;

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
