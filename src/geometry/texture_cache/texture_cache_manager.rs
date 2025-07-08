use super::texture_cache::TextureCache;
use super::texture_cache_generator::TextureCacheGenerator;
use super::texture_cache_map::TextureCacheKey;
use super::texture_cache_map::TextureCacheMap;
use super::texture_cache_size::TextureCacheSize;

use crate::model::scene::Texture;

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;

#[derive(Debug)]
pub struct TextureCacheManager {
    textures: TextureCacheMap,
    generator: Mutex<TextureCacheGenerator>,
}

impl TextureCacheManager {
    pub fn new() -> Self {
        Self {
            textures: TextureCacheMap::default(),
            generator: Mutex::new(TextureCacheGenerator::new()),
        }
    }

    fn find_texture_cache(
        &self,
        texture: &Texture,
        sz: TextureCacheSize,
    ) -> Option<(TextureCacheKey, Option<Arc<RwLock<TextureCache>>>)> {
        let textures = self.textures.read().unwrap();
        let id = texture.get_id();
        let name = texture.get_name();
        let key = TextureCacheKey::from((name, id, sz));
        if let Some(cache) = textures.get(&key) {
            return Some((key.clone(), cache.clone()));
        }
        return None;
    }

    pub fn get_texture_cache(
        &self,
        texture: &Texture,
        sz: TextureCacheSize,
    ) -> Option<Arc<RwLock<TextureCache>>> {
        let id = texture.get_id();
        let name = texture.get_name();
        if let Some((_key, cache)) = self.find_texture_cache(texture, sz) {
            return cache.clone();
        } else {
            {
                let key = TextureCacheKey::from((name, id, sz));
                let mut textures = self.textures.write().unwrap();
                textures.insert(key, None);
            }
            {
                let mut generator = self.generator.lock().unwrap();
                generator.require_texture_cache_imm(
                    texture,
                    sz,
                    &self.textures,
                );
            }
            return None;
        }
    }
}
