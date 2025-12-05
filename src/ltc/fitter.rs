#![allow(non_snake_case)]

use super::brdf::Brdf;
use super::ltc::LTC;
use super::parameters::{EPSILON, NSAMPLE};
use glam::Vec3;

/// Compute the average BRDF terms:
/// - norm (albedo) of the BRDF
/// - average Schlick Fresnel value
/// - average direction of the BRDF
pub fn compute_avg_terms(brdf: &dyn Brdf, V: &Vec3, alpha: f32) -> (f32, f32, Vec3) {
    let mut norm = 0.0;
    let mut fresnel = 0.0;
    let mut average_dir = Vec3::ZERO;

    for j in 0..NSAMPLE {
        for i in 0..NSAMPLE {
            let U1 = (i as f32 + 0.5) / NSAMPLE as f32;
            let U2 = (j as f32 + 0.5) / NSAMPLE as f32;

            // Sample
            let L = brdf.sample(V, alpha, U1, U2);

            // Eval
            let (eval, pdf) = brdf.eval(V, &L, alpha);

            if pdf > 0.0 {
                let weight = eval / pdf;

                let H = (*V + L).normalize();

                // Accumulate
                norm += weight;
                fresnel += weight * (1.0 - V.dot(H).max(0.0)).powi(5);
                average_dir += weight * L;
            }
        }
    }

    norm /= (NSAMPLE * NSAMPLE) as f32;
    fresnel /= (NSAMPLE * NSAMPLE) as f32;

    // Clear y component, which should be zero with isotropic BRDFs
    // This assumption is valid for isotropic BRDFs where rotation around the normal
    // doesn't change the distribution
    average_dir.y = 0.0;

    if average_dir.length_squared() > 0.0 {
        average_dir = average_dir.normalize();
    } else {
        average_dir = Vec3::new(0.0, 0.0, 1.0);
    }

    (norm, fresnel, average_dir)
}

/// Helper function for compute_error to avoid code duplication
fn compute_error_helper(error: f32, pdf: f32) -> f32 {
    if pdf > 0.0 { error / pdf } else { 0.0 }
}

/// Compute the error between the BRDF and the LTC using Multiple Importance Sampling
pub fn compute_error(ltc: &LTC, brdf: &dyn Brdf, V: &Vec3, alpha: f32) -> f32 {
    let mut error = 0.0;

    for j in 0..NSAMPLE {
        for i in 0..NSAMPLE {
            let U1 = (i as f32 + 0.5) / NSAMPLE as f32;
            let U2 = (j as f32 + 0.5) / NSAMPLE as f32;

            // Importance sample LTC
            {
                let L = ltc.sample(U1, U2);

                let (eval_brdf, pdf_brdf) = brdf.eval(V, &L, alpha);
                let eval_ltc = ltc.eval(&L);
                let pdf_ltc = eval_ltc / ltc.magnitude;

                // Error with MIS weight
                let error_ = (eval_brdf - eval_ltc).abs();
                let error_ = error_ * error_ * error_;
                error += compute_error_helper(error_, pdf_ltc + pdf_brdf);
            }

            // Importance sample BRDF
            {
                let L = brdf.sample(V, alpha, U1, U2);

                let (eval_brdf, pdf_brdf) = brdf.eval(V, &L, alpha);
                let eval_ltc = ltc.eval(&L);
                let pdf_ltc = eval_ltc / ltc.magnitude;

                // Error with MIS weight
                let error_ = (eval_brdf - eval_ltc).abs();
                let error_ = error_ * error_ * error_;
                error += compute_error_helper(error_, pdf_ltc + pdf_brdf);
            }
        }
    }

    error / (NSAMPLE * NSAMPLE) as f32
}

/// Fit LTC parameters to BRDF using Nelder-Mead optimization
pub struct FitLTC<'a> {
    pub ltc: &'a mut LTC,
    pub brdf: &'a dyn Brdf,
    pub V: Vec3,
    pub alpha: f32,
    pub isotropic: bool,
}

impl<'a> FitLTC<'a> {
    pub fn new(ltc: &'a mut LTC, brdf: &'a dyn Brdf, V: Vec3, alpha: f32, isotropic: bool) -> Self {
        Self {
            ltc,
            brdf,
            V,
            alpha,
            isotropic,
        }
    }

    pub fn update(&mut self, params: &[f32; 3]) {
        let m11 = params[0].max(EPSILON);
        let m22 = params[1].max(EPSILON);
        let m13 = params[2];

        if self.isotropic {
            self.ltc.m11 = m11;
            self.ltc.m22 = m11;
            self.ltc.m13 = 0.0;
        } else {
            self.ltc.m11 = m11;
            self.ltc.m22 = m22;
            self.ltc.m13 = m13;
        }
        self.ltc.update();
    }

    pub fn eval(&mut self, params: &[f32; 3]) -> f32 {
        self.update(params);
        compute_error(self.ltc, self.brdf, &self.V, self.alpha)
    }
}

/// Simple Nelder-Mead optimization implementation
pub fn nelder_mead<F>(
    start: &[f32; 3],
    epsilon: f32,
    tolerance: f32,
    max_iterations: usize,
    mut func: F,
) -> (f32, [f32; 3])
where
    F: FnMut(&[f32; 3]) -> f32,
{
    const ALPHA: f32 = 1.0; // Reflection
    const GAMMA: f32 = 2.0; // Expansion
    const RHO: f32 = 0.5; // Contraction
    const SIGMA: f32 = 0.5; // Shrink

    // Initialize simplex
    let mut simplex = Vec::new();
    simplex.push((*start, func(start)));

    for i in 0..3 {
        let mut point = *start;
        point[i] += epsilon;
        let value: f32 = func(&point);
        simplex.push((point, value));
    }

    for _iteration in 0..max_iterations {
        // Sort simplex by function value
        simplex.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

        let best = simplex[0].1;
        let worst = simplex[3].1;

        // Check convergence
        if (worst - best).abs() < tolerance {
            break;
        }

        // Compute centroid (excluding worst point)
        let mut centroid = [0.0; 3];
        for i in 0..3 {
            for j in 0..3 {
                centroid[j] += simplex[i].0[j];
            }
        }
        for i in 0..3 {
            centroid[i] /= 3.0;
        }

        // Reflection
        let mut reflected = [0.0; 3];
        for i in 0..3 {
            reflected[i] = centroid[i] + ALPHA * (centroid[i] - simplex[3].0[i]);
        }
        let reflected_value = func(&reflected);

        if reflected_value < simplex[2].1 && reflected_value >= simplex[0].1 {
            simplex[3] = (reflected, reflected_value);
            continue;
        }

        // Expansion
        if reflected_value < simplex[0].1 {
            let mut expanded = [0.0; 3];
            for i in 0..3 {
                expanded[i] = centroid[i] + GAMMA * (reflected[i] - centroid[i]);
            }
            let expanded_value = func(&expanded);

            if expanded_value < reflected_value {
                simplex[3] = (expanded, expanded_value);
            } else {
                simplex[3] = (reflected, reflected_value);
            }
            continue;
        }

        // Contraction
        let mut contracted = [0.0; 3];
        for i in 0..3 {
            contracted[i] = centroid[i] + RHO * (simplex[3].0[i] - centroid[i]);
        }
        let contracted_value = func(&contracted);

        if contracted_value < simplex[3].1 {
            simplex[3] = (contracted, contracted_value);
            continue;
        }

        // Shrink
        for i in 1..4 {
            for j in 0..3 {
                simplex[i].0[j] = simplex[0].0[j] + SIGMA * (simplex[i].0[j] - simplex[0].0[j]);
            }
            simplex[i].1 = func(&simplex[i].0);
        }
    }

    simplex.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    (simplex[0].1, simplex[0].0)
}

/// Fit LTC to BRDF
pub fn fit(
    ltc: &mut LTC,
    brdf: &dyn Brdf,
    V: &Vec3,
    alpha: f32,
    epsilon: f32,
    isotropic: bool,
) -> f32 {
    let start = [ltc.m11, ltc.m22, ltc.m13];

    let mut fitter = FitLTC::new(ltc, brdf, *V, alpha, isotropic);

    let (error, result) = nelder_mead(&start, epsilon, 1e-5, 100, |params| fitter.eval(params));

    fitter.update(&result);

    error
}

#[cfg(test)]
mod tests {
    use super::super::brdf_ggx::BrdfGGX;
    use super::*;

    #[test]
    fn test_compute_avg_terms() {
        let brdf = BrdfGGX::new();
        let V = Vec3::new(0.0, 0.0, 1.0);
        let alpha = 0.5;
        let (norm, fresnel, avg_dir) = compute_avg_terms(&brdf, &V, alpha);
        assert!(norm > 0.0);
        assert!(fresnel >= 0.0);
        assert!(avg_dir.length() > 0.0);
    }

    #[test]
    fn test_fit_ltc() {
        let brdf = BrdfGGX::new();
        let mut ltc = LTC::new();
        let V = Vec3::new(0.0, 0.0, 1.0);
        let alpha = 0.5;
        let error = fit(&mut ltc, &brdf, &V, alpha, 0.05, true);
        assert!(error >= 0.0);
    }
}
