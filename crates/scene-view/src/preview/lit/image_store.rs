//! The binding-array path stores each lit image in the
//! GPU format its bits allow — 8-bit images stay 8-bit (the sRGB ones let
//! the sampler decode), float-use images narrow to one channel — instead of
//! expanding every pixel to RGBA f32 and converting to f16. The portable
//! array path keeps `decode_texture` + `Rgba16Float` unchanged, and light
//! control maps keep the environment's fixed RGBA16Float pipeline.
//!
//! The binding path's mip chain is generated on the GPU, one render
//! pass per level, so this module only produces level 0 — the 8-bit fast
//! classes hand over the decoded bytes (a capped oversized source hands
//! over the full-size bytes, and the GPU downscales them into level 0 from
//! a temporary texture), while the f16 classes (gamma curves, 16-bit,
//! EXR/HDR, float uses of sRGB 8-bit) keep today's f32 pipeline for level
//! 0. The average the demote fallback reads comes from the same decoded
//! pixels; for a capped fast-class source that is the full-size source
//! mean (see `stored_image`).

use protocol::geometry::TextureRecord;
use wgpu::TextureFormat;

/// A decoded image encoded for its GPU texture: level 0 only in
/// the stored format, and the mean linear colour of the decoded pixels —
/// the value a demote falls back to.
#[derive(Debug)]
pub(crate) struct StoredImage {
    pub(crate) format: TextureFormat,
    /// The storage size: the source's own, or scaled within
    /// `TEXTURE_MAX_EDGE`.
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// What level 0 is made of.
    pub(crate) level_0: StoredLevel0,
    /// The mean linear colour of the decoded pixels level 0 came from —
    /// `mean_rgb` of the same values today's decode produced.
    pub(crate) average: [f32; 3],
}

/// How the binding path uploads level 0.
#[derive(Debug)]
pub(crate) enum StoredLevel0 {
    /// Level 0's texel rows at `(StoredImage::width, StoredImage::height)`,
    /// ready for a plain `write_texture`.
    Encoded(Vec<u8>),
    /// The decoded source's own texel rows in the stored format, at its
    /// own size: a capped 8-bit image downscales on the GPU — these bytes
    /// upload into a temporary texture, and one pass renders level 0 of
    /// the stored texture from it. `TEXTURE_MAX_EDGE` keeps the ratio under
    /// 2x, so the pass's single bilinear tap loses less than one texel of
    /// bandwidth.
    Source {
        width: u32,
        height: u32,
        bytes: Vec<u8>,
    },
}

/// Bytes per texel of the formats the binding path stores.
pub(crate) fn texel_bytes(format: TextureFormat) -> u64 {
    match format {
        TextureFormat::R8Unorm => 1,
        TextureFormat::R16Float => 2,
        TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb => 4,
        TextureFormat::Rgba16Float => 8,
        _ => unreachable!("the lit image storage formats are covered above"),
    }
}

/// The texel count of a `width`×`height` mip chain, the per-level sizes the
/// GPU chain generates: level `m` is `(width >> m).max(1)` by
/// `(height >> m).max(1)`, `log2(longest side) + 1` levels.
fn mip_texels(width: u32, height: u32) -> u64 {
    (0..width.max(height).ilog2() + 1)
        .map(|mip| u64::from((width >> mip).max(1)) * u64::from((height >> mip).max(1)))
        .sum()
}

/// The GPU byte cost of a stored mip chain — the binding path's byte
/// budget counts these, per real texel bytes.
pub(crate) fn storage_bytes(width: u32, height: u32, format: TextureFormat) -> u64 {
    mip_texels(width, height) * texel_bytes(format)
}

/// The sample width a header declares, mirroring what decoding the same
/// file produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SampleKind {
    EightBit,
    Sixteen,
    Float,
}

/// image 0.24's `ColorType` is non-exhaustive; anything new falls to f16,
/// today's behaviour for 16-bit-like samples.
pub(crate) fn sample_kind_of(color_type: image::ColorType) -> SampleKind {
    match color_type {
        image::ColorType::L8
        | image::ColorType::La8
        | image::ColorType::Rgb8
        | image::ColorType::Rgba8 => SampleKind::EightBit,
        image::ColorType::Rgb32F | image::ColorType::Rgba32F => SampleKind::Float,
        _ => SampleKind::Sixteen,
    }
}

/// The format the binding path stores an image in: 8-bit
/// sources stay 8-bit under linear or declared-sRGB encodings, float uses
/// narrow to one channel, and everything else keeps f16.
pub(crate) fn classify(
    scalar_use: bool,
    kind: SampleKind,
    encoding: &super::Encoding,
) -> TextureFormat {
    match (scalar_use, kind, encoding) {
        (false, SampleKind::EightBit, super::Encoding::Srgb) => TextureFormat::Rgba8UnormSrgb,
        (false, SampleKind::EightBit, super::Encoding::Linear) => TextureFormat::Rgba8Unorm,
        (true, SampleKind::EightBit, super::Encoding::Linear) => TextureFormat::R8Unorm,
        (true, _, _) => TextureFormat::R16Float,
        (_, _, _) => TextureFormat::Rgba16Float,
    }
}

/// What the binding path will store for an image before decoding it: the
/// format its bits and declared encoding select, the storage size
/// its longest edge caps to, and the byte cost of the whole mip chain —
/// everything the budgets need from the header alone, so a rejected image
/// is never decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TexturePlan {
    pub(crate) format: TextureFormat,
    /// The storage size: the source's own, or scaled within
    /// `TEXTURE_MAX_EDGE`.
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) bytes: u64,
    /// The source's own size, which the decode must reproduce.
    pub(crate) source_width: u32,
    pub(crate) source_height: u32,
}

/// The stored size of a `source` image: its own size while its
/// longest edge fits `TEXTURE_MAX_EDGE`, else scaled down to the cap with
/// the aspect kept.
pub(crate) fn capped_dimensions(source_width: u32, source_height: u32) -> (u32, u32) {
    let long = source_width.max(source_height);
    if long <= super::TEXTURE_MAX_EDGE {
        return (source_width, source_height);
    }
    let scale = super::TEXTURE_MAX_EDGE as f64 / long as f64;
    let scaled = |value: u32| {
        (value as f64 * scale)
            .round()
            .clamp(1., super::TEXTURE_MAX_EDGE as f64) as u32
    };
    (scaled(source_width), scaled(source_height))
}

/// The storage plan for a texture's bytes, from its header alone.
pub(crate) fn plan_texture(record: &TextureRecord, bytes: &[u8]) -> Result<TexturePlan, String> {
    let (source_width, source_height, kind) = header_probe(record, bytes)?;
    let encoding = super::resolve_encoding(record, matches!(kind, SampleKind::Float))?;
    let format = classify(super::scalar_use(record), kind, &encoding);
    let (width, height) = capped_dimensions(source_width, source_height);
    Ok(TexturePlan {
        format,
        width,
        height,
        bytes: storage_bytes(width, height, format),
        source_width,
        source_height,
    })
}

/// The image's dimensions and sample width from its header, without
/// decoding its pixels. Radiance HDRs read the same header the scene-parse
/// decode does; everything else dispatches to the same decoder
/// `load_from_memory` uses, since image 0.24 has no generic metadata API.
pub(crate) fn header_probe(
    record: &TextureRecord,
    bytes: &[u8],
) -> Result<(u32, u32, SampleKind), String> {
    let filename = record
        .parameters
        .iter()
        .find(|parameter| parameter.matches("filename"))
        .and_then(|parameter| parameter.texts.first())
        .map(String::as_str)
        .unwrap_or_default();
    if scene_parse::hdr::is_hdr(bytes, filename) {
        let decoder = image::codecs::hdr::HdrDecoder::new(std::io::Cursor::new(bytes))
            .map_err(|error| format!("cannot decode the HDR image: {error}"))?;
        let metadata = decoder.metadata();
        if metadata.width == 0 || metadata.height == 0 {
            return Err("cannot decode the HDR image: zero-sized header".into());
        }
        return Ok((metadata.width, metadata.height, SampleKind::Float));
    }
    let format = image::guess_format(bytes).map_err(|error| error.to_string())?;
    let cursor = std::io::Cursor::new(bytes);
    match format {
        image::ImageFormat::Png => {
            decoder_header(image::codecs::png::PngDecoder::new(cursor).map_err(error_string)?)
        }
        image::ImageFormat::Jpeg => {
            decoder_header(image::codecs::jpeg::JpegDecoder::new(cursor).map_err(error_string)?)
        }
        image::ImageFormat::Gif => {
            decoder_header(image::codecs::gif::GifDecoder::new(cursor).map_err(error_string)?)
        }
        image::ImageFormat::Bmp => {
            decoder_header(image::codecs::bmp::BmpDecoder::new(cursor).map_err(error_string)?)
        }
        image::ImageFormat::Ico => {
            decoder_header(image::codecs::ico::IcoDecoder::new(cursor).map_err(error_string)?)
        }
        image::ImageFormat::Tiff => {
            decoder_header(image::codecs::tiff::TiffDecoder::new(cursor).map_err(error_string)?)
        }
        image::ImageFormat::WebP => {
            decoder_header(image::codecs::webp::WebPDecoder::new(cursor).map_err(error_string)?)
        }
        image::ImageFormat::Pnm => {
            decoder_header(image::codecs::pnm::PnmDecoder::new(cursor).map_err(error_string)?)
        }
        image::ImageFormat::OpenExr => decoder_header(
            image::codecs::openexr::OpenExrDecoder::new(cursor).map_err(error_string)?,
        ),
        image::ImageFormat::Qoi => {
            decoder_header(image::codecs::qoi::QoiDecoder::new(cursor).map_err(error_string)?)
        }
        image::ImageFormat::Dds => {
            decoder_header(image::codecs::dds::DdsDecoder::new(cursor).map_err(error_string)?)
        }
        image::ImageFormat::Farbfeld => decoder_header(
            image::codecs::farbfeld::FarbfeldDecoder::new(cursor).map_err(error_string)?,
        ),
        image::ImageFormat::Tga => {
            decoder_header(image::codecs::tga::TgaDecoder::new(cursor).map_err(error_string)?)
        }
        other => Err(format!("image format {other:?} is not supported")),
    }
}

/// The header of one constructed decoder: the dimension guard plus the
/// sample kind its color type declares.
fn decoder_header<'a, D: image::ImageDecoder<'a>>(
    decoder: D,
) -> Result<(u32, u32, SampleKind), String> {
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        return Err(format!("cannot read a {width}×{height} image header"));
    }
    Ok((width, height, sample_kind_of(decoder.color_type())))
}

/// The text of a decoder error, the same way `decode_dynamic` reports one.
fn error_string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// One 8-bit sRGB decode per byte value: `decode_texture`'s per-pixel curve
/// of `value / 255`, looked up per texel instead of recomputed.
fn srgb_decode_lut() -> &'static [f32; 256] {
    static LUT: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    LUT.get_or_init(|| {
        std::array::from_fn(|byte| {
            let value = byte as f32 / 255.;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        })
    })
}

/// The per-byte value of the averages, in Q24 fixed point: linear colour
/// for sRGB storage, `byte / 255` for linear storage. A u64 sum of these
/// is exact to 2^-24 per texel — far tighter than the 1e-3 the fallback
/// average is trusted to — while staying a small integer loop; the capped
/// 8-bit sources hand the GPU their full-size bytes, and their average
/// rides the same pass.
fn fixed_decode_lut(srgb: bool) -> &'static [u32; 256] {
    static SRGB: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    static LINEAR: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    if srgb {
        SRGB.get_or_init(|| {
            std::array::from_fn(|byte| (srgb_decode_lut()[byte] * (1 << 24) as f32) as u32)
        })
    } else {
        LINEAR.get_or_init(|| {
            std::array::from_fn(|byte| ((byte as f32 / 255.) * (1 << 24) as f32) as u32)
        })
    }
}

/// The sRGB encode matching the LUT's decode, for re-encoding filtered
/// pixels on the CPU downscale path.
fn encode_srgb_u8(linear: f32) -> u8 {
    let linear = linear.clamp(0., 1.);
    let encoded = if linear <= 0.0031308 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.).round().clamp(0., 255.) as u8
}

/// Linear value → stored byte of an `Rgba8Unorm`/`R8Unorm` texel.
fn encode_linear_u8(linear: f32) -> u8 {
    (linear.clamp(0., 1.) * 255.).round() as u8
}

/// Decode `bytes` into level 0 of the image the binding path uploads
/// following the header's `plan`: the decode must agree
/// with the header inspected before decoding — the cap's budgets answered the real
/// image or nothing — and the mip chain is the renderer's GPU passes, so
/// this only picks the bytes level 0 comes from.
///
/// The 8-bit fast classes keep their bytes — no f32 expansion and no
/// per-pixel CPU sRGB decode for the formats the sampler can decode (an
/// R8 scalar still resolves today's channel mean in byte domain) — and a
/// capped one hands over the bytes at the source's own size for the GPU
/// downscale; `source_fits_device` says whether a temporary texture of
/// that size fits `max_texture_dimension_2d` and is at most twice the cap
/// (one bilinear tap averages no more than 2×), and when it does not the
/// source downscales on the CPU through today's f32 pipeline instead. The
/// stored average of a fast-class source is its full-size mean — the value
/// today's whole-image decode produced, which the downscaled level 0
/// carries within the resample. Gamma curves, 16-bit and float sources,
/// and float uses of sRGB 8-bit images keep the f32 pipeline for level 0
/// and come back f16, narrowed to R for float uses.
pub(crate) fn stored_image(
    record: &TextureRecord,
    bytes: &[u8],
    plan: &TexturePlan,
    source_fits_device: bool,
) -> Result<StoredImage, String> {
    let decoded = super::decode_dynamic(record, bytes)?;
    let kind = sample_kind_of(decoded.color());
    let scalar_use = super::scalar_use(record);
    let encoding = super::resolve_encoding(record, matches!(kind, SampleKind::Float))?;
    if (decoded.width(), decoded.height()) != (plan.source_width, plan.source_height)
        || classify(scalar_use, kind, &encoding) != plan.format
    {
        return Err("the decoded image does not match its header".into());
    }
    let capped = (decoded.width(), decoded.height()) != (plan.width, plan.height);
    // The fast 8-bit classes: `classify` only picks these formats from 8-bit
    // sources under linear or declared-sRGB encodings.
    if matches!(
        plan.format,
        TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb | TextureFormat::R8Unorm
    ) && (!capped || source_fits_device)
    {
        let source = decoded.to_rgba8();
        let (bytes, average) = match plan.format {
            TextureFormat::R8Unorm => {
                // decode_texture's use_alpha needs the source to carry an
                // alpha channel: gray+alpha sources keep their gray value.
                // The average is over the byte values themselves, before
                // the R8 round the GPU samples.
                let four_channels = decoded.color().channel_count() == 4;
                let use_alpha = four_channels && source.pixels().any(|pixel| pixel[3] != 255);
                (
                    scalar_bytes(&source, use_alpha),
                    mean_scalar(&source, use_alpha),
                )
            }
            format => {
                let srgb = format == TextureFormat::Rgba8UnormSrgb;
                (source.as_raw().clone(), rgba8_average(&source, srgb))
            }
        };
        let level_0 = if capped {
            // The GPU downscale's temporary source, at its own size.
            StoredLevel0::Source {
                width: plan.source_width,
                height: plan.source_height,
                bytes,
            }
        } else {
            StoredLevel0::Encoded(bytes)
        };
        return Ok(StoredImage {
            format: plan.format,
            width: plan.width,
            height: plan.height,
            level_0,
            average,
        });
    }
    // Everything else — an oversized 8-bit source the device cannot hold as
    // a temporary, gamma curves, 16-bit and float sources, and float uses
    // of sRGB 8-bit — keeps today's f32 pipeline for level 0,
    // downscaled once to the selected dimensions when capped.
    let mut image = super::linear_rgba32(decoded, encoding, scalar_use);
    if capped {
        image = image::imageops::resize(
            &image,
            plan.width,
            plan.height,
            image::imageops::FilterType::Triangle,
        );
    }
    let average = super::mean_rgb(&image);
    let level_0 = match plan.format {
        TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb => {
            // The scaled linear level re-encoded into the stored domain.
            let srgb = plan.format == TextureFormat::Rgba8UnormSrgb;
            StoredLevel0::Encoded(encode_u8(&image, srgb).into_raw())
        }
        TextureFormat::R8Unorm => StoredLevel0::Encoded(
            image
                .pixels()
                .map(|pixel| (pixel[0] * 255.).round().clamp(0., 255.) as u8)
                .collect(),
        ),
        _ => StoredLevel0::Encoded(f16_bytes(&image, scalar_use)),
    };
    Ok(StoredImage {
        format: plan.format,
        width: plan.width,
        height: plan.height,
        level_0,
        average,
    })
}

/// The linear RGBA f32 image encoded into the 8-bit storage: RGB through
/// the sRGB encode when the storage declares it, alpha as its own 0–1
/// scale.
fn encode_u8(image: &image::Rgba32FImage, srgb: bool) -> image::RgbaImage {
    image::RgbaImage::from_fn(image.width(), image.height(), |x, y| {
        let pixel = image.get_pixel(x, y);
        let encode = |channel: usize| {
            if srgb && channel < 3 {
                encode_srgb_u8(pixel[channel])
            } else {
                encode_linear_u8(pixel[channel])
            }
        };
        image::Rgba([encode(0), encode(1), encode(2), encode(3)])
    })
}

/// Float uses read one scalar per texel (decode_texture's rule: alpha when
/// the source carries an alpha channel and any pixel is nonopaque, the
/// channel mean otherwise), quantized once into the byte an R8 texture
/// samples.
fn scalar_bytes(source: &image::RgbaImage, use_alpha: bool) -> Vec<u8> {
    source
        .pixels()
        .map(|pixel| {
            if use_alpha {
                pixel[3]
            } else {
                ((pixel[0] as f32 + pixel[1] as f32 + pixel[2] as f32) / 3.0).round() as u8
            }
        })
        .collect()
}

/// The mean over `source` pixels of the float scalar today's decode
/// produced — the exact average before the R8 quantization rounds each
/// texel by up to half a byte.
fn mean_scalar(source: &image::RgbaImage, use_alpha: bool) -> [f32; 3] {
    let bytes = source.as_raw();
    let count = bytes.len() / 4;
    if count == 0 {
        return [0.5; 3];
    }
    let scale = if use_alpha { 255. } else { 3. * 255. };
    let mut sum = 0u64;
    for texel in bytes.as_chunks::<4>().0.iter() {
        sum += if use_alpha {
            u64::from(texel[3])
        } else {
            u64::from(texel[0]) + u64::from(texel[1]) + u64::from(texel[2])
        };
    }
    [(sum as f64 / (count as f64 * scale)) as f32; 3]
}

/// `mean_rgb` of the stored texels: RGB through the decode table when the
/// storage is sRGB — exactly the values today's decode produced.
fn rgba8_average(source: &image::RgbaImage, srgb: bool) -> [f32; 3] {
    let bytes = source.as_raw();
    let count = (bytes.len() / 4) as f64;
    if count == 0.0 {
        return [0.5; 3];
    }
    let lut = fixed_decode_lut(srgb);
    let mut sums = [0u64; 3];
    for texel in bytes.as_chunks::<4>().0.iter() {
        for channel in 0..3 {
            sums[channel] += u64::from(lut[texel[channel] as usize]);
        }
    }
    std::array::from_fn(|channel| (sums[channel] as f64 / (count * (1 << 24) as f64)) as f32)
}

/// One f32 level's texels as f16 bits: R only when the use is scalar (the
/// decoded scalar is replicated there), the whole texel otherwise —
/// today's conversion per channel.
fn f16_bytes(level: &image::Rgba32FImage, scalar_use: bool) -> Vec<u8> {
    let words: Vec<u16> = level
        .pixels()
        .flat_map(|pixel| {
            let values = if scalar_use {
                &pixel.0[..1]
            } else {
                &pixel.0[..]
            };
            values
                .iter()
                .copied()
                .map(half::f16::from_f32)
                .map(half::f16::to_bits)
        })
        .collect();
    bytemuck::cast_slice(&words).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn imagemap_record(value_type: &str, filename: &str) -> TextureRecord {
        TextureRecord {
            name: String::new(),
            value_type: value_type.into(),
            implementation: "imagemap".into(),
            world_from_texture: [
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
            ],
            source: None,
            parameters: vec![protocol::geometry::PreviewParameter {
                name: "filename".into(),
                value_type: "string".into(),
                floats: vec![],
                texts: vec![filename.into()],
                truncated: false,
            }],
        }
    }

    fn png_bytes(image: image::RgbaImage) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    /// 4×4, distinct colors per column so filtering has real work.
    fn ramp_png() -> Vec<u8> {
        png_bytes(image::RgbaImage::from_fn(4, 4, |x, _| {
            image::Rgba([
                (x * 60) as u8,
                (60 + x * 30) as u8,
                (255 - x * 50) as u8,
                255,
            ])
        }))
    }

    /// `stored_image`'s answer when the device can hold a capped source's
    /// temporary texture — the GPU downscale path.
    fn stored(record: &TextureRecord, bytes: &[u8]) -> StoredImage {
        let plan = plan_texture(record, bytes).unwrap();
        stored_image(record, bytes, &plan, true).unwrap()
    }

    /// `stored_image`'s answer when it cannot: the CPU downscale.
    fn stored_off_device(record: &TextureRecord, bytes: &[u8]) -> StoredImage {
        let plan = plan_texture(record, bytes).unwrap();
        stored_image(record, bytes, &plan, false).unwrap()
    }

    /// The level 0 bytes of a stored image, whatever upload they are for.
    fn level_0(image: &StoredImage) -> &[u8] {
        match &image.level_0 {
            StoredLevel0::Encoded(bytes) | StoredLevel0::Source { bytes, .. } => bytes,
        }
    }

    /// The plan a record's header produces.
    fn plan(record: &TextureRecord, bytes: &[u8]) -> TexturePlan {
        plan_texture(record, bytes).unwrap()
    }

    /// `decode_texture`'s result — today's pipeline — for the same bytes.
    fn today(record: &TextureRecord, bytes: &[u8]) -> image::Rgba32FImage {
        super::super::decode_texture(record, bytes).unwrap()
    }

    fn average_error(stored: &StoredImage, record: &TextureRecord, bytes: &[u8]) -> f32 {
        let reference = super::super::mean_rgb(&today(record, bytes));
        stored
            .average
            .iter()
            .zip(reference)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max)
    }

    #[test]
    fn eight_bit_srgb_png_stays_rgba8_srgb_with_untouched_level_zero() {
        let bytes = ramp_png();
        let record = imagemap_record("spectrum", "map.png");
        let image = stored(&record, &bytes);
        assert_eq!(image.format, TextureFormat::Rgba8UnormSrgb);
        assert_eq!((image.width, image.height), (4, 4));
        // Level 0 keeps the decoded bytes byte-for-byte: the GPU decodes
        // sRGB, the CPU never expands to f32.
        let decoded = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(level_0(&image), decoded.as_raw().as_slice());
        // The GPU-generated chain accounts for every mip level.
        assert_eq!(
            plan(&record, &bytes).bytes,
            storage_bytes(4, 4, TextureFormat::Rgba8UnormSrgb)
        );
        assert!(
            average_error(&image, &record, &bytes) <= 1e-6,
            "the level-0 mean is the decoded values themselves"
        );
    }

    #[test]
    fn eight_bit_linear_png_stores_rgba8_unorm() {
        let bytes = ramp_png();
        let record = imagemap_record("spectrum", "map.dat");
        let image = stored(&record, &bytes);
        assert_eq!(image.format, TextureFormat::Rgba8Unorm);
        assert!(average_error(&image, &record, &bytes) <= 1e-6);
    }

    #[test]
    fn eight_bit_gamma_png_keeps_rgba16_float() {
        let bytes = ramp_png();
        let mut record = imagemap_record("spectrum", "map.png");
        record
            .parameters
            .push(protocol::geometry::PreviewParameter {
                name: "gamma".into(),
                value_type: "float".into(),
                floats: vec![2.2],
                texts: vec![],
                truncated: false,
            });
        let image = stored(&record, &bytes);
        assert_eq!(image.format, TextureFormat::Rgba16Float);
        // Level 0 is exactly today's f16 conversion of the decoded pixels.
        let today = f16_bytes(&today(&record, &bytes), false);
        assert_eq!(level_0(&image), today.as_slice());
        assert!(average_error(&image, &record, &bytes) <= 1e-6);
    }

    #[test]
    fn sixteen_bit_png_keeps_rgba16_float() {
        let image16 = image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_fn(4, 4, |x, _| {
            image::Rgba([
                (x * 12000) as u16,
                (12000 + x * 3000) as u16,
                (60000 - x * 8000) as u16,
                65535,
            ])
        });
        let mut bytes = std::io::Cursor::new(Vec::new());
        image16
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let record = imagemap_record("spectrum", "map.png"); // default sRGB decode
        let image = stored(&record, bytes.get_ref());
        assert_eq!(image.format, TextureFormat::Rgba16Float);
        assert!(average_error(&image, &record, bytes.get_ref()) <= 1e-3);
    }

    #[test]
    fn radiance_hdr_keeps_rgba16_float() {
        let mut bytes = Vec::new();
        {
            let encoder = image::codecs::hdr::HdrEncoder::new(&mut bytes);
            let pixels: Vec<image::Rgb<f32>> = (0..8)
                .map(|index| image::Rgb([index as f32 * 0.2, 1., 0.1]))
                .collect();
            encoder.encode(&pixels, 4, 2).unwrap();
        }
        let record = imagemap_record("spectrum", "map.hdr");
        let image = stored(&record, &bytes);
        assert_eq!(image.format, TextureFormat::Rgba16Float);
        assert!(average_error(&image, &record, &bytes) <= 1e-3);
    }

    #[test]
    fn float_use_of_an_eight_bit_linear_png_stores_r8_with_today_scalar() {
        let bytes = ramp_png();
        let record = imagemap_record("float", "map.dat");
        let image = stored(&record, &bytes);
        assert_eq!(image.format, TextureFormat::R8Unorm);
        // Level 0: each texel the mean channel bin, alpha 1 in the source.
        let reference = today(&record, &bytes);
        assert_eq!(level_0(&image).len(), 16, "one byte per texel");
        for (index, pixel) in reference.pixels().enumerate() {
            let expected = ((pixel[0] + pixel[1] + pixel[2]) / 3.0 * 255.).round() as u8;
            assert_eq!(
                level_0(&image)[index],
                expected,
                "texel {index} scalar mismatch"
            );
        }
        assert!(
            average_error(&image, &record, &bytes) <= 1e-6,
            "the average comes from the pre-quantization scalars"
        );
    }

    #[test]
    fn float_use_of_an_alpha_png_stores_the_alpha_byte() {
        let bytes = png_bytes(image::RgbaImage::from_fn(4, 4, |x, y| {
            image::Rgba([
                (x * 60) as u8,
                60,
                30,
                if y < 2 { 255 } else { (x * 60) as u8 },
            ])
        }));
        let record = imagemap_record("float", "map.dat");
        let image = stored(&record, &bytes);
        assert_eq!(image.format, TextureFormat::R8Unorm);
        let reference = today(&record, &bytes);
        for (index, pixel) in reference.pixels().enumerate() {
            // Today's decode replicates the scalar (here: the alpha) across
            // RGB and stores alpha 1; channel 0 is that scalar.
            assert_eq!(
                level_0(&image)[index],
                ((pixel[0]) * 255.).round() as u8,
                "texel {index}"
            );
        }
    }

    #[test]
    fn float_use_of_an_eight_bit_srgb_png_stores_r16_float() {
        let bytes = ramp_png();
        let record = imagemap_record("float", "map.png");
        let image = stored(&record, &bytes);
        assert_eq!(image.format, TextureFormat::R16Float);
        // Level 0 is today's scalar (replicated in the f32 decode) in R.
        let reference = f16_bytes(&today(&record, &bytes), true);
        assert_eq!(level_0(&image), reference.as_slice());
        assert!(
            average_error(&image, &record, &bytes) <= 1e-6,
            "float-use averages match the scalarized decode"
        );
    }

    #[test]
    fn level_zero_bytes_match_the_format() {
        let bytes = ramp_png();
        // Level 0 alone is one row of texels per height: width×height at
        // the format's texel bytes. The whole-chain account above stays
        // the full chain's estimate (the renderer's GPU levels cover it).
        let texels = |image: &StoredImage| -> u64 {
            u64::from(image.width) * u64::from(image.height) * texel_bytes(image.format)
        };
        // Spectrum sRGB: RGBA8UnormSrgb, 4 bytes per texel.
        let image = stored(&imagemap_record("spectrum", "map.png"), &bytes);
        assert_eq!(image.format, TextureFormat::Rgba8UnormSrgb);
        assert_eq!(level_0(&image).len() as u64, texels(&image));
        // Float use of the same png: R16Float, 2 bytes per texel.
        let image = stored(&imagemap_record("float", "map.png"), &bytes);
        assert_eq!(image.format, TextureFormat::R16Float);
        assert_eq!(level_0(&image).len() as u64, texels(&image));
        // Float use of a linear png: R8Unorm, 1 byte per texel.
        let image = stored(&imagemap_record("float", "map.dat"), &bytes);
        assert_eq!(image.format, TextureFormat::R8Unorm);
        assert_eq!(level_0(&image).len() as u64, texels(&image));
    }

    #[test]
    fn lut_matches_the_software_srgb_decode() {
        for (byte, lut) in srgb_decode_lut().iter().enumerate() {
            let value = byte as f32 / 255.;
            let reference = if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            };
            assert!(f32::abs(*lut - reference) <= 1e-7, "{byte}");
        }
    }

    /// A 4096×1024 png of four solid quadrants — compresses instantly.
    fn oversized_png() -> Vec<u8> {
        png_bytes(image::RgbaImage::from_fn(4096, 1024, |x, y| {
            let color = if x < 2048 {
                if y < 512 {
                    [200, 40, 40]
                } else {
                    [40, 200, 40]
                }
            } else if y < 512 {
                [40, 40, 200]
            } else {
                [200, 200, 40]
            };
            image::Rgba([color[0], color[1], color[2], 255])
        }))
    }

    #[test]
    fn a_plan_selects_format_and_cap_from_the_header_alone() {
        // Formats are selected from the header without decoding a pixel.
        assert_eq!(
            plan(&imagemap_record("spectrum", "map.png"), &ramp_png()).format,
            TextureFormat::Rgba8UnormSrgb
        );
        assert_eq!(
            plan(&imagemap_record("spectrum", "map.dat"), &ramp_png()).format,
            TextureFormat::Rgba8Unorm
        );
        assert_eq!(
            plan(&imagemap_record("float", "map.dat"), &ramp_png()).format,
            TextureFormat::R8Unorm
        );
        assert_eq!(
            plan(&imagemap_record("float", "map.png"), &ramp_png()).format,
            TextureFormat::R16Float
        );
        let mut gamma = imagemap_record("spectrum", "map.png");
        gamma.parameters.push(protocol::geometry::PreviewParameter {
            name: "gamma".into(),
            value_type: "float".into(),
            floats: vec![2.2],
            texts: vec![],
            truncated: false,
        });
        assert_eq!(plan(&gamma, &ramp_png()).format, TextureFormat::Rgba16Float);
        // Cap: a 4096×1024 header stores 2048×512, aspect kept.
        let oversized = plan(&imagemap_record("spectrum", "map.png"), &oversized_png());
        assert_eq!(
            (oversized.source_width, oversized.source_height),
            (4096, 1024)
        );
        assert_eq!((oversized.width, oversized.height), (2048, 512));
        assert_eq!(
            oversized.bytes,
            storage_bytes(2048, 512, TextureFormat::Rgba8UnormSrgb)
        );
        // An HDR probe knows its float samples and small maps never cap.
        let mut bytes = Vec::new();
        {
            let encoder = image::codecs::hdr::HdrEncoder::new(&mut bytes);
            let pixels: Vec<image::Rgb<f32>> =
                (0..2).map(|_| image::Rgb([0.5, 0.5, 0.5])).collect();
            encoder.encode(&pixels, 2, 1).unwrap();
        }
        let hdr = imagemap_record("float", "map.hdr");
        assert_eq!(plan(&hdr, &bytes).format, TextureFormat::R16Float);
        assert_eq!(
            (plan(&hdr, &bytes).width, plan(&hdr, &bytes).height),
            (2, 1)
        );
        // Garbage bytes answer an error, like a decode failure does.
        assert!(plan_texture(&hdr, b"not an image at all").is_err());
    }

    #[test]
    fn an_oversized_srgb_source_hands_the_gpu_its_full_size_bytes() {
        let record = imagemap_record("spectrum", "map.png");
        let bytes = oversized_png();
        let image = stored(&record, &bytes);
        // The stored shape is capped; level 0 is the GPU downscale of the
        // source bytes at their own size.
        assert_eq!(image.format, TextureFormat::Rgba8UnormSrgb);
        assert_eq!((image.width, image.height), (2048, 512));
        let StoredLevel0::Source {
            width,
            height,
            bytes,
        } = &image.level_0
        else {
            panic!("a fitting capped source downscales on the GPU");
        };
        assert_eq!((*width, *height), (4096, 1024));
        let decoded = image::load_from_memory(oversized_png().as_slice())
            .unwrap()
            .to_rgba8();
        assert_eq!(bytes.as_slice(), decoded.as_raw().as_slice());
        // The average is the full-size source mean — the same values
        // today's whole-image decode produced.
        assert!(
            average_error(&image, &record, oversized_png().as_slice()) <= 1e-3,
            "the source mean stands in for the downscaled level's"
        );
    }

    #[test]
    fn an_oversized_scalar_source_hands_the_gpu_its_full_size_bytes() {
        let record = imagemap_record("float", "map.dat");
        let bytes = oversized_png();
        let image = stored(&record, &bytes);
        assert_eq!(image.format, TextureFormat::R8Unorm);
        let StoredLevel0::Source {
            width,
            height,
            bytes: payload,
        } = &image.level_0
        else {
            panic!("a fitting capped scalar source downscales on the GPU");
        };
        assert_eq!((*width, *height), (4096, 1024));
        assert_eq!(payload.len(), (4096 * 1024) as usize, "one byte per texel");
        assert!(
            average_error(&image, &record, &bytes) <= 1e-3,
            "the scalar mean is the source's own"
        );
    }

    #[test]
    fn an_oversized_source_beyond_the_device_limit_downscales_on_the_cpu() {
        let record = imagemap_record("spectrum", "map.png");
        let bytes = oversized_png();
        let image = stored_off_device(&record, &bytes);
        // A temporary the device cannot hold keeps today's CPU downscale.
        assert_eq!(image.format, TextureFormat::Rgba8UnormSrgb);
        assert_eq!((image.width, image.height), (2048, 512));
        assert_eq!(level_0(&image).len(), (2048 * 512 * 4) as usize);
        // Capped level 0: the four source quadrants survive the downscale,
        // sampled in the linear domain the LUT decodes.
        let quadrant = |x: u32, y: u32| {
            let offset = (y * 2048 + x) as usize * 4;
            [0, 1, 2].map(|channel| level_0(&image)[offset + channel] as i16)
        };
        let expected = [
            [200i16, 40, 40],
            [40, 200, 40],
            [40, 40, 200],
            [200, 200, 40],
        ];
        for (x, y, color) in [
            (16u32, 16u32, expected[0]),
            (2032, 16, expected[2]),
            (16, 496, expected[1]),
            (2032, 496, expected[3]),
        ] {
            for (actual, expected) in quadrant(x, y).iter().zip(color) {
                assert!(
                    (actual - expected).abs() <= 2,
                    "({x},{y}): {:?}",
                    quadrant(x, y)
                );
            }
        }
        // The average still matches today's within the budget.
        assert!(
            average_error(&image, &record, &bytes) <= 1e-3,
            "the downscale keeps the mean within the fallback tolerance"
        );
    }
}
