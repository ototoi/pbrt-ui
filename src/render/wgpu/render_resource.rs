use super::light::RenderLight;
use super::lines::RenderLines;
use super::material::RenderMaterial;
use super::mesh::RenderMesh;
use crate::model::scene::Component;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct RenderResourceManager {
    pub meshes: HashMap<Uuid, Arc<RenderMesh>>,
    pub lights: HashMap<Uuid, Arc<RenderLight>>, // Assuming lights are also stored as RenderLines
    pub lines: HashMap<Uuid, Arc<RenderLines>>,
    pub materials: HashMap<Uuid, Arc<RenderMaterial>>,
}

impl RenderResourceManager {
    pub fn new() -> Self {
        Self {
            meshes: HashMap::new(),
            lights: HashMap::new(),
            lines: HashMap::new(),
            materials: HashMap::new(),
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

    pub fn add_light(&mut self, light: &Arc<RenderLight>) {
        let id = light.get_id();
        self.lights.insert(id, light.clone());
    }

    pub fn get_light(&self, id: Uuid) -> Option<&Arc<RenderLight>> {
        self.lights.get(&id)
    }

    pub fn remove_light(&mut self, id: Uuid) {
        self.lights.remove(&id);
    }

    pub fn add_lines(&mut self, lines: &Arc<RenderLines>) {
        let id = lines.get_id();
        self.lines.insert(id, lines.clone());
    }

    pub fn get_lines(&self, id: Uuid) -> Option<&Arc<RenderLines>> {
        self.lines.get(&id)
    }

    pub fn remove_lines(&mut self, id: Uuid) {
        self.lines.remove(&id);
    }

    pub fn add_material(&mut self, material: &Arc<RenderMaterial>) {
        let id = material.get_id();
        self.materials.insert(id, material.clone());
    }

    pub fn get_material(&self, id: Uuid) -> Option<&Arc<RenderMaterial>> {
        self.materials.get(&id)
    }

    pub fn remove_material(&mut self, id: Uuid) {
        self.materials.remove(&id);
    }
}

#[derive(Debug, Clone)]
pub struct RenderResourceComponent {
    pub resource_manager: Arc<RwLock<RenderResourceManager>>,
}

impl RenderResourceComponent {
    pub fn new() -> Self {
        Self {
            resource_manager: Arc::new(RwLock::new(RenderResourceManager::new())),
        }
    }

    pub fn get_resource_manager(&self) -> Arc<RwLock<RenderResourceManager>> {
        self.resource_manager.clone()
    }
}

impl Component for RenderResourceComponent {}
