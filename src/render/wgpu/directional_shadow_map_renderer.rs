use super::camera::RenderCamera;
use super::light::RenderLight;
use super::mesh::RenderVertex;
use super::render_item::RenderItem;
use super::render_resource::RenderResourceManager;
use super::shadow::DIRECTIONAL_SHADOW_CASCADE_MAX_COUNT;
use super::shadow::DirectionalShadowMaps;
use super::shadow::RenderDirectionalLightShadow;
use super::shadow::create_directional_light_shadows;
use super::shadow::get_shader_uses_shadow;
use super::texture::RenderTexture;
use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use eframe::wgpu::util::DeviceExt;
use std::collections::HashMap;
use std::mem::size_of;
use std::sync::Arc;
use uuid::Uuid;
use wgpu::util::align_to;

const MAX_DIRECTIONAL_LIGHT_NUM: usize = 4;
const MAX_DIRECTIONAL_SHADOW_NUM: usize =
    MAX_DIRECTIONAL_LIGHT_NUM * DIRECTIONAL_SHADOW_CASCADE_MAX_COUNT;

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
struct DirectionalShadowInfo {
    light_view_proj: [[f32; 4]; 4],
    split_end: f32,
    bias: f32,
    slope_bias: f32,
    map_layer: i32,
}

#[derive(Debug, Clone)]
pub struct DirectionalShadowMapRenderer {
    min_uniform_buffer_offset_alignment: wgpu::BufferAddress,
    global_bind_group_layout: wgpu::BindGroupLayout,
    local_bind_group_layout: wgpu::BindGroupLayout,
    local_bind_group: wgpu::BindGroup,
    local_uniform_buffer: wgpu::Buffer,
    directional_shadow_pipeline: Option<wgpu::RenderPipeline>,
    directional_shadow_info_buffer: wgpu::Buffer,
    shadow_map_array: RenderTexture,
}

fn create_local_uniform_buffer(device: &wgpu::Device, num_items: usize) -> wgpu::Buffer {
    let uniform_alignment = {
        let alignment = device.limits().min_uniform_buffer_offset_alignment as wgpu::BufferAddress;
        align_to(
            std::mem::size_of::<LocalUniforms>() as wgpu::BufferAddress,
            alignment,
        )
    };
    let required_size = uniform_alignment * num_items.max(1) as wgpu::BufferAddress;
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("DirectionalShadowMapRenderer Local Uniform Buffer"),
        size: required_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
        mapped_at_creation: false,
    })
}

impl DirectionalShadowMapRenderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let min_uniform_buffer_offset_alignment =
            device.limits().min_uniform_buffer_offset_alignment as wgpu::BufferAddress;

        let global_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("DirectionalShadowMapRenderer Global Bind Group Layout"),
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
                label: Some("DirectionalShadowMapRenderer Local Bind Group Layout"),
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
            label: Some("DirectionalShadowMapRenderer Local Bind Group"),
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

        let directional_shadow_info_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("DirectionalShadowMapRenderer Directional Shadow Info Buffer"),
            size: (MAX_DIRECTIONAL_SHADOW_NUM * size_of::<DirectionalShadowInfo>())
                as wgpu::BufferAddress,
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
        });

        let shadow_map_array = Self::create_directional_shadow_map_array(device, 1, 1, 1);

        Self {
            min_uniform_buffer_offset_alignment,
            global_bind_group_layout,
            local_bind_group_layout,
            local_bind_group,
            local_uniform_buffer,
            directional_shadow_pipeline: None,
            directional_shadow_info_buffer,
            shadow_map_array,
        }
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        render_resource_manager: &mut RenderResourceManager,
        render_items: &[Arc<RenderItem>],
        camera: &RenderCamera,
    ) -> Option<DirectionalShadowMaps> {
        let (mesh_items, light_items) = Self::split_items(render_items);
        if light_items.is_empty() {
            // No directional lights, no need to prepare shadow maps
            return None;
        }
        self.prepare_locals(device, queue, &mesh_items);

        let shadow_mesh_indices = mesh_items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                item.get_material().and_then(|material| {
                    if material
                        .passes
                        .iter()
                        .any(|pass| get_shader_uses_shadow(pass.render_category))
                    {
                        Some(i)
                    } else {
                        None
                    }
                })
            })
            .collect::<Vec<_>>();

        let directional_light_shadows = create_directional_light_shadows(
            device,
            queue,
            camera,
            &mesh_items,
            &light_items,
            render_resource_manager,
        );
        if directional_light_shadows.is_empty() {
            return None;
        }

        let mut directional_shadow_index_map = HashMap::new();
        let mut base_shadow_index = 0i32;
        for shadow in &directional_light_shadows {
            directional_shadow_index_map.insert(shadow.id, base_shadow_index);
            base_shadow_index += shadow.cascades.len() as i32;
        }

        let (directional_shadow_info_buffer, directional_shadow_map_texture) = self
            .prepare_and_render_directional_shadow_maps(
                device,
                queue,
                encoder,
                &mesh_items,
                &directional_light_shadows,
                &shadow_mesh_indices,
            )?;
        let directional_shadow_maps = DirectionalShadowMaps {
            directional_shadow_index_map,
            directional_shadow_info_buffer,
            directional_shadow_map_texture,
        };
        return Some(directional_shadow_maps);
    }

    fn split_items(
        render_items: &[Arc<RenderItem>],
    ) -> (Vec<Arc<RenderItem>>, Vec<Arc<RenderItem>>) {
        let mut mesh_items = Vec::new();
        let mut light_items = Vec::new();
        for item in render_items {
            match item.as_ref() {
                RenderItem::Mesh(_) => mesh_items.push(item.clone()),
                RenderItem::Light(light_item) => {
                    if let RenderLight::Directional(directional_light) = light_item.light.as_ref() {
                        if directional_light.cast_shadow {
                            // Only consider directional lights that cast shadows
                            light_items.push(item.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        (mesh_items, light_items)
    }

    fn prepare_locals(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mesh_items: &[Arc<RenderItem>],
    ) {
        let local_uniform_alignment = {
            let alignment = self.min_uniform_buffer_offset_alignment;
            align_to(
                std::mem::size_of::<LocalUniforms>() as wgpu::BufferAddress,
                alignment,
            )
        };
        if self.local_uniform_buffer.size()
            < (mesh_items.len().max(1) as wgpu::BufferAddress * local_uniform_alignment)
        {
            let new_buffer = create_local_uniform_buffer(device, mesh_items.len());
            let new_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("DirectionalShadowMapRenderer Local Bind Group"),
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
            };
            let offset = i as wgpu::BufferAddress * local_uniform_alignment;
            queue.write_buffer(
                &self.local_uniform_buffer,
                offset,
                bytemuck::bytes_of(&uniform),
            );
        }
    }

    fn prepare_and_render_directional_shadow_maps(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        mesh_items: &[Arc<RenderItem>],
        directional_light_shadows: &[Arc<RenderDirectionalLightShadow>],
        shadow_mesh_indices: &[usize],
    ) -> Option<(wgpu::Buffer, RenderTexture)> {
        if directional_light_shadows.is_empty() || shadow_mesh_indices.is_empty() {
            return None;
        }

        self.ensure_directional_shadow_pipeline(device);
        self.update_shadow_info_and_array(device, queue, directional_light_shadows);
        self.render_directional_shadow_maps(
            device,
            encoder,
            mesh_items,
            directional_light_shadows,
            shadow_mesh_indices,
        );

        Some((
            self.directional_shadow_info_buffer.clone(),
            self.shadow_map_array.clone(),
        ))
    }

    fn ensure_directional_shadow_pipeline(&mut self, device: &wgpu::Device) {
        if self.directional_shadow_pipeline.is_some() {
            return;
        }
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("DirectionalShadowMapRenderer Directional Shadow Pipeline Shader"),
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
            label: Some("DirectionalShadowMapRenderer Directional Shadow Pipeline Layout"),
            bind_group_layouts: &[
                &self.global_bind_group_layout,
                &self.local_bind_group_layout,
            ],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("DirectionalShadowMapRenderer Directional Shadow Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &vertex_buffer_layout,
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: None,
            primitive: wgpu::PrimitiveState::default(),
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
        self.directional_shadow_pipeline = Some(pipeline);
    }

    fn update_shadow_info_and_array(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        directional_light_shadows: &[Arc<RenderDirectionalLightShadow>],
    ) {
        let mut infos = vec![DirectionalShadowInfo::default(); MAX_DIRECTIONAL_SHADOW_NUM];
        let mut info_index = 0usize;
        for shadow in directional_light_shadows {
            for cascade in &shadow.cascades {
                if info_index >= MAX_DIRECTIONAL_SHADOW_NUM {
                    break;
                }
                infos[info_index] = DirectionalShadowInfo {
                    light_view_proj: cascade.light_view_proj.to_cols_array_2d(),
                    split_end: cascade.split_end,
                    bias: shadow.shadow_bias,
                    slope_bias: shadow.shadow_slope_bias,
                    map_layer: info_index as i32,
                };
                info_index += 1;
            }
        }
        queue.write_buffer(
            &self.directional_shadow_info_buffer,
            0,
            bytemuck::cast_slice(&infos),
        );

        let total_cascade_count = directional_light_shadows
            .iter()
            .map(|s| s.cascades.len())
            .sum::<usize>();
        let layer_count = total_cascade_count.max(1) as u32;
        let current_size = self.shadow_map_array.texture.size();
        let (width, height) = directional_light_shadows
            .iter()
            .find_map(|s| s.cascades.first())
            .map(|c| (c.texture.texture.width(), c.texture.texture.height()))
            .unwrap_or((current_size.width, current_size.height));
        if current_size.depth_or_array_layers != layer_count
            || current_size.width != width
            || current_size.height != height
        {
            self.shadow_map_array =
                Self::create_directional_shadow_map_array(device, layer_count, width, height);
        }
    }

    fn render_directional_shadow_maps(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        mesh_items: &[Arc<RenderItem>],
        directional_light_shadows: &[Arc<RenderDirectionalLightShadow>],
        shadow_mesh_indices: &[usize],
    ) {
        let Some(pipeline) = self.directional_shadow_pipeline.as_ref() else {
            return;
        };
        let local_uniform_alignment = {
            let alignment = self.min_uniform_buffer_offset_alignment;
            align_to(
                std::mem::size_of::<LocalUniforms>() as wgpu::BufferAddress,
                alignment,
            )
        };
        let mut layer: u32 = 0;
        for shadow in directional_light_shadows {
            for cascade in &shadow.cascades {
                let shadow_camera =
                    RenderCamera::from_matrices(cascade.light_view, cascade.light_proj);
                let global_uniforms = GlobalUniforms {
                    world_to_camera: shadow_camera.world_to_camera.to_cols_array_2d(),
                    camera_to_clip: shadow_camera.camera_to_clip.to_cols_array_2d(),
                    camera_to_world: shadow_camera.camera_to_world.to_cols_array_2d(),
                    camera_position: [
                        shadow_camera.position.x,
                        shadow_camera.position.y,
                        shadow_camera.position.z,
                        1.0,
                    ],
                };
                let global_uniform_buffer =
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("DirectionalShadowMapRenderer Global Uniform Buffer"),
                        contents: bytemuck::bytes_of(&global_uniforms),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
                let global_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("DirectionalShadowMapRenderer Global Bind Group"),
                    layout: &self.global_bind_group_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: global_uniform_buffer.as_entire_binding(),
                    }],
                });

                let shadow_texture = &cascade.texture;
                {
                    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("DirectionalShadowMapRenderer Directional Shadow Render Pass"),
                        color_attachments: &[],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &shadow_texture.view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.0),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    render_pass.set_pipeline(pipeline);
                    render_pass.set_bind_group(0, &global_bind_group, &[]);
                    for item_index in shadow_mesh_indices.iter().copied() {
                        if let RenderItem::Mesh(mesh_item) = mesh_items[item_index].as_ref() {
                            let local_uniform_offset = item_index as wgpu::DynamicOffset
                                * local_uniform_alignment as wgpu::DynamicOffset;
                            render_pass.set_bind_group(
                                1,
                                &self.local_bind_group,
                                &[local_uniform_offset],
                            );
                            render_pass
                                .set_vertex_buffer(0, mesh_item.mesh.vertex_buffer.slice(..));
                            render_pass.set_index_buffer(
                                mesh_item.mesh.index_buffer.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            render_pass.draw_indexed(0..mesh_item.mesh.index_count, 0, 0..1);
                        }
                    }
                }

                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &shadow_texture.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::DepthOnly,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.shadow_map_array.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer,
                        },
                        aspect: wgpu::TextureAspect::DepthOnly,
                    },
                    wgpu::Extent3d {
                        width: shadow_texture
                            .texture
                            .width()
                            .min(self.shadow_map_array.texture.width()),
                        height: shadow_texture
                            .texture
                            .height()
                            .min(self.shadow_map_array.texture.height()),
                        depth_or_array_layers: 1,
                    },
                );
                layer += 1;
            }
        }
    }

    fn create_directional_shadow_map_array(
        device: &wgpu::Device,
        layer_count: u32,
        width: u32,
        height: u32,
    ) -> RenderTexture {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("DirectionalShadowMapRenderer Directional Shadow Map Array"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: layer_count,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[wgpu::TextureFormat::Depth32Float],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("DirectionalShadowMapRenderer Directional Shadow Map Array View"),
            format: Some(wgpu::TextureFormat::Depth32Float),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            usage: Some(wgpu::TextureUsages::TEXTURE_BINDING),
            aspect: wgpu::TextureAspect::All,
            base_mip_level: 0,
            mip_level_count: Some(1),
            base_array_layer: 0,
            array_layer_count: Some(layer_count),
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        RenderTexture {
            id: Uuid::new_v4(),
            edition: Uuid::new_v4().to_string(),
            texture,
            view,
            sampler,
            scale: [1.0, 1.0],
            delta: [0.0, 0.0],
        }
    }
}
