#![allow(non_snake_case)]

use glam::{Mat3, Vec3};

/// LTC (Linearly Transformed Cosines) structure
/// Represents a lobe distribution for BRDF approximation
#[derive(Clone, Debug)]
pub struct LTC {
    /// Lobe magnitude
    pub magnitude: f32,

    /// Average Schlick Fresnel term
    pub fresnel: f32,

    /// Parametric representation
    pub m11: f32,
    pub m22: f32,
    pub m13: f32,
    pub X: Vec3,
    pub Y: Vec3,
    pub Z: Vec3,

    /// Matrix representation
    pub M: Mat3,
    pub invM: Mat3,
    pub detM: f32,
}

impl Default for LTC {
    fn default() -> Self {
        Self::new()
    }
}

impl LTC {
    pub fn new() -> Self {
        let mut ltc = Self {
            magnitude: 1.0,
            fresnel: 1.0,
            m11: 1.0,
            m22: 1.0,
            m13: 0.0,
            X: Vec3::new(1.0, 0.0, 0.0),
            Y: Vec3::new(0.0, 1.0, 0.0),
            Z: Vec3::new(0.0, 0.0, 1.0),
            M: Mat3::IDENTITY,
            invM: Mat3::IDENTITY,
            detM: 1.0,
        };
        ltc.update();
        ltc
    }

    /// Compute matrix from parameters
    pub fn update(&mut self) {
        // Create basis matrix from X, Y, Z vectors
        let basis = Mat3::from_cols(self.X, self.Y, self.Z);
        
        // Create scale/shear matrix
        let scale = Mat3::from_cols(
            Vec3::new(self.m11, 0.0, self.m13),
            Vec3::new(0.0, self.m22, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        );
        
        self.M = basis * scale;
        self.invM = self.M.inverse();
        self.detM = self.M.determinant().abs();
    }

    /// Evaluate the LTC distribution for a given light direction L
    pub fn eval(&self, L: &Vec3) -> f32 {
        let Loriginal = (self.invM * *L).normalize();
        let L_ = self.M * Loriginal;

        let l = L_.length();
        let Jacobian = self.detM / (l * l * l);

        // Clamped cosine distribution
        let D = (1.0 / std::f32::consts::PI) * Loriginal.z.max(0.0);

        let res = self.magnitude * D / Jacobian;
        res
    }

    /// Sample the LTC distribution
    pub fn sample(&self, U1: f32, U2: f32) -> Vec3 {
        let theta = U1.sqrt().acos();
        let phi = 2.0 * std::f32::consts::PI * U2;
        
        let cos_theta = theta.cos();
        let sin_theta = theta.sin();
        let cos_phi = phi.cos();
        let sin_phi = phi.sin();
        
        let L_local = Vec3::new(
            sin_theta * cos_phi,
            sin_theta * sin_phi,
            cos_theta,
        );
        
        (self.M * L_local).normalize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ltc_default() {
        let ltc = LTC::new();
        assert_eq!(ltc.magnitude, 1.0);
        assert_eq!(ltc.fresnel, 1.0);
        assert_eq!(ltc.m11, 1.0);
        assert_eq!(ltc.m22, 1.0);
        assert_eq!(ltc.m13, 0.0);
    }

    #[test]
    fn test_ltc_eval() {
        let ltc = LTC::new();
        let L = Vec3::new(0.0, 0.0, 1.0);
        let value = ltc.eval(&L);
        assert!(value > 0.0);
    }
}
