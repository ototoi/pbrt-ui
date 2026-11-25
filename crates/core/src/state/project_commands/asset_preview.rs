//! B: an image asset's preview. The verified bytes come from the shared
//! read texture previews also use; the image then follows the Image Viewer's
//! display rules — exr and hdr pass Reinhard → sRGB with opaque alpha,
//! plain formats keep their stored pixels — fits its long edge inside a
//! square and centers with a transparent letterbox.
use super::texture_preview::{cache_key, encode_png, verified_asset_bytes};
use project::{Asset, ProjectSnapshot};

/// Bumped whenever the decode, tone map, fit or PNG encoding change, so
/// cached previews from older code miss.
const PREVIEW_VERSION: &str = "asset-preview 2";

/// One decoded edge larger than this refuses the preview, checked from the
/// header before the decode so no worker time or memory is spent first.
const MAX_EDGE: u32 = 8192;

/// The cache key of `asset`'s preview at `size`: the asset's content, so
/// renaming or moving the file does not invalidate it.
pub(super) fn key(asset: &Asset, size: u32) -> String {
    cache_key(
        "asset",
        PREVIEW_VERSION,
        size,
        &[asset.sha256.clone(), asset.media_type.clone()],
    )
}

/// The preview as PNG bytes: the asset's verified bytes through the shared
/// decode-and-fit pipeline.
pub(super) fn png(
    snapshot: &ProjectSnapshot,
    asset_root: Option<&str>,
    asset: &Asset,
    size: u32,
) -> Result<Vec<u8>, String> {
    let bytes = verified_asset_bytes(Some(snapshot), asset_root, asset)?;
    encode_png(square(&bytes, &asset.media_type, size)?.into_raw(), size)
}

/// The fully decoded image fit inside a `size`×`size` square.
fn square(bytes: &[u8], media_type: &str, size: u32) -> Result<image::RgbaImage, String> {
    let format = image::ImageFormat::from_mime_type(media_type)
        .ok_or_else(|| format!("unsupported image type {media_type}"))?;
    // The size bound applies before the decode: a huge image refuses
    // instead of spending worker time and memory first (the desktop Image
    // Viewer makes the same check after decoding).
    {
        let mut reader = image::io::Reader::new(std::io::Cursor::new(bytes));
        reader.set_format(format);
        let (width, height) = reader
            .into_dimensions()
            .map_err(|error| format!("cannot read the image header: {error}"))?;
        if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE {
            return Err(format!(
                "image is too large to preview ({width} × {height}); the limit is {MAX_EDGE} × {MAX_EDGE}"
            ));
        }
    }
    // exr and hdr carry linear light through the shared Reinhard → sRGB pass
    // with opaque alpha; plain formats keep their stored pixels, like the
    // desktop Image Viewer.
    let decoded = if media_type == "image/vnd.radiance" {
        // The shared HDR decode keeps the float light the tone map needs.
        scene_parse::tonemap::reinhard_srgb(&scene_parse::hdr::decode(bytes)?.to_rgb32f())
    } else {
        let mut reader = image::io::Reader::new(std::io::Cursor::new(bytes));
        reader.set_format(format);
        let mut limits = image::io::Limits::default();
        limits.max_image_width = Some(16_384);
        limits.max_image_height = Some(16_384);
        limits.max_alloc = Some(1024 * 1024 * 1024);
        reader.limits(limits);
        let decoded = reader
            .decode()
            .map_err(|error| format!("cannot decode the image: {error}"))?;
        if media_type == "image/x-exr" {
            scene_parse::tonemap::reinhard_srgb(&decoded.to_rgb32f())
        } else {
            decoded.to_rgba8()
        }
    };
    let (fitted_width, fitted_height) = fitted_size(decoded.width(), decoded.height(), size);
    let fitted = image::imageops::thumbnail(&decoded, fitted_width, fitted_height);
    // The letterbox is transparent: no theme or background color leaks in.
    // `replace`, not `overlay`: copying keeps stored pixels exact, while
    // blending onto the transparent canvas can shift partial-alpha colors.
    let mut square = image::RgbaImage::new(size, size);
    image::imageops::replace(
        &mut square,
        &fitted,
        i64::from((size - fitted_width) / 2),
        i64::from((size - fitted_height) / 2),
    );
    Ok(square)
}

/// The fit inside the square: the long edge maps to `size`, the short edge
/// scales by the same factor but never rounds below one pixel.
fn fitted_size(width: u32, height: u32, size: u32) -> (u32, u32) {
    if width >= height {
        (size, scaled_edge(height, width, size))
    } else {
        (scaled_edge(width, height, size), size)
    }
}

fn scaled_edge(short: u32, long: u32, size: u32) -> u32 {
    (f64::from(size) * f64::from(short) / f64::from(long))
        .round()
        .max(1.) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(image: &image::RgbaImage) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut bytes, image::ImageOutputFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    #[test]
    fn wide_images_fit_their_long_edge_and_keep_a_transparent_letterbox() {
        let image = image::RgbaImage::from_pixel(8, 4, image::Rgba([255, 0, 0, 255]));
        let content = image::Rgba([255, 0, 0, 255]);
        let square = square(&png_bytes(&image), "image/png", 8).unwrap();
        assert_eq!(square.dimensions(), (8, 8));
        // 8×4 inside 8×8 lands at (0, 2): content rows 2..6.
        assert_eq!(square.get_pixel(0, 0), &image::Rgba([0, 0, 0, 0]));
        assert_eq!(square.get_pixel(0, 1), &image::Rgba([0, 0, 0, 0]));
        assert_eq!(square.get_pixel(0, 2), &content);
        assert_eq!(square.get_pixel(7, 5), &content);
        assert_eq!(square.get_pixel(0, 6), &image::Rgba([0, 0, 0, 0]));
        assert_eq!(square.get_pixel(7, 7), &image::Rgba([0, 0, 0, 0]));
    }

    #[test]
    fn tall_images_fit_their_long_edge_and_keep_a_transparent_letterbox() {
        let image = image::RgbaImage::from_pixel(4, 8, image::Rgba([0, 255, 0, 255]));
        let content = image::Rgba([0, 255, 0, 255]);
        let square = square(&png_bytes(&image), "image/png", 8).unwrap();
        assert_eq!(square.dimensions(), (8, 8));
        // 4×8 inside 8×8 lands at (2, 0): content columns 2..6.
        assert_eq!(square.get_pixel(0, 0), &image::Rgba([0, 0, 0, 0]));
        assert_eq!(square.get_pixel(1, 7), &image::Rgba([0, 0, 0, 0]));
        assert_eq!(square.get_pixel(2, 0), &content);
        assert_eq!(square.get_pixel(5, 7), &content);
        assert_eq!(square.get_pixel(6, 0), &image::Rgba([0, 0, 0, 0]));
        assert_eq!(square.get_pixel(7, 7), &image::Rgba([0, 0, 0, 0]));
    }

    #[test]
    fn radiance_hdr_tone_maps_its_float_samples_like_the_image_viewer() {
        // Light above 1.0 must reach the tone map: 4.0 maps to ≈231, while
        // a decode clamped at 1.0 (image 0.24's DynamicImage path) gives 188.
        let pixels = vec![image::Rgb([4.0f32, 0.25, 1.0]); 4];
        let mut bytes = Vec::new();
        image::codecs::hdr::HdrEncoder::new(&mut bytes)
            .encode(&pixels, 2, 2)
            .unwrap();
        let square = square(&bytes, "image/vnd.radiance", 2).unwrap();
        let expected = scene_parse::tonemap::reinhard_channel(4.0);
        let red = square.get_pixel(0, 0)[0];
        assert!(
            red.abs_diff(expected) <= 1,
            "red {red}, expected ≈{expected}"
        );
        assert!(red > 220, "a clamped decode would give 188, got {red}");
        assert_eq!(square.get_pixel(0, 0)[3], 255);
    }

    #[test]
    fn the_short_edge_never_rounds_below_one_pixel() {
        assert_eq!(fitted_size(8192, 1, 64), (64, 1));
        assert_eq!(fitted_size(1, 8192, 64), (1, 64));
        assert_eq!(fitted_size(8192, 2, 64), (64, 1));
        assert_eq!(fitted_size(64, 48, 64), (64, 48));
    }
}
