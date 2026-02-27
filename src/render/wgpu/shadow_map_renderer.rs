use super::camera::RenderCamera;
use super::directional_shadow_map_renderer::DirectionalShadowMapRenderer;
use super::render_item::RenderItem;
use super::render_resource::RenderResourceManager;
use super::shadow::ShadowMaps;
use std::sync::Arc;

use eframe::wgpu;

#[derive(Debug, Clone)]
pub struct ShadowMapRenderer {
    directional_shadow_map_renderer: DirectionalShadowMapRenderer,
}

impl ShadowMapRenderer {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            directional_shadow_map_renderer: DirectionalShadowMapRenderer::new(device),
        }
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        render_resource_manager: &mut RenderResourceManager,
        render_items: &[Arc<RenderItem>],
        camera: &RenderCamera,
    ) -> ShadowMaps {
        let directional_shadow_maps = self.directional_shadow_map_renderer.prepare(
            device,
            queue,
            encoder,
            render_resource_manager,
            render_items,
            camera,
        );
        ShadowMaps {
            directional_shadow_maps,
        }
    }
}
