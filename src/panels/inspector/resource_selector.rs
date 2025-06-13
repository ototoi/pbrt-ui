use crate::models::scene::Material;
use crate::models::scene::Mesh;
use crate::models::scene::ResourceComponent;
use crate::models::scene::ResourceObject;
use crate::models::scene::Texture;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct ResourceSelector {
    pub materials: HashMap<Uuid, Arc<RwLock<Material>>>,
    pub meshes: HashMap<Uuid, Arc<RwLock<Mesh>>>,
    pub textures: HashMap<Uuid, Arc<RwLock<Texture>>>,
    pub other_resources: HashMap<Uuid, Arc<RwLock<dyn ResourceObject>>>,
}

impl ResourceSelector {
    pub fn new(resources: &ResourceComponent) -> Self {
        Self {
            materials: resources.materials.clone(),
            meshes: resources.meshes.clone(),
            textures: resources.textures.clone(),
            other_resources: resources.other_resources.clone(),
        }
    }
}
