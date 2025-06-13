use super::mesh::RenderMesh;
use super::program::RenderProgram;

use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct GLResourceManager {
    pub meshes: HashMap<Uuid, Arc<RenderMesh>>,
    pub programs: HashMap<Uuid, Arc<RenderProgram>>,
}

impl GLResourceManager {
    pub fn new() -> Self {
        Self {
            meshes: HashMap::new(),
            programs: HashMap::new(),
        }
    }

    pub fn add_mesh(&mut self, mesh: &Arc<RenderMesh>) {
        self.meshes.insert(mesh.id, mesh.clone());
    }
    pub fn get_mesh(&self, id: Uuid) -> Option<&Arc<RenderMesh>> {
        self.meshes.get(&id)
    }
    pub fn remove_mesh(&mut self, id: Uuid) {
        self.meshes.remove(&id);
    }

    pub fn add_program(&mut self, program: &Arc<RenderProgram>) {
        self.programs.insert(program.id, program.clone());
    }
    pub fn get_program(&self, id: Uuid) -> Option<&Arc<RenderProgram>> {
        self.programs.get(&id)
    }
    pub fn remove_program(&mut self, id: Uuid) {
        self.programs.remove(&id);
    }
}
