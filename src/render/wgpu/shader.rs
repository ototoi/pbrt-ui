use std::sync::Arc;
use uuid::Uuid;

use eframe::wgpu;

#[derive(Debug, Clone)]
pub struct RenderShader {
    pub id: Uuid,
    pub shader: Arc<wgpu::ShaderModule>,
}

impl RenderShader {
    pub fn get_id(&self) -> Uuid {
        self.id
    }
}
