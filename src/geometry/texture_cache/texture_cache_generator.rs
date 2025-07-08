use super::texture_cache::TextureCache;
use super::texture_cache_map::TextureCacheKey;
use super::texture_cache_map::TextureCacheMap;
use super::texture_cache_size::TextureCacheSize;
use crate::model::base::Property;
use crate::model::scene::Texture;

use crate::error::PbrtError;

use std::sync::{Arc, RwLock};
use std::thread;

use crypto::digest::Digest;
use image;
use image::DynamicImage;

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

pub fn create_imagemap_texture_cache(
    texture: &Texture,
    size: TextureCacheSize,
    cache_map: &TextureCacheMap,
) -> Result<(), PbrtError> {
    let texture_type = texture.get_type();
    assert!(texture_type == "imagemap", "Texture type must be imagemap");
    if let Some(src) = texture.get_fullpath() {
        let dst = create_texture_cache_path(&src, size);
        println!("Creating texture cache for: {} -> {}", src, dst);
        let src = std::path::PathBuf::from(src);
        let dst = std::path::PathBuf::from(dst);
        if dst.exists() {
            let image = image::open(&dst)
                .map_err(|e| PbrtError::error(&format!("Failed to open cached image: {}", e)))?;
            let image = Arc::new(image);
            {
                let name = texture.get_name();
                let id = texture.get_id();
                let key = TextureCacheKey::from((name, id, size));
                let mut cache_map = cache_map.write().unwrap();
                let texture_cache = TextureCache {
                    id: texture.id,
                    image: image,
                };
                cache_map.insert(key, Some(Arc::new(RwLock::new(texture_cache))));
            }
        } else if src.exists() {
            let src_img = image::open(&src)
                .map_err(|e| PbrtError::error(&format!("Failed to open image: {}", e)))?;
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
                .map_err(|e| PbrtError::error(&format!("Failed to save image: {}", e)))?;

            let image = Arc::new(resized_img);
            {
                let name = texture.get_name();
                let id = texture.get_id();
                let key = TextureCacheKey::from((name, id, size));
                let mut cache_map = cache_map.write().unwrap();
                let texture_cache = TextureCache {
                    id: texture.id,
                    image: image,
                };
                cache_map.insert(key, Some(Arc::new(RwLock::new(texture_cache))));
            }
        }
    }

    return Ok(());
}

pub fn create_default_texture_cache(
    texture: &Texture,
    size: TextureCacheSize,
    cache_map: &TextureCacheMap,
    color: &image::Rgba<u8>,
) -> Result<(), PbrtError> {
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

    {
        let name = texture.get_name();
        let id = texture.get_id();
        let key = TextureCacheKey::from((name, id, size));
        let mut cache_map = cache_map.write().unwrap();
        let texture_cache = TextureCache {
            id: texture.id,
            image: image,
        };
        cache_map.insert(key, Some(Arc::new(RwLock::new(texture_cache))));
    }
    Ok(())
}

pub fn create_constant_texture_cache(
    texture: &Texture,
    size: TextureCacheSize,
    cache_map: &TextureCacheMap,
) -> Result<(), PbrtError> {
    let texture_type = texture.get_type();
    assert!(texture_type == "constant", "Texture type must be constant");
    let color = texture.as_property_map().get_floats("color value");
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

    let image_buffer = image::ImageBuffer::from_pixel(8, 8, color);

    let image = image::DynamicImage::ImageRgba32F(image_buffer);
    let image = Arc::new(image);
    {
        let name = texture.get_name();
        let id = texture.get_id();
        let key = TextureCacheKey::from((name, id, size));
        let mut cache_map = cache_map.write().unwrap();
        let texture_cache = TextureCache {
            id: texture.id,
            image: image,
        };
        cache_map.insert(key, Some(Arc::new(RwLock::new(texture_cache))));
    }
    Ok(())
}

fn find_texture_cache(
    cache_map: &TextureCacheMap,
    name: &str,
    size: TextureCacheSize,
) -> Option<Arc<RwLock<TextureCache>>> {
    let textures = cache_map.read().unwrap();
    for (cache_key, cache) in textures.iter() {
        if cache_key.0 == name && cache_key.2 == size {
            if let Some(cache) = cache {
                return Some(cache.clone());
            }
        }
    }
    None
}

fn mix_texture(tex1: &DynamicImage, tex2: &DynamicImage, amount: &DynamicImage) -> DynamicImage {
    // This function would implement the logic to mix two textures based on the amount.
    // For simplicity, we will just return one of the textures here.
    // In a real implementation, you would blend the two textures based on the amount.
    return tex1.clone();
}

fn get_texture_image(
    texture: &Texture,
    size: TextureCacheSize,
    cache_map: &TextureCacheMap,
    key: &str,
) -> Option<Arc<DynamicImage>> {
    let props = texture.as_property_map();
    if let Some((key_type, _, value)) = props.entry(key) {
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
        } else if let Property::Strings(name) = value {
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
) -> Result<(), PbrtError> {
    let texture_type = texture.get_type();
    assert!(texture_type == "mix", "Texture type must be constant");
    let tex1 = get_texture_image(
        texture, size, cache_map, "tex1"
    );
    let tex2 = get_texture_image(
        texture, size, cache_map, "tex2"
    );

    if tex1.is_none() || tex2.is_none() {
        return Err(PbrtError::error("Missing texture images for mix texture"));
    }

    let amount = get_texture_image(
        texture, size, cache_map, "amount"
    );

    let amount = match amount {
        Some(img) => img,
        None => {
            // If no amount texture is provided, use a default value of 0.5
            let color = image::Rgba([0.5, 0.5, 0.5, 1.0]);
            let image_buffer = image::ImageBuffer::from_pixel(1, 1, color);
            Arc::new(DynamicImage::ImageRgba32F(image_buffer))
        }
    };

    let tex1 = tex1.unwrap();
    let tex2 = tex2.unwrap();

    let mixed_image = mix_texture(&tex1, &tex2, &amount);
    let mixed_image = Arc::new(mixed_image);
    {
        let name = texture.get_name();
        let id = texture.get_id();
        let key = TextureCacheKey::from((name, id, size));
        let mut cache_map = cache_map.write().unwrap();
        let texture_cache = TextureCache {
            id: texture.id,
            image: mixed_image,
        };
        cache_map.insert(key, Some(Arc::new(RwLock::new(texture_cache))));
    }
    Ok(())
}

fn scale_texture(tex1: &DynamicImage, tex2: &DynamicImage) -> DynamicImage {
    // This function would implement the logic to scale a texture.
    // For simplicity, we will just return one of the textures here.
    // In a real implementation, you would scale the texture based on the scale factor.
    return tex1.clone();
}

pub fn create_scale_texture_cache(
    texture: &Texture,
    size: TextureCacheSize,
    cache_map: &TextureCacheMap,
) -> Result<(), PbrtError> {
    let texture_type = texture.get_type();
    assert!(texture_type == "scale", "Texture type must be constant");
    let tex1 = get_texture_image(
        texture, size, cache_map, "tex1"
    );
    let tex2 = get_texture_image(
        texture, size, cache_map, "tex2"
    );

    if tex1.is_none() || tex2.is_none() {
        return Err(PbrtError::error("Missing texture images for mix texture"));
    }

    let tex1 = tex1.unwrap();
    let tex2 = tex2.unwrap();

    let scaled_image = scale_texture(&tex1, &tex2);
    let scaled_image = Arc::new(scaled_image);
    {
        let name = texture.get_name();
        let id = texture.get_id();
        let key = TextureCacheKey::from((name, id, size));
        let mut cache_map = cache_map.write().unwrap();
        let texture_cache = TextureCache {
            id: texture.id,
            image: scaled_image,
        };
        cache_map.insert(key, Some(Arc::new(RwLock::new(texture_cache))));
    }
    Ok(())
}

pub fn create_texture_cache(
    texture: &Texture,
    size: TextureCacheSize,
    cache_map: &TextureCacheMap,
) -> Result<(), PbrtError> {
    let texture_type = texture.get_type();
    match texture_type.as_str() {
        "imagemap" => {
            create_imagemap_texture_cache(texture, size, cache_map)?;
        }
        "constant" => {
            create_constant_texture_cache(texture, size, cache_map)?;
        }
        "mix" => {
            create_mix_texture_cache(texture, size, cache_map)?;
        }
        "scale" => {
            create_scale_texture_cache(texture, size, cache_map)?;
        }
        "bilerp" => {
            create_default_texture_cache(
                texture,
                size,
                cache_map,
                &image::Rgba([255, 0, 255, 255]),
            )?;
        }
        "checkerboard" | "dots" | "fbm" | "windy" | "wrinkled" | "marble" => {
            create_default_texture_cache(
                texture,
                size,
                cache_map,
                &image::Rgba([255, 0, 255, 255]),
            )?;
        }
        _ => {
            log::warn!("Unsupported texture type for caching: {}", texture_type);
        }
    }
    Ok(())
}

fn create_texture_cache_task(
    texture: &Texture,
    size: TextureCacheSize,
    cache_map: &TextureCacheMap,
) -> Result<thread::JoinHandle<()>, PbrtError> {
    let texture = texture.clone();
    let cache_map = cache_map.clone();
    let handle = thread::spawn(move || {
        create_texture_cache(&texture, size, &cache_map)
            .unwrap_or_else(|e| log::error!("Texture cache task failed: {}", e));
    });
    Ok(handle)
}

#[derive(Debug, Default)]
pub struct TextureCacheGenerator {
    // This struct will handle the generation of texture cache entries.
    tasks: Vec<thread::JoinHandle<()>>,
}

impl TextureCacheGenerator {
    pub fn new() -> Self {
        Self {
            // Initialize any necessary fields here.
            tasks: Vec::new(),
        }
    }

    pub fn require_texture_cache_imm(
        &mut self,
        texture: &Texture,
        size: TextureCacheSize,
        cache_map: &TextureCacheMap,
    ) {
        match create_texture_cache(texture, size, cache_map) {
            Ok(()) => {
                //
            }
            Err(e) => {
                log::error!("Error creating texture cache task: {}", e);
            }
        }
    }

    pub fn require_texture_cache(
        &mut self,
        texture: &Texture,
        size: TextureCacheSize,
        cache_map: &TextureCacheMap,
    ) {
        {
            let mut new_tasks = Vec::new();
            for task in self.tasks.drain(..) {
                if task.is_finished() {
                    // If the task is finished, we can safely ignore it.
                    continue;
                }
                new_tasks.push(task);
            }
            self.tasks = new_tasks;
        }
        {
            match create_texture_cache_task(texture, size, cache_map) {
                Ok(handle) => {
                    self.tasks.push(handle);
                }
                Err(e) => {
                    log::error!("Error creating texture cache task: {}", e);
                }
            }
        }
    }
}
