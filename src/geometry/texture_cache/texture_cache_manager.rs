use super::texture_cache::TextureCache;
use super::texture_cache_generator::create_texture_cache;
use super::texture_cache_map::TextureCacheKey;
use super::texture_cache_map::TextureCacheMap;
use super::texture_cache_size::TextureCacheSize;

use crate::model::scene::Texture;

use std::sync::Arc;
use std::sync::RwLock;

#[derive(Debug)]
pub struct TextureCacheManager {
    textures: TextureCacheMap,
}

impl TextureCacheManager {
    pub fn new() -> Self {
        Self {
            textures: TextureCacheMap::default(),
        }
    }

    fn find_texture_cache(
        &self,
        texture: &Texture,
        sz: TextureCacheSize,
    ) -> Option<Arc<RwLock<TextureCache>>> {
        let textures = self.textures.read().unwrap();
        let id = texture.get_id();
        let name = texture.get_name();
        let key = TextureCacheKey::from((name, id, sz));
        if let Some(cache) = textures.get(&key) {
            return Some(cache.clone());
        }
        return None;
    }

    pub fn get_texture_cache(
        &self,
        texture: &Texture,
        sz: TextureCacheSize,
    ) -> Option<Arc<RwLock<TextureCache>>> {
        if let Some(cache) = self.find_texture_cache(texture, sz) {
            return Some(cache);
        } else {
            {
                let _ = create_texture_cache(texture, sz, &self.textures);
            }
            if let Some(cache) = self.find_texture_cache(texture, sz) {
                return Some(cache);
            }
            return None;
        }
    }
}
