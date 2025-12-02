use pbrt_ui::ltc::brdf::Brdf;
use pbrt_ui::ltc::brdf_beckmann::BrdfBeckmann;
use pbrt_ui::ltc::brdf_disneydiffuse::BrdfDisneyDiffuse;
use pbrt_ui::ltc::brdf_ggx::BrdfGGX;
use pbrt_ui::ltc::fitter::{compute_avg_terms, fit};
use pbrt_ui::ltc::ltc::LTC;
use pbrt_ui::ltc::parameters::{MIN_ALPHA, N};
use pbrt_ui::ltc::sphere_tab::gen_sphere_tab;

use std::sync::Arc;

use clap::*;
use glam::{Mat3, Vec2, Vec3};

#[derive(Debug, Parser)]
#[clap(author, about, version, disable_help_flag = true)]
struct Options {
    #[arg(default_value = "ggx")]
    brdf: String,
}

fn create_brdf(brdf_name: &str) -> Result<Arc<dyn Brdf>, String> {
    match brdf_name {
        "ggx" => Ok(Arc::new(BrdfGGX::new())),
        "beckmann" => Ok(Arc::new(BrdfBeckmann::new())),
        "disneydiffuse" => Ok(Arc::new(BrdfDisneyDiffuse::new())),
        _ => Err(format!("Unknown BRDF name: {}", brdf_name)),
    }
}

/// Fit LTC table for a given BRDF
fn fit_tab(brdf: &dyn Brdf) -> (Vec<Mat3>, Vec<Vec2>) {
    let mut tab = vec![Mat3::IDENTITY; N * N];
    let mut tab_mag_fresnel = vec![Vec2::ZERO; N * N];

    // Loop over theta and alpha
    for a in (0..N).rev() {
        for t in 0..N {
            // Parameterized by sqrt(1 - cos(theta))
            let x = t as f32 / (N - 1) as f32;
            let ct = 1.0 - x * x;
            let theta = std::f32::consts::FRAC_PI_2.min(ct.acos());
            let V = Vec3::new(theta.sin(), 0.0, theta.cos());

            // alpha = roughness^2
            let roughness = a as f32 / (N - 1) as f32;
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

                if a == N - 1 {
                    // roughness = 1
                    ltc.m11 = 1.0;
                    ltc.m22 = 1.0;
                } else {
                    // Init with roughness of previous fit
                    ltc.m11 = tab[a + 1 + t * N].col(0).x;
                    ltc.m22 = tab[a + 1 + t * N].col(1).y;
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
            tab[a + t * N] = ltc.M;
            tab_mag_fresnel[a + t * N] = Vec2::new(ltc.magnitude, ltc.fresnel);

            // Print matrix
            println!("{:.6}\t {:.6}\t {:.6}", ltc.M.col(0).x, ltc.M.col(1).x, ltc.M.col(2).x);
            println!("{:.6}\t {:.6}\t {:.6}", ltc.M.col(0).y, ltc.M.col(1).y, ltc.M.col(2).y);
            println!("{:.6}\t {:.6}\t {:.6}", ltc.M.col(0).z, ltc.M.col(1).z, ltc.M.col(2).z);
            println!();
        }
    }

    (tab, tab_mag_fresnel)
}

fn main() -> Result<(), String> {
    let options = Options::parse();
    let brdf = create_brdf(&options.brdf)?;

    println!("Fitting LTC table for BRDF: {}", options.brdf);
    println!("Table size: {}x{}", N, N);
    println!();

    let (_tab, _tab_mag_fresnel) = fit_tab(brdf.as_ref());

    println!();
    println!("Generating sphere table...");
    let _tab_sphere = gen_sphere_tab(N);
    println!("Sphere table generated successfully!");

    println!();
    println!("LTC fitting completed successfully!");
    
    Ok(())
}