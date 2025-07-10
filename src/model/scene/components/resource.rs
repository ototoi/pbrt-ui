use crate::model::scene::Component;
use crate::model::scene::Material;
use crate::model::scene::OtherResource;
use crate::model::scene::ResourceObject;
use crate::model::scene::Shape;
use crate::model::scene::Texture;
use crate::model::scene::components::resource;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct ResourceManager {
    pub materials: HashMap<Uuid, Arc<RwLock<Material>>>,
    pub meshes: HashMap<Uuid, Arc<RwLock<Shape>>>,
    pub textures: HashMap<Uuid, Arc<RwLock<Texture>>>,
    pub other_resources: HashMap<Uuid, Arc<RwLock<dyn ResourceObject>>>,
}

impl ResourceManager {
    pub fn find_texture_by_name(&self, name: &str) -> Option<Arc<RwLock<Texture>>> {
        self.textures
            .values()
            .find(|texture| texture.read().unwrap().get_name() == name)
            .cloned()
    }

    pub fn find_spectrum_by_filename(&self, name: &str) -> Option<Arc<RwLock<dyn ResourceObject>>> {
        self.other_resources
            .values()
            .find(|resource| {
                let resource = resource.read().unwrap();
                if let Some(filename) = resource.get_filename() {
                    if filename == name {
                        return true;
                    }
                }
                return false;
            })
            .cloned()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ResourceComponent {
    pub resource_manager: Arc<RwLock<ResourceManager>>,
}

impl ResourceComponent {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_resource_manager(&self) -> Arc<RwLock<ResourceManager>> {
        self.resource_manager.clone()
    }
}

impl Component for ResourceComponent {}
