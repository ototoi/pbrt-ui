use super::noise::fbm;
use super::noise::noise;
use super::texture_cache::TextureCache;
use super::texture_cache_map::TextureCacheKey;
use super::texture_cache_map::TextureCacheMap;
use super::texture_cache_size::TextureCacheSize;
use crate::conversion::spectrum::Spectrum;
use crate::model::base::Property;
use crate::model::base::Quaternion;
use crate::model::base::Vector3;
use crate::model::scene::Texture;

use crate::error::PbrtError;

use std::sync::{Arc, RwLock};

use crypto::digest::Digest;
use image;
use image::DynamicImage;
use image::GenericImageView;
use image::ImageBuffer;

fn get_digest(path: &str) -> String {
    let mut hasher = crypto::sha1::Sha1::new();
    hasher.input_str(path);
    let digest = hasher.result_str();
    return digest;
}

fn create_texture_cache_path(src: &str, size: TextureCacheSize) -> String {
    let dir = dirs::cache_dir()
        .unwrap()
        .join("pbrt_ui")
        .join("cache")
        .join("texture")
        .join(size.to_string());
    let src_path = std::path::PathBuf::from(src);
    let extension = src_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("png");
    //println!("Creating texture cache path: {:?}", dir);
    std::fs::create_dir_all(&dir).expect("Failed to create cache directory");
    let path = dir.join(format!("{}.{}", get_digest(src), extension));
    return path.to_str().unwrap().to_string();
}

pub fn create_one_pixel_image(pixel: &image::Rgba<f32>) -> Arc<DynamicImage> {
    let image_buffer = image::ImageBuffer::from_pixel(
        1, 1, *pixel, // Magenta color as default
    );
    let image = DynamicImage::ImageRgba32F(image_buffer);
    return Arc::new(image);
}

pub fn create_imagemap_texture_cache(
    texture: &Texture,
    size: TextureCacheSize,
) -> Result<Arc<RwLock<TextureCache>>, PbrtError> {
    let texture_type = texture.get_type();
    let id = texture.get_id();
    let edition = texture.get_edition();
    assert!(texture_type == "imagemap", "Texture type must be imagemap");
    if let Some(src) = texture.get_fullpath() {
        let dst = create_texture_cache_path(&src, size);
        //println!("Creating texture cache for: {} -> {}", src, dst);
        let src = std::path::PathBuf::from(src);
        let dst = std::path::PathBuf::from(dst);
        if dst.exists() {
            let image = image::open(&dst).map_err(|e| PbrtError::error(&format!("{}", e)))?;
            let image = Arc::new(image);
            let texture_cache = TextureCache { id, edition, image };
            return Ok(Arc::new(RwLock::new(texture_cache)));
        } else if src.exists() {
            let src_img = image::open(&src).map_err(|e| PbrtError::error(&format!("{}", e)))?;
            let resized_img = match size {
                TextureCacheSize::Icon => {
                    let factor = f32::min(
                        64.0 / src_img.width() as f32,
                        64.0 / src_img.height() as f32,
                    );
                    src_img.resize_exact(
                        ((src_img.width() as f32 * factor).ceil() as u32).min(64),
                        ((src_img.height() as f32 * factor).ceil() as u32).min(64),
                        image::imageops::FilterType::Lanczos3,
                    )
                }
                TextureCacheSize::Full => src_img,
                TextureCacheSize::Size((w, h)) => {
                    src_img.resize_exact(w as u32, h as u32, image::imageops::FilterType::Lanczos3)
                }
            };
            resized_img
                .save(&dst)
                .map_err(|e| PbrtError::error(&format!("{}", e)))?;

            let image = Arc::new(resized_img);
            let texture_cache = TextureCache { id, edition, image };
            return Ok(Arc::new(RwLock::new(texture_cache)));
        }
    }
    return Err(PbrtError::error(
        "Texture does not have a valid source path",
    ));
}

pub fn create_default_texture_cache(
    texture: &Texture,
    size: TextureCacheSize,
    color: &image::Rgba<u8>,
) -> Result<Arc<RwLock<TextureCache>>, PbrtError> {
    let id = texture.get_id();
    let edition = texture.get_edition();
    let resolution = match size {
        TextureCacheSize::Icon => (64, 64),
        TextureCacheSize::Full => (256, 256),
        TextureCacheSize::Size((w, h)) => (w as u32, h as u32),
    };
    let image_buffer = image::ImageBuffer::from_pixel(
        resolution.0,
        resolution.1,
        *color, // Magenta color as default
    );

    let image = image::DynamicImage::ImageRgba8(image_buffer);
    let image = Arc::new(image);
    let texture_cache = TextureCache { id, edition, image };
    return Ok(Arc::new(RwLock::new(texture_cache)));
}

pub fn create_constant_texture_cache(
    texture: &Texture,
) -> Result<Arc<RwLock<TextureCache>>, PbrtError> {
    let texture_type = texture.get_type();
    let id = texture.get_id();
    let edition = texture.get_edition();
    assert!(texture_type == "constant", "Texture type must be constant");
    if let Some(image) = get_color_texture_image(texture, "value") {
        let texture_cache = TextureCache { id, edition, image };
        return Ok(Arc::new(RwLock::new(texture_cache)));
    } else {
        return Err(PbrtError::error(
            "Constant texture does not have a valid color value",
        ));
    }
}

fn find_texture_cache(
    cache_map: &TextureCacheMap,
    name: &str,
    size: TextureCacheSize,
) -> Option<Arc<RwLock<TextureCache>>> {
    let textures = cache_map.read().unwrap();
    for (cache_key, cache) in textures.iter() {
        if cache_key.0 == name && cache_key.2 == size {
            return Some(cache.clone());
        }
    }
    return None;
}

fn mix_texture(tex1: &DynamicImage, tex2: &DynamicImage, amount: &DynamicImage) -> DynamicImage {
    let dim1 = tex1.dimensions();
    let dim2 = tex2.dimensions();
    let dim3 = amount.dimensions();
    let dimf = (
        dim1.0.max(dim2.0).max(dim3.0),
        dim1.1.max(dim2.1).max(dim3.1),
    );

    let tex1 = tex1.resize_exact(dimf.0, dimf.1, image::imageops::FilterType::Lanczos3);
    let tex2 = tex2.resize_exact(dimf.0, dimf.1, image::imageops::FilterType::Lanczos3);
    let amount = amount.resize_exact(dimf.0, dimf.1, image::imageops::FilterType::Lanczos3);

    let tex1 = tex1.to_rgba32f();
    let tex2 = tex2.to_rgba32f();
    let amount = amount.to_rgba32f();
    let mut image_buffer = image::ImageBuffer::new(dimf.0, dimf.1);
    for (x, y, pixel) in image_buffer.enumerate_pixels_mut() {
        let p1 = tex1.get_pixel(x, y);
        let p2 = tex2.get_pixel(x, y);
        let a = amount.get_pixel(x, y)[0]; // Assuming amount is a grayscale image
        let r = p1[0] * (1.0 - a) + p2[0] * a;
        let g = p1[1] * (1.0 - a) + p2[1] * a;
        let b = p1[2] * (1.0 - a) + p2[2] * a;
        let alpha = p1[3] * (1.0 - a) + p2[3] * a;
        *pixel = image::Rgba([r, g, b, alpha]);
    }
    return DynamicImage::ImageRgba32F(image_buffer);
}

fn get_color_texture_image(texture: &Texture, key: &str) -> Option<Arc<DynamicImage>> {
    let props = texture.as_property_map();
    if let Some((key_type, key_name, value)) = props.entry(key) {
        if let Property::Floats(color) = value {
            let color = match color.len() {
                1 => [color[0], color[0], color[0], 1.0],
                2 => [color[0], color[1], color[1], 1.0],
                3 => [color[0], color[1], color[2], 1.0],
                4 => [color[0], color[1], color[2], color[3]],
                _ => [1.0, 1.0, 1.0, 1.0], // Default to white if not specified correctly
            };
            let color = image::Rgba([
                (color[0]) as f32,
                (color[1]) as f32,
                (color[2]) as f32,
                (color[3]) as f32,
            ]);
            let image_buffer = image::ImageBuffer::from_pixel(1, 1, color);
            return Some(Arc::new(DynamicImage::ImageRgba32F(image_buffer)));
        } else if let Property::Strings(_name) = value {
            if key_type == "spectrum" {
                let fullpath_name = format!("{}_fullpath", key_name);
                if let Some(src) = props.get(&fullpath_name) {
                    if let Property::Strings(v) = src {
                        assert!(
                            v.len() == 1,
                            "Spectrum fullpath must have exactly one value"
                        );
                        let fullpath = v[0].clone();
                        if let Ok(s) = Spectrum::load_from_file(&fullpath) {
                            let color = s.to_rgb();
                            let color = image::Rgba([
                                color[0] as f32,
                                color[1] as f32,
                                color[2] as f32,
                                1.0,
                            ]);
                            let image_buffer = image::ImageBuffer::from_pixel(1, 1, color);
                            return Some(Arc::new(DynamicImage::ImageRgba32F(image_buffer)));
                        }
                    }
                }
            }
        }
    }
    return None;
}

fn get_texture_image(
    texture: &Texture,
    size: TextureCacheSize,
    cache_map: &TextureCacheMap,
    key: &str,
) -> Option<Arc<DynamicImage>> {
    if let Some(image) = get_color_texture_image(texture, key) {
        return Some(image);
    }
    let props = texture.as_property_map();
    if let Some((key_type, _, value)) = props.entry(key) {
        if let Property::Strings(name) = value {
            if key_type == "texture" {
                let texture_name = name[0].clone();
                if let Some(cache) = find_texture_cache(&cache_map, &texture_name, size) {
                    return Some(cache.read().unwrap().image.clone());
                }
            }
        }
    }
    return None;
}

pub fn create_mix_texture_cache(
    texture: &Texture,
    size: TextureCacheSize,
    cache_map: &TextureCacheMap,
) -> Result<Arc<RwLock<TextureCache>>, PbrtError> {
    let texture_type = texture.get_type();
    let id = texture.get_id();
    let edition = texture.get_edition();
    assert!(texture_type == "mix", "Texture type must be constant");
    let tex1 = get_texture_image(texture, size, cache_map, "tex1")
        .unwrap_or(create_one_pixel_image(&image::Rgba([1.0, 1.0, 1.0, 1.0])));
    let tex2 = get_texture_image(texture, size, cache_map, "tex2")
        .unwrap_or(create_one_pixel_image(&image::Rgba([1.0, 1.0, 1.0, 1.0])));
    let amount = get_texture_image(texture, size, cache_map, "amount")
        .unwrap_or(create_one_pixel_image(&image::Rgba([0.5, 0.5, 0.5, 1.0])));

    let image = mix_texture(&tex1, &tex2, &amount);
    let image = Arc::new(image);
    let texture_cache = TextureCache { id, edition, image };
    return Ok(Arc::new(RwLock::new(texture_cache)));
}

fn scale_texture(tex1: &DynamicImage, tex2: &DynamicImage) -> DynamicImage {
    let dim1 = tex1.dimensions();
    let dim2 = tex2.dimensions();
    let dimf = (dim1.0.max(dim2.0), dim1.1.max(dim2.1));
    let tex1 = tex1.resize_exact(dimf.0, dimf.1, image::imageops::FilterType::Lanczos3);
    let tex2 = tex2.resize_exact(dimf.0, dimf.1, image::imageops::FilterType::Lanczos3);
    let tex1 = tex1.to_rgba32f();
    let tex2 = tex2.to_rgba32f();
    let mut image_buffer = image::ImageBuffer::new(dimf.0, dimf.1);
    for (x, y, pixel) in image_buffer.enumerate_pixels_mut() {
        let p1 = tex1.get_pixel(x, y);
        let p2 = tex2.get_pixel(x, y);
        let r = p1[0] * p2[0];
        let g = p1[1] * p2[1];
        let b = p1[2] * p2[2];
        let alpha = p1[3] * p2[3];
        *pixel = image::Rgba([r, g, b, alpha]);
    }
    return DynamicImage::ImageRgba32F(image_buffer);
}

pub fn create_scale_texture_cache(
    texture: &Texture,
    size: TextureCacheSize,
    cache_map: &TextureCacheMap,
) -> Result<Arc<RwLock<TextureCache>>, PbrtError> {
    let texture_type = texture.get_type();
    let id = texture.get_id();
    let edition = texture.get_edition();
    assert!(texture_type == "scale", "Texture type must be scale");
    let tex1 = get_texture_image(texture, size, cache_map, "tex1")
        .unwrap_or(create_one_pixel_image(&image::Rgba([1.0, 1.0, 1.0, 1.0])));
    let tex2 = get_texture_image(texture, size, cache_map, "tex2")
        .unwrap_or(create_one_pixel_image(&image::Rgba([1.0, 1.0, 1.0, 1.0])));

    let image = scale_texture(&tex1, &tex2);
    let image = Arc::new(image);
    let texture_cache = TextureCache { id, edition, image };
    return Ok(Arc::new(RwLock::new(texture_cache)));
}

fn fetch_image(
    image: &ImageBuffer<image::Rgba<f32>, Vec<f32>>,
    u: f32,
    v: f32,
) -> image::Rgba<f32> {
    let width = image.width() as f32;
    let height = image.height() as f32;
    let x = (u * width).clamp(0.0, width - 1.0) as u32;
    let y = (v * height).clamp(0.0, height - 1.0) as u32;
    return image.get_pixel(x, y).clone();
}

pub fn create_checkerboard_texture_cache(
    texture: &Texture,
    size: TextureCacheSize,
    cache_map: &TextureCacheMap,
) -> Result<Arc<RwLock<TextureCache>>, PbrtError> {
    let texture_type = texture.get_type();
    let id = texture.get_id();
    let edition = texture.get_edition();
    assert!(
        texture_type == "checkerboard",
        "Texture type must be checkerboard"
    );
    let tex1 = get_texture_image(texture, size, cache_map, "tex1")
        .unwrap_or(create_one_pixel_image(&image::Rgba([1.0, 1.0, 1.0, 1.0])));
    let tex2 = get_texture_image(texture, size, cache_map, "tex2")
        .unwrap_or(create_one_pixel_image(&image::Rgba([0.0, 0.0, 0.0, 1.0])));

    let uscale = texture
        .as_property_map()
        .find_one_float("uscale")
        .unwrap_or(1.0);
    let vscale = texture
        .as_property_map()
        .find_one_float("vscale")
        .unwrap_or(1.0);

    let tex1 = tex1.to_rgba32f();
    let tex2 = tex2.to_rgba32f();

    let dim = match size {
        TextureCacheSize::Icon => (64, 64),
        TextureCacheSize::Full => (256, 256),
        TextureCacheSize::Size((w, h)) => (w as u32, h as u32),
    };

    let mut image_buffer = image::ImageBuffer::new(dim.0, dim.1);
    for y in 0..dim.1 {
        for x in 0..dim.0 {
            let u = x as f32 / dim.0 as f32;
            let v = y as f32 / dim.1 as f32;
            let u = u * uscale;
            let v = v * vscale;
            let xx = u as i32 & 1;
            let yy = v as i32 & 1;
            let zz = (xx + yy) & 1; // Checkerboard pattern
            let pixel = if zz == 0 {
                fetch_image(&tex1, u, v)
            } else {
                fetch_image(&tex2, u, v)
            };
            image_buffer.put_pixel(x, y, pixel);
        }
    }
    let image = DynamicImage::ImageRgba32F(image_buffer);
    let image = Arc::new(image);
    let texture_cache = TextureCache { id, edition, image };
    return Ok(Arc::new(RwLock::new(texture_cache)));
}

pub fn create_dots_texture_cache(
    texture: &Texture,
    size: TextureCacheSize,
    cache_map: &TextureCacheMap,
) -> Result<Arc<RwLock<TextureCache>>, PbrtError> {
    let texture_type = texture.get_type();
    let id = texture.get_id();
    let edition = texture.get_edition();
    assert!(texture_type == "dots", "Texture type must be dots");
    let tex1 = get_texture_image(texture, size, cache_map, "tex1")
        .unwrap_or(create_one_pixel_image(&image::Rgba([1.0, 1.0, 1.0, 1.0])));
    let tex2 = get_texture_image(texture, size, cache_map, "tex2")
        .unwrap_or(create_one_pixel_image(&image::Rgba([0.0, 0.0, 0.0, 1.0])));

    let uscale = texture
        .as_property_map()
        .find_one_float("uscale")
        .unwrap_or(1.0);
    let vscale = texture
        .as_property_map()
        .find_one_float("vscale")
        .unwrap_or(1.0);

    let outside = tex1.to_rgba32f();
    let inside = tex2.to_rgba32f();

    let dim = match size {
        TextureCacheSize::Icon => (64, 64),
        TextureCacheSize::Full => (256, 256),
        TextureCacheSize::Size((w, h)) => (w as u32, h as u32),
    };

    let mut image_buffer = image::ImageBuffer::new(dim.0, dim.1);
    for y in 0..dim.1 {
        for x in 0..dim.0 {
            let u = x as f32 / dim.0 as f32;
            let v = y as f32 / dim.1 as f32;
            let u = u * uscale;
            let v = v * vscale;
            let s_cell = (u + 0.5).floor();
            let t_cell = (v + 0.5).floor();
            if noise(s_cell + 0.5, t_cell + 0.5, 0.0) > 0.0 {
                let radius = 0.35;
                let max_shift = 0.5 - radius;
                let s_center = s_cell + max_shift * noise(s_cell + 1.5, t_cell + 2.8, 0.0);
                let t_center = t_cell + max_shift * noise(s_cell + 4.5, t_cell + 9.8, 0.0);
                let dist2 = (u - s_center).powi(2) + (v - t_center).powi(2);
                if dist2 < radius * radius {
                    let pixel = fetch_image(&inside, u, v);
                    image_buffer.put_pixel(x, y, pixel);
                } else {
                    let pixel = fetch_image(&outside, u, v);
                    image_buffer.put_pixel(x, y, pixel);
                }
            } else {
                let pixel = fetch_image(&outside, u, v);
                image_buffer.put_pixel(x, y, pixel);
            }
        }
    }
    let image = DynamicImage::ImageRgba32F(image_buffer);
    let image = Arc::new(image);
    let texture_cache = TextureCache { id, edition, image };
    return Ok(Arc::new(RwLock::new(texture_cache)));
}

pub fn create_fbm_texture_cache(
    texture: &Texture,
    size: TextureCacheSize,
) -> Result<Arc<RwLock<TextureCache>>, PbrtError> {
    let texture_type = texture.get_type();
    let id = texture.get_id();
    let edition = texture.get_edition();
    assert!(texture_type == "fbm", "Texture type must be fbm");
    let octaves = texture
        .as_property_map()
        .find_one_int("octaves")
        .unwrap_or(8);
    let roughness = texture
        .as_property_map()
        .find_one_float("roughness")
        .unwrap_or(0.5);

    let mat = texture.get_transform();
    let (t, _r, s) = mat.decompose(0.1).unwrap_or((
        Vector3::zero(),
        Quaternion::identity(),
        Vector3::new(1.0, 1.0, 1.0),
    ));

    let dim = match size {
        TextureCacheSize::Icon => (64, 64),
        TextureCacheSize::Full => (256, 256),
        TextureCacheSize::Size((w, h)) => (w as u32, h as u32),
    };

    let octaves = octaves.max(1);
    let roughness = roughness.clamp(0.0, 1.0);

    let mut image_buffer = image::ImageBuffer::new(dim.0, dim.1);
    for y in 0..dim.1 {
        for x in 0..dim.0 {
            let u = x as f32 / dim.0 as f32;
            let v = y as f32 / dim.1 as f32;
            let w = 0.0; // Assuming 2D texture, z-coordinate is not used
            let u = u * s.x + t.x;
            let v = v * s.y + t.y;
            let w = w * s.z + t.z;
            let p = Vector3::new(u as f32, v as f32, w as f32);
            let dpdx = Vector3::new(1.0 / dim.0 as f32, 0.0, 0.0);
            let dpdy = Vector3::new(0.0, 1.0 / dim.1 as f32, 0.0);
            let f = fbm(&p, &dpdx, &dpdy, roughness, octaves as u32);
            let pixel = image::Rgba([
                f as f32, f as f32, f as f32, 1.0, // Alpha channel
            ]);
            image_buffer.put_pixel(x, y, pixel);
        }
    }
    let image = DynamicImage::ImageRgba32F(image_buffer);
    let image = Arc::new(image);
    let texture_cache = TextureCache { id, edition, image };
    return Ok(Arc::new(RwLock::new(texture_cache)));
}

pub fn create_texture_cache_core(
    texture: &Texture,
    size: TextureCacheSize,
    cache_map: &TextureCacheMap,
) -> Result<Arc<RwLock<TextureCache>>, PbrtError> {
    let texture_type = texture.get_type();
    match texture_type.as_str() {
        "imagemap" => create_imagemap_texture_cache(texture, size),
        "constant" => create_constant_texture_cache(texture),
        "mix" => create_mix_texture_cache(texture, size, cache_map),
        "scale" => create_scale_texture_cache(texture, size, cache_map),
        "bilerp" => create_default_texture_cache(texture, size, &image::Rgba([255, 0, 255, 255])),
        "checkerboard" => create_checkerboard_texture_cache(texture, size, cache_map),
        "dots" => create_dots_texture_cache(texture, size, cache_map),
        "fbm" => create_fbm_texture_cache(texture, size),
        "windy" | "wrinkled" | "marble" => {
            create_default_texture_cache(texture, size, &image::Rgba([255, 0, 255, 255]))
        }
        _ => {
            return Err(PbrtError::error(&format!(
                "Unsupported texture type: {}",
                texture_type
            )));
        }
    }
}

pub fn create_texture_cache(
    texture: &Texture,
    size: TextureCacheSize,
    cache_map: &TextureCacheMap,
) -> Result<Arc<RwLock<TextureCache>>, PbrtError> {
    match create_texture_cache_core(texture, size, cache_map) {
        Ok(cache) => {
            let mut cache_map = cache_map.write().unwrap();
            let key = TextureCacheKey::from((texture.get_name(), texture.get_id(), size));
            cache_map.insert(key, cache.clone());
            return Ok(cache);
        }
        Err(e) => {
            return Err(e);
        }
    }
}
