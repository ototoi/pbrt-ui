#![allow(non_snake_case)]

use super::brdf::Brdf;
use super::math::*;
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

// Trowbridge-Reitz sampling functions

fn trowbridge_reitz_sample_11(cos_theta: f32, u1: f32, u2: f32) -> (f32, f32) {
    // Special case (normal incidence)
    if cos_theta > 0.9999 {
        let r = (u1 / (1.0 - u1)).sqrt();
        let phi = 2.0 * std::f32::consts::PI * u2;
        return (r * phi.cos(), r * phi.sin());
    }

    let sin_theta = (0.0_f32).max(1.0 - cos_theta * cos_theta).sqrt();
    let tan_theta = sin_theta / cos_theta;
    let a = 1.0 / tan_theta;
    let g1 = 2.0 / (1.0 + (1.0 + 1.0 / (a * a)).sqrt());

    // Sample slope_x
    let a_sample = 2.0 * u1 / g1 - 1.0;
    let tmp = (1.0 / (a_sample * a_sample - 1.0)).min(1e10);
    let b = tan_theta;
    let d = (b * b * tmp * tmp - (a_sample * a_sample - b * b) * tmp)
        .max(0.0)
        .sqrt();
    let slope_x_1 = b * tmp - d;
    let slope_x_2 = b * tmp + d;
    let slope_x = if a_sample < 0.0 || slope_x_2 > 1.0 / tan_theta {
        slope_x_1
    } else {
        slope_x_2
    };

    // Sample slope_y
    let (s, u2) = if u2 > 0.5 {
        (1.0, 2.0 * (u2 - 0.5))
    } else {
        (-1.0, 2.0 * (0.5 - u2))
    };
    let z = (u2 * (u2 * (u2 * 0.27385 - 0.73369) + 0.46341))
        / (u2 * (u2 * (u2 * 0.093073 + 0.309420) - 1.000000) + 0.597999);
    let slope_y = s * z * (1.0 + slope_x * slope_x).sqrt();

    (slope_x, slope_y)
}

fn trowbridge_reitz_sample(
    wi: &glam::Vec3,
    alpha_x: f32,
    alpha_y: f32,
    u1: f32,
    u2: f32,
) -> glam::Vec3 {
    // 1. stretch wi
    let wi_stretched = glam::Vec3::new(alpha_x * wi.x, alpha_y * wi.y, wi.z).normalize();

    // 2. simulate P22_{wi}(x_slope, y_slope, 1, 1)
    let (mut slope_x, mut slope_y) = trowbridge_reitz_sample_11(cos_theta(&wi_stretched), u1, u2);

    // 3. rotate
    let tmp = cos_phi(&wi_stretched) * slope_x - sin_phi(&wi_stretched) * slope_y;
    slope_y = sin_phi(&wi_stretched) * slope_x + cos_phi(&wi_stretched) * slope_y;
    slope_x = tmp;

    // 4. unstretch
    slope_x *= alpha_x;
    slope_y *= alpha_y;

    // 5. compute normal
    glam::Vec3::new(-slope_x, -slope_y, 1.0).normalize()
}

#[derive(Debug, Clone)]
pub struct TrowbridgeReitzDistribution {
    alphax: f32,
    alphay: f32,
}

impl MicrofacetDistribution for TrowbridgeReitzDistribution {
    fn d(&self, wh: &glam::Vec3) -> f32 {
        let tan_2_theta = tan_2_theta(wh);
        if tan_2_theta.is_infinite() {
            return 0.0;
        }
        let cos_2_theta = cos_2_theta(wh);
        let cos_4_theta = cos_2_theta * cos_2_theta;
        let e = (cos_2_phi(wh) / (self.alphax * self.alphax)
            + sin_2_phi(wh) / (self.alphay * self.alphay))
            * tan_2_theta;
        let e2 = (1.0 + e) * (1.0 + e);
        1.0 / (std::f32::consts::PI * self.alphax * self.alphay * cos_4_theta * e2)
    }

    fn lambda(&self, w: &glam::Vec3) -> f32 {
        let abs_tan_theta = tan_theta(w).abs();
        if abs_tan_theta.is_infinite() {
            return 0.0;
        }
        let alpha = (cos_2_phi(w) * self.alphax * self.alphax
            + sin_2_phi(w) * self.alphay * self.alphay)
            .sqrt();
        let alpha_2_tan_2_theta = (alpha * abs_tan_theta) * (alpha * abs_tan_theta);
        (-1.0 + (1.0 + alpha_2_tan_2_theta).sqrt()) / 2.0
    }

    fn sample_wh(&self, wo: &glam::Vec3, u: &glam::Vec2) -> glam::Vec3 {
        let flip = wo.z < 0.0;
        let wo_corrected = if flip { -*wo } else { *wo };
        let mut wh = trowbridge_reitz_sample(&wo_corrected, self.alphax, self.alphay, u.x, u.y);
        if flip {
            wh = -wh;
        }
        wh
    }

    fn pdf(&self, wo: &glam::Vec3, wh: &glam::Vec3) -> f32 {
        self.d(wh) * self.g1(wo) * wo.dot(*wh).abs() / abs_cos_theta(wo)
    }
}

fn safe_div(a: f32, b: f32) -> f32 {
    if b.abs() < 1e-6 {
        0.0
    } else {
        (a / b).clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct FresnelDielectric {
    eta_i: f32,
    eta_t: f32,
}

impl FresnelDielectric {
    fn new(eta_i: f32, eta_t: f32) -> Self {
        Self { eta_i, eta_t }
    }

    fn evaluate(&self, cos_theta_i: f32) -> f32 {
        // Schlick's approximation could be used here, but for dielectric we use the exact Fresnel equations
        let mut cos_theta_i = cos_theta_i.clamp(-1.0, 1.0);
        let entering = cos_theta_i > 0.0;
        let (eta_i, eta_t) = if entering {
            (self.eta_i, self.eta_t)
        } else {
            (self.eta_t, self.eta_i)
        };
        cos_theta_i = cos_theta_i.abs();

        // Compute sin_theta_t using Snell's law
        let sin_theta_i = (0.0_f32).max(1.0 - cos_theta_i * cos_theta_i).sqrt();
        let sin_theta_t = eta_i / eta_t * sin_theta_i;

        // Handle total internal reflection
        if sin_theta_t >= 1.0 {
            return 1.0;
        }

        let cos_theta_t = (0.0_f32).max(1.0 - sin_theta_t * sin_theta_t).sqrt();

        let r_parl = safe_div(
            (eta_t * cos_theta_i) - (eta_i * cos_theta_t),
            (eta_t * cos_theta_i) + (eta_i * cos_theta_t),
        );
        let r_perp = safe_div(
            (eta_i * cos_theta_i) - (eta_t * cos_theta_t),
            (eta_i * cos_theta_i) + (eta_t * cos_theta_t),
        );
        //
        (r_parl * r_parl + r_perp * r_perp) / 2.0
    }
}

fn face_forward(v: &glam::Vec3, n: &glam::Vec3) -> glam::Vec3 {
    if v.dot(*n) < 0.0 { -*v } else { *v }
}

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
    })
}

impl Brdf for BrdfMicrofacetReflection {
    fn eval(&self, V: &glam::Vec3, L: &glam::Vec3, alpha: f32) -> (f32, f32) {
        //alpha = roughnes * roughnes;
        let roughness = alpha.sqrt();
        let distribution = create_distribution(roughness); //

        let wo = V;
        let wi = L;

        let cos_theta_o = wo.z.abs();
        let cos_theta_i = wi.z.abs();

        if cos_theta_o <= 1e-6 || cos_theta_i <= 1e-6 {
            return (0.0, 0.0);
        }

        // Check if V and L are opposite directions
        let v_dot_l = wo.dot(*wi);
        if v_dot_l < 1e-6 {
            // V and L are opposite directions, no valid BRDF contribution
            return (0.0, 0.0);
        }

        // Compute half vector
        let wh = (wo + wi).normalize();
        let v_dot_wh = wo.dot(wh);
        if v_dot_wh <= 0.0 {
            return (0.0, 0.0);
        }

        let fresnel = FresnelDielectric::new(1.5, 1.0);
        let f = fresnel.evaluate(face_forward(&wh, &glam::Vec3::new(0.0, 0.0, 1.0)).dot(*wi));
        let f = 1.0; //

        // Compute the BRDF value
        let d = distribution.d(&wh);
        let g = distribution.g(&wo, &wi);

        // BRDF formula: f(wo, wi) = F * D * G / (4 * cos_theta_o * cos_theta_i)
        let value = safe_div(f * d * g, 4.0 * cos_theta_o * cos_theta_i);
        // Compute PDF
        let pdf = distribution.pdf(&wo, &wh) / (4.0 * v_dot_wh.abs());

        (value, pdf)
    }

    fn sample(&self, V: &glam::Vec3, alpha: f32, U1: f32, U2: f32) -> glam::Vec3 {
        let roughness = alpha.sqrt();
        let distribution = create_distribution(roughness);

        let u = glam::Vec2::new(U1, U2);

        // Sample microfacet normal wh
        let wh = distribution.sample_wh(V, &u);

        // Reflect V about wh to get wi (L)
        // Standard reflection formula: wi = -V + 2(wh·V)wh
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
            assert!(
                L.length() > 0.0,
                "Sampled direction should be non-zero for alpha={}",
                alpha
            );
        }
    }

    #[test]
    fn test_sample_with_zero_alpha() {
        let brdf = BrdfMicrofacetReflection::new();
        let V = glam::Vec3::new(0.0, 0.0, 1.0);
        let L = brdf.sample(&V, 0.001, 0.5, 0.5);
        assert!(
            L.length() > 0.0,
            "Sample should return non-zero even with small alpha"
        );
    }

    #[test]
    fn test_sample_with_large_alpha() {
        let brdf = BrdfMicrofacetReflection::new();
        let V = glam::Vec3::new(0.0, 0.0, 1.0);
        let L = brdf.sample(&V, 10.0, 0.5, 0.5);
        assert!(
            L.length() > 0.0,
            "Sample should return non-zero even with large alpha"
        );
    }

    #[test]
    fn test_sample_with_grazing_angle() {
        let brdf = BrdfMicrofacetReflection::new();
        let V = glam::Vec3::new(0.99, 0.0, 0.14).normalize();
        let L = brdf.sample(&V, 0.5, 0.5, 0.5);
        assert!(
            L.length() > 0.0,
            "Sample should return non-zero at grazing angles"
        );
    }

    #[test]
    fn test_eval_with_different_angles() {
        let brdf = BrdfMicrofacetReflection::new();
        let V = glam::Vec3::new(0.5, 0.5, 0.7).normalize();
        let L = glam::Vec3::new(-0.5, 0.5, 0.7).normalize();
        let alpha = 0.5;
        let (value, pdf) = brdf.eval(&V, &L, alpha);
        assert!(value >= 0.0, "BRDF value should be non-negative");
        assert!(pdf >= 0.0, "PDF should be non-negative");
    }

    #[test]
    fn test_eval_with_opposite_directions() {
        let brdf = BrdfMicrofacetReflection::new();
        let V = glam::Vec3::new(0.0, 0.0, 1.0);
        let L = glam::Vec3::new(0.0, 0.0, -1.0);
        let alpha = 0.5;
        let (value, pdf) = brdf.eval(&V, &L, alpha);
        // When V and L are opposite, should return 0
        assert_eq!(
            value, 0.0,
            "BRDF value should be zero for opposite directions"
        );
        assert_eq!(pdf, 0.0, "PDF should be zero for opposite directions");
    }
}
