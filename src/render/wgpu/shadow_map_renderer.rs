use super::camera::RenderCamera;
use super::lighting_mesh_renderer::get_shader_uses_shadow;
use super::lighting_mesh_renderer::LightingMeshRenderer;
use super::render_item::RenderItem;
use super::render_resource::RenderResourceManager;
use super::shadow::create_directional_light_shadows;
use eframe::wgpu;
use std::collections::HashMap;
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
        let (mesh_items, light_items) =
            mesh_renderer.prepare_without_shadow_render(device, queue, render_items, camera);

        let shadow_mesh_indices = mesh_items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                item.get_material().and_then(|material| {
                    if material
                        .passes
                        .iter()
                        .any(|pass| get_shader_uses_shadow(pass.render_category))
                    {
                        Some(i)
                    } else {
                        None
                    }
                })
            })
            .collect::<Vec<_>>();

        let directional_light_shadows = create_directional_light_shadows(
            device,
            queue,
            camera,
            &mesh_items,
            &light_items,
            render_resource_manager,
        );

        let mut directional_shadow_index_map = HashMap::new();
        let mut base_shadow_index = 0i32;
        for shadow in directional_light_shadows.iter() {
            directional_shadow_index_map.insert(shadow.id, base_shadow_index);
            base_shadow_index += shadow.cascades.len() as i32;
        }

        let directional_shadow_resources = mesh_renderer.prepare_and_render_directional_shadow_maps(
            device,
            queue,
            encoder,
            camera,
            &directional_light_shadows,
            &shadow_mesh_indices,
        );
        if let Some((directional_shadow_info_buffer, directional_shadow_map)) =
            directional_shadow_resources
        {
            mesh_renderer.set_directional_shadow_resources(
                device,
                &directional_shadow_info_buffer,
                &directional_shadow_map,
            );
        }

        mesh_renderer.prepare_lights(
            device,
            queue,
            &light_items,
            Some(&directional_shadow_index_map),
        );
    }
}
