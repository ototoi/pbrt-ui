use super::resource_manager::GLResourceManager;
use crate::models::scene::Component;

use std::sync::Arc;
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct GLResourceComponent {
    pub resource_manager: Arc<Mutex<GLResourceManager>>,
}

impl GLResourceComponent {
    pub fn new() -> Self {
        Self {
            resource_manager: Arc::new(Mutex::new(GLResourceManager::new())),
        }
    }

    pub fn get_resource_manager(&self) -> Arc<Mutex<GLResourceManager>> {
        self.resource_manager.clone()
    }
}

impl Component for GLResourceComponent {}
