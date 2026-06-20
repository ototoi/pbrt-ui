use super::camera::RenderCamera;
use super::light::DirectionalRenderLight;
use super::light::DirectionalShadowProjection;
use super::light::RenderLight;
use super::material::RenderCategory;
use super::render_item::RenderItem;
use super::render_resource::RenderResourceManager;
use super::texture::RenderTexture;
use std::collections::HashMap;
use std::sync::Arc;

use eframe::wgpu;
use uuid::Uuid;

const SHADOW_BOUNDS_MARGIN: f32 = 0.1;
const DIRECTIONAL_SHADOW_MAP_SIZE: u32 = 2048;
pub const DIRECTIONAL_SHADOW_CASCADE_MAX_COUNT: usize = 4;
const SHADOW_XY_EPSILON: f32 = 1e-3;
const SHADOW_CASTER_Z_EPSILON: f32 = 1e-3;
const SPLIT_DEPTH_EPSILON: f32 = 1e-4;
const CSM_MIN_SPLIT_SPAN: f32 = 50.0;
// Blend factor between uniform and logarithmic CSM split distributions.
// split = lerp(uniform_split, log_split, CASCADE_SPLIT_LAMBDA)
// 0.0 -> fully uniform, 1.0 -> fully logarithmic.
// Higher values allocate more resolution to near-camera cascades.
const CASCADE_SPLIT_LAMBDA: f32 = 0.9;

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

fn build_cascade_splits(near: f32, far: f32, cascade_count: usize) -> Vec<f32> {
    let mut splits = vec![far; cascade_count];
    for i in 1..=cascade_count {
        let t = i as f32 / cascade_count as f32;
        let log = near * (far / near).powf(t);
        let uni = near + (far - near) * t;
        splits[i - 1] = uni * (1.0 - CASCADE_SPLIT_LAMBDA) + log * CASCADE_SPLIT_LAMBDA;
    }
    splits
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
    let mut world_min_c = glam::vec3(f32::INFINITY, f32::INFINITY, f32::INFINITY);
    let mut world_max_c = glam::vec3(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for p in virtual_frustum_points.iter().copied() {
        expand_bounds(&mut world_min_c, &mut world_max_c, p);
    }
    let world_center = 0.5 * (world_min_c + world_max_c);
    let world_extent = world_max_c - world_min_c;
    let world_radius = world_extent.length() * 0.5;
    let light_distance = world_radius.max(1.0) * 2.0;
    let eye = world_center - light_dir * light_distance;
    let light_view = glam::Mat4::look_at_rh(eye, world_center, up);

    let mut light_min = glam::vec3(f32::INFINITY, f32::INFINITY, 0.0);
    let mut light_max = glam::vec3(f32::NEG_INFINITY, f32::NEG_INFINITY, 0.0);
    // Build XY bounds from receiver footprint (virtual frustum slice).
    // Z is derived from caster points only.
    for p_world in virtual_frustum_points.iter().copied() {
        let p = light_view.transform_point3(p_world);
        light_min.x = light_min.x.min(p.x);
        light_min.y = light_min.y.min(p.y);
        light_max.x = light_max.x.max(p.x);
        light_max.y = light_max.y.max(p.y);
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
    match directional_light.shadow_projection {
        DirectionalShadowProjection::Lspsm => create_directional_light_shadow_lspsm(
            device,
            render_camera,
            render_resource_manager,
            light_matrix,
            directional_light,
            ctx,
        ),
        DirectionalShadowProjection::Csm => create_directional_light_shadow_csm(
            device,
            render_camera,
            render_resource_manager,
            light_matrix,
            directional_light,
            ctx,
        ),
    }
}

fn build_directional_light_basis(
    render_camera: &RenderCamera,
    light_matrix: glam::Mat4,
    directional_light: &DirectionalRenderLight,
) -> Option<(glam::Vec3, glam::Vec3)> {
    let mut light_dir = glam::vec3(
        directional_light.direction[0],
        directional_light.direction[1],
        directional_light.direction[2],
    );
    light_dir = light_matrix
        .transform_vector3(light_dir)
        .normalize_or_zero();
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
    Some((light_dir, up))
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

fn create_directional_light_shadow_csm(
    device: &wgpu::Device,
    render_camera: &RenderCamera,
    render_resource_manager: &mut RenderResourceManager,
    light_matrix: glam::Mat4,
    directional_light: &DirectionalRenderLight,
    ctx: &ShadowSceneContext,
) -> Option<Arc<RenderDirectionalLightShadow>> {
    assert!(
        directional_light.cast_shadow,
        "create_directional_light_shadow_csm called for non-shadow-casting light"
    );

    let cascade_count = directional_light
        .cascade_count
        .clamp(1, DIRECTIONAL_SHADOW_CASCADE_MAX_COUNT as u32) as usize;
    let (light_dir, up) =
        build_directional_light_basis(render_camera, light_matrix, directional_light)?;
    let split_basis = build_split_basis(render_camera, light_dir)?;

    let mut split_min = f32::INFINITY;
    let mut split_max = f32::NEG_INFINITY;
    for world in &ctx.receiver_points_world {
        let split_depth = split_basis.project_depth(*world);
        if split_depth.is_finite() && split_depth > SPLIT_DEPTH_EPSILON {
            split_min = split_min.min(split_depth);
            split_max = split_max.max(split_depth);
        }
    }
    if !split_min.is_finite() || !split_max.is_finite() || split_max <= split_min {
        for corner in ctx.full_frustum_corners {
            let split_depth = split_basis.project_depth(corner);
            if split_depth.is_finite() && split_depth > SPLIT_DEPTH_EPSILON {
                split_min = split_min.min(split_depth);
                split_max = split_max.max(split_depth);
            }
        }
    }
    if !split_min.is_finite() || !split_max.is_finite() || split_max <= split_min {
        return None;
    }
    split_min = split_min.max(SPLIT_DEPTH_EPSILON);
    split_max = split_max.max(split_min + CSM_MIN_SPLIT_SPAN);
    log::info!(
        "CSM light={} cascades={} split_range=[{:.4}, {:.4}]",
        directional_light.id,
        cascade_count,
        split_min,
        split_max,
    );
    let split_splits = build_cascade_splits(split_min, split_max, cascade_count);
    assert_eq!(
        split_splits.len(),
        cascade_count,
        "Unexpected split count: expected={}, actual={}",
        cascade_count,
        split_splits.len()
    );
    let mut cascades = Vec::with_capacity(cascade_count);
    let mut cascade_near = split_min;
    for (cascade_index, cascade_far) in split_splits.iter().copied().enumerate() {
        assert!(
            cascade_far > cascade_near,
            "Invalid cascade interval: index={}, near={}, far={}",
            cascade_index,
            cascade_near,
            cascade_far
        );
        log::info!(
            "CSM light={} cascade={} near={:.4} far={:.4} split_end={:.4}",
            directional_light.id,
            cascade_index,
            cascade_near,
            cascade_far,
            split_splits[cascade_index]
        );
        let virtual_points = get_frustum_slice_corners_from_split(
            render_camera,
            &ctx.full_frustum_corners,
            &split_basis,
            cascade_near,
            cascade_far,
        )?;
        for p in virtual_points.iter() {
            assert!(p.is_finite(), "Non-finite frustum slice point: {:?}", p);
        }
        let virtual_frustum_points = virtual_points.to_vec();
        let (light_view, light_proj, light_view_proj) =
            build_light_matrices_from_virtual_points_orthographic(
                &virtual_frustum_points,
                &ctx.caster_points_world,
                light_dir,
                up,
            );
        let render_texture = get_directional_shadow_texture(
            device,
            render_resource_manager,
            directional_light,
            cascade_index,
        );

        cascades.push(RenderDirectionalLightShadowCascade {
            light_view,
            light_proj,
            light_view_proj,
            split_origin: split_basis.origin,
            split_forward: split_basis.z,
            split_end: split_splits[cascade_index],
            texture: render_texture,
        });
        cascade_near = cascade_far;
    }

    Some(Arc::new(RenderDirectionalLightShadow {
        id: directional_light.id,
        edition: directional_light.edition.clone(),
        shadow_bias: directional_light.shadow_bias,
        shadow_slope_bias: directional_light.shadow_slope_bias,
        cascades,
    }))
}

fn create_directional_light_shadow_lspsm(
    _device: &wgpu::Device,
    _render_camera: &RenderCamera,
    _render_resource_manager: &mut RenderResourceManager,
    _light_matrix: glam::Mat4,
    _directional_light: &DirectionalRenderLight,
    _ctx: &ShadowSceneContext,
) -> Option<Arc<RenderDirectionalLightShadow>> {
    todo!("create_directional_light_shadow_lspsm");
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
