mod config;
mod data_xyz;
mod float_file;
mod spectrum;
mod utils;

pub use spectrum::Spectrum;

use crate::error::PbrtError;
use config::*;
use data_xyz::*;
use float_file::*;
use log;
use utils::*;

impl Spectrum {
    pub fn from_sampled(lambda: &[f32], vals: &[f32]) -> Spectrum {
        if !spectrum_samples_sorted(lambda, vals) {
            let mut slambda = Vec::new();
            let mut sv = Vec::new();
            slambda.extend_from_slice(lambda);
            sv.extend_from_slice(vals);
            sort_spectrum_samples(&mut slambda, &mut sv);
            return Self::from_sampled(&slambda, &sv);
        }
        return Spectrum {
            c: sample_spectrum(lambda, vals),
        };
    }

    pub fn load_from_file(path: &str) -> Result<Spectrum, PbrtError> {
        match read_float_file(path) {
            Ok(vals) => {
                if vals.len() % 2 != 0 {
                    log::warn!(
                        "Extra value found in spectrum file \"{}\". Ignoring it.",
                        path
                    );
                }
                let mut wls = Vec::new();
                let mut v = Vec::new();
                for j in 0..(vals.len() / 2) {
                    wls.push(vals[2 * j] as f32);
                    v.push(vals[2 * j + 1] as f32);
                }
                return Ok(Spectrum::from_sampled(&wls, &v));
            }
            Err(e) => {
                return Err(e);
            }
        }
    }

    pub fn to_xyz(&self) -> [f32; 3] {
        let c = &self.c;
        let mut xyz: [f32; 3] = [0.0; 3];
        for i in 0..c.len() {
            xyz[0] += ARRAY_CIE_X[i] * c[i];
            xyz[1] += ARRAY_CIE_Y[i] * c[i];
            xyz[2] += ARRAY_CIE_Z[i] * c[i];
        }
        let scale = (SAMPLED_LAMBDA_END - SAMPLED_LAMBDA_START) as f32
            / (CIE_Y_INTEGRAL * SPECTRAL_SAMPLES as f32);
        xyz[0] *= scale;
        xyz[1] *= scale;
        xyz[2] *= scale;
        return xyz;
    }

    pub fn to_rgb(&self) -> [f32; 3] {
        let xyz = self.to_xyz();
        return xyz_to_rgb(&xyz);
    }
}
