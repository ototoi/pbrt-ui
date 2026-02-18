use super::light::RenderLight;
use super::render_item::RenderItem;
use super::render_resource::RenderResourceManager;
use super::texture::RenderTexture;
use std::sync::Arc;

use eframe::wgpu;
use uuid::Uuid;

const SHADOW_MAP_SIZE: u32 = 2048;
const SHADOW_BOUNDS_MARGIN: f32 = 0.1;

#[derive(Debug, Clone)]
pub struct RenderDirectionalLightShadow {
    pub id: Uuid,
    pub edition: String,
    pub light_view: glam::Mat4,
    pub light_proj: glam::Mat4,
    pub light_view_proj: glam::Mat4,
    pub textures: Vec<Arc<RenderTexture>>,
}

fn get_aabb_corners(min: glam::Vec3, max: glam::Vec3) -> [glam::Vec3; 8] {
    [
        glam::vec3(min.x, min.y, min.z),
        glam::vec3(min.x, min.y, max.z),
        glam::vec3(min.x, max.y, min.z),
        glam::vec3(min.x, max.y, max.z),
        glam::vec3(max.x, min.y, min.z),
        glam::vec3(max.x, min.y, max.z),
        glam::vec3(max.x, max.y, min.z),
        glam::vec3(max.x, max.y, max.z),
    ]
}

fn expand_bounds(min: &mut glam::Vec3, max: &mut glam::Vec3, p: glam::Vec3) {
    *min = min.min(p);
    *max = max.max(p);
}

pub fn create_directional_light_shadows(
    device: &wgpu::Device,
    _queue: &wgpu::Queue,
    render_items: &[Arc<RenderItem>],
    render_resource_manager: &mut RenderResourceManager,
) -> Vec<Arc<RenderDirectionalLightShadow>> {
    let mut shadows = Vec::new();

    //
    let mut need_compute_shadows = false;
    for item in render_items {
        if let RenderItem::Light(light_item) = item.as_ref() {
            if let RenderLight::Directional(light) = light_item.light.as_ref() {
                if light.cast_shadow {
                    need_compute_shadows = true;
                    break;
                }
            }
        }
    }

    if !need_compute_shadows {
        return shadows;
    }

    let mut world_min = glam::vec3(f32::INFINITY, f32::INFINITY, f32::INFINITY);
    let mut world_max = glam::vec3(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    let mut has_mesh = false;

    for item in render_items {
        if let RenderItem::Mesh(mesh_item) = item.as_ref() {
            has_mesh = true;
            let aabb_min = glam::vec3(
                mesh_item.mesh.aabb.min[0],
                mesh_item.mesh.aabb.min[1],
                mesh_item.mesh.aabb.min[2],
            );
            let aabb_max = glam::vec3(
                mesh_item.mesh.aabb.max[0],
                mesh_item.mesh.aabb.max[1],
                mesh_item.mesh.aabb.max[2],
            );
            let corners = get_aabb_corners(aabb_min, aabb_max);
            for corner in corners {
                let world = mesh_item.matrix.transform_point3(corner);
                expand_bounds(&mut world_min, &mut world_max, world);
            }
        }
    }

    if !has_mesh {
        return shadows;
    }

    let world_center = 0.5 * (world_min + world_max);
    let world_extent = world_max - world_min;
    let world_radius = world_extent.length() * 0.5;
    let light_distance = world_radius.max(1.0) * 2.0;

    for item in render_items {
        let (light_item, directional_light) = match item.as_ref() {
            RenderItem::Light(light_item) => match light_item.light.as_ref() {
                RenderLight::Directional(light) => (light_item, light),
                _ => continue,
            },
            _ => continue,
        };

        let mut light_dir = glam::vec3(
            directional_light.direction[0],
            directional_light.direction[1],
            directional_light.direction[2],
        );
        light_dir = light_item.matrix.transform_vector3(light_dir).normalize_or_zero();
        if light_dir.length_squared() < 1e-8 {
            continue;
        }

        let up = if light_dir.abs().dot(glam::vec3(0.0, 1.0, 0.0)) < 0.99 {
            glam::vec3(0.0, 1.0, 0.0)
        } else {
            glam::vec3(1.0, 0.0, 0.0)
        };
        let eye = world_center - light_dir * light_distance;
        let light_view = glam::Mat4::look_at_rh(eye, world_center, up);

        let corners = get_aabb_corners(world_min, world_max);
        let mut light_min = glam::vec3(f32::INFINITY, f32::INFINITY, f32::INFINITY);
        let mut light_max = glam::vec3(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
        for corner in corners {
            let p = light_view.transform_point3(corner);
            expand_bounds(&mut light_min, &mut light_max, p);
        }

        let margin = SHADOW_BOUNDS_MARGIN;
        let span = light_max - light_min;
        let mx = span.x.abs().max(1e-3) * margin;
        let my = span.y.abs().max(1e-3) * margin;
        let mz = span.z.abs().max(1e-3) * margin;

        let left = light_min.x - mx;
        let right = light_max.x + mx;
        let bottom = light_min.y - my;
        let top = light_max.y + my;

        let near = (-light_max.z - mz).max(0.01);
        let far = (-light_min.z + mz).max(near + 0.01);
        let light_proj = glam::Mat4::orthographic_rh(left, right, bottom, top, near, far);
        let light_view_proj = light_proj * light_view;

        let shadow_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Directional Shadow Map"),
            size: wgpu::Extent3d {
                width: SHADOW_MAP_SIZE,
                height: SHADOW_MAP_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[wgpu::TextureFormat::Depth32Float],
        });
        let shadow_view = shadow_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });

        let tex_id = Uuid::new_v4();
        let render_texture = Arc::new(RenderTexture {
            id: tex_id,
            edition: directional_light.edition.clone(),
            texture: shadow_texture,
            view: shadow_view,
            sampler: shadow_sampler,
            scale: [1.0, 1.0],
            delta: [0.0, 0.0],
        });
        render_resource_manager.add_texture(&render_texture);

        shadows.push(Arc::new(RenderDirectionalLightShadow {
            id: directional_light.id,
            edition: directional_light.edition.clone(),
            light_view,
            light_proj,
            light_view_proj,
            textures: vec![render_texture],
        }));
    }

    shadows
}
