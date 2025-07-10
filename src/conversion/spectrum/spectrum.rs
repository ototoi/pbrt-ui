use super::config::*;

#[derive(Debug, PartialEq, Clone, Copy)]
pub struct Spectrum {
    pub c: [f32; SPECTRAL_SAMPLES],
}
