use crate::models::scene::Component;
use crate::models::scene::Material;
use crate::models::scene::Mesh;
use crate::models::scene::ResourceObject;
use crate::models::scene::Texture;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct ResourceComponent {
    pub materials: HashMap<Uuid, Arc<RwLock<Material>>>,
    pub meshes: HashMap<Uuid, Arc<RwLock<Mesh>>>,
    pub textures: HashMap<Uuid, Arc<RwLock<Texture>>>,
    pub other_resources: HashMap<Uuid, Arc<RwLock<dyn ResourceObject>>>,
}

impl ResourceComponent {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Component for ResourceComponent {}
