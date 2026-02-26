use super::camera::RenderCamera;
use super::material::RenderCategory;
use super::material::RenderPass;
use super::mesh::RenderVertex;
use super::render_item::RenderItem;
use super::shader::RenderShader;
use super::shadow::DIRECTIONAL_SHADOW_CASCADE_MAX_COUNT;
use super::shadow::RenderDirectionalLightShadow;
use super::texture::RenderTexture;
use crate::render::wgpu::light::RenderLight;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

//use eframe::egui;
//use eframe::egui_wgpu;
use eframe::wgpu;
use eframe::wgpu::util::DeviceExt;
use uuid::Uuid;
use wgpu::util::align_to;

use bytemuck::{Pod, Zeroable};

mod shadow_map_renderer;

const MIN_LOCAL_BUFFER_NUM: usize = 64;
const MAX_DIRECTIONAL_LIGHT_NUM: usize = 4; // Maximum number of directional lights
const MAX_DIRECTIONAL_SHADOW_NUM: usize =
    MAX_DIRECTIONAL_LIGHT_NUM * DIRECTIONAL_SHADOW_CASCADE_MAX_COUNT;
const MAX_SPHERE_LIGHT_NUM: usize = 256; // Maximum number of point lights
const MAX_DISK_LIGHT_NUM: usize = 32; // Maximum number of spot lights
const MAX_RECT_LIGHT_NUM: usize = 32; // Maximum number of rectangle lights
const MAX_INFINITE_LIGHT_NUM: usize = 1; // Maximum number of infinite lights
const MAX_LIGHT_TEXTURE_NUM: usize = 1; // Maximum number of light textures

const DEFAULT_LIGHT_TEXTURE_ID: Uuid = Uuid::from_u128(0xb7814152_c24b_4af1_89a8_40a5fa168488);
const DIRECTIONAL_SHADOW_PIPELINE_ID: Uuid =
    Uuid::from_u128(0x77dd5bcc_89c5_4eff_8adf_9b5f8667d9f1);
const Z_PREPASS_PIPELINE_ID: Uuid = Uuid::from_u128(0x9979b259_39f8_4ce5_8ea5_d8078618620f);

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod, Zeroable)]
struct GlobalUniforms {
    world_to_camera: [[f32; 4]; 4], // 4 * 4 * 4 = 64
    camera_to_clip: [[f32; 4]; 4],  // 4 * 4 * 4 = 64
    camera_to_world: [[f32; 4]; 4], // 4 * 4 * 4 = 64
    camera_position: [f32; 4],      // Camera position in world space // 4 * 4 = 16
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod, Zeroable)]
struct LocalUniforms {
    local_to_world: [[f32; 4]; 4], // 4 * 4 * 4 = 64
    world_to_local: [[f32; 4]; 4], // 4 * 4 * 4 = 64
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod, Zeroable)]
struct LightUniforms {
    num_directional_lights: u32, // Number of directional lights
    num_sphere_lights: u32,      // Number of point lights
    num_disk_lights: u32,        // Number of spot lights
    num_rect_lights: u32,        // Number of rectangle lights
    num_infinite_lights: u32,    // Number of infinite lights
    _pad1: u32,
    _pad2: u32,
    _pad3: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod, Zeroable)]
struct DirectionalLight {
    direction: [f32; 4], // Direction of the light // 4 * 4 = 16
    intensity: [f32; 4], // Intensity of the light // 4 * 4 = 16
    radius: f32,         // Radius of the light // 1 * 4 = 4
    shadow_index: i32,   // Index for the shadow map -1 if no shadow // 1 * 4 = 4
    cascade_count: u32,  // CSM cascade count (1..4)
    _pad1: u32,          // Padding
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod, Zeroable)]
struct SphereLight {
    position: [f32; 4],  // Position of the light // 4 * 4 = 16
    intensity: [f32; 4], // Intensity of the light // 4 * 4 = 16
    radius: f32,         // Radius of the light // 1 * 4 = 4
    range: f32,          // Range of the light // 1 * 4 = 4
    _pad1: [f32; 2],     // Range of the light // 4 * 4 = 8
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod, Zeroable)]
struct DiskLight {
    position: [f32; 4],   // Position of the light // 4 * 4 = 16
    direction: [f32; 4],  // Direction of the light // 4 * 4 = 16
    intensity: [f32; 4],  // Intensity of the light // 4 * 4 = 16
    radius: f32,          // Radius of the light // 1 * 4 = 4
    range: f32,           // Range of the light // 1 * 4 = 4
    cos_inner_angle: f32, // Angle of the spotlight
    cos_outer_angle: f32, // Angle of the spotlight
    u_axis: [f32; 4],     // U axis for rectangle // 4 * 4 = 16
    v_axis: [f32; 4],     // V axis for rectangle // 4 * 4 = 16
    twosided: u32,        // Whether the rectangle emits light on both sides
    _pad1: u32,
    _pad2: u32,
    _pad3: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod, Zeroable)]
struct RectLight {
    position: [f32; 4],  // Position of the light // 4 * 4 = 16
    direction: [f32; 4], // Direction of the light // 4 * 4 = 16
    u_axis: [f32; 4],    // U axis for rectangle // 4 * 4 = 16
    v_axis: [f32; 4],    // V axis for rectangle // 4 * 4 = 16
    intensity: [f32; 4], // Intensity of the light // 4 * 4 = 16
    twosided: u32,       // Whether the rectangle emits light on both sides
    _pad1: u32,
    _pad2: u32,
    _pad3: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod, Zeroable)]
struct InfiniteLight {
    intensity: [f32; 4],       // Intensity of the light // 4 * 4 = 16
    indices: [i32; 4],         // Indices for the light texture
    inv_matrix: [[f32; 4]; 4], // Inverse matrix for the light texture
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod, Zeroable)]
struct DirectionalShadowInfo {
    light_view_proj: [[f32; 4]; 4], // 4 * 4 * 4 = 64
    split_end: f32,                 // 1 * 4 = 4
    bias: f32,                      // 1 * 4 = 4
    slope_bias: f32,                // 1 * 4 = 4
    map_layer: i32,                 // 1 * 4 = 4
}

#[derive(Debug, Clone)]
struct TmpPipelineEntry {
    pub shader: Arc<RenderShader>,
    pub mesh_indices: Vec<usize>,
    pub material_indices: Vec<usize>,
    pub material_indices_map: HashMap<Uuid, (usize, Arc<RenderPass>)>,
}

impl TmpPipelineEntry {
    pub fn new(shader: &Arc<RenderShader>) -> Self {
        Self {
            shader: shader.clone(),
            mesh_indices: Vec::new(),
            material_indices: Vec::new(),
            material_indices_map: HashMap::new(),
        }
    }
}

fn get_uv_axis(direction: &glam::Vec3) -> (glam::Vec3, glam::Vec3) {
    let up = if direction.abs().dot(glam::vec3(0.0, 1.0, 0.0)) < 0.999 {
        glam::vec3(0.0, 1.0, 0.0)
    } else {
        glam::vec3(1.0, 0.0, 0.0)
    };
    let u_axis = direction.cross(up).normalize();
    let v_axis = direction.cross(u_axis).normalize();
    return (u_axis, v_axis);
}

#[derive(Debug, Clone)]
struct MaterialBindGroupEntry {
    pub id: Uuid,
    pub material_bind_group: wgpu::BindGroup,
    #[allow(dead_code)]
    pub uniform_buffer: wgpu::Buffer,
    #[allow(dead_code)]
    pub textures: Vec<Option<Arc<RenderTexture>>>,

    pub ltc_bind_group: Option<wgpu::BindGroup>,
    #[allow(dead_code)]
    pub ltc_texture: Option<Arc<RenderTexture>>,
}

#[derive(Debug, Clone)]
struct ShadingPipelineEntry {
    pub pipeline: wgpu::RenderPipeline,
    pub material_bind_group_layout: wgpu::BindGroupLayout,
    pub material_bind_groups: Vec<Arc<MaterialBindGroupEntry>>,
    pub mesh_indices: Vec<usize>,
    pub material_indices: Vec<usize>,
    pub sort_order: u32,
    pub enable_lighting: bool,
}

#[derive(Debug, Clone)]
struct ZPrepassPipelineEntry {
    pub pipeline: wgpu::RenderPipeline,
    pub mesh_indices: Vec<usize>,
}

#[derive(Debug, Clone)]
struct DirectionalShadowPipelineEntry {
    pub pipeline: wgpu::RenderPipeline,
    pub mesh_indices: Vec<usize>,
    pub directional_light_shadows: Vec<Arc<RenderDirectionalLightShadow>>,
    pub directional_shadow_info_buffer: wgpu::Buffer,
    pub shadow_map_array: RenderTexture,
}

#[derive(Debug, Clone)]
enum PipelineEntry {
    Shading(ShadingPipelineEntry),
    ZPrepass(ZPrepassPipelineEntry),
    DirectionalShadow(DirectionalShadowPipelineEntry),
}

#[derive(Debug, Clone)]
pub struct LightingMeshRenderer {
    target_format: wgpu::TextureFormat,
    // Global bind group layout and buffer
    min_uniform_buffer_offset_alignment: wgpu::BufferAddress,
    #[allow(dead_code)]
    global_bind_group_layout: wgpu::BindGroupLayout,
    global_bind_group: wgpu::BindGroup,
    global_uniform_buffer: wgpu::Buffer,
    // Local bind group layout and buffer
    local_bind_group_layout: wgpu::BindGroupLayout,
    local_bind_group: wgpu::BindGroup,
    local_uniform_buffer: wgpu::Buffer,
    // Light bind group layout and buffer
    #[allow(dead_code)]
    light_bind_group_layout: wgpu::BindGroupLayout,
    light_bind_group: wgpu::BindGroup,
    light_uniform_buffer: wgpu::Buffer,
    directional_light_buffer: wgpu::Buffer,
    sphere_light_buffer: wgpu::Buffer,
    disk_light_buffer: wgpu::Buffer,
    rect_light_buffer: wgpu::Buffer,
    infinite_light_buffer: wgpu::Buffer,

    #[allow(dead_code)]
    ltc_bind_group_layout: wgpu::BindGroupLayout,
    #[allow(dead_code)]
    directional_shadow_bind_group_layout: wgpu::BindGroupLayout,
    directional_shadow_bind_group: wgpu::BindGroup,

    // Mesh items to render
    mesh_items: Vec<Arc<RenderItem>>,
    // Textures used in the materials
    textures: HashMap<Uuid, Arc<RenderTexture>>,
    // Material entries
    pipelines: HashMap<Uuid, Arc<RwLock<PipelineEntry>>>,
}

fn create_local_uniform_buffer(device: &wgpu::Device, num_items: usize) -> wgpu::Buffer {
    let uniform_alignment = {
        let alignment = device.limits().min_uniform_buffer_offset_alignment as wgpu::BufferAddress;
        align_to(
            std::mem::size_of::<LocalUniforms>() as wgpu::BufferAddress,
            alignment,
        )
    };
    let required_size = uniform_alignment * num_items as wgpu::BufferAddress;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Item Matrices Buffer"),
        size: required_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
        mapped_at_creation: false,
    });
    return buffer;
}

fn get_shader_has_lighting(category: RenderCategory) -> bool {
    if category == RenderCategory::Emissive {
        return false;
    }
    return true;
}

fn get_shader_uses_z_prepass(category: RenderCategory) -> bool {
    category == RenderCategory::Opaque || category == RenderCategory::Emissive
}

pub(super) fn get_shader_uses_shadow(category: RenderCategory) -> bool {
    // Keep shadow-caster selection independent from z-prepass policy.
    category == RenderCategory::Opaque || category == RenderCategory::Emissive
}

impl LightingMeshRenderer {
    pub(crate) fn set_directional_shadow_resources(
        &mut self,
        device: &wgpu::Device,
        directional_shadow_info_buffer: &wgpu::Buffer,
        directional_shadow_map: &RenderTexture,
    ) {
        self.directional_shadow_bind_group = self.create_directional_shadow_bind_group(
            device,
            directional_shadow_info_buffer,
            &directional_shadow_map.view,
            &directional_shadow_map.sampler,
        );
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        render_items: &[Arc<RenderItem>],
        camera: &RenderCamera,
    ) {
        let (_mesh_items, light_items) = self.prepare_without_shadow_render(
            device,
            queue,
            render_items,
            camera,
        );
        self.prepare_lights(device, queue, &light_items, None);
    }

    pub(super) fn prepare_without_shadow_render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        render_items: &[Arc<RenderItem>],
        camera: &RenderCamera,
    ) -> (Vec<Arc<RenderItem>>, Vec<Arc<RenderItem>>) {
        self.prepare_global(device, queue, camera); //group(0)
        let (mesh_items, light_items) = Self::split_items(render_items);
        self.prepare_locals(device, queue, &mesh_items); //group(1)
        self.prepare_z_prepass(device, &mesh_items); //group(2)
        self.prepare_materials(device, queue, &mesh_items); //group(2)
        (mesh_items, light_items)
    }

    pub fn paint(&self, render_pass: &mut wgpu::RenderPass) {
        if !self.mesh_items.is_empty() {
            self.render(render_pass, &self.mesh_items);
        }
    }

    // -------------------------------------------------------

    pub(super) fn split_items(
        render_items: &[Arc<RenderItem>],
    ) -> (Vec<Arc<RenderItem>>, Vec<Arc<RenderItem>>) {
        let mut mesh_items = Vec::new();
        let mut light_items = Vec::new();
        for item in render_items.iter() {
            match item.as_ref() {
                RenderItem::Mesh(_) => mesh_items.push(item.clone()),
                RenderItem::Light(_) => light_items.push(item.clone()),
                _ => {}
            }
        }
        return (mesh_items, light_items);
    }

    // -------------------------------------------------------

    fn render(&self, render_pass: &mut wgpu::RenderPass, render_items: &[Arc<RenderItem>]) {
        let local_uniform_alignment = {
            let alignment = self.min_uniform_buffer_offset_alignment;
            align_to(
                std::mem::size_of::<LocalUniforms>() as wgpu::BufferAddress,
                alignment,
            )
        };

        let mut z_prepass_pipelines = Vec::new();
        let mut shading_pipelines = Vec::new();
        for pipeline_entry in self.pipelines.values() {
            let pipeline_entry = pipeline_entry.read().unwrap();
            match &*pipeline_entry {
                PipelineEntry::ZPrepass(p) if !p.mesh_indices.is_empty() => {
                    z_prepass_pipelines.push(pipeline_entry.clone())
                }
                PipelineEntry::Shading(p) if !p.mesh_indices.is_empty() => {
                    shading_pipelines.push((p.sort_order, pipeline_entry.clone()))
                }
                _ => {}
            }
        }

        //TODO: sort pipelines to minimize pipeline switching
        shading_pipelines.sort_by(|a, b| a.0.cmp(&b.0));

        // Depth-only prepass for opaque-like geometry.
        for pipeline_entry in z_prepass_pipelines.iter() {
            if let PipelineEntry::ZPrepass(p) = pipeline_entry {
                render_pass.set_pipeline(&p.pipeline);
                render_pass.set_bind_group(0, &self.global_bind_group, &[]);
                for item_index in p.mesh_indices.iter().copied() {
                    if let RenderItem::Mesh(mesh_item) = render_items[item_index].as_ref() {
                        let local_uniform_offset = item_index as wgpu::DynamicOffset
                            * local_uniform_alignment as wgpu::DynamicOffset;
                        render_pass.set_bind_group(
                            1,
                            &self.local_bind_group,
                            &[local_uniform_offset],
                        );
                        render_pass.set_vertex_buffer(0, mesh_item.mesh.vertex_buffer.slice(..));
                        render_pass.set_index_buffer(
                            mesh_item.mesh.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        render_pass.draw_indexed(0..mesh_item.mesh.index_count, 0, 0..1);
                    }
                }
            }
        }

        for (_, pipeline_entry) in shading_pipelines.iter() {
            if let PipelineEntry::Shading(p) = pipeline_entry {
                debug_assert!(!p.mesh_indices.is_empty());
                render_pass.set_pipeline(&p.pipeline);
                render_pass.set_bind_group(0, &self.global_bind_group, &[]);
                if p.enable_lighting {
                    render_pass.set_bind_group(3, &self.light_bind_group, &[]);
                    render_pass.set_bind_group(5, &self.directional_shadow_bind_group, &[]);
                }
                debug_assert!(p.mesh_indices.len() == p.material_indices.len());
                let length = p.mesh_indices.len();
                for i in 0..length {
                    let item_index = p.mesh_indices[i];
                    let material_index = p.material_indices[i];
                    if let RenderItem::Mesh(mesh_item) = render_items[item_index].as_ref() {
                        let local_uniform_offset = item_index as wgpu::DynamicOffset
                            * local_uniform_alignment as wgpu::DynamicOffset;
                        render_pass.set_bind_group(
                            1,
                            &self.local_bind_group,
                            &[local_uniform_offset],
                        );
                        render_pass.set_bind_group(
                            2,
                            &p.material_bind_groups[material_index].material_bind_group,
                            &[],
                        );
                        if let Some(ltc_bind_group) =
                            &p.material_bind_groups[material_index].ltc_bind_group
                        {
                            render_pass.set_bind_group(4, ltc_bind_group, &[]);
                        }
                        render_pass.set_vertex_buffer(0, mesh_item.mesh.vertex_buffer.slice(..));
                        render_pass.set_index_buffer(
                            mesh_item.mesh.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        render_pass.draw_indexed(0..mesh_item.mesh.index_count, 0, 0..1);
                    }
                }
            }
        }
    }

    fn prepare_global(&self, _device: &wgpu::Device, queue: &wgpu::Queue, camera: &RenderCamera) {
        let global_uniforms = GlobalUniforms {
            world_to_camera: camera.world_to_camera.to_cols_array_2d(),
            camera_to_clip: camera.camera_to_clip.to_cols_array_2d(),
            camera_to_world: camera.camera_to_world.to_cols_array_2d(),
            camera_position: [camera.position.x, camera.position.y, camera.position.z, 1.0],
        };
        let global_unifrom_buffer = &self.global_uniform_buffer;
        queue.write_buffer(
            global_unifrom_buffer,
            0,
            bytemuck::bytes_of(&global_uniforms),
        );
    }

    pub fn prepare_locals(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mesh_items: &[Arc<RenderItem>],
    ) {
        let num_mesh_items = mesh_items.len();
        let local_uniform_alignment = {
            let alignment = self.min_uniform_buffer_offset_alignment;
            align_to(
                std::mem::size_of::<LocalUniforms>() as wgpu::BufferAddress,
                alignment,
            )
        };
        if self.local_uniform_buffer.size()
            < (num_mesh_items as wgpu::BufferAddress * local_uniform_alignment)
        {
            let new_buffer = create_local_uniform_buffer(device, num_mesh_items);
            let new_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Lighting Local Bind Group"),
                layout: &self.local_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &new_buffer,
                        offset: 0,
                        size: wgpu::BufferSize::new(size_of::<LocalUniforms>() as _),
                    }),
                }],
            });
            self.local_uniform_buffer = new_buffer;
            self.local_bind_group = new_bind_group;
        }
        for (i, item) in mesh_items.iter().enumerate() {
            let local_to_world = item.get_matrix();
            let world_to_local = local_to_world.inverse();
            let uniform = LocalUniforms {
                local_to_world: local_to_world.to_cols_array_2d(),
                world_to_local: world_to_local.to_cols_array_2d(),
                ..Default::default()
            };
            let offset = i as wgpu::BufferAddress * local_uniform_alignment;
            queue.write_buffer(
                &self.local_uniform_buffer,
                offset,
                bytemuck::bytes_of(&uniform),
            );
        }
    }

    fn create_material_bind_group(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        material_bind_group_layout: &wgpu::BindGroupLayout,
        ltc_bind_group_layout: &wgpu::BindGroupLayout,
        render_pass: &RenderPass,
    ) -> MaterialBindGroupEntry {
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Material Uniform Buffer"),
            contents: &render_pass.uniform_values,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &uniform_buffer,
                offset: 0,
                size: wgpu::BufferSize::new(render_pass.uniform_values.len() as _),
            }),
        }];

        let material_binding_offset = 1;
        for (i, texture) in render_pass.textures.iter().enumerate() {
            let binding_texture = (material_binding_offset + 2 * i) as u32;
            let binding_sampler = (material_binding_offset + 2 * i + 1) as u32;
            if let Some(texture) = texture {
                entries.push(wgpu::BindGroupEntry {
                    binding: binding_texture,
                    resource: wgpu::BindingResource::TextureView(&texture.view),
                });
                entries.push(wgpu::BindGroupEntry {
                    binding: binding_sampler,
                    resource: wgpu::BindingResource::Sampler(&texture.sampler),
                });
            }
        }

        let material_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Material Bind Group"),
            layout: material_bind_group_layout,
            entries: &entries,
        });

        let (ltc_bind_group, ltc_texture) =
            if let Some(ltc_texture) = render_pass.ltc_texture.as_ref() {
                let ltc_entries = vec![
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&ltc_texture.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&ltc_texture.sampler),
                    },
                ];
                let ltc_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("LTC Bind Group"),
                    layout: ltc_bind_group_layout,
                    entries: &ltc_entries,
                });
                (Some(ltc_bind_group), Some(ltc_texture.clone()))
            } else {
                (None, None)
            };

        let id = render_pass.id;
        let textures = render_pass.textures.clone();
        return MaterialBindGroupEntry {
            id,
            material_bind_group,
            uniform_buffer,
            textures,
            ltc_bind_group,
            ltc_texture,
        };
    }

    pub fn prepare_materials(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mesh_items: &[Arc<RenderItem>],
    ) {
        {
            let mut prev_bind_groups = HashMap::new(); //store existing bind groups to reuse
            for entry in self.pipelines.values_mut() {
                let mut entry = entry.write().unwrap();
                match &mut *entry {
                    PipelineEntry::Shading(p) => {
                        p.mesh_indices.clear();
                        p.material_indices.clear();
                        for bind_group in p.material_bind_groups.iter() {
                            prev_bind_groups.insert(bind_group.id, bind_group.clone());
                        }
                        p.material_bind_groups.clear();
                    }
                    _ => {}
                }
            }
            let mut tmp_pipelines: HashMap<Uuid, TmpPipelineEntry> = HashMap::new();
            for (mesh_index, item) in mesh_items.iter().enumerate() {
                if let Some(material) = item.get_material() {
                    for pass in material.passes.iter() {
                        let shader = pass.shader.clone();
                        let shader_id = shader.id;
                        let entry = tmp_pipelines
                            .entry(shader_id)
                            .or_insert(TmpPipelineEntry::new(&shader));
                        entry.mesh_indices.push(mesh_index);
                        let pass_id = pass.id;
                        let new_material_index = entry.material_indices_map.len();
                        if let Some((material_index, _)) = entry.material_indices_map.get(&pass_id)
                        {
                            entry.material_indices.push(*material_index);
                        } else {
                            entry.material_indices.push(new_material_index);
                            entry
                                .material_indices_map
                                .insert(pass_id, (new_material_index, pass.clone()));
                        }
                    }
                }
            }

            for (shader_id, tmp_entry) in tmp_pipelines.iter() {
                let mesh_indices = &tmp_entry.mesh_indices;
                let material_indices = &tmp_entry.material_indices;
                let num_materials = tmp_entry.material_indices_map.len();
                if num_materials == 0 {
                    continue;
                }
                let (_, pass) = tmp_entry.material_indices_map.values().next().unwrap();

                if !self.pipelines.contains_key(shader_id) {
                    let shader = tmp_entry.shader.clone();
                    let shader_id = shader.id;
                    let pipeline = self.create_pipeline(device, queue, &shader.shader, pass);
                    let pipeline = Arc::new(RwLock::new(pipeline));
                    self.pipelines.insert(shader_id, pipeline);
                }
                let entry = self
                    .pipelines
                    .get_mut(shader_id)
                    .expect("Pipeline for basic material not found");
                let mut entry = entry.write().unwrap();
                if let PipelineEntry::Shading(p) = &mut *entry {
                    p.mesh_indices = mesh_indices.clone();
                    p.material_indices = material_indices.clone();
                    assert!(p.mesh_indices.len() == p.material_indices.len());
                    let mut passes = Vec::with_capacity(num_materials);
                    for (_, (index, pass)) in tmp_entry.material_indices_map.iter() {
                        passes.push((index, pass));
                    }
                    passes.sort_by(|a, b| a.0.cmp(b.0));
                    for (_, pass) in passes.iter() {
                        let id = pass.id;
                        if let Some(bind_group_entry) = prev_bind_groups.get(&id) {
                            p.material_bind_groups.push(bind_group_entry.clone());
                        } else {
                            let bind_group_entry = Self::create_material_bind_group(
                                device,
                                queue,
                                &p.material_bind_group_layout,
                                &self.ltc_bind_group_layout,
                                pass,
                            );
                            p.material_bind_groups.push(Arc::new(bind_group_entry));
                        }
                    }
                }
            }
        }
        self.mesh_items = mesh_items.to_vec();
    }

    fn prepare_z_prepass(&mut self, device: &wgpu::Device, mesh_items: &[Arc<RenderItem>]) {
        if !self.pipelines.contains_key(&Z_PREPASS_PIPELINE_ID) {
            let entry = self.create_z_prepass_pipeline(device);
            self.pipelines
                .insert(Z_PREPASS_PIPELINE_ID, Arc::new(RwLock::new(entry)));
        }
        //todo: check z-prepass should be one pipeline for all meshes or multiple pipelines for different shaders.
        
        if let Some(entry) = self.pipelines.get_mut(&Z_PREPASS_PIPELINE_ID) {
            let mut entry = entry.write().unwrap();
            if let PipelineEntry::ZPrepass(p) = &mut *entry {
                let mut indices: Vec<usize> = Vec::with_capacity(mesh_items.len());
                for (mesh_index, item) in mesh_items.iter().enumerate() {
                    if let Some(material) = item.get_material()
                        && material
                            .passes
                            .iter()
                            .any(|pass| get_shader_uses_z_prepass(pass.render_category))
                    {
                        indices.push(mesh_index);
                    }
                }
                p.mesh_indices = indices;
            }
        }
    }

    fn create_light_bind_group(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        light_texture_view: &wgpu::TextureView,
        light_sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        let layout = &self.light_bind_group_layout;
        let entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: self.light_uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: self.directional_light_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: self.sphere_light_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: self.disk_light_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: self.rect_light_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: self.infinite_light_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(light_texture_view),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::Sampler(light_sampler),
            },
        ];
        let light_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Lighting Light Bind Group"),
            layout,
            entries: &entries,
        });
        return light_bind_group;
    }


    pub(super) fn prepare_lights(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        light_items: &[Arc<RenderItem>],
        directional_shadow_index_map: Option<&HashMap<Uuid, i32>>,
    ) {
        let mut light_uniforms = LightUniforms::default();
        let mut light_textures = Vec::new();
        // Point lights
        {
            let mut light_buffer = Vec::new();
            for item in light_items.iter() {
                if let RenderItem::Light(light_item) = item.as_ref()
                    && let RenderItem::Light(item) = item.as_ref()
                    && let RenderLight::Sphere(light) = item.light.as_ref()
                {
                    if light_buffer.len() >= MAX_SPHERE_LIGHT_NUM {
                        break;
                    }
                    let matrix = light_item.matrix; //local_to_world
                    let position = light.position;
                    let position =
                        matrix.transform_point3(glam::vec3(position[0], position[1], position[2]));
                    //println!("Point light position: {:?}", position);
                    let intensity = light.intensity;
                    let radius = light.radius;
                    let light = SphereLight {
                        position: [position.x, position.y, position.z, 1.0],
                        intensity: [intensity[0], intensity[1], intensity[2], 1.0],
                        radius,
                        _pad1: [0.0; 2], // Range of the light // 4 * 4 = 8
                        ..Default::default()
                    };
                    light_buffer.push(light);
                }
            }
            if !light_buffer.is_empty() {
                queue.write_buffer(
                    &self.sphere_light_buffer,
                    0,
                    bytemuck::cast_slice(&light_buffer),
                );
            }
            light_uniforms.num_sphere_lights = light_buffer.len() as u32;
        }

        // Spot lights
        {
            let mut light_buffer = Vec::new();
            for item in light_items.iter() {
                if let RenderItem::Light(light_item) = item.as_ref()
                    && let RenderLight::Disk(light) = light_item.light.as_ref()
                {
                    if light_buffer.len() >= MAX_DISK_LIGHT_NUM {
                        break;
                    }
                    let matrix = light_item.matrix; //local_to_world
                    let position = light.position;
                    let position =
                        matrix.transform_point3(glam::vec3(position[0], position[1], position[2]));
                    let direction = light.direction;
                    let direction = matrix.transform_vector3(glam::vec3(
                        direction[0],
                        direction[1],
                        direction[2],
                    ));
                    //println!("Point light position: {:?}", position);
                    let intensity = light.intensity;
                    let radius = light.radius;
                    let cos_inner_angle = f32::cos(light.inner_angle);
                    let cos_outer_angle = f32::cos(light.outer_angle);
                    let (u_axis, v_axis) = get_uv_axis(&direction);
                    let light = DiskLight {
                        position: [position.x, position.y, position.z, 1.0],
                        direction: [direction.x, direction.y, direction.z, 0.0],
                        intensity: [intensity[0], intensity[1], intensity[2], 1.0],
                        radius,
                        cos_inner_angle,
                        cos_outer_angle,
                        u_axis: [u_axis.x, u_axis.y, u_axis.z, 0.0],
                        v_axis: [v_axis.x, v_axis.y, v_axis.z, 0.0],
                        twosided: if light.twosided { 1 } else { 0 },
                        ..Default::default()
                    };
                    light_buffer.push(light);
                }
            }
            if !light_buffer.is_empty() {
                queue.write_buffer(
                    &self.disk_light_buffer,
                    0,
                    bytemuck::cast_slice(&light_buffer),
                );
            }
            light_uniforms.num_disk_lights = light_buffer.len() as u32;
        }

        // Rect lights
        {
            let mut light_buffer = Vec::new();
            for item in light_items.iter() {
                if let RenderItem::Light(light_item) = item.as_ref()
                    && let RenderLight::Rect(rect) = light_item.light.as_ref()
                {
                    //println!("Rect light item: {:?}", item);
                    if light_buffer.len() >= MAX_RECT_LIGHT_NUM {
                        break;
                    }
                    let matrix = light_item.matrix; //local_to_world
                    let position = rect.position;
                    let position =
                        matrix.transform_point3(glam::vec3(position[0], position[1], position[2]));
                    let direction = rect.direction;
                    let direction = matrix.transform_vector3(glam::vec3(
                        direction[0],
                        direction[1],
                        direction[2],
                    ));
                    let u_axis = rect.u_axis;
                    let u_axis =
                        matrix.transform_vector3(glam::vec3(u_axis[0], u_axis[1], u_axis[2]));
                    let v_axis = rect.v_axis;
                    let v_axis =
                        matrix.transform_vector3(glam::vec3(v_axis[0], v_axis[1], v_axis[2]));
                    let intensity = rect.intensity;
                    let light = RectLight {
                        position: [position.x, position.y, position.z, 1.0],
                        direction: [direction.x, direction.y, direction.z, 0.0],
                        u_axis: [u_axis.x, u_axis.y, u_axis.z, 0.0],
                        v_axis: [v_axis.x, v_axis.y, v_axis.z, 0.0],
                        intensity: [intensity[0], intensity[1], intensity[2], 1.0],
                        twosided: if rect.twosided { 1 } else { 0 },
                        ..Default::default()
                    };
                    light_buffer.push(light);
                }
            }
            if !light_buffer.is_empty() {
                queue.write_buffer(
                    &self.rect_light_buffer,
                    0,
                    bytemuck::cast_slice(&light_buffer),
                );
            }
            light_uniforms.num_rect_lights = light_buffer.len() as u32;
        }

        // Infinite lights
        {
            let mut light_buffer = Vec::new();
            for item in light_items.iter() {
                if let RenderItem::Light(light_item) = item.as_ref()
                    && let RenderLight::Infinite(light) = light_item.light.as_ref()
                {
                    if light_buffer.len() >= MAX_INFINITE_LIGHT_NUM {
                        break;
                    }
                    if light_textures.len() >= MAX_LIGHT_TEXTURE_NUM {
                        break;
                    }
                    let mut texture_index = -1;
                    if let Some(texture) = &light.texture {
                        // New texture, add it to the list
                        texture_index = light_textures.len() as i32;
                        light_textures.push(texture.clone());
                    }

                    let inv_matrix = light_item.matrix.inverse();
                    let intensity = light.intensity;
                    let light = InfiniteLight {
                        intensity: [intensity[0], intensity[1], intensity[2], 1.0],
                        indices: [texture_index, 0, 0, 0], // Indices for the light texture
                        inv_matrix: inv_matrix.to_cols_array_2d(),
                    };
                    light_buffer.push(light);
                }
            }
            if !light_buffer.is_empty() {
                queue.write_buffer(
                    &self.infinite_light_buffer,
                    0,
                    bytemuck::cast_slice(&light_buffer),
                );
            }
            light_uniforms.num_infinite_lights = light_buffer.len() as u32;
        }

        // Directional lights
        {
            let mut light_buffer = Vec::new();
            for item in light_items.iter() {
                if let RenderItem::Light(light_item) = item.as_ref()
                    && let RenderLight::Directional(light) = light_item.light.as_ref()
                {
                    if light_buffer.len() >= MAX_DIRECTIONAL_LIGHT_NUM {
                        break;
                    }
                    let matrix = light_item.matrix; //local_to_world
                    let direction = light.direction;
                    let direction = matrix.transform_vector3(glam::vec3(
                        direction[0],
                        direction[1],
                        direction[2],
                    ));
                    let intensity = light.intensity;

                    let radius = (0.5 * light.source_angle).tan();

                    let shadow_index = if light.cast_shadow {
                        //log::info!("Directional light {:?} casts shadow", light_item.light.get_id());
                        directional_shadow_index_map
                            .and_then(|map| map.get(&light.id).copied())
                            .unwrap_or(-1)
                    } else {
                        -1
                    };
                    let cascade_count = if light.cast_shadow && shadow_index >= 0 {
                        light
                            .cascade_count
                            .clamp(1, DIRECTIONAL_SHADOW_CASCADE_MAX_COUNT as u32)
                    } else {
                        0
                    };

                    let light = DirectionalLight {
                        direction: [direction[0], direction[1], direction[2], 0.0],
                        intensity: [intensity[0], intensity[1], intensity[2], 1.0],
                        radius,
                        shadow_index,
                        cascade_count,
                        _pad1: 0,
                    };
                    light_buffer.push(light);
                }
            }
            if !light_buffer.is_empty() {
                queue.write_buffer(
                    &self.directional_light_buffer,
                    0,
                    bytemuck::cast_slice(&light_buffer),
                );
            }
            light_uniforms.num_directional_lights = light_buffer.len() as u32;
        }

        {
            if light_textures.len() <= 0 {
                let light_texture = self
                    .textures
                    .get(&DEFAULT_LIGHT_TEXTURE_ID)
                    .expect("Default light texture not found");
                self.light_bind_group = self.create_light_bind_group(
                    device,
                    queue,
                    &light_texture.view,
                    &light_texture.sampler,
                );
            } else {
                let light_texture = &light_textures[0];
                self.light_bind_group = self.create_light_bind_group(
                    device,
                    queue,
                    &light_texture.view,
                    &light_texture.sampler,
                );
            }
        }

        // Finally, write light uniforms
        queue.write_buffer(
            &self.light_uniform_buffer,
            0,
            bytemuck::bytes_of(&light_uniforms),
        );
    }

    fn create_pipeline(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        shader: &wgpu::ShaderModule,
        pass: &RenderPass,
    ) -> PipelineEntry {
        let render_category = pass.render_category;
        let material_uniform_size = pass.uniform_values.len();
        let has_lighting = get_shader_has_lighting(render_category);
        let sort_order = render_category as u32;

        let vertex_buffer_layout = [wgpu::VertexBufferLayout {
            array_stride: size_of::<RenderVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: std::mem::size_of::<f32>() as u64 * 3,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: std::mem::size_of::<f32>() as u64 * 6,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: std::mem::size_of::<f32>() as u64 * 9,
                    shader_location: 3,
                },
            ],
        }];

        let mut material_bind_group_entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(material_uniform_size as _),
            },
            count: None,
        }];
        let material_binding_offset = 1;
        for (i, texture) in pass.textures.iter().enumerate() {
            let binding_texture = material_binding_offset + (2 * i) as u32;
            let binding_sampler = material_binding_offset + (2 * i + 1) as u32;
            if texture.is_some() {
                material_bind_group_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: binding_texture,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                });
                material_bind_group_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: binding_sampler,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                });
            }
        }

        let material_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Material Bind Group Layout"),
                entries: &material_bind_group_entries,
            });

        let mut bind_group_layouts = vec![
            &self.global_bind_group_layout, //group(0)
            &self.local_bind_group_layout,  //group(1)
            &material_bind_group_layout,    //group(2)
        ];
        if has_lighting {
            bind_group_layouts.push(&self.light_bind_group_layout); //group(3)
            bind_group_layouts.push(&self.ltc_bind_group_layout); //group(4)
            bind_group_layouts.push(&self.directional_shadow_bind_group_layout); //group(5)
        }

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Lighting Pipeline Layout"),
            bind_group_layouts: &bind_group_layouts,
            push_constant_ranges: &[],
        });

        let primitive = wgpu::PrimitiveState {
            //front_face: wgpu::FrontFace::Ccw,
            //cull_mode: Some(wgpu::Face::Back),
            topology: wgpu::PrimitiveTopology::TriangleList,
            polygon_mode: wgpu::PolygonMode::Fill,
            ..Default::default()
        };

        let color_texture_format = self.target_format;
        let depth_texture_format = wgpu::TextureFormat::Depth32Float;
        //let color_texture_format = wgpu::TextureFormat::Rgba16Float;
        //let depth_texture_format = wgpu::TextureFormat::Depth32Float;

        let mut blendstate = wgpu::BlendState::REPLACE;
        if render_category == RenderCategory::Transparent {
            blendstate = wgpu::BlendState::ALPHA_BLENDING;
        } else if render_category == RenderCategory::TransparentSpecular {
            blendstate = wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent::OVER,
            };
        }

        let mut depth_write_enabled = true;
        if render_category == RenderCategory::Transparent
            || render_category == RenderCategory::TransparentSpecular
        {
            depth_write_enabled = false;
        }
        if get_shader_uses_z_prepass(render_category) {
            depth_write_enabled = false;
        }

        let name = pass.shader.name.clone();
        let label = format!("Lighting Pipeline {}", name);

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label.as_str()),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_main"),
                buffers: &vertex_buffer_layout,
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_main"),
                //targets: &[Some(wgpu_render_state.target_format.into())],
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_texture_format,
                    blend: Some(blendstate),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive,
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_texture_format,
                depth_write_enabled,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Create a uniform buffer for material properties
        PipelineEntry::Shading(ShadingPipelineEntry {
            pipeline,
            material_bind_group_layout,
            material_bind_groups: Vec::new(),
            mesh_indices: Vec::new(),
            material_indices: Vec::new(),
            sort_order,
            enable_lighting: has_lighting,
        })
    }

    fn create_z_prepass_pipeline(&self, device: &wgpu::Device) -> PipelineEntry {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Lighting Z Prepass Shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("shaders/lighting_z_prepass.wgsl").into(),
            ),
        });

        let vertex_buffer_layout = [wgpu::VertexBufferLayout {
            array_stride: size_of::<RenderVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: std::mem::size_of::<f32>() as u64 * 3,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: std::mem::size_of::<f32>() as u64 * 6,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: std::mem::size_of::<f32>() as u64 * 9,
                    shader_location: 3,
                },
            ],
        }];

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Lighting Z Prepass Pipeline Layout"),
            bind_group_layouts: &[
                &self.global_bind_group_layout,
                &self.local_bind_group_layout,
            ],
            push_constant_ranges: &[],
        });

        let primitive = wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            polygon_mode: wgpu::PolygonMode::Fill,
            ..Default::default()
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Lighting Z Prepass Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &vertex_buffer_layout,
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::empty(),
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive,
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        PipelineEntry::ZPrepass(ZPrepassPipelineEntry {
            pipeline,
            mesh_indices: Vec::new(),
        })
    }
}

impl LightingMeshRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let global_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Global Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(size_of::<GlobalUniforms>() as _),
                    },
                    count: None,
                }],
            });

        let local_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Local Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(size_of::<LocalUniforms>() as _),
                    },
                    count: None,
                }],
            });

        let point_light_buffer_size =
            (MAX_SPHERE_LIGHT_NUM * size_of::<SphereLight>()) as wgpu::BufferAddress;
        let spot_light_buffer_size =
            (MAX_DISK_LIGHT_NUM * size_of::<DiskLight>()) as wgpu::BufferAddress;
        let rect_light_buffer_size =
            (MAX_RECT_LIGHT_NUM * size_of::<RectLight>()) as wgpu::BufferAddress;
        let directional_light_buffer_size =
            (MAX_DIRECTIONAL_LIGHT_NUM * size_of::<DirectionalLight>()) as wgpu::BufferAddress;
        let infinite_light_buffer_size =
            (MAX_INFINITE_LIGHT_NUM * size_of::<InfiniteLight>()) as wgpu::BufferAddress;
        let light_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Light Bind Group Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(size_of::<
                                LightUniforms,
                            >()
                                as _),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage {
                                read_only: true,
                            },
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(directional_light_buffer_size),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage {
                                read_only: true,
                            },
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(point_light_buffer_size),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage {
                                read_only: true,
                            },
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(spot_light_buffer_size),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage {
                                read_only: true,
                            },
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(rect_light_buffer_size),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage {
                                read_only: true,
                            },
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(infinite_light_buffer_size),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 6,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 7,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let ltc_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("LTC Bind Group Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let directional_shadow_info_buffer_size = (MAX_DIRECTIONAL_SHADOW_NUM
            * size_of::<DirectionalShadowInfo>())
            as wgpu::BufferAddress;
        let directional_shadow_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Directional Shadow Bind Group Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(
                                directional_shadow_info_buffer_size,
                            ),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                        count: None,
                    },
                ],
            });

        let global_unifroms = GlobalUniforms {
            world_to_camera: glam::Mat4::IDENTITY.to_cols_array_2d(), // Identity matrix for now
            camera_to_clip: glam::Mat4::IDENTITY.to_cols_array_2d(),  // Identity matrix for now
            camera_to_world: glam::Mat4::IDENTITY.to_cols_array_2d(), // Identity matrix for now
            camera_position: [0.0, 0.0, 0.0, 1.0], // Camera position in world space
        };

        let global_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer for Matrix"),
            contents: bytemuck::bytes_of(&global_unifroms),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
        });

        let global_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Lighting Global Bind Group"),
            layout: &global_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: global_uniform_buffer.as_entire_binding(),
            }],
        });

        let local_uniform_buffer = create_local_uniform_buffer(device, MIN_LOCAL_BUFFER_NUM);
        let local_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Lighting Local Bind Group"),
            layout: &local_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &local_uniform_buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(size_of::<LocalUniforms>() as _),
                }),
            }],
        });

        let light_uniforms = LightUniforms::default();
        let light_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer for Light"),
            contents: bytemuck::bytes_of(&light_uniforms),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
        });

        let sphere_light_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Buffer for Point Lights"),
            size: point_light_buffer_size,
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
        });
        let disk_light_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Buffer for Spot Lights"),
            size: spot_light_buffer_size,
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
        });
        let rect_light_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Buffer for Rect Lights"),
            size: rect_light_buffer_size,
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
        });
        let directional_light_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Buffer for Directional Lights"),
            size: directional_light_buffer_size,
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
        });
        let infinite_light_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Buffer for Infinite Lights"),
            size: infinite_light_buffer_size,
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
        });
        let directional_shadow_info_buffer = Self::create_directional_shadow_info_buffer(device);

        let directional_shadow_map_array =
            Self::create_directional_shadow_map_array(device, 1, 1, 1);

        let directional_shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Lighting Directional Shadow Bind Group"),
            layout: &directional_shadow_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: directional_shadow_info_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &directional_shadow_map_array.view,
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&directional_shadow_map_array.sampler),
                },
            ],
        });

        let default_light_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Dummy Light Texture"),
            size: wgpu::Extent3d::default(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let default_light_texture_view =
            default_light_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let default_light_sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());

        let default_light_texture = RenderTexture {
            id: DEFAULT_LIGHT_TEXTURE_ID,
            edition: Uuid::new_v4().to_string(),
            texture: default_light_texture,
            view: default_light_texture_view,
            sampler: default_light_sampler,
            scale: [1.0, 1.0],
            delta: [0.0, 0.0],
        };
        let default_light_texture = Arc::new(default_light_texture);

        //let default_ltc_texture = create_default_ltc_texture(device, queue);
        //let default_ltc_texture = Arc::new(default_ltc_texture);

        let light_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Lighting Light Bind Group"),
            layout: &light_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: light_uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: directional_light_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: sphere_light_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: disk_light_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: rect_light_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: infinite_light_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&default_light_texture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::Sampler(&default_light_texture.sampler),
                },
            ],
        });

        let mut textures = HashMap::new();
        textures.insert(default_light_texture.id, default_light_texture);

        let min_uniform_buffer_offset_alignment =
            device.limits().min_uniform_buffer_offset_alignment as wgpu::BufferAddress;

        let mesh_items = Vec::with_capacity(MIN_LOCAL_BUFFER_NUM);
        let materials = HashMap::new();
        let mut pass = LightingMeshRenderer {
            target_format,
            min_uniform_buffer_offset_alignment,
            global_bind_group_layout,
            global_bind_group,
            global_uniform_buffer,
            local_bind_group_layout,
            local_bind_group,
            local_uniform_buffer,
            light_bind_group_layout,
            light_bind_group,
            light_uniform_buffer,
            directional_light_buffer,
            sphere_light_buffer,
            disk_light_buffer,
            rect_light_buffer,
            infinite_light_buffer,
            ltc_bind_group_layout,
            directional_shadow_bind_group_layout,
            directional_shadow_bind_group,
            mesh_items,
            textures,
            pipelines: materials,
        };
        pass.init(device, queue);
        return pass;
    }

    pub fn init(&mut self, _device: &wgpu::Device, _queue: &wgpu::Queue) {
        // Initialize pipelines or other resources if needed
    }
}
