use super::resource_manager::GLResourceManager;
use crate::model::scene::Component;

use std::sync::Arc;
use std::sync::RwLock;

use eframe::egui_glow;
use egui_glow::glow;

#[derive(Debug, Clone)]
pub struct GLResourceComponent {
    pub resource_manager: Arc<RwLock<GLResourceManager>>,
}

impl GLResourceComponent {
    pub fn new(gl: &Arc<glow::Context>) -> Self {
        Self {
            resource_manager: Arc::new(RwLock::new(GLResourceManager::new(gl))),
        }
    }

    pub fn get_resource_manager(&self) -> Arc<RwLock<GLResourceManager>> {
        self.resource_manager.clone()
    }
}

impl Component for GLResourceComponent {}
