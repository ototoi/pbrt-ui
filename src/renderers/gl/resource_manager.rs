use crate::renderers::gl::line::RenderLine;

use super::gizmo::RenderGizmo;
use super::material::RenderMaterial;
use super::mesh::RenderMesh;
use super::program::RenderProgram;

use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use eframe::egui_glow;
use egui_glow::glow;

#[derive(Debug, Clone)]
pub struct GLResourceManager {
    meshes: HashMap<Uuid, Arc<RenderMesh>>,
    gizmos: HashMap<Uuid, Arc<RenderGizmo>>, //for gizmos
    materials: HashMap<Uuid, Arc<RenderMaterial>>,
    programs: HashMap<Uuid, Arc<RenderProgram>>, //shader
    gl: Arc<glow::Context>,
}

impl GLResourceManager {
    pub fn new(gl: &Arc<glow::Context>) -> Self {
        Self {
            meshes: HashMap::new(),
            gizmos: HashMap::new(), //for gizmos
            materials: HashMap::new(),
            programs: HashMap::new(),
            gl: gl.clone(),
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

    pub fn add_gizmo(&mut self, gizmo: &Arc<RenderGizmo>) {
        let id = gizmo.get_id();
        self.gizmos.insert(id, gizmo.clone());
    }
    pub fn get_gizmo(&self, id: Uuid) -> Option<&Arc<RenderGizmo>> {
        self.gizmos.get(&id)
    }
    pub fn remove_gizmo(&mut self, id: Uuid) {
        self.gizmos.remove(&id);
    }

    pub fn add_material(&mut self, material: &Arc<RenderMaterial>) {
        self.materials.insert(material.id, material.clone());
    }
    pub fn get_material(&self, id: Uuid) -> Option<&Arc<RenderMaterial>> {
        self.materials.get(&id)
    }
    pub fn remove_material(&mut self, id: Uuid) {
        self.materials.remove(&id);
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
