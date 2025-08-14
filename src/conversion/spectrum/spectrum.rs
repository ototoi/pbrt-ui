use super::config::*;

#[derive(Debug, PartialEq, Clone, Copy)]
pub struct Spectrum {
    pub c: [f32; SPECTRAL_SAMPLES],
}

impl std::ops::AddAssign<Spectrum> for Spectrum {
    #[inline]
    fn add_assign(&mut self, rhs: Spectrum) {
        for i in 0..SPECTRAL_SAMPLES {
            self.c[i] += rhs.c[i];
        }
    }
}

impl std::ops::Mul<f32> for Spectrum {
    type Output = Spectrum;
    #[inline]
    fn mul(self, rhs: f32) -> Spectrum {
        let mut result = Spectrum {
            c: [0.0; SPECTRAL_SAMPLES],
        };
        for i in 0..SPECTRAL_SAMPLES {
            result.c[i] = self.c[i] * rhs;
        }
        return result;
    }
}

impl std::ops::Mul<Spectrum> for Spectrum {
    type Output = Spectrum;
    #[inline]
    fn mul(self, rhs: Spectrum) -> Spectrum {
        let mut result = Spectrum {
            c: [0.0; SPECTRAL_SAMPLES],
        };
        for i in 0..SPECTRAL_SAMPLES {
            result.c[i] = self.c[i] * rhs.c[i];
        }
        return result;
    }
}
