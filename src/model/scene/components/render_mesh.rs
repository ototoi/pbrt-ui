use super::component::Component;
use crate::render::wgpu::mesh::RenderMesh;

use std::sync::Arc;

#[derive(Debug, Clone, Default)]
pub struct RenderMeshComponent {
    render_mesh: Option<Arc<RenderMesh>>,
}

impl RenderMeshComponent {
    pub fn new() -> Self {
        Self { render_mesh: None }
    }

    pub fn get_render_mesh(&self) -> Option<Arc<RenderMesh>> {
        self.render_mesh.clone()
    }

    pub fn set_render_mesh(&mut self, render_mesh: Arc<RenderMesh>) {
        self.render_mesh = Some(render_mesh);
    }

    pub fn clear(&mut self) {
        self.render_mesh = None;
    }
}

impl Component for RenderMeshComponent {}
