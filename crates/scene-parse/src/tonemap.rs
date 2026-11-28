//! The shared display pass for HDR images: Reinhard tone
//! mapping into sRGB, used by the capture adapter and the Image Viewer so a
//! captured preview and an opened EXR/HDR render show the same colors.
//! Non-finite and negative samples collapse to black; alpha is always opaque.
//!
//! [`reinhard_channel`] is the version-free primitive (`f32 -> u8`): desktop
//! decodes through `image 0.25` while this crate uses the workspace pin, so
//! buffers do not cross the boundary — only the math does.

/// One sample through Reinhard (value / (1 + value)) into sRGB display space.
/// Non-finite and negative samples collapse to black.
pub fn reinhard_channel(value: f32) -> u8 {
    let value = if value.is_finite() { value.max(0.) } else { 0. };
    let mapped = value / (1. + value);
    let srgb = if mapped <= 0.0031308 {
        12.92 * mapped
    } else {
        1.055 * mapped.powf(1. / 2.4) - 0.055
    };
    (srgb.clamp(0., 1.) * 255.).round() as u8
}

/// Reinhard tone map into sRGB, ready for display (alpha always opaque).
pub fn reinhard_srgb(image: &image::Rgb32FImage) -> image::RgbaImage {
    image::RgbaImage::from_fn(image.width(), image.height(), |x, y| {
        let pixel = image.get_pixel(x, y);
        image::Rgba([
            reinhard_channel(pixel[0]),
            reinhard_channel(pixel[1]),
            reinhard_channel(pixel[2]),
            255,
        ])
    })
}

#[cfg(test)]
mod tests {
    use super::reinhard_srgb;

    fn sample(values: [f32; 3]) -> image::Rgb32FImage {
        image::Rgb32FImage::from_raw(1, 1, values.to_vec()).unwrap()
    }

    #[test]
    fn black_stays_black_and_alpha_is_opaque() {
        let display = reinhard_srgb(&sample([0., 0., 0.]));
        assert_eq!(display.get_pixel(0, 0), &image::Rgba([0, 0, 0, 255]));
    }

    #[test]
    fn unit_white_maps_to_the_expected_display_level() {
        let display = reinhard_srgb(&sample([1., 1., 1.]));
        let level = display.get_pixel(0, 0)[0];
        // 0.5 linear maps to ≈0.735 sRGB, so a clean white lands near 188.
        assert!((186..=190).contains(&level), "unexpected level {level}");
    }

    #[test]
    fn bright_values_flatten_and_never_clip_channels() {
        let dim = reinhard_srgb(&sample([1., 1., 1.]));
        let bright = reinhard_srgb(&sample([100., 100., 100.]));
        let blinding = reinhard_srgb(&sample([f32::MAX, f32::MAX, f32::MAX]));
        assert!(bright.get_pixel(0, 0)[0] > dim.get_pixel(0, 0)[0]);
        assert!(bright.get_pixel(0, 0)[0] < 255);
        assert!(
            blinding.get_pixel(0, 0)[0] >= bright.get_pixel(0, 0)[0],
            "mapping must stay monotonic"
        );
    }

    #[test]
    fn non_finite_and_negative_samples_collapse_to_black() {
        let display = reinhard_srgb(&sample([f32::NAN, -1., 0.25]));
        let pixel = display.get_pixel(0, 0);
        assert_eq!(pixel[0], 0, "NaN has nothing to show");
        assert_eq!(pixel[1], 0, "negative light does not exist");
        assert!(pixel[2] > 0, "finite positive channels survive");
        assert!(pixel[3] == 255);
    }
}
