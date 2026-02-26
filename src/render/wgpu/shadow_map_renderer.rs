use super::camera::RenderCamera;
use super::lighting_mesh_renderer::LightingMeshRenderer;
use super::render_item::RenderItem;
use super::render_resource::RenderResourceManager;
use eframe::wgpu;
use std::sync::Arc;

#[derive(Debug, Default, Clone)]
pub struct ShadowMapRenderer;

impl ShadowMapRenderer {
    pub fn new() -> Self {
        Self
    }

    pub fn prepare(
        &mut self,
        mesh_renderer: &mut LightingMeshRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        render_resource_manager: &mut RenderResourceManager,
        render_items: &[Arc<RenderItem>],
        camera: &RenderCamera,
    ) {
        mesh_renderer.prepare_with_shadow_maps(
            device,
            queue,
            encoder,
            render_resource_manager,
            render_items,
            camera,
        );
    }
}
