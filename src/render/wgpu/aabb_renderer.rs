use super::lines::RenderLinesVertex;
use super::render_item::RenderItem;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use eframe::wgpu::util::DeviceExt;
use wgpu::util::align_to;

const MIN_LOCAL_BUFFER_NUM: usize = 64;

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
}

#[derive(Debug, Clone)]
pub struct AabbRenderer {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
    #[allow(dead_code)]
    global_bind_group_layout: wgpu::BindGroupLayout,
    global_bind_group: wgpu::BindGroup,
    global_uniform_buffer: wgpu::Buffer,
    local_bind_group_layout: wgpu::BindGroupLayout,
    local_bind_group: wgpu::BindGroup,
    local_uniform_buffer: wgpu::Buffer,
    local_uniform_alignment: wgpu::BufferAddress,
    local_matrices: Vec<glam::Mat4>,
}

fn create_local_uniform_buffer(device: &wgpu::Device, num_items: usize) -> wgpu::Buffer {
    let local_uniform_size = std::mem::size_of::<LocalUniforms>() as wgpu::BufferAddress;
    let uniform_alignment = {
        let alignment = device.limits().min_uniform_buffer_offset_alignment as wgpu::BufferAddress;
        align_to(local_uniform_size, alignment)
    };
    let required_size = uniform_alignment * num_items as wgpu::BufferAddress;
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("AABB Local Uniform Buffer"),
        size: required_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
        mapped_at_creation: false,
    })
}

fn create_box_line_vertices() -> Vec<RenderLinesVertex> {
    let p000 = [-1.0, -1.0, -1.0];
    let p001 = [-1.0, -1.0, 1.0];
    let p010 = [-1.0, 1.0, -1.0];
    let p011 = [-1.0, 1.0, 1.0];
    let p100 = [1.0, -1.0, -1.0];
    let p101 = [1.0, -1.0, 1.0];
    let p110 = [1.0, 1.0, -1.0];
    let p111 = [1.0, 1.0, 1.0];

    let edges = [
        (p000, p001),
        (p001, p011),
        (p011, p010),
        (p010, p000),
        (p100, p101),
        (p101, p111),
        (p111, p110),
        (p110, p100),
        (p000, p100),
        (p001, p101),
        (p011, p111),
        (p010, p110),
    ];

    let mut vertices = Vec::with_capacity(edges.len() * 2);
    for (start, end) in edges {
        vertices.push(RenderLinesVertex { position: start });
        vertices.push(RenderLinesVertex { position: end });
    }
    vertices
}

fn create_aabb_local_transform(min: [f32; 3], max: [f32; 3]) -> glam::Mat4 {
    let min = glam::vec3(min[0], min[1], min[2]);
    let max = glam::vec3(max[0], max[1], max[2]);
    let center = (min + max) * 0.5;
    let half_extent = (max - min) * 0.5;
    let epsilon = 1e-5;
    let safe_half_extent = glam::vec3(
        half_extent.x.max(epsilon),
        half_extent.y.max(epsilon),
        half_extent.z.max(epsilon),
    );
    glam::Mat4::from_scale_rotation_translation(safe_half_extent, glam::Quat::IDENTITY, center)
}

impl AabbRenderer {
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        render_items: &[Arc<RenderItem>],
        world_to_camera: &glam::Mat4,
        camera_to_clip: &glam::Mat4,
    ) {
        let mesh_items = render_items
            .iter()
            .filter_map(|item| match item.as_ref() {
                RenderItem::Mesh(mesh_item) => Some(mesh_item),
                _ => None,
            })
            .collect::<Vec<_>>();

        let num_items = mesh_items.len();
        self.local_matrices.clear();
        self.local_matrices.reserve(num_items);

        let local_uniform_alignment = self.local_uniform_alignment;
        if self.local_uniform_buffer.size()
            < (num_items as wgpu::BufferAddress * local_uniform_alignment)
        {
            let new_buffer = create_local_uniform_buffer(device, num_items);
            let new_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("AABB Local Bind Group"),
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

        for (i, mesh_item) in mesh_items.iter().enumerate() {
            let local =
                create_aabb_local_transform(mesh_item.mesh.aabb.min, mesh_item.mesh.aabb.max);
            let local_to_world = mesh_item.matrix * local;
            self.local_matrices.push(local_to_world);

            let uniform = LocalUniforms {
                local_to_world: local_to_world.to_cols_array_2d(),
            };
            let offset = i as wgpu::BufferAddress * local_uniform_alignment;
            queue.write_buffer(
                &self.local_uniform_buffer,
                offset,
                bytemuck::bytes_of(&uniform),
            );
        }

        let global_uniforms = GlobalUniforms {
            world_to_camera: world_to_camera.to_cols_array_2d(),
            camera_to_clip: camera_to_clip.to_cols_array_2d(),
        };
        queue.write_buffer(
            &self.global_uniform_buffer,
            0,
            bytemuck::bytes_of(&global_uniforms),
        );
    }

    pub fn paint(&self, render_pass: &mut wgpu::RenderPass) {
        if self.local_matrices.is_empty() {
            return;
        }
        let local_uniform_alignment = self.local_uniform_alignment;
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.global_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        for i in 0..self.local_matrices.len() {
            let local_uniform_offset =
                i as wgpu::DynamicOffset * local_uniform_alignment as wgpu::DynamicOffset;
            render_pass.set_bind_group(1, &self.local_bind_group, &[local_uniform_offset]);
            render_pass.draw(0..self.vertex_count, 0..1);
        }
    }

    pub fn new(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("AABB Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/render_aabb.wgsl").into()),
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
                label: Some("AABB Global Bind Group Layout"),
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
                label: Some("AABB Local Bind Group Layout"),
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
            label: Some("AABB Pipeline Layout"),
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
            label: Some("AABB Pipeline"),
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
            primitive,
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let global_uniforms = GlobalUniforms {
            world_to_camera: glam::Mat4::IDENTITY.to_cols_array_2d(),
            camera_to_clip: glam::Mat4::IDENTITY.to_cols_array_2d(),
        };
        let global_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("AABB Global Uniform Buffer"),
            contents: bytemuck::bytes_of(&global_uniforms),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
        });

        let global_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("AABB Global Bind Group"),
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
            align_to(local_uniform_size, alignment)
        };

        let local_uniform_buffer = create_local_uniform_buffer(device, MIN_LOCAL_BUFFER_NUM);
        let local_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("AABB Local Bind Group"),
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

        let vertices = create_box_line_vertices();
        let vertex_count = vertices.len() as u32;
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("AABB Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            pipeline,
            vertex_buffer,
            vertex_count,
            global_bind_group_layout,
            global_bind_group,
            global_uniform_buffer,
            local_bind_group_layout,
            local_bind_group,
            local_uniform_buffer,
            local_uniform_alignment,
            local_matrices: Vec::new(),
        }
    }
}
