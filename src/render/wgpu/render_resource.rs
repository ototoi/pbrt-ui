use super::mesh::RenderMesh;
use crate::model::scene::Component;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct RenderResourceManager {
    pub meshes: HashMap<Uuid, Arc<RenderMesh>>,
}

impl RenderResourceManager {
    pub fn new() -> Self {
        Self {
            meshes: HashMap::new(),
        }
    }
    pub fn add_mesh(&mut self, mesh: &Arc<RenderMesh>) {
        let id = mesh.get_id();
        self.meshes.insert(id, mesh.clone());
    }

    pub fn get_mesh(&self, id: Uuid) -> Option<&Arc<RenderMesh>> {
        self.meshes.get(&id)
    }

    pub fn remove_mesh(&mut self, id: Uuid) {
        self.meshes.remove(&id);
    }
}

#[derive(Debug, Clone)]
pub struct RenderResourceComponent {
    pub resource_manager: Arc<RwLock<RenderResourceManager>>,
}

impl RenderResourceComponent {
    pub fn new() -> Self {
        Self {
            resource_manager: Arc::new(RwLock::new(RenderResourceManager {
                meshes: HashMap::new(),
            })),
        }
    }

    pub fn get_resource_manager(&self) -> Arc<RwLock<RenderResourceManager>> {
        self.resource_manager.clone()
    }
}

impl Component for RenderResourceComponent {}