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

            if pdf > 0.0 && eval > 0.0 {
                let weight = eval / pdf;
                let fresnel_val = brdf.fresnel(V, &L);
                // Accumulate
                norm += weight;
                fresnel += weight * fresnel_val;
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
fn compute_error_helper(error: f64, pdf: f64) -> f64 {
    if error.abs() < 1e-12 && pdf.abs() < 1e-12 {
        0.0
    } else {
        error / pdf
    }
}

/// Compute the error between the BRDF and the LTC using Multiple Importance Sampling
pub fn compute_error(ltc: &LTC, brdf: &dyn Brdf, V: &Vec3, alpha: f32) -> f32 {
    let mut error: f64 = 0.0;

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
                let error_ = (eval_brdf - eval_ltc).abs() as f64;
                let error_ = error_ * error_ * error_;
                error += compute_error_helper(error_, pdf_ltc as f64 + pdf_brdf as f64);
            }

            // Importance sample BRDF
            {
                let L = brdf.sample(V, alpha, U1, U2);

                let (eval_brdf, pdf_brdf) = brdf.eval(V, &L, alpha);
                let eval_ltc = ltc.eval(&L);
                let pdf_ltc = eval_ltc / ltc.magnitude;

                // Error with MIS weight
                let error_ = (eval_brdf - eval_ltc).abs() as f64;
                let error_ = error_ * error_ * error_;
                error += compute_error_helper(error_, pdf_ltc as f64 + pdf_brdf as f64);
            }
        }
    }

    error as f32 / (NSAMPLE * NSAMPLE) as f32
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
    delta: f32,
    tolerance: f32,
    max_iters: usize,
    mut objective_fn: F,
) -> (f32, [f32; 3])
where
    F: FnMut(&[f32; 3]) -> f32,
{
    const DIM: usize = 3;
    const NB_POINTS: usize = DIM + 1;
    const REFLECT: f32 = 1.0;
    const EXPAND: f32 = 2.0;
    const CONTRACT: f32 = 0.5;
    const SHRINK: f32 = 0.5;

    type Point = [f32; DIM];

    let mut s = [[0.0; DIM]; NB_POINTS];
    let mut f = [0.0; NB_POINTS];

    // Initialize simplex
    s[0] = *start;
    for i in 1..NB_POINTS {
        s[i] = *start;
        s[i][i - 1] += delta;
    }

    for i in 0..NB_POINTS {
        f[i] = objective_fn(&s[i]);
    }

    let (mut lo, mut hi, mut nh);

    for _ in 0..max_iters {
        // lo, hi, nh
        lo = 0;
        hi = 0;
        nh = 0;
        for i in 1..NB_POINTS {
            if f[i] < f[lo] {
                lo = i;
            }
            if f[i] > f[hi] {
                nh = hi;
                hi = i;
            } else if f[i] > f[nh] {
                nh = i;
            }
        }

        // termination condition
        let a = f[lo].abs();
        let b = f[hi].abs();
        if 2.0 * (a - b).abs() < (a + b) * tolerance {
            break;
        }

        // centroid
        let mut o = [0.0; DIM];
        for i in 0..NB_POINTS {
            if i == hi {
                continue;
            }
            for j in 0..DIM {
                o[j] += s[i][j];
            }
        }
        for j in 0..DIM {
            o[j] /= DIM as f32;
        }

        // reflection
        let mut r = [0.0; DIM];
        for j in 0..DIM {
            r[j] = o[j] + REFLECT * (o[j] - s[hi][j]);
        }
        let fr = objective_fn(&r);

        if fr < f[nh] {
            if fr < f[lo] {
                // expansion
                let mut e = [0.0; DIM];
                for j in 0..DIM {
                    e[j] = o[j] + EXPAND * (o[j] - s[hi][j]);
                }
                let fe = objective_fn(&e);
                if fe < fr {
                    s[hi] = e;
                    f[hi] = fe;
                    continue;
                }
            }
            s[hi] = r;
            f[hi] = fr;
            continue;
        }

        // contraction
        let mut c = [0.0; DIM];
        for j in 0..DIM {
            c[j] = o[j] - CONTRACT * (o[j] - s[hi][j]);
        }
        let fc = objective_fn(&c);
        if fc < f[hi] {
            s[hi] = c;
            f[hi] = fc;
            continue;
        }

        // shrink
        for k in 0..NB_POINTS {
            if k == lo {
                continue;
            }
            for j in 0..DIM {
                s[k][j] = s[lo][j] + SHRINK * (s[k][j] - s[lo][j]);
            }
            f[k] = objective_fn(&s[k]);
        }
    }

    // re-search minimum location
    lo = 0;
    for i in 1..NB_POINTS {
        if f[i] < f[lo] {
            lo = i;
        }
    }
    (f[lo], s[lo])
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
