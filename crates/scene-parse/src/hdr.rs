//! Radiance HDR decoding shared by core and scene-view. image 0.24 decodes
//! `.hdr` through `DynamicImage` as Rgb8 — clamped at 1.0 with gamma 2.2 —
//! while r4 and the desktop Image Viewer (image 0.25) read float light
//! values. Callers that know they hold an HDR image decode it here instead.

/// The largest edge any decode path here accepts.
pub const MAX_EDGE: u32 = 16_384;
/// The largest decoded sample buffer, like the other paths' 1 GiB limit.
pub const MAX_ALLOC: u64 = 1024 * 1024 * 1024;

/// Whether `bytes` are a Radiance HDR file, from its magic or (for files
/// written with the shorter `#?RGBE` header) the file name's extension.
pub fn is_hdr(bytes: &[u8], file_name: impl AsRef<std::path::Path>) -> bool {
    bytes.starts_with(b"#?RADIANCE")
        || bytes.starts_with(b"#?RGBE")
        || file_name
            .as_ref()
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("hdr"))
}

/// The float samples of a Radiance HDR file, as linear light.
pub fn decode(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    let decoder = image::codecs::hdr::HdrDecoder::new(std::io::Cursor::new(bytes))
        .map_err(|error| format!("cannot decode the HDR image: {error}"))?;
    let metadata = decoder.metadata();
    // HdrDecoder takes no image::io::Limits: bound the edge and the f32 RGB
    // allocation like the other decode paths before allocating samples.
    let bytes_needed = u64::from(metadata.width) * u64::from(metadata.height) * 12;
    if metadata.width == 0
        || metadata.height == 0
        || metadata.width > MAX_EDGE
        || metadata.height > MAX_EDGE
        || bytes_needed > MAX_ALLOC
    {
        return Err(format!(
            "cannot decode the HDR image: {} × {} is too large to decode",
            metadata.width, metadata.height
        ));
    }
    let samples = decoder
        .read_image_hdr()
        .map_err(|error| format!("cannot decode the HDR image: {error}"))?;
    image::Rgb32FImage::from_raw(
        metadata.width,
        metadata.height,
        samples.into_iter().flat_map(|pixel| pixel.0).collect(),
    )
    .map(image::DynamicImage::ImageRgb32F)
    .ok_or_else(|| "cannot decode the HDR image: bad buffer".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(pixel: [f32; 3]) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::hdr::HdrEncoder::new(&mut bytes)
            .encode(&[image::Rgb(pixel); 4], 2, 2)
            .unwrap();
        bytes
    }

    #[test]
    fn hdr_samples_keep_light_above_one_and_stay_linear() {
        // RGBE stores 4.0, 0.25 and 1.0 exactly under one shared exponent.
        let image = decode(&encoded([4.0, 0.25, 1.0])).unwrap();
        let image::DynamicImage::ImageRgb32F(image) = image else {
            panic!("HDR must decode to float samples");
        };
        assert_eq!(image.dimensions(), (2, 2));
        let [r, g, b] = image.get_pixel(1, 1).0;
        assert!((r - 4.0).abs() < 0.05, "light above 1.0 survives: {r}");
        assert!((g - 0.25).abs() < 0.01, "no gamma is applied: {g}");
        assert!((b - 1.0).abs() < 0.02, "{b}");
    }

    #[test]
    fn hdr_files_are_recognized_by_magic_or_extension() {
        assert!(is_hdr(&encoded([1.0; 3]), "payload.bin"));
        assert!(is_hdr(b"#?RGBE\n", "sky"));
        assert!(is_hdr(b"anything", "Sky.HDR"));
        assert!(!is_hdr(b"\x89PNG\r\n\x1a\n", "sky.png"));
    }
}
