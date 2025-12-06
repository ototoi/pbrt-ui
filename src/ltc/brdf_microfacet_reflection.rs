#![allow(non_snake_case)]

use super::brdf::Brdf;
use std::sync::Arc;

pub trait MicrofacetDistribution {
    fn d(&self, wh: &glam::Vec3) -> f32;
    fn lambda(&self, w: &glam::Vec3) -> f32;
    fn g1(&self, w: &glam::Vec3) -> f32 {
        return 1.0 / (1.0 + self.lambda(w));
    }
    fn g(&self, wo: &glam::Vec3, wi: &glam::Vec3) -> f32 {
        return 1.0 / (1.0 + self.lambda(wo) + self.lambda(wi));
    }
    fn sample_wh(&self, wo: &glam::Vec3, u: &glam::Vec2) -> glam::Vec3;
    fn pdf(&self, wo: &glam::Vec3, wh: &glam::Vec3) -> f32;
}

#[derive(Debug, Clone)]
pub struct TrowbridgeReitzDistribution {
    alphax: f32,
    alphay: f32,
    samplevis: bool,
}

impl TrowbridgeReitzDistribution {
    fn sample_wh_visible(&self, wo: &glam::Vec3, u: &glam::Vec2) -> glam::Vec3 {
        // Stretch wo
        let wo_stretched = glam::Vec3::new(
            self.alphax * wo.x,
            self.alphay * wo.y,
            wo.z
        ).normalize();
        
        // Simulate P22_{wo}(x_slope, y_slope, 1, 1)
        let (slope_x, slope_y) = self.sample_p22_11(wo_stretched.z, u);
        
        // Rotate and unstretch
        let tmp = wo_stretched.z.signum() * (wo_stretched.x * slope_x + wo_stretched.y * slope_y - wo_stretched.z);
        glam::Vec3::new(
            -self.alphax * slope_x + tmp * wo_stretched.x,
            -self.alphay * slope_y + tmp * wo_stretched.y,
            tmp * wo_stretched.z + wo_stretched.z.signum()
        ).normalize()
    }
    
    fn sample_p22_11(&self, cos_theta: f32, u: &glam::Vec2) -> (f32, f32) {
        // Special case (normal incidence)
        if cos_theta > 0.9999 {
            let r = (u.x / (1.0 - u.x)).sqrt();
            let phi = 2.0 * std::f32::consts::PI * u.y;
            return (r * phi.cos(), r * phi.sin());
        }
        
        let sin_theta = (0.0_f32).max(1.0 - cos_theta * cos_theta).sqrt();
        let tan_theta = sin_theta / cos_theta;
        let a = 1.0 / tan_theta;
        let g1 = 2.0 / (1.0 + (1.0 + 1.0 / (a * a)).sqrt());
        
        // Sample slope_x
        let a = 2.0 * u.x / g1 - 1.0;
        let mut tmp = 1.0 / (a * a - 1.0);
        if tmp > 1e10 {
            tmp = 1e10;
        }
        let b = tan_theta;
        let d = (b * b * tmp * tmp - (a * a - b * b) * tmp).max(0.0).sqrt();
        let slope_x_1 = b * tmp - d;
        let slope_x_2 = b * tmp + d;
        let slope_x = if a < 0.0 || slope_x_2 > 1.0 / tan_theta {
            slope_x_1
        } else {
            slope_x_2
        };
        
        // Sample slope_y
        let s = if u.y > 0.5 {
            1.0
        } else {
            -1.0
        };
        let u2 = (u.y - 0.5).abs() * 2.0;
        let z = (u2 * (u2 * (u2 * 0.27385 - 0.73369) + 0.46341)) /
                (u2 * (u2 * (u2 * 0.093073 + 0.309420) - 1.000000) + 0.597999);
        let slope_y = s * z * (1.0 + slope_x * slope_x).sqrt();
        
        (slope_x, slope_y)
    }
}

impl MicrofacetDistribution for TrowbridgeReitzDistribution {
    fn d(&self, wh: &glam::Vec3) -> f32 {
        let tan2_theta = (wh.x * wh.x + wh.y * wh.y) / (wh.z * wh.z);
        if tan2_theta.is_infinite() {
            return 0.0;
        }
        let cos4_theta = wh.z * wh.z * wh.z * wh.z;
        if cos4_theta < 1e-16 {
            return 0.0;
        }
        let e = tan2_theta * ((wh.x / self.alphax) * (wh.x / self.alphax) + 
                              (wh.y / self.alphay) * (wh.y / self.alphay));
        let result = 1.0 / (std::f32::consts::PI * self.alphax * self.alphay * cos4_theta * (1.0 + e) * (1.0 + e));
        result
    }

    fn lambda(&self, w: &glam::Vec3) -> f32 {
        let abs_tan_theta = ((w.x * w.x + w.y * w.y) / (w.z * w.z)).sqrt().abs();
        if abs_tan_theta.is_infinite() {
            return 0.0;
        }
        // Compute alpha for direction w
        let cos2_phi = if w.x * w.x + w.y * w.y > 0.0 {
            (w.x * w.x) / (w.x * w.x + w.y * w.y)
        } else {
            1.0
        };
        let sin2_phi = if w.x * w.x + w.y * w.y > 0.0 {
            (w.y * w.y) / (w.x * w.x + w.y * w.y)
        } else {
            0.0
        };
        let alpha = (cos2_phi * self.alphax * self.alphax + sin2_phi * self.alphay * self.alphay).sqrt();
        let alpha2_tan2_theta = (alpha * abs_tan_theta) * (alpha * abs_tan_theta);
        (-1.0 + (1.0 + alpha2_tan2_theta).sqrt()) / 2.0
    }

    fn sample_wh(&self, wo: &glam::Vec3, u: &glam::Vec2) -> glam::Vec3 {
        let mut wh = if !self.samplevis {
            // Sample visible area of normals for TrowbridgeReitz distribution
            let cos_theta = if self.alphax == self.alphay {
                let alpha = self.alphax;
                let tan2_theta = alpha * alpha * u.y / (1.0 - u.y);
                (1.0 / (1.0 + tan2_theta)).sqrt()
            } else {
                let phi = std::f32::consts::PI * 2.0 * u.x;
                let cos_phi = phi.cos();
                let sin_phi = phi.sin();
                let alphax2 = self.alphax * self.alphax;
                let alphay2 = self.alphay * self.alphay;
                let alpha2 = 1.0 / (cos_phi * cos_phi / alphax2 + sin_phi * sin_phi / alphay2);
                let tan2_theta = alpha2 * u.y / (1.0 - u.y);
                (1.0 / (1.0 + tan2_theta)).sqrt()
            };
            let sin_theta = (0.0_f32).max(1.0 - cos_theta * cos_theta).sqrt();
            let phi = u.x * 2.0 * std::f32::consts::PI;
            glam::Vec3::new(sin_theta * phi.cos(), sin_theta * phi.sin(), cos_theta)
        } else {
            // Visible normal sampling for GGX
            let flip = wo.z < 0.0;
            let wo_corrected = if flip { -(*wo) } else { *wo };
            let wh_local = self.sample_wh_visible(&wo_corrected, u);
            if flip { -wh_local } else { wh_local }
        };
        
        wh = wh.normalize();
        wh
    }

    fn pdf(&self, wo: &glam::Vec3, wh: &glam::Vec3) -> f32 {
        if self.samplevis {
            self.d(wh) * self.g1(wo) * wo.dot(*wh).abs() / wo.z.abs()
        } else {
            self.d(wh) * wh.z.abs()
        }
    }
}

/* 
pub trait MicrofacetDistributionFactory {
    fn create(&self, alpha: f32) -> Arc<dyn MicrofacetDistribution>;
}

pub struct TrowbridgeReitzDistributionFactory;

impl MicrofacetDistributionFactory for TrowbridgeReitzDistributionFactory {
    fn create(&self, alpha: f32) -> Arc<dyn MicrofacetDistribution> {
        Arc::new(TrowbridgeReitzDistribution {
            alphax: alpha,
            alphay: alpha,
            samplevis: true,
        })
    }
}


#[derive(Clone)]
pub struct BrdfMicrofacetReflection {
    pub factory: Arc<dyn MicrofacetDistributionFactory>,
}


impl BrdfMicrofacetReflection {
    pub fn new(factory: &Arc<dyn MicrofacetDistributionFactory>) -> Self {
        Self {
            factory: factory.clone(),
        }
    }
}
*/

#[derive(Clone, Copy, Debug, Default)]
pub struct BrdfMicrofacetReflection;

impl BrdfMicrofacetReflection {
    pub fn new() -> Self {
        Self::default()
    }
}

fn create_distribution(alpha: f32) -> Arc<dyn MicrofacetDistribution> {
    Arc::new(TrowbridgeReitzDistribution {
        alphax: alpha,
        alphay: alpha,
        samplevis: true,
    })
}

impl Brdf for BrdfMicrofacetReflection {
    fn eval(&self, V: &glam::Vec3, L: &glam::Vec3, alpha: f32) -> (f32, f32) {
        let distribution = create_distribution(alpha);
        
        let cos_theta_o = V.z;
        let cos_theta_i = L.z;
        
        // Compute half vector
        let wh = (*V + *L).normalize();
        
        // Fresnel is fixed to 1.0 as per the problem statement
        let f = 1.0;
        
        // Compute the BRDF value
        let d = distribution.d(&wh);
        let g = distribution.g(V, L);
        
        // BRDF formula: f(wo, wi) = F * D * G / (4 * cos_theta_o * cos_theta_i)
        // But we simplify by removing division by cos_theta_i in the return value
        let brdf_value = if cos_theta_i > 0.0 && cos_theta_o > 0.0 {
            f * d * g / (4.0 * cos_theta_o * cos_theta_i)
        } else {
            0.0
        };
        
        // Compute PDF
        let pdf = distribution.pdf(V, &wh) / (4.0 * V.dot(wh));
        
        (brdf_value, pdf)
    }
    
    fn sample(&self, V: &glam::Vec3, alpha: f32, U1: f32, U2: f32) -> glam::Vec3 {
        let distribution = create_distribution(alpha);
        
        let u = glam::Vec2::new(U1, U2);
        
        // Sample microfacet normal wh
        let wh = distribution.sample_wh(V, &u);
        
        // Reflect V about wh to get wi (L)
        let wi = -(*V) + 2.0 * wh * wh.dot(*V);
        
        wi
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_brdf_microfacet_reflection_eval() {
        let brdf = BrdfMicrofacetReflection::new();
        let V = glam::Vec3::new(0.0, 0.0, 1.0);
        let L = glam::Vec3::new(0.0, 0.0, 1.0);
        let alpha = 0.5;
        let (value, pdf) = brdf.eval(&V, &L, alpha);
        assert!(value > 0.0, "BRDF value should be positive");
        assert!(pdf > 0.0, "PDF should be positive");
    }

    #[test]
    fn test_brdf_microfacet_reflection_sample() {
        let brdf = BrdfMicrofacetReflection::new();
        let V = glam::Vec3::new(0.0, 0.0, 1.0);
        let L = brdf.sample(&V, 0.5, 0.5, 0.5);
        // Sample should produce a valid direction
        assert!(L.length() > 0.0, "Sampled direction should be non-zero");
    }

    #[test]
    fn test_trowbridge_reitz_d() {
        let dist = TrowbridgeReitzDistribution {
            alphax: 0.5,
            alphay: 0.5,
            samplevis: true,
        };
        let wh = glam::Vec3::new(0.0, 0.0, 1.0);
        let d = dist.d(&wh);
        assert!(d > 0.0, "D should be positive for normal facing up");
    }

    #[test]
    fn test_trowbridge_reitz_lambda() {
        let dist = TrowbridgeReitzDistribution {
            alphax: 0.5,
            alphay: 0.5,
            samplevis: true,
        };
        let w = glam::Vec3::new(0.0, 0.0, 1.0);
        let lambda = dist.lambda(&w);
        assert!(lambda >= 0.0, "Lambda should be non-negative");
    }

    #[test]
    fn test_brdf_microfacet_reflection_eval_grazing() {
        let brdf = BrdfMicrofacetReflection::new();
        let V = glam::Vec3::new(0.9, 0.0, 0.1).normalize();
        let L = glam::Vec3::new(0.9, 0.0, 0.1).normalize();
        let alpha = 0.3;
        let (value, pdf) = brdf.eval(&V, &L, alpha);
        assert!(value >= 0.0, "BRDF value should be non-negative");
        assert!(pdf >= 0.0, "PDF should be non-negative");
    }

    #[test]
    fn test_brdf_microfacet_reflection_sample_different_alphas() {
        let brdf = BrdfMicrofacetReflection::new();
        let V = glam::Vec3::new(0.5, 0.5, 0.7).normalize();
        
        for alpha in [0.1, 0.5, 1.0].iter() {
            let L = brdf.sample(&V, *alpha, 0.3, 0.7);
            assert!(L.length() > 0.0, "Sampled direction should be non-zero for alpha={}", alpha);
        }
    }
}

