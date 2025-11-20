use eframe::wgpu;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct RenderTexture {
    pub id: Uuid,
    pub edition: String,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub scale: [f32; 2],
    pub delta: [f32; 2],
}

impl RenderTexture {
    pub fn get_id(&self) -> Uuid {
        self.id
    }
}
