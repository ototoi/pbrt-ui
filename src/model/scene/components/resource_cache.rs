use super::component::Component;
use crate::geometry::texture_cache::TextureCacheManager;

use std::sync::Arc;
use std::sync::RwLock;

#[derive(Debug, Clone)]
pub struct ResourceCacheComponent {
    pub texture_cache_manager: Arc<RwLock<TextureCacheManager>>,
}

impl ResourceCacheComponent {
    pub fn new() -> Self {
        Self {
            texture_cache_manager: Arc::new(RwLock::new(TextureCacheManager::new())),
        }
    }

    pub fn get_texture_cache_manager(&self) -> Arc<RwLock<TextureCacheManager>> {
        self.texture_cache_manager.clone()
    }
}

impl Component for ResourceCacheComponent {}
