use super::ltc::DEFAULT_LTC_UUID;
use super::ltc::create_default_ltc_texture;
use super::material::RenderCategory;
use super::material::RenderUniformValue;
use super::materials::AreaLightDiffuseMaterialUniforms;
use super::materials::BasicMaterialUniforms;
use super::mesh::RenderVertex;
use super::render_item::RenderItem;
use super::texture::RenderTexture;
use crate::render::wgpu::light::RenderLight;
use crate::render::wgpu::material::RenderMaterial;
use std::collections::HashMap;
use std::sync::Arc;

//use eframe::egui;
//use eframe::egui_wgpu;
use eframe::wgpu;
use eframe::wgpu::util::DeviceExt;
use uuid::Uuid;
use wgpu::util::align_to;

use bytemuck::{Pod, Zeroable};

const MIN_LOCAL_BUFFER_NUM: usize = 64;
const MAX_DIRECTIONAL_LIGHT_NUM: usize = 4; // Maximum number of directional lights
const MAX_SPHERE_LIGHT_NUM: usize = 256; // Maximum number of point lights
const MAX_DISK_LIGHT_NUM: usize = 32; // Maximum number of spot lights
const MAX_RECT_LIGHT_NUM: usize = 32; // Maximum number of rectangle lights
const MAX_INFINITE_LIGHT_NUM: usize = 1; // Maximum number of infinite lights
const MAX_LIGHT_TEXTURE_NUM: usize = 1; // Maximum number of light textures

const MIN_MATERIAL_BUFFER_NUM: usize = 8;
const AREA_LIGHT_DIFFUSE_SURFACE_SHADER_NAME: &str = "area_light";
const BASIC_SURFACE_SHADER_NAME: &str = "basic_material";

const DEFAULT_LIGHT_TEXTURE_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_FFFF_000000000000);

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
    _pad1: [f32; 4],
    _pad2: [f32; 4],
    _pad3: [f32; 4],
    _pad4: [f32; 4],
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
struct PipelineEntry {
    pub pipeline: wgpu::RenderPipeline,
    pub material_bind_group_layout: wgpu::BindGroupLayout,
    pub material_bind_group: wgpu::BindGroup,
    pub uniform_buffer: wgpu::Buffer,
    pub material_uniform_size: usize,
    pub material_uniform_alignment: wgpu::BufferAddress,
    pub mesh_indices: Vec<usize>,
    pub material_indices: Vec<usize>,
    pub has_lighting: bool,
}

impl PipelineEntry {
    fn allocate_material_uniform_buffer(&mut self, device: &wgpu::Device, num_items: usize) {
        let material_uniform_size = self.material_uniform_size;
        let material_uniform_alignment = self.material_uniform_alignment;
        let required_size = material_uniform_alignment * num_items as wgpu::BufferAddress;
        if self.uniform_buffer.size() >= required_size {
            return;
        }
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Material Buffer"),
            size: required_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Material Bind Group"),
            layout: &self.material_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniform_buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(material_uniform_size as _),
                }),
            }],
        });
        self.uniform_buffer = uniform_buffer;
        self.material_bind_group = bind_group;
    }
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

    // Mesh items to render
    mesh_items: Vec<Arc<RenderItem>>,
    // Textures used in the materials
    textures: HashMap<Uuid, Arc<RenderTexture>>,
    // Material entries
    pipelines: HashMap<Uuid, PipelineEntry>,
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

fn get_color_value(material: &RenderMaterial, key: &str) -> Option<[f32; 4]> {
    if let Some(value) = material.get_uniform_value(key) {
        if let RenderUniformValue::Vec4(color) = value {
            return Some(*color);
        }
        if let RenderUniformValue::Float(f) = value {
            return Some([*f, *f, *f, 1.0]);
        }
    }
    None
}

fn get_base_color(material: &RenderMaterial) -> [f32; 4] {
    let keys = ["Kd", "base_color"];
    for key in keys {
        if let Some(value) = get_color_value(material, key) {
            return value;
        }
    }
    [1.0, 0.0, 1.0, 1.0] // Default color
}

fn get_specular_color(material: &RenderMaterial) -> [f32; 4] {
    let keys = ["Ks", "specular_color"];
    for key in keys {
        if let Some(value) = get_color_value(material, key) {
            return value;
        }
    }
    [0.0, 0.0, 0.0, 1.0] // Default color for specular
}

fn get_shader_type_to_type(shader_type: &str) -> String {
    if shader_type.starts_with("area_light") {
        return AREA_LIGHT_DIFFUSE_SURFACE_SHADER_NAME.to_string();
    }
    return BASIC_SURFACE_SHADER_NAME.to_string();
}

fn get_shader_id_from_type(shader_type: &str) -> Uuid {
    return Uuid::new_v3(&Uuid::NAMESPACE_OID, shader_type.as_bytes());
}

fn get_material_uniform_size(shader_type: &str) -> usize {
    match shader_type {
        AREA_LIGHT_DIFFUSE_SURFACE_SHADER_NAME => size_of::<AreaLightDiffuseMaterialUniforms>(),
        BASIC_SURFACE_SHADER_NAME => size_of::<BasicMaterialUniforms>(),
        _ => size_of::<BasicMaterialUniforms>(),
    }
}

fn get_material_uniform_buffer(material: &RenderMaterial) -> Vec<u8> {
    let shader_type = material.get_shader_type();
    let shader_type = get_shader_type_to_type(&shader_type);
    match shader_type.as_str() {
        AREA_LIGHT_DIFFUSE_SURFACE_SHADER_NAME => {
            let uniform = AreaLightDiffuseMaterialUniforms {
                l: get_color_value(material, "L").unwrap_or([1.0, 1.0, 1.0, 1.0]),
                scale: get_color_value(material, "scale").unwrap_or([1.0, 1.0, 1.0, 1.0]),
                _pad1: [0.0; 4],
                _pad2: [0.0; 4],
            };
            let bytes = bytemuck::bytes_of(&uniform);
            return bytes.to_vec();
        }
        BASIC_SURFACE_SHADER_NAME => {
            let uniform = BasicMaterialUniforms {
                kd: get_base_color(material),
                ks: get_specular_color(material),
                _pad1: [0.0; 4],
                _pad2: [0.0; 4],
            };
            let bytes = bytemuck::bytes_of(&uniform);
            return bytes.to_vec();
        }
        _ => {
            let uniform = BasicMaterialUniforms {
                kd: get_base_color(material),
                ks: get_specular_color(material),
                _pad1: [0.0; 4],
                _pad2: [0.0; 4],
            };
            let bytes = bytemuck::bytes_of(&uniform);
            return bytes.to_vec();
        }
    }
}

fn get_shader_has_lighting(shader_type: &str, _category: RenderCategory) -> bool {
    match shader_type {
        AREA_LIGHT_DIFFUSE_SURFACE_SHADER_NAME => false,
        BASIC_SURFACE_SHADER_NAME => true,
        _ => true,
    }
}

fn get_shader(device: &wgpu::Device, shader_type: &str) -> wgpu::ShaderModule {
    let source = match shader_type {
        AREA_LIGHT_DIFFUSE_SURFACE_SHADER_NAME => {
            include_str!("shaders/area_light_diffuse.wgsl")
        }
        BASIC_SURFACE_SHADER_NAME => include_str!("shaders/render_lighting_mesh.wgsl"),
        _ => include_str!("shaders/render_lighting_mesh.wgsl"),
    };
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Shader"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    return shader;
}

impl LightingMeshRenderer {
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        render_items: &[Arc<RenderItem>],
        world_to_camera: &glam::Mat4,
        camera_to_clip: &glam::Mat4,
    ) {
        self.prepare_global(device, queue, world_to_camera, camera_to_clip); //group(0)
        {
            let (mesh_items, light_items) = Self::split_items(render_items);
            self.prepare_locals(device, queue, &mesh_items); //group(1)
            self.prepare_materials(device, queue, &mesh_items); //group(2)
            self.prepare_lights(device, queue, &light_items); //group(3)
        }
    }

    pub fn paint(&self, render_pass: &mut wgpu::RenderPass) {
        if !self.mesh_items.is_empty() {
            self.render(render_pass, &self.mesh_items);
        }
    }

    // -------------------------------------------------------

    fn split_items(
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

        for pipeline_entry in self.pipelines.values() {
            if pipeline_entry.mesh_indices.is_empty() {
                continue;
            }
            render_pass.set_pipeline(&pipeline_entry.pipeline); //
            render_pass.set_bind_group(0, &self.global_bind_group, &[]);
            if pipeline_entry.has_lighting {
                render_pass.set_bind_group(3, &self.light_bind_group, &[]);
            }
            debug_assert!(
                pipeline_entry.mesh_indices.len() == pipeline_entry.material_indices.len()
            );
            let length = pipeline_entry.mesh_indices.len();
            for i in 0..length {
                let item_index = pipeline_entry.mesh_indices[i];
                let material_index = pipeline_entry.material_indices[i];
                if let RenderItem::Mesh(mesh_item) = render_items[item_index].as_ref() {
                    let local_uniform_offset = item_index as wgpu::DynamicOffset
                        * local_uniform_alignment as wgpu::DynamicOffset;
                    let material_uniform_offset = material_index as wgpu::DynamicOffset
                        * pipeline_entry.material_uniform_alignment as wgpu::DynamicOffset;
                    //local params
                    render_pass.set_bind_group(1, &self.local_bind_group, &[local_uniform_offset]);
                    //material params
                    render_pass.set_bind_group(
                        2,
                        &pipeline_entry.material_bind_group,
                        &[material_uniform_offset],
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

    fn prepare_global(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        world_to_camera: &glam::Mat4,
        camera_to_clip: &glam::Mat4,
    ) {
        let camera_to_world = world_to_camera.inverse();
        let camera_position = camera_to_world.transform_point3(glam::vec3(0.0, 0.0, 0.0));
        let global_uniforms = GlobalUniforms {
            world_to_camera: world_to_camera.to_cols_array_2d(), // Identity matrix for now
            camera_to_clip: camera_to_clip.to_cols_array_2d(),   // Identity matrix for now
            camera_to_world: camera_to_world.to_cols_array_2d(), // Identity matrix for now
            camera_position: [camera_position.x, camera_position.y, camera_position.z, 1.0],
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

    pub fn prepare_materials(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mesh_items: &[Arc<RenderItem>],
    ) {
        #[derive(Debug, Clone, Default)]
        struct TmpPipelineEntry {
            mesh_indices: Vec<usize>,
            material_indices: Vec<usize>,
            material_indices_map: HashMap<Uuid, (usize, Arc<RenderMaterial>)>,
            has_lighting: bool,
        }
        {
            for entry in self.pipelines.values_mut() {
                entry.mesh_indices.clear();
                entry.material_indices.clear();
            }
            let mut tmp_pipelines: HashMap<String, TmpPipelineEntry> = HashMap::new();
            for (i, item) in mesh_items.iter().enumerate() {
                if let Some(material) = item.get_material() {
                    let shader_type_org = material.get_shader_type();
                    let shader_type = get_shader_type_to_type(&shader_type_org);
                    let has_lighting =
                        get_shader_has_lighting(&shader_type, material.render_category);
                    //println!("Material {} uses shader type: {}", shader_type_org, shader_type);
                    let entry = tmp_pipelines
                        .entry(shader_type)
                        .or_insert(TmpPipelineEntry::default());
                    entry.mesh_indices.push(i);
                    let material_id = material.get_id();
                    let new_material_index = entry.material_indices_map.len();
                    if let Some((material_index, _)) = entry.material_indices_map.get(&material_id)
                    {
                        entry.material_indices.push(*material_index);
                    } else {
                        entry.material_indices.push(new_material_index);
                        entry
                            .material_indices_map
                            .insert(material_id, (new_material_index, material.clone()));
                    }
                    entry.has_lighting |= has_lighting;
                }
            }

            for (shader_type, tmp_entry) in tmp_pipelines.iter() {
                let mesh_indices = &tmp_entry.mesh_indices;
                let material_indices = &tmp_entry.material_indices;
                let has_lighting = tmp_entry.has_lighting;
                let shader_id = get_shader_id_from_type(shader_type);
                let num_materials = tmp_entry.material_indices_map.len();
                if !self.pipelines.contains_key(&shader_id) {
                    let material_uniform_size = get_material_uniform_size(shader_type);
                    let shader = get_shader(device, shader_type);
                    self.pipelines.insert(
                        shader_id,
                        self.create_pipeline(
                            device,
                            queue,
                            &shader,
                            has_lighting,
                            material_uniform_size,
                            num_materials,
                        ),
                    );
                }
                let entry = self
                    .pipelines
                    .get_mut(&shader_id)
                    .expect("Pipeline for basic material not found");
                entry.allocate_material_uniform_buffer(device, material_indices.len());
                entry.mesh_indices = mesh_indices.clone();
                entry.material_indices = material_indices.clone();
                assert!(entry.mesh_indices.len() == entry.material_indices.len());
                //let length = entry.mesh_indices.len();
                for (_id, (material_index, material)) in tmp_entry.material_indices_map.iter() {
                    let material_index = *material_index;
                    let material_uniform_buffer = get_material_uniform_buffer(material.as_ref());
                    Self::bind_pipeline(
                        device,
                        queue,
                        material_index,
                        entry,
                        &material_uniform_buffer,
                    );
                }
            }
        }
        self.mesh_items = mesh_items.to_vec();
    }

    fn create_light_bind_group(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        light_texture_view: &wgpu::TextureView,
        light_sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        let default_ltc_texture = self
            .textures
            .get(&DEFAULT_LTC_UUID)
            .expect("Default LTC texture not found");
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
            wgpu::BindGroupEntry {
                binding: 8,
                resource: wgpu::BindingResource::TextureView(&default_ltc_texture.view),
            },
            wgpu::BindGroupEntry {
                binding: 9,
                resource: wgpu::BindingResource::Sampler(&default_ltc_texture.sampler),
            },
        ];
        let light_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Lighting Light Bind Group"),
            layout: layout,
            entries: &entries,
        });
        return light_bind_group;
    }

    fn prepare_lights(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        render_items: &[Arc<RenderItem>],
    ) {
        let mut light_uniforms = LightUniforms::default();
        let mut light_textures = Vec::new();
        // Point lights
        {
            let mut light_buffer = Vec::new();
            for item in render_items.iter() {
                if let RenderItem::Light(light_item) = item.as_ref() {
                    if let RenderItem::Light(item) = item.as_ref() {
                        if let RenderLight::Sphere(light) = item.light.as_ref() {
                            if light_buffer.len() >= MAX_SPHERE_LIGHT_NUM {
                                break;
                            }
                            let matrix = light_item.matrix; //local_to_world
                            let position = light.position;
                            let position = matrix.transform_point3(glam::vec3(
                                position[0],
                                position[1],
                                position[2],
                            ));
                            //println!("Point light position: {:?}", position);
                            let intensity = light.intensity;
                            let radius = light.radius;
                            let light = SphereLight {
                                position: [position.x, position.y, position.z, 1.0],
                                intensity: [intensity[0], intensity[1], intensity[2], 1.0],
                                radius: radius,
                                ..Default::default()
                            };
                            light_buffer.push(light);
                        }
                    }
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
            for item in render_items.iter() {
                if let RenderItem::Light(light_item) = item.as_ref() {
                    if let RenderLight::Disk(light) = light_item.light.as_ref() {
                        if light_buffer.len() >= MAX_DISK_LIGHT_NUM {
                            break;
                        }
                        let matrix = light_item.matrix; //local_to_world
                        let position = light.position;
                        let position = matrix.transform_point3(glam::vec3(
                            position[0],
                            position[1],
                            position[2],
                        ));
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
                            radius: radius,
                            cos_inner_angle: cos_inner_angle,
                            cos_outer_angle: cos_outer_angle,
                            u_axis: [u_axis.x, u_axis.y, u_axis.z, 0.0],
                            v_axis: [v_axis.x, v_axis.y, v_axis.z, 0.0],
                            twosided: if light.twosided { 1 } else { 0 },
                            ..Default::default()
                        };
                        light_buffer.push(light);
                    }
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
            for item in render_items.iter() {
                if let RenderItem::Light(light_item) = item.as_ref() {
                    if let RenderLight::Rect(rect) = light_item.light.as_ref() {
                        //println!("Rect light item: {:?}", item);
                        if light_buffer.len() >= MAX_RECT_LIGHT_NUM {
                            break;
                        }
                        let matrix = light_item.matrix; //local_to_world
                        let position = rect.position;
                        let position = matrix.transform_point3(glam::vec3(
                            position[0],
                            position[1],
                            position[2],
                        ));
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
            for item in render_items.iter() {
                if let RenderItem::Light(light_item) = item.as_ref() {
                    if let RenderLight::Infinite(light) = light_item.light.as_ref() {
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
            for item in render_items.iter() {
                if let RenderItem::Light(light_item) = item.as_ref() {
                    if let RenderLight::Directional(light) = light_item.light.as_ref() {
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
                        let light = DirectionalLight {
                            direction: [direction[0], direction[1], direction[2], 0.0],
                            intensity: [intensity[0], intensity[1], intensity[2], 1.0],
                        };
                        light_buffer.push(light);
                    }
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
        has_lighting: bool,
        material_uniform_size: usize,
        num_materials: usize,
    ) -> PipelineEntry {
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

        let material_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Material Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(material_uniform_size as _),
                    },
                    count: None,
                }],
            });

        let mut bind_group_layouts = vec![
            &self.global_bind_group_layout, //group(0)
            &self.local_bind_group_layout,  //group(1)
            &material_bind_group_layout,    //group(2)
        ];
        if has_lighting {
            bind_group_layouts.push(&self.light_bind_group_layout); //group(3)
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

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Lighting Pipeline"),
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
                //targets: &[Some(wgpu_render_state.target_format.into())],
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_texture_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: primitive,
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_texture_format,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Create a uniform buffer for material properties
        let material_uniform_alignment = {
            let alignment =
                device.limits().min_uniform_buffer_offset_alignment as wgpu::BufferAddress;
            align_to(material_uniform_size as wgpu::BufferAddress, alignment)
        };
        let required_size = material_uniform_alignment * num_materials as wgpu::BufferAddress;
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Item Matrices Buffer"),
            size: required_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });

        let material_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Lighting Local Bind Group"),
            layout: &material_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniform_buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(material_uniform_size as _),
                }),
            }],
        });

        let entry = PipelineEntry {
            pipeline,
            material_bind_group_layout,
            material_bind_group,
            uniform_buffer,
            material_uniform_size,
            material_uniform_alignment,
            mesh_indices: Vec::new(),
            material_indices: Vec::new(),
            has_lighting: has_lighting,
        };
        return entry;
    }

    fn bind_pipeline(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        i: usize,
        pipeline: &PipelineEntry,
        material_uniform_buffer: &[u8],
    ) {
        let uniform_alignment = {
            let alignment =
                device.limits().min_uniform_buffer_offset_alignment as wgpu::BufferAddress;
            align_to(
                material_uniform_buffer.len() as wgpu::BufferAddress,
                alignment,
            )
        };
        let offset = i as wgpu::BufferAddress * uniform_alignment;
        queue.write_buffer(&pipeline.uniform_buffer, offset, material_uniform_buffer);
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
                    wgpu::BindGroupLayoutEntry {
                        binding: 8,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 9,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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
        };
        let default_light_texture = Arc::new(default_light_texture);

        let default_ltc_texture = create_default_ltc_texture(device, queue);
        let default_ltc_texture = Arc::new(default_ltc_texture);

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
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(&default_ltc_texture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::Sampler(&default_ltc_texture.sampler),
                },
            ],
        });

        let mut textures = HashMap::new();
        textures.insert(default_light_texture.id, default_light_texture);
        textures.insert(default_ltc_texture.id, default_ltc_texture);

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
            mesh_items,
            textures,
            pipelines: materials,
        };
        pass.init(device, queue);
        return pass;
    }

    pub fn init(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.pipelines.is_empty() {
            let shader_type = BASIC_SURFACE_SHADER_NAME;
            let render_category = RenderCategory::Opaque;
            let shader_id = get_shader_id_from_type(shader_type);
            let uniform_size = get_material_uniform_size(shader_type);
            let shader = get_shader(device, shader_type);
            let has_lighting = get_shader_has_lighting(shader_type, render_category);
            self.pipelines.insert(
                shader_id,
                self.create_pipeline(
                    device,
                    queue,
                    &shader,
                    has_lighting,
                    uniform_size,
                    MIN_MATERIAL_BUFFER_NUM,
                ),
            );
        }
    }
}
