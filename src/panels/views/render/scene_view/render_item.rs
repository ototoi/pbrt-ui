use crate::models::base::Matrix4x4;
use crate::renderers::gl::RenderMaterial;
use crate::renderers::gl::RenderMesh;
use crate::renderers::gl::RenderProgram;

use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct MeshRenderItem {
    pub local_to_world: Matrix4x4,
    pub mesh: Arc<RenderMesh>,
    pub material: Arc<RenderMaterial>,
}

#[derive(Debug, Clone)]
pub struct GizmoRenderItem {
    pub local_to_world: Matrix4x4,
    pub program: Arc<RenderProgram>,
}

#[derive(Debug, Clone)]
pub struct ManipulatorRenderItem {
    pub local_to_world: Matrix4x4,
    pub program: Arc<RenderProgram>,
}

#[derive(Debug, Clone)]
pub enum RenderItem {
    Mesh(MeshRenderItem),
    Gizmo(GizmoRenderItem),
    Manipulator(ManipulatorRenderItem),
}
