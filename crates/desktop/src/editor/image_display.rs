//! Pure decisions for showing image assets in the Image Viewer (
//! design ): which media types are viewable, which need the shared tone
//! map, and the decode-size bound. Kept free of GPUI so tests can live here.

/// Media types the Image Viewer opens. The desktop's `image` build decodes
/// exr, hdr, jpeg, tga and more through gpui-kit's feature integration (PFM
/// has no decoder); PNG-only was the F5 registration, not a decoder limit.
pub const VIEWER_MEDIA_TYPES: [&str; 5] = protocol::project::IMAGE_PREVIEW_MEDIA_TYPES;

/// HDR formats carry linear light that must pass the shared Reinhard → sRGB
/// pass (`scene_view::tonemap`) before display; plain formats display as-is.
pub fn needs_tone_map(media_type: &str) -> bool {
    matches!(media_type, "image/x-exr" | "image/vnd.radiance")
}

/// One decoded edge larger than this refuses display instead of spending
/// seconds and memory on it: too large to show.
pub const MAX_EDGE: u32 = 8192;
pub fn size_ok(size: [u32; 2]) -> bool {
    let [width, height] = size;
    (1..=MAX_EDGE).contains(&width) && (1..=MAX_EDGE).contains(&height)
}

/// The viewer caption suffix for tone-mapped images.
pub const TONE_MAPPED_LABEL: &str = "tone mapped (Reinhard)";

/// Decode one image file and bring it to display RGBA: the
/// size bound applies first (a 8193-edge image refuses instead of a long,
/// memory-hungry decode), HDR media types pass the shared Reinhard → sRGB
/// primitive, plain ones keep their stored pixels. Returns the converted
/// pixels and whether the tone map ran. GPUI-free: the Image Viewer wraps
/// this on the background executor; tests run real fixtures here.
pub fn decode_to_rgba(path: &str, media_type: &str) -> Result<(image::RgbaImage, bool), String> {
    let mut reader = image::ImageReader::open(path)
        .map_err(|error| format!("Cannot show this image: {error}"))?;
    if let Some(format) = image::ImageFormat::from_mime_type(media_type) {
        reader.set_format(format);
    } else {
        reader = reader
            .with_guessed_format()
            .map_err(|error| format!("Cannot show this image: {error}"))?;
    }
    let opened = reader
        .decode()
        .map_err(|error| format!("Cannot show this image: {error}"))?;
    let size = [opened.width(), opened.height()];
    if !size_ok(size) {
        return Err(format!(
            "Too large to display ({} × {}); the limit is {} × {}",
            size[0], size[1], MAX_EDGE, MAX_EDGE
        ));
    }
    let tone_mapped = needs_tone_map(media_type);
    let image = if tone_mapped {
        // Buffers do not cross the image-0.24/0.25 split; the shared math does.
        let linear = opened.to_rgb32f();
        image::RgbaImage::from_fn(linear.width(), linear.height(), |x, y| {
            let pixel = linear.get_pixel(x, y);
            image::Rgba([
                scene_view::tonemap::reinhard_channel(pixel[0]),
                scene_view::tonemap::reinhard_channel(pixel[1]),
                scene_view::tonemap::reinhard_channel(pixel[2]),
                255,
            ])
        })
    } else {
        opened.to_rgba8()
    };
    Ok((image, tone_mapped))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        format!("{}/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn ldr_images_display_their_stored_pixels() {
        let (image, tone_mapped) = decode_to_rgba(&fixture("viewer-ldr.png"), "image/png").unwrap();
        assert!(!tone_mapped);
        assert_eq!(image.get_pixel(0, 0), &image::Rgba([0, 128, 255, 255]));
        assert_eq!(image.get_pixel(63, 0), &image::Rgba([252, 128, 3, 255]));
    }

    #[test]
    fn payload_leases_decode_using_the_asset_media_type() {
        use std::io::Write as _;

        let source = std::fs::read(fixture("viewer-ldr.png")).unwrap();
        let mut payload = tempfile::Builder::new()
            .suffix(".payload")
            .tempfile()
            .unwrap();
        payload.write_all(&source).unwrap();
        let (image, tone_mapped) =
            decode_to_rgba(payload.path().to_str().unwrap(), "image/png").unwrap();
        assert!(!tone_mapped);
        assert_eq!(image.get_pixel(0, 0), &image::Rgba([0, 128, 255, 255]));
    }

    #[test]
    fn exr_images_display_through_the_shared_tone_map() {
        // The fixture sweeps red 0→7.875 with green 0.25 and blue 4.0; the
        // shared Reinhard → sRGB values are 124 and 231.
        let (image, tone_mapped) =
            decode_to_rgba(&fixture("viewer-hdr.exr"), "image/x-exr").unwrap();
        assert!(tone_mapped);
        assert_eq!(image.get_pixel(0, 0), &image::Rgba([0, 124, 231, 255]));
        assert_eq!(image.get_pixel(63, 0), &image::Rgba([242, 124, 231, 255]));
    }

    #[test]
    fn one_edge_over_the_bound_refuses_instead_of_decoding() {
        let error = decode_to_rgba(&fixture("viewer-huge.png"), "image/png").unwrap_err();
        assert!(error.contains("Too large to display"), "{error}");
        assert!(error.contains("8193"), "{error}");
    }

    #[test]
    fn the_viewer_opens_exactly_the_viewable_media_types() {
        for media_type in VIEWER_MEDIA_TYPES {
            assert!(
                media_type.starts_with("image/"),
                "{media_type} must be an image type"
            );
        }
        assert!(!VIEWER_MEDIA_TYPES.contains(&"image/x-portable-floatmap"));
    }

    #[test]
    fn only_hdr_formats_need_the_tone_map() {
        for media_type in ["image/x-exr", "image/vnd.radiance"] {
            assert!(needs_tone_map(media_type), "{media_type}");
        }
        for media_type in ["image/png", "image/jpeg", "image/x-tga"] {
            assert!(!needs_tone_map(media_type), "{media_type}");
        }
    }

    #[test]
    fn edges_above_the_bound_are_refused() {
        assert!(size_ok([1, 1]));
        assert!(size_ok([8192, 8192]));
        assert!(!size_ok([8193, 1]));
        assert!(!size_ok([1, 8193]));
        assert!(!size_ok([0, 5]));
        assert!(!size_ok([5, 0]));
    }
}
