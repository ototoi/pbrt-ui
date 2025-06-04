use eframe::glow;

use super::resource_manager::ResourceManager;
use crate::models::scene::Component;

use std::sync::Arc;
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct ResourceComponent {
    pub resource_manager: Arc<Mutex<ResourceManager>>,
}

impl ResourceComponent {
    pub fn new() -> Self {
        Self {
            resource_manager: Arc::new(Mutex::new(ResourceManager::new())),
        }
    }

    pub fn get_resource_manager(&self) -> Arc<Mutex<ResourceManager>> {
        self.resource_manager.clone()
    }
}

impl Component for ResourceComponent {}
