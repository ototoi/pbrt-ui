use super::texture_size::TextureSize;

use std::sync::Arc;
use std::sync::RwLock;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct TextureCacheManager {
    textures: Arc<RwLock<HashMap<(String, TextureSize), String>>>,
}

impl TextureCacheManager {
    pub fn new() -> Self {
        Self {
            textures: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn get_texture(&mut self, key: &str, size: TextureSize) -> Option<String> {
        let textures = self.textures.read().unwrap();
        if let Some(path) = textures.get(&(key.to_string(), size)) {
            return Some(path.clone());
        } else {
            return None;
        }
    }
}
