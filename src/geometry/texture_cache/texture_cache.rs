use image::DynamicImage;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct TextureCache {
    pub id: Uuid,
    pub image: Arc<DynamicImage>,
}

impl TextureCache {
    pub fn get_id(&self) -> Uuid {
        self.id
    }
}
