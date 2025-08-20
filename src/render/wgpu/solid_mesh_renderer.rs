use super::material::RenderUniformValue;
use super::mesh::RenderVertex;
use super::render_item::RenderItem;
use std::sync::Arc;

use eframe::egui;
use eframe::egui_wgpu;
use eframe::wgpu;
use eframe::wgpu::util::DeviceExt;
use wgpu::util::align_to;

use bytemuck::{Pod, Zeroable};

const MIN_LOCAL_BUFFER_NUM: usize = 64;

//pub struct SolidMeshRenderer {}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct GlobalUniforms {
    world_to_camera: [[f32; 4]; 4], // 4 * 4 * 4 = 64
    camera_to_clip: [[f32; 4]; 4],  // 4 * 4 * 4 = 64
    camera_to_world: [[f32; 4]; 4], // 4 * 4 * 4 = 64
    light_direction: [f32; 4],
    light_intensity: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct LocalUniforms {
    local_to_world: [[f32; 4]; 4], // 4 * 4 * 4 = 64
    world_to_local: [[f32; 4]; 4], // 4 * 4 * 4 = 64
    base_color: [f32; 4],          // Base color for the mesh
    _pad1: [f32; 4],               // Padding to ensure alignment
    _pad2: [f32; 4],               // Padding to ensure alignment
    _pad3: [f32; 4],               // Padding to ensure alignment
}

#[derive(Debug, Clone)]
pub struct SolidMeshRenderer {
    pipeline: wgpu::RenderPipeline,
    #[allow(dead_code)]
    global_bind_group_layout: wgpu::BindGroupLayout,
    global_bind_group: wgpu::BindGroup,
    global_uniform_buffer: wgpu::Buffer,
    local_bind_group_layout: wgpu::BindGroupLayout,
    local_bind_group: wgpu::BindGroup,
    local_uniform_buffer: wgpu::Buffer,
    local_uniform_alignment: wgpu::BufferAddress,
}

#[derive(Debug, Clone)]
struct PerFrameResources {
    render_items: Vec<Arc<RenderItem>>,
}

fn create_local_uniform_buffer(device: &wgpu::Device, num_items: usize) -> wgpu::Buffer {
    let local_uniform_size = std::mem::size_of::<LocalUniforms>() as wgpu::BufferAddress; // 4x4 matrix
    let uniform_alignment = {
        let alignment = device.limits().min_uniform_buffer_offset_alignment as wgpu::BufferAddress;
        //let alignment = 64;
        align_to(local_uniform_size, alignment)
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
    return [1.0, 0.0, 1.0, 1.0]; // Default color for Solid
}

impl SolidMeshRenderer {
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
        render_items: &[Arc<RenderItem>],
        world_to_camera: &glam::Mat4,
        camera_to_clip: &glam::Mat4,
    ) -> Vec<wgpu::CommandBuffer> {
        let num_items = render_items.len();
        if num_items != 0 {
            {
                let local_uniform_alignment = self.local_uniform_alignment;
                if self.local_uniform_buffer.size()
                    < (num_items as wgpu::BufferAddress * local_uniform_alignment)
                {
                    let new_buffer = create_local_uniform_buffer(device, num_items);
                    let new_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Solid Local Bind Group"),
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
                for (i, item) in render_items.iter().enumerate() {
                    let local_to_world = item.get_matrix();
                    let world_to_local = local_to_world.inverse();
                    let uniform = LocalUniforms {
                        local_to_world: local_to_world.to_cols_array_2d(),
                        world_to_local: world_to_local.to_cols_array_2d(),
                        base_color: get_base_color(item),
                        _pad1: [0.0; 4], // Padding to ensure alignment
                        _pad2: [0.0; 4], // Padding to ensure alignment
                        _pad3: [0.0; 4], // Padding to ensure alignment
                    };
                    let offset = i as wgpu::BufferAddress * local_uniform_alignment;
                    queue.write_buffer(
                        &self.local_uniform_buffer,
                        offset,
                        bytemuck::bytes_of(&uniform),
                    );
                }
            }

            {
                let camera_to_world = world_to_camera.inverse();
                let global_uniforms = GlobalUniforms {
                    world_to_camera: world_to_camera.to_cols_array_2d(), // Identity matrix for now
                    camera_to_clip: camera_to_clip.to_cols_array_2d(),   // Identity matrix for now
                    camera_to_world: camera_to_world.to_cols_array_2d(), // Identity matrix for now
                    light_direction: [0.0, 0.0, -1.0, 0.0],              // Default light direction
                    light_intensity: [1.0, 1.0, 1.0, 1.0],               // Default light intensity
                };
                let global_unifrom_buffer = &self.global_uniform_buffer;
                queue.write_buffer(
                    global_unifrom_buffer,
                    0,
                    bytemuck::bytes_of(&global_uniforms),
                );
            }
        }

        let per_frame_resources = PerFrameResources {
            render_items: render_items.to_vec(),
        };
        resources.insert(per_frame_resources);

        return vec![];
    }

    pub fn paint(
        &self,
        _info: &egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(per_frame_resources) = resources.get::<PerFrameResources>() {
            if !per_frame_resources.render_items.is_empty() {
                let local_uniform_alignment = self.local_uniform_alignment;
                render_pass.set_pipeline(&self.pipeline); //
                render_pass.set_bind_group(0, &self.global_bind_group, &[]);
                for (i, item) in per_frame_resources.render_items.iter().enumerate() {
                    let i = i as wgpu::DynamicOffset;
                    if let RenderItem::Mesh(mesh_item) = item.as_ref() {
                        let local_uniform_offset =
                            i * local_uniform_alignment as wgpu::DynamicOffset;
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
    }
}

impl SolidMeshRenderer {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Solid Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/render_solid_mesh.wgsl").into()),
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Solid Pipeline Layout"),
            bind_group_layouts: &[&global_bind_group_layout, &local_bind_group_layout],
            push_constant_ranges: &[],
        });

        let primitive = wgpu::PrimitiveState {
            //front_face: wgpu::FrontFace::Ccw,
            //cull_mode: Some(wgpu::Face::Back),
            topology: wgpu::PrimitiveTopology::TriangleList,
            polygon_mode: wgpu::PolygonMode::Fill,
            ..Default::default()
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Solid Pipeline"),
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
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: primitive,
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let global_unifroms = GlobalUniforms {
            world_to_camera: glam::Mat4::IDENTITY.to_cols_array_2d(), // Identity matrix for now
            camera_to_clip: glam::Mat4::IDENTITY.to_cols_array_2d(),  // Identity matrix for now
            camera_to_world: glam::Mat4::IDENTITY.to_cols_array_2d(), // Identity matrix for now
            light_direction: [0.0, 0.0, -1.0, 0.0],                   // Default light direction
            light_intensity: [1.0, 1.0, 1.0, 1.0],                    // Default light intensity
        };
        let global_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer for Matrix"),
            contents: bytemuck::bytes_of(&global_unifroms),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
        });

        let global_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Solid Global Bind Group"),
            layout: &global_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: global_uniform_buffer.as_entire_binding(),
            }],
        });

        let local_uniform_size = size_of::<LocalUniforms>() as wgpu::BufferAddress;
        let local_uniform_alignment = {
            let alignment =
                device.limits().min_uniform_buffer_offset_alignment as wgpu::BufferAddress;
            //let alignment = 64;
            align_to(local_uniform_size, alignment)
        };

        let local_uniform_buffer = create_local_uniform_buffer(device, MIN_LOCAL_BUFFER_NUM);
        let local_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Solid Local Bind Group"),
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

        return SolidMeshRenderer {
            pipeline,
            global_bind_group_layout,
            global_bind_group,
            global_uniform_buffer,
            local_bind_group_layout,
            local_bind_group,
            local_uniform_buffer,
            local_uniform_alignment,
        };
    }
}
