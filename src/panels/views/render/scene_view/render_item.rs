use crate::models::base::Matrix4x4;
use crate::renderers::gl::RenderGizmo;
use crate::renderers::gl::RenderMaterial;
use crate::renderers::gl::RenderMesh;

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
    pub gizmo: Arc<RenderGizmo>,
    pub material: Arc<RenderMaterial>,
}

#[derive(Debug, Clone)]
pub enum RenderItem {
    Mesh(MeshRenderItem),
    Gizmo(GizmoRenderItem),
}
