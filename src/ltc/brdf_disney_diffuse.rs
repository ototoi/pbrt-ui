#![allow(non_snake_case)]

use super::brdf::Brdf;

#[derive(Clone, Copy, Debug, Default)]
pub struct BrdfDisneyDiffuse;

impl BrdfDisneyDiffuse {
    pub fn new() -> Self {
        Self
    }
}

impl Brdf for BrdfDisneyDiffuse {
    fn eval(&self, V: &glam::Vec3, L: &glam::Vec3, alpha: f32) -> (f32, f32) {
        if V.z <= 0.0 || L.z <= 0.0 {
            return (0.0, 0.0);
        }

        let pdf = L.z / std::f32::consts::PI;

        let NdotV = V.z;
        let NdotL = L.z;
        let H = (*V + *L).normalize();
        let LdotH = L.dot(H);
        let perceptualRoughness = alpha.sqrt();
        let fd90 = 0.5 + 2.0 * LdotH * LdotH * perceptualRoughness;
        let lightScatter = 1.0 + (fd90 - 1.0) * (1.0 - NdotL).powi(5);
        let viewScatter = 1.0 + (fd90 - 1.0) * (1.0 - NdotV).powi(5);
        let value = lightScatter * viewScatter * L.z / std::f32::consts::PI;

        (value, pdf)
    }

    fn sample(&self, _V: &glam::Vec3, _alpha: f32, U1: f32, U2: f32) -> glam::Vec3 {
        let r = U1.sqrt();
        let phi = 2.0 * std::f32::consts::PI * U2;

        glam::Vec3::new(r * phi.cos(), r * phi.sin(), (1.0 - r * r).sqrt())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_brdf_disneydiffuse_eval() {
        let brdf = BrdfDisneyDiffuse::new();
        let V = glam::Vec3::new(0.0, 0.0, 1.0);
        let L = glam::Vec3::new(0.0, 0.0, 1.0);
        let alpha = 0.5;
        let (value, pdf) = brdf.eval(&V, &L, alpha);
        assert!(value > 0.0);
        assert!(pdf > 0.0);
    }

    #[test]
    fn test_brdf_disneydiffuse_sample() {
        let brdf = BrdfDisneyDiffuse::new();
        let V = glam::Vec3::new(0.0, 0.0, 1.0);
        let L = brdf.sample(&V, 0.5, 0.5, 0.5);
        // Sample should produce a valid direction
        assert!(L.length() > 0.0);
        // Should be in upper hemisphere
        assert!(L.z >= 0.0);
    }

    #[test]
    fn test_brdf_disneydiffuse_eval_grazing() {
        let brdf = BrdfDisneyDiffuse::new();
        let V = glam::Vec3::new(0.9, 0.0, 0.1).normalize();
        let L = glam::Vec3::new(0.0, 0.0, 1.0);
        let alpha = 0.3;
        let (value, pdf) = brdf.eval(&V, &L, alpha);
        assert!(value > 0.0);
        assert!(pdf > 0.0);
    }

    #[test]
    fn test_brdf_disneydiffuse_eval_invalid_v() {
        let brdf = BrdfDisneyDiffuse::new();
        let V = glam::Vec3::new(0.0, 0.0, -1.0);
        let L = glam::Vec3::new(0.0, 0.0, 1.0);
        let alpha = 0.5;
        let (value, pdf) = brdf.eval(&V, &L, alpha);
        assert_eq!(value, 0.0);
        assert_eq!(pdf, 0.0);
    }

    #[test]
    fn test_brdf_disneydiffuse_eval_invalid_l() {
        let brdf = BrdfDisneyDiffuse::new();
        let V = glam::Vec3::new(0.0, 0.0, 1.0);
        let L = glam::Vec3::new(0.0, 0.0, -1.0);
        let alpha = 0.5;
        let (value, pdf) = brdf.eval(&V, &L, alpha);
        assert_eq!(value, 0.0);
        assert_eq!(pdf, 0.0);
    }

    #[test]
    fn test_brdf_disneydiffuse_roughness_effect() {
        let brdf = BrdfDisneyDiffuse::new();
        let V = glam::Vec3::new(0.5, 0.0, 0.866).normalize();
        let L = glam::Vec3::new(0.0, 0.0, 1.0);

        // Test with different roughness values
        let (value1, _) = brdf.eval(&V, &L, 0.1);
        let (value2, _) = brdf.eval(&V, &L, 0.9);

        // Both should be positive
        assert!(value1 > 0.0);
        assert!(value2 > 0.0);
    }
}
