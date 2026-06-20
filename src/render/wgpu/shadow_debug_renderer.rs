use super::camera::RenderCamera;
use super::mesh::RenderVertex;
use super::render_item::RenderItem;
use super::shadow::ShadowMaps;
use crate::render::wgpu::light::RenderLight;
use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use eframe::wgpu::util::DeviceExt;
use std::mem::size_of;
use std::sync::Arc;
use wgpu::util::align_to;

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectionalShadowDebugMode {
    Off = 0,
    ShadowFactor = 1,
    ShadowDepth = 2,
    ProjectedUvz = 3,
    CascadeColor = 4,
    SplitDepth = 5,
    SplitPick = 6,
    LightViewDepth = 7,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod, Zeroable)]
struct GlobalUniforms {
    world_to_camera: [[f32; 4]; 4],
    camera_to_clip: [[f32; 4]; 4],
    camera_to_world: [[f32; 4]; 4],
    camera_position: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod, Zeroable)]
struct LocalUniforms {
    local_to_world: [[f32; 4]; 4],
    world_to_local: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod, Zeroable)]
struct DebugUniforms {
    light_direction: [f32; 4],
    shadow_index: i32,
    cascade_count: u32,
    debug_mode: u32,
    _pad0: u32,
}

#[derive(Debug, Clone)]
pub struct ShadowDebugRenderer {
    pipeline: wgpu::RenderPipeline,
    mesh_items: Vec<Arc<RenderItem>>,
    global_bind_group: wgpu::BindGroup,
    global_uniform_buffer: wgpu::Buffer,
    local_bind_group_layout: wgpu::BindGroupLayout,
    local_bind_group: wgpu::BindGroup,
    local_uniform_buffer: wgpu::Buffer,
    local_uniform_alignment: wgpu::BufferAddress,
    debug_bind_group: wgpu::BindGroup,
    debug_uniform_buffer: wgpu::Buffer,
    shadow_bind_group_layout: wgpu::BindGroupLayout,
    shadow_bind_group: wgpu::BindGroup,
}

fn create_local_uniform_buffer(device: &wgpu::Device, num_items: usize) -> wgpu::Buffer {
    let alignment = device.limits().min_uniform_buffer_offset_alignment as wgpu::BufferAddress;
    let uniform_size = size_of::<LocalUniforms>() as wgpu::BufferAddress;
    let required_size = align_to(uniform_size, alignment) * num_items.max(1) as wgpu::BufferAddress;
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ShadowDebug Local Uniform Buffer"),
        size: required_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
        mapped_at_creation: false,
    })
}

impl ShadowDebugRenderer {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shadow Debug Shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("shaders/shadow_debug.wgsl").into(),
            ),
        });

        let global_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("ShadowDebug Global Bind Group Layout"),
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
        let global_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ShadowDebug Global Uniform Buffer"),
            size: size_of::<GlobalUniforms>() as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let global_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ShadowDebug Global Bind Group"),
            layout: &global_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: global_uniform_buffer.as_entire_binding(),
            }],
        });

        let local_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("ShadowDebug Local Bind Group Layout"),
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
        let local_uniform_buffer = create_local_uniform_buffer(device, 1);
        let local_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ShadowDebug Local Bind Group"),
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

        let debug_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("ShadowDebug Params Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(size_of::<DebugUniforms>() as _),
                    },
                    count: None,
                }],
            });
        let debug_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ShadowDebug Params Uniform Buffer"),
            size: size_of::<DebugUniforms>() as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let debug_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ShadowDebug Params Bind Group"),
            layout: &debug_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: debug_uniform_buffer.as_entire_binding(),
            }],
        });

        let shadow_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("ShadowDebug Shadow Bind Group Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
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
        let dummy_info = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ShadowDebug Dummy Shadow Info"),
            contents: &[0u8; 96],
            usage: wgpu::BufferUsages::STORAGE,
        });
        let dummy_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ShadowDebug Dummy Shadow Map"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let dummy_view = dummy_tex.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let dummy_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ShadowDebug Shadow Bind Group"),
            layout: &shadow_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: dummy_info.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&dummy_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&dummy_sampler),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ShadowDebug Pipeline Layout"),
            bind_group_layouts: &[
                &global_bind_group_layout,
                &local_bind_group_layout,
                &debug_bind_group_layout,
                &shadow_bind_group_layout,
            ],
            push_constant_ranges: &[],
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
                    offset: 12,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 24,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 36,
                    shader_location: 3,
                },
            ],
        }];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Shadow Debug Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &vertex_buffer_layout,
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        Self {
            pipeline,
            mesh_items: Vec::new(),
            global_bind_group,
            global_uniform_buffer,
            local_bind_group_layout,
            local_bind_group,
            local_uniform_buffer,
            local_uniform_alignment: align_to(
                size_of::<LocalUniforms>() as u64,
                device.limits().min_uniform_buffer_offset_alignment as u64,
            ),
            debug_bind_group,
            debug_uniform_buffer,
            shadow_bind_group_layout,
            shadow_bind_group,
        }
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        render_items: &[Arc<RenderItem>],
        camera: &RenderCamera,
        shadow_maps: &ShadowMaps,
        debug_mode: DirectionalShadowDebugMode,
    ) {
        self.mesh_items = render_items
            .iter()
            .filter(|item| matches!(item.as_ref(), RenderItem::Mesh(_)))
            .cloned()
            .collect();
        if self.mesh_items.is_empty() {
            return;
        }
        if self.local_uniform_buffer.size()
            < self.local_uniform_alignment * self.mesh_items.len().max(1) as u64
        {
            self.local_uniform_buffer = create_local_uniform_buffer(device, self.mesh_items.len());
            self.local_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ShadowDebug Local Bind Group"),
                layout: &self.local_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.local_uniform_buffer,
                        offset: 0,
                        size: wgpu::BufferSize::new(size_of::<LocalUniforms>() as _),
                    }),
                }],
            });
        }
        for (i, item) in self.mesh_items.iter().enumerate() {
            let mat = item.get_matrix();
            let inv = mat.inverse();
            let uniform = LocalUniforms {
                local_to_world: mat.to_cols_array_2d(),
                world_to_local: inv.to_cols_array_2d(),
            };
            queue.write_buffer(
                &self.local_uniform_buffer,
                self.local_uniform_alignment * i as u64,
                bytemuck::bytes_of(&uniform),
            );
        }
        let global = GlobalUniforms {
            world_to_camera: camera.world_to_camera.to_cols_array_2d(),
            camera_to_clip: camera.camera_to_clip.to_cols_array_2d(),
            camera_to_world: camera.camera_to_world.to_cols_array_2d(),
            camera_position: [camera.position.x, camera.position.y, camera.position.z, 1.0],
        };
        queue.write_buffer(&self.global_uniform_buffer, 0, bytemuck::bytes_of(&global));

        let mut light_direction = [0.0, -1.0, 0.0, 0.0];
        let mut shadow_index = -1;
        if let Some(maps) = shadow_maps.directional_shadow_maps.as_ref() {
            for item in render_items {
                if let RenderItem::Light(light_item) = item.as_ref()
                    && let RenderLight::Directional(light) = light_item.light.as_ref()
                    && let Some(base_index) = maps.directional_shadow_index_map.get(&light.id)
                {
                    light_direction = [light.direction[0], light.direction[1], light.direction[2], 0.0];
                    shadow_index = *base_index;
                    break;
                }
            }
            let uniform = DebugUniforms {
                light_direction,
                shadow_index,
                cascade_count: 1,
                debug_mode: debug_mode as u32,
                _pad0: 0,
            };
            queue.write_buffer(&self.debug_uniform_buffer, 0, bytemuck::bytes_of(&uniform));
            self.shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ShadowDebug Shadow Bind Group"),
                layout: &self.shadow_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: maps.directional_shadow_info_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(
                            &maps.directional_shadow_map_texture.view,
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(
                            &maps.directional_shadow_map_texture.sampler,
                        ),
                    },
                ],
            });
        }
    }

    pub fn paint(&self, render_pass: &mut wgpu::RenderPass) {
        if self.mesh_items.is_empty() {
            return;
        }
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.global_bind_group, &[]);
        render_pass.set_bind_group(2, &self.debug_bind_group, &[]);
        render_pass.set_bind_group(3, &self.shadow_bind_group, &[]);
        for (i, item) in self.mesh_items.iter().enumerate() {
            if let RenderItem::Mesh(mesh_item) = item.as_ref() {
                let offset = (self.local_uniform_alignment * i as u64) as u32;
                render_pass.set_bind_group(1, &self.local_bind_group, &[offset]);
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
