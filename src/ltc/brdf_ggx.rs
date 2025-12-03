#![allow(non_snake_case)]

use super::brdf::Brdf;

#[derive(Clone, Copy, Debug, Default)]
pub struct BrdfGGX;

impl BrdfGGX {
    pub fn new() -> Self {
        Self::default()
    }
}

fn lambda(alpha: f32, cosTheta: f32) -> f32 {
    if cosTheta < 0.0 {
        let a = 1.0 / (alpha * cosTheta.acos().tan());
        return 0.5 * (-1.0 + (1.0 + 1.0 / (a * a)).sqrt());
    } else {
        return 0.0;
    }
}

impl Brdf for BrdfGGX {
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
            1.0 / (1.0 + LambdaV + lambda(alpha, L.z))
        };
        let H = (*V + *L).normalize();
        let slopex = H.x / H.z;
        let slopey = H.y / H.z;
        let D = 1.0 / (1.0 + (slopex * slopex + slopey * slopey) / alpha / alpha);
        let D = D * D;
        let D = D / (std::f32::consts::PI * alpha * alpha * H.z * H.z * H.z * H.z);
        let pdf = (D * H.z / (4.0 * V.dot(H))).abs();
        let res = D * G2 / 4.0 / V.z;
        (res, pdf)
    }

    fn sample(&self, V: &glam::Vec3, alpha: f32, U1: f32, U2: f32) -> glam::Vec3 {
        let phi = 2.0 * std::f32::consts::PI * U1;
        let r = alpha * (U2 / (1.0 - U2)).sqrt();
        let N = glam::Vec3::new(r * phi.cos(), r * phi.sin(), 1.0).normalize();
        let L = -(*V) + 2.0 * N * N.dot(*V);
        L
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_brdf_ggx_eval() {
        let brdf = BrdfGGX::new();
        let V = glam::Vec3::new(0.0, 0.0, 1.0);
        let L = glam::Vec3::new(0.0, 0.0, 1.0);
        let alpha = 0.5;
        let (value, pdf) = brdf.eval(&V, &L, alpha);
        assert!(value > 0.0);
        assert!(pdf > 0.0);
    }

    #[test]
    fn test_brdf_ggx_sample() {
        let brdf = BrdfGGX::new();
        let V = glam::Vec3::new(0.0, 0.0, 1.0);
        let L = brdf.sample(&V, 0.5, 0.5, 0.5);
        // Sample should produce a valid direction
        assert!(L.length() > 0.0);
    }
}
