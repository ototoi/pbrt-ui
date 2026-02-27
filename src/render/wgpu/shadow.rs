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
const LSPSM_PARALLEL_THRESHOLD_COS: f32 = 0.985;
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

fn get_camera_near_far(render_camera: &RenderCamera) -> Option<(f32, f32)> {
    let clip_to_camera = render_camera.camera_to_clip.inverse();
    let p_near = clip_to_camera * glam::vec4(0.0, 0.0, 0.0, 1.0);
    let p_far = clip_to_camera * glam::vec4(0.0, 0.0, 1.0, 1.0);
    if p_near.w.abs() < 1e-8 || p_far.w.abs() < 1e-8 {
        return None;
    }
    let p_near = p_near / p_near.w;
    let p_far = p_far / p_far.w;
    let near = -p_near.z;
    let far = -p_far.z;
    if near <= 1e-5 || far <= near {
        return None;
    }
    Some((near, far))
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
    let clip_to_world = render_camera.camera_to_world * render_camera.camera_to_clip.inverse();
    let clip_corners = [
        glam::vec4(-1.0, -1.0, 0.0, 1.0),
        glam::vec4(1.0, -1.0, 0.0, 1.0),
        glam::vec4(-1.0, 1.0, 0.0, 1.0),
        glam::vec4(1.0, 1.0, 0.0, 1.0),
        glam::vec4(-1.0, -1.0, 1.0, 1.0),
        glam::vec4(1.0, -1.0, 1.0, 1.0),
        glam::vec4(-1.0, 1.0, 1.0, 1.0),
        glam::vec4(1.0, 1.0, 1.0, 1.0),
    ];
    let mut out = [glam::Vec3::ZERO; 8];
    for (i, c) in clip_corners.iter().enumerate() {
        let w = clip_to_world * *c;
        if w.w.abs() < 1e-8 {
            out[i] = w.truncate();
        } else {
            out[i] = w.truncate() / w.w;
        }
    }
    out
}

fn get_frustum_slice_corners_world(
    full_corners: &[glam::Vec3; 8],
    camera_pos: glam::Vec3,
    camera_near: f32,
    near_depth: f32,
    far_depth: f32,
) -> [glam::Vec3; 8] {
    let near_ratio = (near_depth / camera_near).max(0.0);
    let far_ratio = (far_depth / camera_near).max(near_ratio);
    let mut out = [glam::Vec3::ZERO; 8];
    for i in 0..4 {
        let n = full_corners[i];
        out[i] = camera_pos + (n - camera_pos) * near_ratio;
    }
    for i in 0..4 {
        let n = full_corners[i];
        out[4 + i] = camera_pos + (n - camera_pos) * far_ratio;
    }
    out
}

fn get_frustum_slice_corners_from_full_frustum(
    full_frustum_corners: &[glam::Vec3; 8],
    near_t: f32,
    far_t: f32,
) -> [glam::Vec3; 8] {
    let near_t = near_t.clamp(0.0, 1.0);
    let far_t = far_t.clamp(near_t, 1.0);
    let mut out = [glam::Vec3::ZERO; 8];
    for i in 0..4 {
        let n = full_frustum_corners[i];
        let f = full_frustum_corners[4 + i];
        out[i] = n + (f - n) * near_t;
        out[4 + i] = n + (f - n) * far_t;
    }
    out
}

fn build_virtual_frustum_points_lspsm(
    receiver_points: &[glam::Vec3; 8],
    render_camera: &RenderCamera,
    light_dir: glam::Vec3,
    tan_half_fov_x: f32,
) -> Option<[glam::Vec3; 8]> {
    let camera_forward = render_camera.forward.normalize_or_zero();
    // LSPSM virtual frustum becomes ill-conditioned when camera and light are near-parallel.
    // In that case we intentionally fail so caller can fall back to CSM (orthographic).
    if camera_forward.length_squared() < 1e-8
        || camera_forward.abs().dot(light_dir) >= LSPSM_PARALLEL_THRESHOLD_COS
    {
        return None;
    }
    let forward = (camera_forward - light_dir * camera_forward.dot(light_dir)).normalize_or_zero();
    if forward.length_squared() < 1e-8 {
        return None;
    }
    let up = light_dir.cross(forward).normalize_or_zero();
    if up.length_squared() < 1e-8 {
        return None;
    }
    let right = forward.cross(up).normalize_or_zero();
    if right.length_squared() < 1e-8 {
        return None;
    }

    let mut center = glam::Vec3::ZERO;
    for p in receiver_points {
        center += *p;
    }
    center /= receiver_points.len() as f32;

    let eps = 1e-3_f32;
    let mut d_required = eps;
    let mut min_z = f32::INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    for p in receiver_points {
        let rel = *p - center;
        let x = rel.dot(right);
        let z = rel.dot(forward);
        min_z = min_z.min(z);
        max_z = max_z.max(z);
        d_required = d_required.max(x.abs() / tan_half_fov_x - z);
        d_required = d_required.max(eps - z);
    }
    if !d_required.is_finite() || !min_z.is_finite() || !max_z.is_finite() {
        return None;
    }

    let near = (min_z + d_required).max(eps);
    let far = (max_z + d_required).max(near + eps);
    let origin = center - forward * d_required;

    // Vertical FOV is not constrained to camera FOV.
    // Derive the minimum needed to include all receivers.
    let mut tan_half_fov_y = 1e-3_f32;
    for p in receiver_points {
        let rel = *p - center;
        let y = rel.dot(up);
        let z = rel.dot(forward);
        let denom = (z + d_required).max(eps);
        tan_half_fov_y = tan_half_fov_y.max(y.abs() / denom);
    }

    let near_c = origin + forward * near;
    let far_c = origin + forward * far;
    let near_h = near * tan_half_fov_y;
    let near_w = near * tan_half_fov_x;
    let far_h = far * tan_half_fov_y;
    let far_w = far * tan_half_fov_x;

    let out = [
        near_c - right * near_w - up * near_h,
        near_c + right * near_w - up * near_h,
        near_c - right * near_w + up * near_h,
        near_c + right * near_w + up * near_h,
        far_c - right * far_w - up * far_h,
        far_c + right * far_w - up * far_h,
        far_c - right * far_w + up * far_h,
        far_c + right * far_w + up * far_h,
    ];
    Some(out)
}

fn build_light_matrices_from_virtual_points_orthographic(
    virtual_frustum_points: &[glam::Vec3],
    caster_points_world: &[glam::Vec3],
    full_scene_corners: &[glam::Vec3; 8],
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

    let mut light_min = glam::vec3(f32::INFINITY, f32::INFINITY, f32::INFINITY);
    let mut light_max = glam::vec3(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    // Build XY bounds from receiver footprint (virtual frustum slice).
    // Depth (Z) is extended below with caster points.
    for p_world in virtual_frustum_points.iter().copied() {
        let p = light_view.transform_point3(p_world);
        expand_bounds(&mut light_min, &mut light_max, p);
    }

    let margin = SHADOW_BOUNDS_MARGIN;
    let span = light_max - light_min;
    let mx = span.x.abs().max(1e-3) * margin;
    let my = span.y.abs().max(1e-3) * margin;
    let mz = span.z.abs().max(1e-3) * margin;

    // Extend depth only with casters that overlap this cascade footprint in light space.
    // This uses both camera slice (receiver bounds XY) and light-space caster positions.
    let caster_x_min = light_min.x - mx;
    let caster_x_max = light_max.x + mx;
    let caster_y_min = light_min.y - my;
    let caster_y_max = light_max.y + my;
    let mut has_overlap_caster = false;
    for p_world in caster_points_world {
        let p = light_view.transform_point3(*p_world);
        if p.x >= caster_x_min && p.x <= caster_x_max && p.y >= caster_y_min && p.y <= caster_y_max
        {
            light_min.z = light_min.z.min(p.z);
            light_max.z = light_max.z.max(p.z);
            has_overlap_caster = true;
        }
    }
    // Fallback to full-scene casters when no overlap is found to avoid missing shadows.
    if !has_overlap_caster {
        for corner in full_scene_corners.iter().copied() {
            let p = light_view.transform_point3(corner);
            light_min.z = light_min.z.min(p.z);
            light_max.z = light_max.z.max(p.z);
        }
    }

    // Snap projection center to shadow texel grid to reduce shimmering and
    // improve effective resolution usage per cascade.
    let width = (light_max.x - light_min.x).abs() + 2.0 * mx;
    let height = (light_max.y - light_min.y).abs() + 2.0 * my;
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
    let light_proj = glam::Mat4::orthographic_rh(left, right, bottom, top, near, far);
    let light_view_proj = light_proj * light_view;
    (light_view, light_proj, light_view_proj)
}

fn build_light_matrices_from_virtual_points_lspsm(
    virtual_frustum_points: &[glam::Vec3],
    caster_points_world: &[glam::Vec3],
    light_dir: glam::Vec3,
    up: glam::Vec3,
) -> Option<(glam::Mat4, glam::Mat4, glam::Mat4)> {
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

    // Receiver-driven angular footprint (XY) and depth span.
    let mut tan_half_x = 0.0_f32;
    let mut tan_half_y = 0.0_f32;
    let mut near_depth = f32::INFINITY;
    let mut far_depth = 0.0_f32;

    for p_world in virtual_frustum_points.iter().copied() {
        let p = light_view.transform_point3(p_world);
        let depth = -p.z;
        if depth <= 1e-5 {
            continue;
        }
        near_depth = near_depth.min(depth);
        far_depth = far_depth.max(depth);
        tan_half_x = tan_half_x.max(p.x.abs() / depth);
        tan_half_y = tan_half_y.max(p.y.abs() / depth);
    }

    // Extend depth with caster geometry that falls inside receiver angular footprint.
    // This keeps XY from receiver while expanding Z for casters.
    let overlap_pad = 1.0 + SHADOW_BOUNDS_MARGIN * 2.0;
    let angular_x = tan_half_x * overlap_pad;
    let angular_y = tan_half_y * overlap_pad;
    let mut has_overlap_caster = false;
    let mut far_caster_depth = far_depth;
    for p_world in caster_points_world.iter().copied() {
        let p = light_view.transform_point3(p_world);
        let depth = -p.z;
        if depth <= 1e-5 {
            continue;
        }
        let ax = p.x.abs() / depth;
        let ay = p.y.abs() / depth;
        if ax <= angular_x && ay <= angular_y {
            far_caster_depth = far_caster_depth.max(depth);
            has_overlap_caster = true;
        }
    }
    if has_overlap_caster {
        far_depth = far_caster_depth;
    } else {
        // Conservative fallback when overlap test misses.
        for p_world in caster_points_world.iter().copied() {
            let p = light_view.transform_point3(p_world);
            let depth = -p.z;
            if depth > 1e-5 {
                far_depth = far_depth.max(depth);
            }
        }
    }

    if !near_depth.is_finite() || far_depth <= near_depth {
        return None;
    }
    if tan_half_x <= 1e-6 || tan_half_y <= 1e-6 {
        return None;
    }

    let margin_scale = 1.0 + SHADOW_BOUNDS_MARGIN;
    tan_half_x *= margin_scale;
    tan_half_y *= margin_scale;

    let mut near = (near_depth * (1.0 - SHADOW_BOUNDS_MARGIN)).max(0.05);
    let mut far = (far_depth * (1.0 + SHADOW_BOUNDS_MARGIN)).max(near + 0.05);
    if far <= near {
        far = near + 0.05;
    }
    // Clamp far/near to avoid unstable precision in warped perspective.
    let max_depth_ratio = 800.0_f32;
    if far / near > max_depth_ratio {
        near = (far / max_depth_ratio).max(0.05);
    }
    near = near.min(far - 0.05).max(0.05);

    let fov_y = 2.0 * tan_half_y.atan();
    let aspect = (tan_half_x / tan_half_y).max(0.1);
    let mut light_proj = glam::Mat4::perspective_rh(fov_y, aspect, near, far);
    let base_light_view_proj = light_proj * light_view;

    // Post-warp crop in NDC: fit receiver footprint tightly to improve map usage.
    let mut ndc_min = glam::vec2(f32::INFINITY, f32::INFINITY);
    let mut ndc_max = glam::vec2(f32::NEG_INFINITY, f32::NEG_INFINITY);
    for p_world in virtual_frustum_points.iter().copied() {
        let clip = base_light_view_proj * p_world.extend(1.0);
        if clip.w.abs() < 1e-6 {
            continue;
        }
        let ndc = clip.truncate() / clip.w;
        ndc_min = ndc_min.min(ndc.truncate());
        ndc_max = ndc_max.max(ndc.truncate());
    }

    let mut light_view_proj = base_light_view_proj;
    if ndc_min.x.is_finite()
        && ndc_min.y.is_finite()
        && ndc_max.x > ndc_min.x + 1e-4
        && ndc_max.y > ndc_min.y + 1e-4
    {
        let crop_margin = 0.02_f32;
        let min_x = (ndc_min.x - crop_margin).clamp(-1.5, 1.5);
        let max_x = (ndc_max.x + crop_margin).clamp(-1.5, 1.5);
        let min_y = (ndc_min.y - crop_margin).clamp(-1.5, 1.5);
        let max_y = (ndc_max.y + crop_margin).clamp(-1.5, 1.5);
        let span_x = (max_x - min_x).max(1e-4);
        let span_y = (max_y - min_y).max(1e-4);
        let sx = 2.0 / span_x;
        let sy = 2.0 / span_y;
        let tx = -(max_x + min_x) / span_x;
        let ty = -(max_y + min_y) / span_y;
        let crop = glam::Mat4::from_cols(
            glam::vec4(sx, 0.0, 0.0, 0.0),
            glam::vec4(0.0, sy, 0.0, 0.0),
            glam::vec4(0.0, 0.0, 1.0, 0.0),
            glam::vec4(tx, ty, 0.0, 1.0),
        );
        light_proj = crop * light_proj;
        light_view_proj = light_proj * light_view;
    }
    Some((light_view, light_proj, light_view_proj))
}

struct DirectionalShadowBuildContext {
    split_near: f32,
    split_far: f32,
    full_scene_corners: [glam::Vec3; 8],
    full_frustum_corners: [glam::Vec3; 8],
    caster_points_world: Vec<glam::Vec3>,
}

fn build_directional_shadow_build_context(
    render_camera: &RenderCamera,
    mesh_items: &[Arc<RenderItem>],
) -> Option<DirectionalShadowBuildContext> {
    let mut world_min = glam::vec3(f32::INFINITY, f32::INFINITY, f32::INFINITY);
    let mut world_max = glam::vec3(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    let mut caster_points_world = Vec::new();
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
                expand_bounds(&mut world_min, &mut world_max, world);
                caster_points_world.push(world);
            }
        }
    }
    if !has_mesh {
        return None;
    }

    let (camera_near, camera_far) = {
        let world_extent = world_max - world_min;
        let world_radius = world_extent.length() * 0.5;
        let near = 0.1_f32.max(world_radius * 0.01);
        let far = (world_radius * 4.0).max(near + 1.0);
        (near, far)
    };
    let split_near = camera_near;
    let split_far = camera_far;
    let full_scene_corners = get_aabb_corners(world_min, world_max);
    let full_frustum_corners = get_full_frustum_corners_world(render_camera);

    Some(DirectionalShadowBuildContext {
        split_near,
        split_far,
        full_scene_corners,
        full_frustum_corners,
        caster_points_world,
    })
}

fn create_directional_light_shadow(
    device: &wgpu::Device,
    render_camera: &RenderCamera,
    render_resource_manager: &mut RenderResourceManager,
    light_matrix: glam::Mat4,
    directional_light: &DirectionalRenderLight,
    ctx: &DirectionalShadowBuildContext,
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
    ctx: &DirectionalShadowBuildContext,
) -> Option<Arc<RenderDirectionalLightShadow>> {
    assert!(
        directional_light.cast_shadow,
        "create_directional_light_shadow_csm called for non-shadow-casting light"
    );

    let cascade_count = directional_light
        .cascade_count
        .clamp(1, DIRECTIONAL_SHADOW_CASCADE_MAX_COUNT as u32) as usize;
    let cascade_splits = build_cascade_splits(ctx.split_near, ctx.split_far, cascade_count);
    let (light_dir, up) =
        build_directional_light_basis(render_camera, light_matrix, directional_light)?;

    let mut cascades = Vec::with_capacity(cascade_count);
    let mut cascade_near = ctx.split_near;
    for (cascade_index, cascade_far) in cascade_splits.iter().copied().enumerate() {
        let virtual_points = get_frustum_slice_corners_world(
            &ctx.full_frustum_corners,
            render_camera.position,
            ctx.split_near,
            cascade_near,
            cascade_far,
        );
        let virtual_frustum_points = virtual_points.to_vec();
        let (light_view, light_proj, light_view_proj) = build_light_matrices_from_virtual_points_orthographic(
            &virtual_frustum_points,
            &ctx.caster_points_world,
            &ctx.full_scene_corners,
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
            split_end: cascade_far,
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
    _ctx: &DirectionalShadowBuildContext,
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
        return None;
    }

    let build_ctx = build_directional_shadow_build_context(render_camera, mesh_items)?;
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
        return None;
    } else {
        return Some(shadows);
    }
}
