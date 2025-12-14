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

/// Nelder-Mead optimization implementation
/// Faithful port from C++ ltc_code/fit/nelder_mead.h
/// Fixed for 3D optimization with simplex of 4 points
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
    const ALPHA: f32 = 1.0;   // Reflection coefficient
    const GAMMA: f32 = 2.0;   // Expansion coefficient
    const RHO: f32 = 0.5;     // Contraction coefficient
    const SIGMA: f32 = 0.5;   // Shrink coefficient
    const MIN_SUM: f32 = 1e-10; // Minimum sum threshold for termination check

    // Initialize simplex: 4 points for 3D
    let mut simplex: [[f32; 3]; 4] = [[0.0; 3]; 4];
    let mut values: [f32; 4] = [0.0; 4];

    // First point: starting point
    simplex[0] = *start;
    values[0] = func(start);

    // Other 3 points: perturb each dimension
    for i in 0..3 {
        simplex[i + 1] = *start;
        simplex[i + 1][i] += epsilon;
        values[i + 1] = func(&simplex[i + 1]);
    }

    for _iteration in 0..max_iterations {
        // Find lo (best), hi (worst), and nh (next-to-worst) indices by comparison
        let mut lo = 0;
        let mut hi = 0;
        let mut nh = 0;

        // Find lo (minimum)
        for i in 1..4 {
            if values[i] < values[lo] {
                lo = i;
            }
        }

        // Find hi (maximum)
        for i in 0..4 {
            if values[i] > values[hi] {
                hi = i;
            }
        }

        // Find nh (next-to-maximum): largest value that is not hi
        nh = if hi == 0 { 1 } else { 0 };
        for i in 0..4 {
            if i != hi && values[i] > values[nh] {
                nh = i;
            }
        }

        // Check termination: 2.0 * |hi - lo| < (|hi| + |lo|) * tolerance
        let a = values[hi];
        let b = values[lo];
        let sum = a.abs() + b.abs();
        
        // Edge case: if both values are near zero, consider converged
        if sum < MIN_SUM {
            break;
        }
        
        // Main convergence criterion from C++ reference
        if 2.0 * (a - b).abs() < sum * tolerance {
            break;
        }

        // Compute centroid (excluding worst point hi)
        let mut centroid = [0.0; 3];
        for i in 0..4 {
            if i != hi {
                for j in 0..3 {
                    centroid[j] += simplex[i][j];
                }
            }
        }
        for j in 0..3 {
            centroid[j] /= 3.0;
        }

        // Reflection: xr = centroid + alpha * (centroid - x[hi])
        let mut reflected = [0.0; 3];
        for j in 0..3 {
            reflected[j] = centroid[j] + ALPHA * (centroid[j] - simplex[hi][j]);
        }
        let reflected_value = func(&reflected);

        // If reflected is better than nh but not better than lo, accept it
        if reflected_value >= values[lo] && reflected_value < values[nh] {
            simplex[hi] = reflected;
            values[hi] = reflected_value;
            continue;
        }

        // Expansion: if reflected is better than lo
        if reflected_value < values[lo] {
            let mut expanded = [0.0; 3];
            for j in 0..3 {
                expanded[j] = centroid[j] + GAMMA * (reflected[j] - centroid[j]);
            }
            let expanded_value = func(&expanded);

            // Accept the better of expanded or reflected
            if expanded_value < reflected_value {
                simplex[hi] = expanded;
                values[hi] = expanded_value;
            } else {
                simplex[hi] = reflected;
                values[hi] = reflected_value;
            }
            continue;
        }

        // Contraction: if reflected is worse than nh
        let mut contracted = [0.0; 3];
        for j in 0..3 {
            contracted[j] = centroid[j] + RHO * (simplex[hi][j] - centroid[j]);
        }
        let contracted_value = func(&contracted);

        // If contraction is better than hi, accept it
        if contracted_value < values[hi] {
            simplex[hi] = contracted;
            values[hi] = contracted_value;
            continue;
        }

        // Reduction: shrink all points toward lo
        for i in 0..4 {
            if i != lo {
                for j in 0..3 {
                    simplex[i][j] = simplex[lo][j] + SIGMA * (simplex[i][j] - simplex[lo][j]);
                }
                values[i] = func(&simplex[i]);
            }
        }
    }

    // Find best point
    let mut lo = 0;
    for i in 1..4 {
        if values[i] < values[lo] {
            lo = i;
        }
    }

    (values[lo], simplex[lo])
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
