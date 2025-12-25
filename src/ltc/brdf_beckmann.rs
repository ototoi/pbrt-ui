#![allow(non_snake_case)]

use super::brdf::Brdf;

#[derive(Clone, Copy, Debug, Default)]
pub struct BrdfBeckmann;

impl BrdfBeckmann {
    pub fn new() -> Self {
        Self
    }
}

// Beckmann lambda function coefficients
// These come from the rational approximation of the Smith masking-shadowing function
// Reference: Understanding the Masking-Shadowing Function in Microfacet-Based BRDFs
const LAMBDA_A: f32 = 1.259;
const LAMBDA_B: f32 = 0.396;
const LAMBDA_C: f32 = 3.535;
const LAMBDA_D: f32 = 2.181;

fn lambda(alpha: f32, cosTheta: f32) -> f32 {
    const EPSILON: f32 = 1e-6;
    if cosTheta >= 1.0 - EPSILON {
        return 0.0;
    }
    let a = 1.0 / (alpha * cosTheta.acos().tan());
    (1.0 - LAMBDA_A * a + LAMBDA_B * a * a) / (LAMBDA_C * a + LAMBDA_D * a * a)
}

impl Brdf for BrdfBeckmann {
    fn eval(&self, V: &glam::Vec3, L: &glam::Vec3, alpha: f32) -> (f32, f32) {
        if V.z <= 0.0 {
            return (0.0, 0.0);
        }

        // masking
        let LambdaV = lambda(alpha, V.z);

        // shadowing
        let G2 = if L.z <= 0.0 {
            0.0
        } else {
            let LambdaL = lambda(alpha, L.z);
            1.0 / (1.0 + LambdaV + LambdaL)
        };

        // D (Beckmann distribution)
        let H = (*V + *L).normalize();
        let slopex = H.x / H.z;
        let slopey = H.y / H.z;
        let D = (-(slopex * slopex + slopey * slopey) / (alpha * alpha)).exp()
            / (std::f32::consts::PI * alpha * alpha * H.z * H.z * H.z * H.z);

        let pdf = (D * H.z / 4.0 / V.dot(H)).abs();
        let res = D * G2 / 4.0 / V.z;

        (res, pdf)
    }

    fn sample(&self, V: &glam::Vec3, alpha: f32, U1: f32, U2: f32) -> glam::Vec3 {
        let phi = 2.0 * std::f32::consts::PI * U1;
        let r = alpha * (-U2.ln()).sqrt();
        let N = glam::Vec3::new(r * phi.cos(), r * phi.sin(), 1.0).normalize();

        -(*V) + 2.0 * N * N.dot(*V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_brdf_beckmann_eval() {
        let brdf = BrdfBeckmann::new();
        let V = glam::Vec3::new(0.0, 0.0, 1.0);
        let L = glam::Vec3::new(0.0, 0.0, 1.0);
        let alpha = 0.5;
        let (value, pdf) = brdf.eval(&V, &L, alpha);
        assert!(value > 0.0);
        assert!(pdf > 0.0);
    }

    #[test]
    fn test_brdf_beckmann_sample() {
        let brdf = BrdfBeckmann::new();
        let V = glam::Vec3::new(0.0, 0.0, 1.0);
        let L = brdf.sample(&V, 0.5, 0.5, 0.5);
        // Sample should produce a valid direction
        assert!(L.length() > 0.0);
    }

    #[test]
    fn test_brdf_beckmann_eval_grazing() {
        let brdf = BrdfBeckmann::new();
        let V = glam::Vec3::new(0.9, 0.0, 0.1).normalize();
        let L = glam::Vec3::new(0.9, 0.0, 0.1).normalize();
        let alpha = 0.3;
        let (value, pdf) = brdf.eval(&V, &L, alpha);
        assert!(value >= 0.0);
        assert!(pdf >= 0.0);
    }

    #[test]
    fn test_brdf_beckmann_eval_invalid_v() {
        let brdf = BrdfBeckmann::new();
        let V = glam::Vec3::new(0.0, 0.0, -1.0);
        let L = glam::Vec3::new(0.0, 0.0, 1.0);
        let alpha = 0.5;
        let (value, pdf) = brdf.eval(&V, &L, alpha);
        assert_eq!(value, 0.0);
        assert_eq!(pdf, 0.0);
    }
}
