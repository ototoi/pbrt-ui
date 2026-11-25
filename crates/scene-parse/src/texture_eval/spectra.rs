//! Spectra as the RGB triples the previews draw with.
//!
//! The tables and the reads live in the `spectrum` crate, one
//! implementation for Lit and the UI's colour boxes; this module keeps
//! the scene-parse API as an `f32` view of it.
//!
//! A spectrum is read at one representative wavelength per channel:
//! 630 nm (R), 532 nm (G) and 465 nm (B). The builtin names are pbrt-r4's
//! (16b0aab) named spectra sampled there, printed once by a throwaway
//! program linked against r4 (never a pbrt-ui dependency).
use super::Rgb;

/// The wavelengths, in nm, each channel reads.
pub const WAVELENGTHS: [f32; 3] = [630., 532., 465.];

/// A builtin named spectrum (metal eta / k, glass eta), if known.
pub fn named(name: &str) -> Option<Rgb> {
    spectrum::named(name).map(|rgb| rgb.map(|value| value as f32))
}

/// A sampled spectrum given as `λ value` pairs (ascending λ), read
/// piecewise linearly and held constant past either end, as pbrt does.
pub fn sampled(pairs: &[f32]) -> Option<Rgb> {
    let pairs: Vec<f64> = pairs.iter().map(|value| f64::from(*value)).collect();
    spectrum::channel_rgb(&pairs).map(|rgb| rgb.map(|value| value as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_spectra_read_r4_at_the_channel_wavelengths() {
        assert_eq!(
            named("metal-Cu-eta"),
            Some([0.24588375, 1.072285, 1.156862])
        );
        assert_eq!(named("glass-BK7").map(|v| v[1]), Some(1.5194908));
        assert_eq!(named("metal-Cu"), None, "exact names only, as r4");
    }

    #[test]
    fn sampled_spectra_interpolate_and_hold_their_ends() {
        let rgb = sampled(&[400., 0., 500., 1., 600., 2.]).unwrap();
        for (got, want) in rgb.into_iter().zip([2., 1.32, 0.65]) {
            assert!((got - want).abs() < 1e-5, "{rgb:?}");
        }
        assert_eq!(sampled(&[400., 1.]), Some([1.; 3]));
        assert_eq!(sampled(&[400.]), None);
    }
}
