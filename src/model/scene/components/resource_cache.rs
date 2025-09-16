use super::component::Component;
use crate::conversion::texture_cache::TextureNode;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ResourceCacheManager {
    pub textures: HashMap<Uuid, Arc<RwLock<TextureNode>>>,
}

impl ResourceCacheManager {
    pub fn new() -> Self {
        Self {
            textures: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResourceCacheComponent {
    resource_cache_manager: Arc<RwLock<ResourceCacheManager>>,
}

impl ResourceCacheComponent {
    pub fn new() -> Self {
        Self {
            resource_cache_manager: Arc::new(RwLock::new(ResourceCacheManager::new())),
        }
    }

    pub fn get_resource_cache_manager(&self) -> Arc<RwLock<ResourceCacheManager>> {
        return self.resource_cache_manager.clone();
    }
}

impl Component for ResourceCacheComponent {}
