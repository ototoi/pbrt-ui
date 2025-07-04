use super::resource_manager::GLResourceManager;
use crate::model::scene::Component;

use std::sync::Arc;
use std::sync::Mutex;

use eframe::egui_glow;
use egui_glow::glow;

#[derive(Debug, Clone)]
pub struct GLResourceComponent {
    pub resource_manager: Arc<Mutex<GLResourceManager>>,
}

impl GLResourceComponent {
    pub fn new(gl: &Arc<glow::Context>) -> Self {
        Self {
            resource_manager: Arc::new(Mutex::new(GLResourceManager::new(gl))),
        }
    }

    pub fn get_resource_manager(&self) -> Arc<Mutex<GLResourceManager>> {
        self.resource_manager.clone()
    }
}

impl Component for GLResourceComponent {}
