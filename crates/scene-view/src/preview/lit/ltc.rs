pub(crate) const LTC_SIZE: u32 = 64;
const HEADER_SIZE: usize = 8;
const LAYER_FLOATS: usize = LTC_SIZE as usize * LTC_SIZE as usize * 4;
const DATA: &[u8] = include_bytes!("ltc_ggx.bin");

pub(crate) fn ltc_layers() -> [Vec<f32>; 2] {
    assert_eq!(&DATA[..4], b"LTC1");
    assert_eq!(u32::from_le_bytes(DATA[4..8].try_into().unwrap()), LTC_SIZE);
    let decode = |layer: &[u8]| {
        layer
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| f32::from_le_bytes(*bytes))
            .collect()
    };
    let data = &DATA[HEADER_SIZE..];
    let split = LAYER_FLOATS * 4;
    [decode(&data[..split]), decode(&data[split..])]
}

/// Table coordinates in [0, 1]² for a GGX α and the view's cosθ. The fitter
/// stores entry `a + t·64` (`fit_tab`): across (x) is `sqrt(α)`, down (y) is
/// `sqrt(1 − cosθ)`.
#[cfg(test)]
pub(crate) fn ltc_uv(alpha: f32, cos_theta: f32) -> [f32; 2] {
    [
        alpha.clamp(0.0, 1.0).sqrt(),
        (1.0 - cos_theta.clamp(0.0, 1.0)).sqrt(),
    ]
}

/// Nearest table entry (layer 1: magnitude, Fresnel, 0, sphere) for tests.
#[cfg(test)]
fn terms_at(alpha: f32, cos_theta: f32) -> [f32; 4] {
    let [x, y] = ltc_uv(alpha, cos_theta);
    let ix = (x * (LTC_SIZE - 1) as f32).round() as usize;
    let iy = (y * (LTC_SIZE - 1) as f32).round() as usize;
    let at = (iy * LTC_SIZE as usize + ix) * 4;
    ltc_layers()[1][at..at + 4].try_into().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The published GGX fit (selfshadow/ltc_code), as the old
    /// implementation embedded it (`bd3edc3^:legacy/src/render/wgpu/ltc/
    /// textures/ltc_ggx.rs`). The matrix layer is a numerical fit, so ours
    /// matches it to 0.03 absolute; the magnitude/Fresnel layer is an
    /// integral and matches to 1e-3 relative.
    fn near_matrix(actual: f32, expected: f32) {
        assert!((actual - expected).abs() <= 0.03, "{actual} != {expected}");
    }
    fn near_terms(actual: f32, expected: f32) {
        let tolerance = (expected.abs() * 1e-3).max(1e-4);
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} != {expected}"
        );
    }

    #[test]
    fn ltc_table_has_two_rgba_layers_of_64_squared() {
        assert_eq!(&DATA[..4], b"LTC1");
        assert_eq!(u32::from_le_bytes(DATA[4..8].try_into().unwrap()), LTC_SIZE);
        assert_eq!(
            DATA.len(),
            8 + 2 * LTC_SIZE as usize * LTC_SIZE as usize * 4 * 4
        );
        assert_eq!(ltc_layers()[0].len(), LAYER_FLOATS);
        assert_eq!(ltc_layers()[1].len(), LAYER_FLOATS);
    }

    #[test]
    fn ltc_table_matches_published_spot_values() {
        let layers = ltc_layers();
        let (matrix, terms) = (&layers[0], &layers[1]);
        for (index, expected) in [
            (0, 1.0),
            (3, 2e-5),
            (7, 0.000503905),
            (255, 1.12813),
            (8192, 0.538716),
            (8320, 0.626092),
            (16380, 0.996389),
        ] {
            near_matrix(matrix[index], expected);
        }
        for (index, expected) in [
            (8320, 0.88238),
            (8321, 0.00296637),
            (8323, 0.19188),
            (16128, 0.999804),
            (16129, 0.995833),
        ] {
            near_terms(terms[index], expected);
        }
        assert!(layers.iter().flatten().all(|v| v.is_finite()));
    }

    /// Fresnel weight grows toward grazing views: (1 − cosθ)^5 at α → 0.
    /// This pins which axis is the view angle.
    #[test]
    fn fresnel_weight_grows_toward_grazing_views() {
        assert!(terms_at(1e-5, 1.0)[1] < 0.01);
        assert!(terms_at(1e-5, 0.0)[1] > 0.9);
    }

    #[test]
    fn magnitude_is_one_at_normal_incidence_for_smooth_ggx() {
        let uv = ltc_uv(1e-5, 1.0);
        let layers = ltc_layers();
        let layer = &layers[1];
        let ix = (uv[0] * (LTC_SIZE - 1) as f32).round() as usize;
        let iy = (uv[1] * (LTC_SIZE - 1) as f32).round() as usize;
        let magnitude = layer[(iy * LTC_SIZE as usize + ix) * 4];
        assert!((magnitude - 1.0).abs() <= 0.02);
    }
}
