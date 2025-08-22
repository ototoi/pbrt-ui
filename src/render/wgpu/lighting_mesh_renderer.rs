use crate::render::wgpu::light::RenderLightType;

use super::material::RenderUniformValue;
use super::mesh::RenderVertex;
use super::render_item::RenderItem;
use std::collections::HashMap;
use std::sync::Arc;

//use eframe::egui;
//use eframe::egui_wgpu;
use eframe::wgpu;
use eframe::wgpu::util::DeviceExt;
use wgpu::util::align_to;

use bytemuck::{Pod, Zeroable};

const MIN_LOCAL_BUFFER_NUM: usize = 64;
const MAX_DIRECTIONAL_LIGHT_NUM: usize = 4; // Maximum number of directional lights
const MAX_SPHERE_LIGHT_NUM: usize = 256; // Maximum number of point lights
const MAX_DISK_LIGHT_NUM: usize = 32; // Maximum number of spot lights
const TEST_PIPELINE_ID: &str = "basic_pipeline";

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
    base_color: [f32; 4],          // Base color for the mesh
    _pad1: [f32; 4],               // Padding to ensure alignment
    _pad2: [f32; 4],               // Padding to ensure alignment
    _pad3: [f32; 4],               // Padding to ensure alignment
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod, Zeroable)]
struct LightUniforms {
    num_directional_lights: u32, // Number of directional lights
    num_sphere_lights: u32,      // Number of point lights
    num_disk_lights: u32,        // Number of spot lights
    _pad1: u32,                  // Padding to ensure alignment
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
    position: [f32; 4],  // Position of the light // 4 * 4 = 16
    direction: [f32; 4], // Direction of the light // 4 * 4 = 16
    intensity: [f32; 4], // Intensity of the light // 4 * 4 = 16
    range: [f32; 4],     // Range of the light // 2 * 4 = 8
    outer_angle: f32,    // Angle of the spotlight
    inner_angle: f32,    // Angle of the spotlight
    _pad1: [f32; 2],     // Padding to ensure alignmentå
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
    point_light_buffer: wgpu::Buffer,
    spot_light_buffer: wgpu::Buffer,
    // Mesh items to render
    mesh_items: Vec<Arc<RenderItem>>,
    //
    pipelines: HashMap<String, wgpu::RenderPipeline>,
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

fn get_base_color(item: &RenderItem) -> [f32; 4] {
    match item {
        RenderItem::Mesh(mesh_item) => {
            if let Some(material) = &mesh_item.material {
                // Assuming the material has a base color property
                if let Some(value) = material.get_uniform_value("base_color") {
                    if let RenderUniformValue::Vec4(color) = value {
                        return *color;
                    }
                }
            }
        }
        _ => {} // Default color for other items
    }
    return [1.0, 1.0, 1.0, 1.0]; // Default color for 
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
        self.prepare_global(device, queue, world_to_camera, camera_to_clip);
        self.prepare_lights(device, queue, render_items);
        self.prepare_local(device, queue, render_items);
    }

    pub fn paint(&self, render_pass: &mut wgpu::RenderPass) {
        if !self.mesh_items.is_empty() {
            self.render(render_pass, &self.mesh_items);
        }
    }

    fn render(&self, render_pass: &mut wgpu::RenderPass, render_items: &[Arc<RenderItem>]) {
        let local_uniform_alignment = {
            let alignment = self.min_uniform_buffer_offset_alignment;
            align_to(
                std::mem::size_of::<LocalUniforms>() as wgpu::BufferAddress,
                alignment,
            )
        };
        let pipeline = self
            .pipelines
            .get(TEST_PIPELINE_ID)
            .expect("Pipeline for basic material not found");
        render_pass.set_pipeline(pipeline); //
        render_pass.set_bind_group(0, &self.global_bind_group, &[]);
        render_pass.set_bind_group(2, &self.light_bind_group, &[]);
        for (i, item) in render_items.iter().enumerate() {
            let i = i as wgpu::DynamicOffset;
            if let RenderItem::Mesh(mesh_item) = item.as_ref() {
                let local_uniform_offset = i * local_uniform_alignment as wgpu::DynamicOffset;
                render_pass.set_bind_group(1, &self.local_bind_group, &[local_uniform_offset]);
                render_pass.set_vertex_buffer(0, mesh_item.mesh.vertex_buffer.slice(..));
                render_pass.set_index_buffer(
                    mesh_item.mesh.index_buffer.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                render_pass.draw_indexed(0..mesh_item.mesh.index_count, 0, 0..1);
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

    pub fn prepare_local(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        render_items: &[Arc<RenderItem>],
    ) {
        let mesh_items = render_items
            .iter()
            .filter(|item| matches!(item.as_ref(), RenderItem::Mesh(_)))
            .cloned()
            .collect::<Vec<_>>();
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
            let base_color = get_base_color(item);
            let uniform = LocalUniforms {
                local_to_world: local_to_world.to_cols_array_2d(),
                world_to_local: world_to_local.to_cols_array_2d(),
                base_color: base_color,
                _pad1: [0.0; 4],
                _pad2: [0.0; 4],
                _pad3: [0.0; 4],
            };
            let offset = i as wgpu::BufferAddress * local_uniform_alignment;
            queue.write_buffer(
                &self.local_uniform_buffer,
                offset,
                bytemuck::bytes_of(&uniform),
            );
        }
        //for (i, item) in mesh_items.iter().enumerate() {
        //
        //}
        {
            if !self.pipelines.contains_key(TEST_PIPELINE_ID) {
                self.pipelines.insert(
                    TEST_PIPELINE_ID.to_string(),
                    self.create_pipeline(device, TEST_PIPELINE_ID),
                );
            }
        }

        self.mesh_items = mesh_items;
    }

    fn prepare_lights(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        render_items: &[Arc<RenderItem>],
    ) {
        let mut light_uniforms = LightUniforms::default();
        // Point lights
        {
            let light_items = render_items
                .iter()
                .filter(|item| {
                    if let RenderItem::Light(item) = item.as_ref() {
                        if item.light.light_type == RenderLightType::Point
                            || item.light.light_type == RenderLightType::Sphere
                        {
                            return true;
                        }
                    }
                    return false;
                })
                .cloned()
                .collect::<Vec<_>>();
            let num_light_items = light_items.len();
            light_uniforms.num_sphere_lights = num_light_items as u32;
            for (i, item) in light_items.iter().enumerate() {
                //println!("Point light item: {:?}", item);
                if let RenderItem::Light(light_item) = item.as_ref() {
                    if i < MAX_SPHERE_LIGHT_NUM {
                        let matrix = light_item.matrix; //local_to_world
                        let position = light_item.light.position;
                        let position = matrix.transform_point3(glam::vec3(
                            position[0],
                            position[1],
                            position[2],
                        ));
                        //println!("Point light position: {:?}", position);
                        let intensity = light_item.light.intensity;
                        let range = light_item.light.range;
                        let light = SphereLight {
                            position: [position.x, position.y, position.z, 1.0],
                            intensity: [intensity[0], intensity[1], intensity[2], 1.0],
                            radius: range[0],
                            range: range[1],
                            ..Default::default()
                        };
                        queue.write_buffer(
                            &self.point_light_buffer,
                            (i * size_of::<SphereLight>()) as wgpu::BufferAddress,
                            bytemuck::bytes_of(&light),
                        );
                    }
                }
            }
        }

        // Spot lights
        {
            let light_items = render_items
                .iter()
                .filter(|item| {
                    if let RenderItem::Light(item) = item.as_ref() {
                        if item.light.light_type == RenderLightType::Spot
                            || item.light.light_type == RenderLightType::Disk
                        {
                            return true;
                        }
                    }
                    return false;
                })
                .cloned()
                .collect::<Vec<_>>();
            let num_light_items = light_items.len();
            light_uniforms.num_disk_lights = num_light_items as u32;
            for (i, item) in light_items.iter().enumerate() {
                //println!("Point light item: {:?}", item);
                if let RenderItem::Light(light_item) = item.as_ref() {
                    if i < MAX_DISK_LIGHT_NUM {
                        let matrix = light_item.matrix; //local_to_world
                        let position = light_item.light.position;
                        let position = matrix.transform_point3(glam::vec3(
                            position[0],
                            position[1],
                            position[2],
                        ));
                        let direction = light_item.light.direction;
                        let direction = matrix.transform_vector3(glam::vec3(
                            direction[0],
                            direction[1],
                            direction[2],
                        ));
                        //println!("Point light position: {:?}", position);
                        let intensity = light_item.light.intensity;
                        let range = light_item.light.range;
                        let light = DiskLight {
                            position: [position.x, position.y, position.z, 1.0],
                            direction: [direction.x, direction.y, direction.z, 0.0],
                            intensity: [intensity[0], intensity[1], intensity[2], 1.0],
                            range: [range[0], range[1], 0.0, 0.0], // min and max range
                            outer_angle: light_item.light.angle[1],
                            inner_angle: light_item.light.angle[0],
                            ..Default::default()
                        };
                        queue.write_buffer(
                            &self.spot_light_buffer,
                            (i * size_of::<DiskLight>()) as wgpu::BufferAddress,
                            bytemuck::bytes_of(&light),
                        );
                    }
                }
            }
        }

        // Directional lights
        {
            let light_items = render_items
                .iter()
                .filter(|item| {
                    if let RenderItem::Light(item) = item.as_ref() {
                        if item.light.light_type == RenderLightType::Directional {
                            return true;
                        }
                    }
                    return false;
                })
                .cloned()
                .collect::<Vec<_>>();
            let num_light_items = light_items.len();
            light_uniforms.num_directional_lights = num_light_items as u32;
            for (i, item) in light_items.iter().enumerate() {
                if let RenderItem::Light(light_item) = item.as_ref() {
                    if i < MAX_DIRECTIONAL_LIGHT_NUM {
                        let matrix = light_item.matrix; //local_to_world
                        let direction = light_item.light.direction;
                        let direction = matrix.transform_vector3(glam::vec3(
                            direction[0],
                            direction[1],
                            direction[2],
                        ));
                        let intensity = light_item.light.intensity;
                        let light = DirectionalLight {
                            direction: [direction[0], direction[1], direction[2], 0.0],
                            intensity: [intensity[0], intensity[1], intensity[2], 1.0],
                        };
                        queue.write_buffer(
                            &self.directional_light_buffer,
                            (i * size_of::<DirectionalLight>()) as wgpu::BufferAddress,
                            bytemuck::bytes_of(&light),
                        );
                    }
                }
            }
        }

        queue.write_buffer(
            &self.light_uniform_buffer,
            0,
            bytemuck::bytes_of(&light_uniforms),
        );
    }

    pub fn create_pipeline(
        &self,
        device: &wgpu::Device,
        _material_id: &str,
    ) -> wgpu::RenderPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Lighting Shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("shaders/render_lighting_mesh.wgsl").into(),
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
            label: Some("Lighting Pipeline Layout"),
            bind_group_layouts: &[
                &self.global_bind_group_layout,
                &self.local_bind_group_layout,
                &self.light_bind_group_layout,
            ],
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
        return pipeline;
    }
}

impl LightingMeshRenderer {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
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
        let directional_light_buffer_size =
            (MAX_DIRECTIONAL_LIGHT_NUM * size_of::<DirectionalLight>()) as wgpu::BufferAddress;
        let light_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Light Bind Group Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
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
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
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
                        binding: 2,
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
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
                        binding: 3,
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage {
                                read_only: true,
                            },
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(directional_light_buffer_size),
                        },
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

        let point_light_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Buffer for Point Lights"),
            size: point_light_buffer_size,
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
        });
        let spot_light_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Buffer for Spot Lights"),
            size: spot_light_buffer_size,
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
        });
        let directional_light_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Buffer for Point Lights"),
            size: directional_light_buffer_size,
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
        });

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
                    resource: point_light_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: spot_light_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: directional_light_buffer.as_entire_binding(),
                },
            ],
        });

        let min_uniform_buffer_offset_alignment =
            device.limits().min_uniform_buffer_offset_alignment as wgpu::BufferAddress;

        let mesh_items = Vec::with_capacity(MIN_LOCAL_BUFFER_NUM);
        let pipelines = HashMap::new();
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
            point_light_buffer,
            spot_light_buffer,
            mesh_items,
            pipelines,
        };
        pass.init(device);
        return pass;
    }

    pub fn init(&mut self, device: &wgpu::Device) {
        if self.pipelines.is_empty() {
            self.pipelines.insert(
                TEST_PIPELINE_ID.to_string(),
                self.create_pipeline(device, TEST_PIPELINE_ID),
            );
        }
    }
}
