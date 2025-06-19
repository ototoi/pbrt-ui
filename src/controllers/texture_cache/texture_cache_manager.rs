use super::texture_size::TextureSize;
use super::texture_cache_generator::TextureCacheGenerator;

use std::sync::Arc;
use std::sync::RwLock;
use std::collections::HashMap;

#[derive(Debug)]
pub struct TextureCacheManager {
    textures: Arc<RwLock<HashMap<(String, TextureSize), String>>>,
    generator: TextureCacheGenerator,
}

impl TextureCacheManager {
    pub fn new() -> Self {
        Self {
            textures: Arc::new(RwLock::new(HashMap::new())),
            generator: TextureCacheGenerator::new(),
        }
    }

    pub fn get_texture(&mut self, key: &str, size: TextureSize) -> Option<String> {
        let textures = self.textures.read().unwrap();
        if let Some(path) = textures.get(&(key.to_string(), size)) {
            return Some(path.clone());
        } else {
            self.generator.require_texture_cache(key, size, &self.textures);
            return None;
        }
    }
}
