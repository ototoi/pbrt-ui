use crate::render::wgpu::material::RenderUniformValue;

use super::lines::RenderLinesVertex;
use super::render_item::RenderItem;
use std::sync::Arc;

use eframe::egui;
use eframe::egui_wgpu;
use eframe::wgpu;
use eframe::wgpu::util::DeviceExt;
use wgpu::util::align_to;

use bytemuck::{Pod, Zeroable};

const MIN_LOCAL_BUFFER_NUM: usize = 64;

//pub struct LinesMeshRenderer {}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct GlobalUniforms {
    world_to_camera: [[f32; 4]; 4],
    camera_to_clip: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct LocalUniforms {
    local_to_world: [[f32; 4]; 4],
    base_color: [f32; 4], // RGBA
}

#[derive(Debug, Clone)]
pub struct LinesRenderer {
    pipeline: wgpu::RenderPipeline,
    #[allow(dead_code)]
    global_bind_group_layout: wgpu::BindGroupLayout,
    global_bind_group: wgpu::BindGroup,
    global_uniform_buffer: wgpu::Buffer,
    local_bind_group_layout: wgpu::BindGroupLayout,
    local_bind_group: wgpu::BindGroup,
    local_uniform_buffer: wgpu::Buffer,
    local_uniform_alignment: wgpu::BufferAddress,
    //
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

impl LinesRenderer {
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        render_items: &[Arc<RenderItem>],
        world_to_camera: &glam::Mat4,
        camera_to_clip: &glam::Mat4,
    ) {
        let render_items = render_items
            .iter()
            .filter(|item| matches!(item.as_ref(), RenderItem::Lines(_)))
            .cloned()
            .collect::<Vec<_>>();
        let num_items = render_items.len();
        {
            let local_uniform_alignment = self.local_uniform_alignment;
            if self.local_uniform_buffer.size()
                < (num_items as wgpu::BufferAddress * local_uniform_alignment)
            {
                let new_buffer = create_local_uniform_buffer(device, num_items);
                let new_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Lines Local Bind Group"),
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
                let matrix = item.get_matrix();

                let mut base_color = [1.0, 1.0, 0.0, 1.0]; // Default color for Lines
                if let RenderItem::Lines(line_item) = item.as_ref() {
                    if let Some(material) = line_item.material.as_ref() {
                        if let Some(value) = material.get_uniform_value("base_color") {
                            if let RenderUniformValue::Vec4(color) = value {
                                base_color = [
                                    color[0] as f32,
                                    color[1] as f32,
                                    color[2] as f32,
                                    color[3] as f32,
                                ];
                            }
                        }
                    }
                }

                let uniform = LocalUniforms {
                    local_to_world: matrix.to_cols_array_2d(),
                    base_color,
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
            let global_uniforms = GlobalUniforms {
                world_to_camera: world_to_camera.to_cols_array_2d(), // Identity matrix for now
                camera_to_clip: camera_to_clip.to_cols_array_2d(),   // Identity matrix for now
            };
            let global_unifrom_buffer = &self.global_uniform_buffer;
            queue.write_buffer(
                global_unifrom_buffer,
                0,
                bytemuck::bytes_of(&global_uniforms),
            );
        }

        self.render_items = render_items.to_vec();
    }

    pub fn paint(
        &self,
        render_pass: &mut wgpu::RenderPass,
    ) {
        if !self.render_items.is_empty() {
            let local_uniform_alignment = self.local_uniform_alignment;
            render_pass.set_pipeline(&self.pipeline); //
            render_pass.set_bind_group(0, &self.global_bind_group, &[]);
            for (i, item) in self.render_items.iter().enumerate() {
                let i = i as wgpu::DynamicOffset;
                if let RenderItem::Lines(line_item) = item.as_ref() {
                    let local_uniform_offset = i * local_uniform_alignment as wgpu::DynamicOffset;
                    render_pass.set_bind_group(1, &self.local_bind_group, &[local_uniform_offset]);
                    render_pass.set_vertex_buffer(0, line_item.lines.vertex_buffer.slice(..));
                    render_pass.draw(0..line_item.lines.vertex_count, 0..1);
                }
            }
        }
    }
}

impl LinesRenderer {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Lines Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/render_lines.wgsl").into()),
        });

        let vertex_buffer_layout = [wgpu::VertexBufferLayout {
            array_stride: size_of::<RenderLinesVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            }],
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
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(size_of::<LocalUniforms>() as _),
                    },
                    count: None,
                }],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Lines Pipeline Layout"),
            bind_group_layouts: &[&global_bind_group_layout, &local_bind_group_layout],
            push_constant_ranges: &[],
        });

        let primitive = wgpu::PrimitiveState {
            cull_mode: None,
            topology: wgpu::PrimitiveTopology::LineList,
            polygon_mode: wgpu::PolygonMode::Line,
            ..Default::default()
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Lines Pipeline"),
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
                targets: &[Some(target_format.into())],
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
        };
        let global_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer for Matrix"),
            contents: bytemuck::bytes_of(&global_unifroms),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
        });

        let global_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Lines Global Bind Group"),
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
            label: Some("Lines Local Bind Group"),
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

        let render_items = Vec::new();

        return LinesRenderer {
            pipeline,
            global_bind_group_layout,
            global_bind_group,
            global_uniform_buffer,
            local_bind_group_layout,
            local_bind_group,
            local_uniform_buffer,
            local_uniform_alignment,
            render_items,
        };
    }
}
