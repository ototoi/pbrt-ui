use crate::models::scene::Material;
use crate::models::scene::Mesh;
use crate::models::scene::ResourceComponent;
use crate::models::scene::ResourceManager;
use crate::models::scene::ResourceObject;
use crate::models::scene::Texture;

use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default)]
pub struct ResourceSelector {
    pub resouce_manager: Arc<Mutex<ResourceManager>>,
}

impl ResourceSelector {
    pub fn new(resources: &ResourceComponent) -> Self {
        Self {
            resouce_manager: resources.get_resource_manager(),
        }
    }
}