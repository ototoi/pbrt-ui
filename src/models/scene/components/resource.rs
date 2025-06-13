use crate::models::scene::Component;
use crate::models::scene::Material;
use crate::models::scene::Mesh;
use crate::models::scene::ResourceObject;
use crate::models::scene::Texture;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;

use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct ResourceManager {
    pub materials: HashMap<Uuid, Arc<RwLock<Material>>>,
    pub meshes: HashMap<Uuid, Arc<RwLock<Mesh>>>,
    pub textures: HashMap<Uuid, Arc<RwLock<Texture>>>,
    pub other_resources: HashMap<Uuid, Arc<RwLock<dyn ResourceObject>>>,
}

#[derive(Debug, Clone, Default)]
pub struct ResourceComponent {
    pub resource_manager: Arc<Mutex<ResourceManager>>,
}

impl ResourceComponent {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_resource_manager(&self) -> Arc<Mutex<ResourceManager>> {
        self.resource_manager.clone()
    }
}

impl Component for ResourceComponent {}
