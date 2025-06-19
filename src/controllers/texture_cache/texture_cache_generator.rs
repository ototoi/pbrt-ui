use crate::error::PbrtError;

use super::texture_size::TextureSize;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;
use std::thread;

use crypto::digest::Digest;
use image;

type TextureCacheMap = Arc<RwLock<HashMap<(String, TextureSize), String>>>;

fn get_digest(path: &str) -> String {
    let mut hasher = crypto::sha1::Sha1::new();
    hasher.input_str(path);
    let digest = hasher.result_str();
    return digest;
}

fn create_texture_cache_path(key: &str, size: TextureSize) -> String {
    let dir = dirs::cache_dir()
        .unwrap()
        .join("pbrt_ui")
        .join("cache")
        .join("texture")
        .join(size.to_string());
    std::fs::create_dir_all(&dir).expect("Failed to create cache directory");
    let path = dir.join(format!("{}.png", get_digest(key)));
    return path.to_str().unwrap().to_string();
}

fn resize_image(img: &image::DynamicImage, size: TextureSize) -> image::DynamicImage {
    match size {
        TextureSize::Icon => img.resize(64, 64, image::imageops::FilterType::Lanczos3),
        TextureSize::Large => img.resize(256, 256, image::imageops::FilterType::Lanczos3),
        TextureSize::Size((width, height)) => img.resize(
            width as u32,
            height as u32,
            image::imageops::FilterType::Lanczos3,
        ),
    }
}

fn create_texture_cache(src: &str, dst: &str, size: TextureSize) -> Result<(), PbrtError> {
    let src = std::path::PathBuf::from(src);
    let dst = std::path::PathBuf::from(dst);
    if !dst.exists() {
        let src_img = image::open(&src)
            .map_err(|e| PbrtError::error(&format!("Failed to open image: {}", e)))?;
        let resized_img = resize_image(&src_img, size);
        resized_img
            .save(&dst)
            .map_err(|e| PbrtError::error(&format!("Failed to save image: {}", e)))?;
    }
    return Ok(());
}

fn create_texture_cache_task(
    src: &str,
    dst: &str,
    size: TextureSize,
    textures: &TextureCacheMap,
) -> Result<thread::JoinHandle<()>, PbrtError> {
    let textures = textures.clone();
    let src = src.to_string();
    let dst = dst.to_string();
    let handle = thread::spawn(move || {
        // Here we would implement the logic to copy the texture from src to dst.
        // This is a placeholder for the actual implementation.
        if let Err(e) = create_texture_cache(&src, &dst, size) {
            log::error!("Failed to create texture cache: {}", e);
        }
        let mut textures = textures.write().unwrap();
        textures.insert((src.clone(), size), dst.clone());
    });
    Ok(handle)
}

#[derive(Debug)]
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

    pub fn require_texture_cache(
        &mut self,
        key: &str,
        size: TextureSize,
        textures: &TextureCacheMap,
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
            let org_path = std::path::PathBuf::from(key);
            if !org_path.exists() {
                return;
            }
            let cache_path = create_texture_cache_path(key, size);
            match create_texture_cache_task(key, &cache_path, size, textures) {
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
