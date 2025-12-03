#![allow(non_snake_case)]

use super::brdf::Brdf;
use super::fitter::{compute_avg_terms, fit};
use super::ltc::LTC;
use super::parameters::MIN_ALPHA;
use glam::{Mat3, Vec2, Vec3};

/// Fit LTC table for a given BRDF
/// 
/// # Arguments
/// * `brdf` - The BRDF to fit
/// * `width` - Width of the table (theta dimension)
/// * `height` - Height of the table (alpha/roughness dimension)
/// 
/// # Returns
/// * Tuple of (tab, tab_mag_fresnel) where both are flattened width*height vectors
pub fn fit_tab(brdf: &dyn Brdf, width: usize, height: usize) -> (Vec<Mat3>, Vec<Vec2>) {
    let mut tab = vec![Mat3::IDENTITY; width * height];
    let mut tab_mag_fresnel = vec![Vec2::ZERO; width * height];

    // Loop over theta and alpha
    for a in (0..height).rev() {
        for t in 0..width {
            // Parameterized by sqrt(1 - cos(theta))
            let x = t as f32 / (width - 1) as f32;
            let ct = 1.0 - x * x;
            let theta = std::f32::consts::FRAC_PI_2.min(ct.acos());
            let V = Vec3::new(theta.sin(), 0.0, theta.cos());

            // alpha = roughness^2
            let roughness = a as f32 / (height - 1) as f32;
            let alpha = (roughness * roughness).max(MIN_ALPHA);

            println!("a = {}\t t = {}", a, t);
            println!("alpha = {}\t theta = {}", alpha, theta);
            println!();

            let mut ltc = LTC::new();
            
            // Compute average terms
            let (magnitude, fresnel, average_dir) = compute_avg_terms(brdf, &V, alpha);
            ltc.magnitude = magnitude;
            ltc.fresnel = fresnel;

            let isotropic;

            // First guess for the fit
            if t == 0 {
                // If theta == 0, the lobe is rotationally symmetric and aligned with Z
                ltc.X = Vec3::new(1.0, 0.0, 0.0);
                ltc.Y = Vec3::new(0.0, 1.0, 0.0);
                ltc.Z = Vec3::new(0.0, 0.0, 1.0);

                if a == height - 1 {
                    // roughness = 1
                    ltc.m11 = 1.0;
                    ltc.m22 = 1.0;
                } else if a + 1 < height {
                    // Init with roughness of previous fit
                    ltc.m11 = tab[a + 1 + t * height].col(0).x;
                    ltc.m22 = tab[a + 1 + t * height].col(1).y;
                }

                ltc.m13 = 0.0;
                ltc.update();

                isotropic = true;
            } else {
                // Use previous configuration as first guess
                let L = average_dir;
                let T1 = Vec3::new(L.z, 0.0, -L.x);
                let T2 = Vec3::new(0.0, 1.0, 0.0);
                ltc.X = T1;
                ltc.Y = T2;
                ltc.Z = L;

                ltc.update();

                isotropic = false;
            }

            // Fit (explore parameter space and refine first guess)
            let epsilon = 0.05;
            fit(&mut ltc, brdf, &V, alpha, epsilon, isotropic);

            // Copy data
            tab[a + t * height] = ltc.invM;  // Store inverse matrix for packing
            tab_mag_fresnel[a + t * height] = Vec2::new(ltc.magnitude, ltc.fresnel);

            // Print matrix
            println!("{:.6}\t {:.6}\t {:.6}", ltc.M.col(0).x, ltc.M.col(1).x, ltc.M.col(2).x);
            println!("{:.6}\t {:.6}\t {:.6}", ltc.M.col(0).y, ltc.M.col(1).y, ltc.M.col(2).y);
            println!("{:.6}\t {:.6}\t {:.6}", ltc.M.col(0).z, ltc.M.col(1).z, ltc.M.col(2).z);
            println!();
        }
    }

    (tab, tab_mag_fresnel)
}
