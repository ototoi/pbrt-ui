use super::texture_cache::TextureCache;
use super::texture_cache_size::TextureCacheSize;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextureCacheKey(pub String, pub Uuid, pub TextureCacheSize);

impl From<(String, Uuid, TextureCacheSize)> for TextureCacheKey {
    fn from(value: (String, Uuid, TextureCacheSize)) -> Self {
        TextureCacheKey(value.0, value.1, value.2)
    }
}

pub type TextureCacheMap = Arc<RwLock<HashMap<TextureCacheKey, Option<Arc<RwLock<TextureCache>>>>>>;
