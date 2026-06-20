use super::camera::RenderCamera;
use super::light::DirectionalRenderLight;
use super::light::RenderLight;
use super::material::RenderCategory;
use super::render_item::RenderItem;
use super::render_resource::RenderResourceManager;
use super::texture::RenderTexture;
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};

use eframe::wgpu;
use uuid::Uuid;

const SHADOW_BOUNDS_MARGIN: f32 = 0.1;
const DIRECTIONAL_SHADOW_MAP_SIZE: u32 = 2048;
pub const DIRECTIONAL_SHADOW_CASCADE_MAX_COUNT: usize = 4;
const SHADOW_XY_EPSILON: f32 = 1e-3;
const SHADOW_CASTER_Z_EPSILON: f32 = 1e-3;
const SPLIT_DEPTH_EPSILON: f32 = 1e-4;

static LAST_SHADOW_DEBUG_INFO: OnceLock<Mutex<String>> = OnceLock::new();

fn set_last_shadow_debug_info(text: String) {
    let slot = LAST_SHADOW_DEBUG_INFO.get_or_init(|| Mutex::new(String::new()));
    let file_text = text.clone();
    if let Ok(mut guard) = slot.lock() {
        *guard = text;
    }
    let _ = fs::write("/tmp/pbrt_shadow_debug.txt", file_text);
}

pub fn get_last_shadow_debug_info() -> String {
    let slot = LAST_SHADOW_DEBUG_INFO.get_or_init(|| Mutex::new(String::new()));
    slot.lock().map(|s| s.clone()).unwrap_or_default()
}

#[derive(Debug, Clone, Default)]
pub struct ShadowMaps {
    pub directional_shadow_maps: Option<DirectionalShadowMaps>,
}

#[derive(Debug, Clone)]
pub struct DirectionalShadowMaps {
    pub directional_shadow_index_map: HashMap<Uuid, i32>,
    pub directional_shadow_info_buffer: wgpu::Buffer,
    pub directional_shadow_map_texture: Arc<RenderTexture>,
}

#[derive(Debug, Clone)]
pub struct RenderDirectionalLightShadowCascade {
    pub light_view: glam::Mat4,
    pub light_proj: glam::Mat4,
    pub light_view_proj: glam::Mat4,
    pub split_origin: glam::Vec3,
    pub split_forward: glam::Vec3,
    pub split_end: f32,
    pub texture: Arc<RenderTexture>,
}

#[derive(Debug, Clone)]
pub struct RenderDirectionalLightShadow {
    pub id: Uuid,
    pub edition: String,
    pub shadow_bias: f32,
    pub shadow_slope_bias: f32,
    pub cascades: Vec<RenderDirectionalLightShadowCascade>,
}

pub fn get_shader_uses_shadow(category: RenderCategory) -> bool {
    category == RenderCategory::Opaque || category == RenderCategory::Emissive
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

fn get_full_frustum_corners_world(render_camera: &RenderCamera) -> [glam::Vec3; 8] {
    let near = render_camera.near.max(1e-5);
    let far = render_camera.far.max(near + 1e-5);
    let tan_half_hfov = (0.5 * render_camera.hfov).tan();
    let tan_half_vfov = (0.5 * render_camera.vfov).tan();

    let forward = render_camera.forward.normalize_or_zero();
    let right = render_camera.right.normalize_or_zero();
    let up = render_camera.up.normalize_or_zero();
    let position = render_camera.position;

    let near_center = position + forward * near;
    let far_center = position + forward * far;

    let near_w = near * tan_half_hfov;
    let near_h = near * tan_half_vfov;
    let far_w = far * tan_half_hfov;
    let far_h = far * tan_half_vfov;

    [
        near_center - right * near_w - up * near_h,
        near_center + right * near_w - up * near_h,
        near_center - right * near_w + up * near_h,
        near_center + right * near_w + up * near_h,
        far_center - right * far_w - up * far_h,
        far_center + right * far_w - up * far_h,
        far_center - right * far_w + up * far_h,
        far_center + right * far_w + up * far_h,
    ]
}

#[derive(Debug, Clone, Copy)]
struct SplitBasis {
    origin: glam::Vec3,
    z: glam::Vec3,
}

impl SplitBasis {
    fn project_depth(&self, world: glam::Vec3) -> f32 {
        (world - self.origin).dot(self.z)
    }
}

fn build_split_basis(render_camera: &RenderCamera, light_dir: glam::Vec3) -> Option<SplitBasis> {
    let camera_forward = render_camera.forward.normalize_or_zero();
    let camera_up = render_camera.up.normalize_or_zero();
    if camera_forward.length_squared() < 1e-8 {
        return None;
    }

    let mut right = camera_forward.cross(light_dir).normalize_or_zero();
    if right.length_squared() < 1e-8 {
        right = camera_up.cross(light_dir).normalize_or_zero();
    }
    if right.length_squared() < 1e-8 {
        right = glam::vec3(0.0, 1.0, 0.0).cross(light_dir).normalize_or_zero();
    }
    if right.length_squared() < 1e-8 {
        return None;
    }

    let mut split_forward = light_dir.cross(right).normalize_or_zero();
    if split_forward.length_squared() < 1e-8 {
        return None;
    }
    if split_forward.dot(camera_forward) < 0.0 {
        split_forward = -split_forward;
        right = -right;
    }
    let up = split_forward.cross(right).normalize_or_zero();
    if up.length_squared() < 1e-8 {
        return None;
    }

    Some(SplitBasis {
        origin: render_camera.position,
        z: split_forward,
    })
}

fn get_frustum_slice_corners_from_split(
    render_camera: &RenderCamera,
    full_frustum_corners: &[glam::Vec3; 8],
    split_basis: &SplitBasis,
    split_near: f32,
    split_far: f32,
) -> Option<[glam::Vec3; 8]> {
    let split_near = split_near.max(SPLIT_DEPTH_EPSILON);
    let split_far = split_far.max(split_near + SPLIT_DEPTH_EPSILON);
    let origin = render_camera.position;
    let mut out = [glam::Vec3::ZERO; 8];
    for i in 0..4 {
        let far_corner = full_frustum_corners[4 + i];
        let ray = far_corner - origin;
        let ray_split_depth = ray.dot(split_basis.z);
        if ray_split_depth <= SPLIT_DEPTH_EPSILON {
            return None;
        }
        let near_scale = split_near / ray_split_depth;
        let far_scale = split_far / ray_split_depth;
        out[i] = origin + ray * near_scale;
        out[4 + i] = origin + ray * far_scale;
    }
    Some(out)
}

fn build_light_matrices_from_virtual_points_orthographic(
    virtual_frustum_points: &[glam::Vec3],
    caster_points_world: &[glam::Vec3],
    light_dir: glam::Vec3,
    up: glam::Vec3,
) -> (glam::Mat4, glam::Mat4, glam::Mat4) {
    let forward = light_dir.normalize_or_zero();
    let right = up.cross(forward).normalize_or_zero();
    let up = forward.cross(right).normalize_or_zero();

    let mut receiver_min = glam::vec3(f32::INFINITY, f32::INFINITY, f32::INFINITY);
    let mut receiver_max = glam::vec3(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for p in virtual_frustum_points.iter().copied() {
        let lp = glam::vec3(p.dot(right), p.dot(up), p.dot(-forward));
        expand_bounds(&mut receiver_min, &mut receiver_max, lp);
    }

    let mut caster_min = glam::vec3(f32::INFINITY, f32::INFINITY, f32::INFINITY);
    let mut caster_max = glam::vec3(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for p in caster_points_world.iter().copied() {
        let lp = glam::vec3(p.dot(right), p.dot(up), p.dot(-forward));
        expand_bounds(&mut caster_min, &mut caster_max, lp);
    }

    let light_space_center = glam::vec3(
        0.5 * (receiver_min.x + receiver_max.x),
        0.5 * (receiver_min.y + receiver_max.y),
        0.5 * (caster_min.z + caster_max.z),
    );
    let origin =
        right * light_space_center.x + up * light_space_center.y - forward * light_space_center.z;

    let light_view = glam::Mat4::from_cols_array(&[
        right.x,
        up.x,
        -forward.x,
        0.0,
        right.y,
        up.y,
        -forward.y,
        0.0,
        right.z,
        up.z,
        -forward.z,
        0.0,
        -origin.dot(right),
        -origin.dot(up),
        origin.dot(forward),
        1.0,
    ]);

    let mut light_min = glam::vec3(f32::INFINITY, f32::INFINITY, f32::INFINITY);
    let mut light_max = glam::vec3(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for p_world in virtual_frustum_points.iter().copied() {
        let p = light_view.transform_point3(p_world);
        expand_bounds(&mut light_min, &mut light_max, p);
    }

    let margin = SHADOW_BOUNDS_MARGIN;
    let span = glam::vec3(light_max.x - light_min.x, light_max.y - light_min.y, 1.0);
    let mx = span.x.abs().max(1e-3) * margin;
    let my = span.y.abs().max(1e-3) * margin;
    let mut caster_z_min = f32::INFINITY;
    let mut caster_z_max = f32::NEG_INFINITY;

    // Z-range is caster-driven. Prefer casters overlapping receiver XY footprint.
    let caster_x_min = light_min.x - mx;
    let caster_x_max = light_max.x + mx;
    let caster_y_min = light_min.y - my;
    let caster_y_max = light_max.y + my;
    let mut has_overlap_caster = false;
    for p_world in caster_points_world {
        let p = light_view.transform_point3(*p_world);
        if p.x >= caster_x_min && p.x <= caster_x_max && p.y >= caster_y_min && p.y <= caster_y_max
        {
            caster_z_min = caster_z_min.min(p.z);
            caster_z_max = caster_z_max.max(p.z);
            has_overlap_caster = true;
        }
    }
    if !has_overlap_caster {
        for p_world in caster_points_world.iter().copied() {
            let p = light_view.transform_point3(p_world);
            caster_z_min = caster_z_min.min(p.z);
            caster_z_max = caster_z_max.max(p.z);
        }
    }
    assert!(
        caster_z_min.is_finite() && caster_z_max.is_finite(),
        "Invalid caster Z range in light-view: z_min={}, z_max={}",
        caster_z_min,
        caster_z_max
    );
    if caster_z_max <= caster_z_min {
        // Degenerate slice (all caster samples on nearly the same light-space Z).
        // Keep the algorithm deterministic by enforcing a minimal depth thickness.
        let half = (SHADOW_CASTER_Z_EPSILON * 0.5).max(5e-4);
        caster_z_min -= half;
        caster_z_max += half;
    }
    assert!(
        caster_z_max > caster_z_min,
        "Degenerate caster Z range after regularization: z_min={}, z_max={}",
        caster_z_min,
        caster_z_max
    );
    light_min.z = caster_z_min;
    light_max.z = caster_z_max;
    let mz = (light_max.z - light_min.z).abs().max(1e-3) * margin;

    // Snap projection center to shadow texel grid to reduce shimmering and
    // improve effective resolution usage per cascade.
    let width = (light_max.x - light_min.x).abs() + 2.0 * mx;
    let height = (light_max.y - light_min.y).abs() + 2.0 * my;
    assert!(
        width.is_finite() && height.is_finite() && width > SHADOW_XY_EPSILON && height > SHADOW_XY_EPSILON,
        "Invalid receiver XY bounds in light-view: width={}, height={}",
        width,
        height
    );
    let mut center_x = 0.5 * (light_min.x + light_max.x);
    let mut center_y = 0.5 * (light_min.y + light_max.y);
    let units_per_texel_x = (width / DIRECTIONAL_SHADOW_MAP_SIZE as f32).max(1e-6);
    let units_per_texel_y = (height / DIRECTIONAL_SHADOW_MAP_SIZE as f32).max(1e-6);
    center_x = (center_x / units_per_texel_x).floor() * units_per_texel_x;
    center_y = (center_y / units_per_texel_y).floor() * units_per_texel_y;

    let left = center_x - 0.5 * width;
    let right = center_x + 0.5 * width;
    let bottom = center_y - 0.5 * height;
    let top = center_y + 0.5 * height;

    let near = (-light_max.z - mz).max(0.01);
    let far = (-light_min.z + mz).max(near + 0.01);
    assert!(
        near.is_finite() && far.is_finite() && near > 0.0 && far > near,
        "Invalid light projection near/far: near={}, far={}",
        near,
        far
    );
    let debug_text = format!(
        "origin=({:.2},{:.2},{:.2})\nlight_dir=({:.2},{:.2},{:.2})\nxy=[{:.2},{:.2}]x[{:.2},{:.2}] size=({:.2},{:.2})\ncenter_xy=({:.2},{:.2})\ncaster_z=[{:.4},{:.4}] mz={:.4}\nnear/far=[{:.4},{:.4}] overlap={}",
        origin.x,
        origin.y,
        origin.z,
        light_dir.x,
        light_dir.y,
        light_dir.z,
        light_min.x,
        light_max.x,
        light_min.y,
        light_max.y,
        width,
        height,
        center_x,
        center_y,
        caster_z_min,
        caster_z_max,
        mz,
        near,
        far,
        has_overlap_caster
    );
    set_last_shadow_debug_info(debug_text);
    let light_proj = glam::Mat4::orthographic_rh(left, right, bottom, top, near, far);
    let light_view_proj = light_proj * light_view;
    (light_view, light_proj, light_view_proj)
}

struct ShadowSceneContext {
    full_frustum_corners: [glam::Vec3; 8],
    receiver_points_world: Vec<glam::Vec3>,
    caster_points_world: Vec<glam::Vec3>,
}

fn build_shadow_scene_context(
    render_camera: &RenderCamera,
    mesh_items: &[Arc<RenderItem>],
) -> Option<ShadowSceneContext> {
    let mut caster_points_world = Vec::new();
    let mut receiver_points_world = Vec::new();
    let mut has_mesh = false;

    for item in mesh_items {
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
                caster_points_world.push(world);
                receiver_points_world.push(world);
            }
        }
    }
    if !has_mesh {
        return None;
    }

    let full_frustum_corners = get_full_frustum_corners_world(render_camera);

    Some(ShadowSceneContext {
        full_frustum_corners,
        receiver_points_world,
        caster_points_world,
    })
}

fn create_directional_light_shadow(
    device: &wgpu::Device,
    render_camera: &RenderCamera,
    render_resource_manager: &mut RenderResourceManager,
    light_matrix: glam::Mat4,
    directional_light: &DirectionalRenderLight,
    ctx: &ShadowSceneContext,
) -> Option<Arc<RenderDirectionalLightShadow>> {
    assert!(
        directional_light.cast_shadow,
        "create_directional_light_shadow called for non-shadow-casting light"
    );

    let mut light_dir = glam::vec3(
        directional_light.direction[0],
        directional_light.direction[1],
        directional_light.direction[2],
    );
    light_dir = light_matrix.transform_vector3(light_dir).normalize_or_zero();
    if light_dir.length_squared() < 1e-8 {
        return None;
    }
    let camera_up = render_camera.up.normalize_or_zero();
    let up = if camera_up.length_squared() > 1e-8 && light_dir.abs().dot(camera_up) < 0.99 {
        camera_up
    } else if light_dir.abs().dot(glam::vec3(0.0, 1.0, 0.0)) < 0.99 {
        glam::vec3(0.0, 1.0, 0.0)
    } else {
        glam::vec3(1.0, 0.0, 0.0)
    };
    let split_basis = build_split_basis(render_camera, light_dir)?;
    let virtual_frustum_points = if ctx.receiver_points_world.is_empty() {
        ctx.full_frustum_corners.to_vec()
    } else {
        ctx.receiver_points_world.clone()
    };
    let (light_view, light_proj, light_view_proj) =
        build_light_matrices_from_virtual_points_orthographic(
            &virtual_frustum_points,
            &ctx.caster_points_world,
            light_dir,
            up,
        );

    let mut split_end = f32::NEG_INFINITY;
    for corner in ctx.full_frustum_corners {
        let split_depth = split_basis.project_depth(corner);
        if split_depth.is_finite() && split_depth > SPLIT_DEPTH_EPSILON {
            split_end = split_end.max(split_depth);
        }
    }
    if !split_end.is_finite() {
        return None;
    }

    let render_texture =
        get_directional_shadow_texture(device, render_resource_manager, directional_light, 0);
    let mut debug_text = get_last_shadow_debug_info();
    if !debug_text.is_empty() {
        debug_text.push('\n');
    }
    debug_text.push_str(&format!(
        "split_end={:.4} bias={:.5} slope_bias={:.5}",
        split_end, directional_light.shadow_bias, directional_light.shadow_slope_bias
    ));
    set_last_shadow_debug_info(debug_text);
    let cascades = vec![RenderDirectionalLightShadowCascade {
        light_view,
        light_proj,
        light_view_proj,
        split_origin: split_basis.origin,
        split_forward: split_basis.z,
        split_end,
        texture: render_texture,
    }];

    Some(Arc::new(RenderDirectionalLightShadow {
        id: directional_light.id,
        edition: directional_light.edition.clone(),
        shadow_bias: directional_light.shadow_bias,
        shadow_slope_bias: directional_light.shadow_slope_bias,
        cascades,
    }))
}

fn get_directional_shadow_texture(
    device: &wgpu::Device,
    render_resource_manager: &mut RenderResourceManager,
    directional_light: &DirectionalRenderLight,
    cascade_index: usize,
) -> Arc<RenderTexture> {
    let tex_id = Uuid::new_v3(
        &Uuid::NAMESPACE_OID,
        format!(
            "directional-shadow:{}:cascade:{}",
            directional_light.id, cascade_index
        )
        .as_bytes(),
    );
    if let Some(tex) = render_resource_manager.get_texture(tex_id) {
        if tex.edition == directional_light.edition {
            return tex.clone();
        }
    }

    let shadow_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Directional Shadow Map"),
        size: wgpu::Extent3d {
            width: DIRECTIONAL_SHADOW_MAP_SIZE,
            height: DIRECTIONAL_SHADOW_MAP_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
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
    let tex = Arc::new(RenderTexture {
        id: tex_id,
        edition: directional_light.edition.clone(),
        texture: shadow_texture,
        view: shadow_view,
        sampler: shadow_sampler,
        scale: [1.0, 1.0],
        delta: [0.0, 0.0],
    });
    render_resource_manager.add_texture(&tex);
    tex
}

pub fn create_directional_light_shadows(
    device: &wgpu::Device,
    _queue: &wgpu::Queue,
    render_camera: &RenderCamera,
    mesh_items: &[Arc<RenderItem>],
    light_items: &[Arc<RenderItem>],
    render_resource_manager: &mut RenderResourceManager,
) -> Option<Vec<Arc<RenderDirectionalLightShadow>>> {
    let target_light_items = light_items
        .iter()
        .filter_map(|item| match item.as_ref() {
            RenderItem::Light(light_item) => match light_item.light.as_ref() {
                RenderLight::Directional(light) if light.cast_shadow => Some(item.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    //
    if target_light_items.is_empty() {
        log::info!("CSM: no directional lights with cast_shadow");
        return None;
    }

    let Some(build_ctx) = build_shadow_scene_context(render_camera, mesh_items) else {
        log::info!("CSM: build_shadow_scene_context returned None");
        return None;
    };
    let mut shadows = Vec::new();

    for item in target_light_items {
        if let RenderItem::Light(light_item) = item.as_ref() {
            if let RenderLight::Directional(directional_light) = light_item.light.as_ref() {
                if let Some(shadow) = create_directional_light_shadow(
                    device,
                    render_camera,
                    render_resource_manager,
                    light_item.matrix,
                    directional_light,
                    &build_ctx,
                ) {
                    shadows.push(shadow);
                }
            }
        }
    }
    if shadows.is_empty() {
        log::info!("CSM: no directional shadow built");
        return None;
    } else {
        log::info!("CSM: built {} directional shadow(s)", shadows.len());
        return Some(shadows);
    }
}
