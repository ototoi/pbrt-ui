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
    todo!()
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
        return (res, pdf);
    }

    fn sample(&self, V: &glam::Vec3, U1: f32, U2: f32, alpha: f32) -> glam::Vec3 {
        // Placeholder implementation for GGX BRDF sampling
        glam::Vec3::ZERO
    }
}
