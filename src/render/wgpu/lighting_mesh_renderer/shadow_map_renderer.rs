use super::*;
use eframe::wgpu;
use eframe::wgpu::util::DeviceExt;
use std::sync::Arc;
use std::sync::RwLock;
use wgpu::util::align_to;

impl LightingMeshRenderer {
    pub(super) fn create_directional_shadow_bind_group(
        &self,
        device: &wgpu::Device,
        directional_shadow_info_buffer: &wgpu::Buffer,
        directional_shadow_map_view: &wgpu::TextureView,
        directional_shadow_sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Lighting Directional Shadow Bind Group"),
            layout: &self.directional_shadow_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: directional_shadow_info_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(directional_shadow_map_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(directional_shadow_sampler),
                },
            ],
        })
    }

    pub(super) fn create_directional_shadow_map_array(
        device: &wgpu::Device,
        layer_count: u32,
        width: u32,
        height: u32,
    ) -> RenderTexture {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Directional Shadow Map Array"),
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
            label: Some("Directional Shadow Map Array View"),
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

    pub(super) fn create_directional_shadow_info_buffer(device: &wgpu::Device) -> wgpu::Buffer {
        let directional_shadow_info_buffer_size = (MAX_DIRECTIONAL_SHADOW_NUM
            * std::mem::size_of::<DirectionalShadowInfo>())
            as wgpu::BufferAddress;
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Buffer for Directional Shadow Infos"),
            size: directional_shadow_info_buffer_size,
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
        })
    }

    pub(super) fn compute_directional_shadow_map_extent(pipeline: &PipelineEntry) -> (u32, u32, u32) {
        let (directional_light_shadows, current_size) =
            if let PipelineEntry::DirectionalShadow(p) = pipeline {
                let size = p.shadow_map_array.texture.size();
                (&p.directional_light_shadows, (size.width, size.height))
            } else {
                return (1, 1, 1);
            };
        let total_cascade_count = directional_light_shadows
            .iter()
            .map(|s| s.cascades.len())
            .sum::<usize>();
        let layer_count = total_cascade_count.max(1) as u32;
        let (width, height) = directional_light_shadows
            .iter()
            .find_map(|s| s.cascades.first())
            .map(|c| (c.texture.texture.width(), c.texture.texture.height()))
            .unwrap_or(current_size);
        (layer_count, width, height)
    }

    pub(super) fn get_directional_shadow_pipeline_entry(&self) -> Option<Arc<RwLock<PipelineEntry>>> {
        self.pipelines.get(&DIRECTIONAL_SHADOW_PIPELINE_ID).cloned()
    }

    pub(super) fn ensure_directional_shadow_map_array(
        device: &wgpu::Device,
        pipeline: &mut PipelineEntry,
        layer_count: u32,
        width: u32,
        height: u32,
    ) -> Option<RenderTexture> {
        if let PipelineEntry::DirectionalShadow(p) = pipeline {
            let size = p.shadow_map_array.texture.size();
            if size.depth_or_array_layers != layer_count
                || size.width != width
                || size.height != height
            {
                p.shadow_map_array =
                    Self::create_directional_shadow_map_array(device, layer_count, width, height);
            }
            return Some(p.shadow_map_array.clone());
        }
        None
    }

    pub(super) fn update_directional_shadow_pipeline_entry(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        directional_light_shadows: &[Arc<RenderDirectionalLightShadow>],
        shadow_mesh_indices: &[usize],
    ) -> Option<Arc<RwLock<PipelineEntry>>> {
        if !directional_light_shadows.is_empty() && !shadow_mesh_indices.is_empty() {
            let _ = self.build_directional_shadow_pipeline_entry(device, shadow_mesh_indices);
        }

        let mut infos = vec![DirectionalShadowInfo::default(); MAX_DIRECTIONAL_SHADOW_NUM];
        let mut info_index = 0usize;
        for shadow in directional_light_shadows.iter() {
            for cascade in shadow.cascades.iter() {
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
        if let Some(pipeline_arc) = self.get_directional_shadow_pipeline_entry() {
            {
                let mut pipeline = pipeline_arc.write().unwrap();
                if let PipelineEntry::DirectionalShadow(p) = &mut *pipeline {
                    p.directional_light_shadows = directional_light_shadows.to_vec();
                }
                let (layer_count, shadow_map_width, shadow_map_height) =
                    Self::compute_directional_shadow_map_extent(&pipeline);
                let _ = Self::ensure_directional_shadow_map_array(
                    device,
                    &mut pipeline,
                    layer_count,
                    shadow_map_width,
                    shadow_map_height,
                );

                if let PipelineEntry::DirectionalShadow(p) = &*pipeline {
                    queue.write_buffer(
                        &p.directional_shadow_info_buffer,
                        0,
                        bytemuck::cast_slice(&infos),
                    );
                }
            }
            return Some(pipeline_arc.clone());
        }
        None
    }

    pub(super) fn render_shadow_maps(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        main_camera: &RenderCamera,
        shadow_pipelines: &[Arc<RwLock<PipelineEntry>>],
    ) {
        for shadow_pipeline in shadow_pipelines {
            self.render_directional_shadow_maps(
                device,
                queue,
                encoder,
                main_camera,
                shadow_pipeline,
            );
        }
    }

    pub(crate) fn update_directional_shadow_pipeline_state(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        directional_light_shadows: &[Arc<RenderDirectionalLightShadow>],
        shadow_mesh_indices: &[usize],
    ) -> bool {
        self.update_directional_shadow_pipeline_entry(
            device,
            queue,
            directional_light_shadows,
            shadow_mesh_indices,
        )
        .is_some()
    }

    pub(crate) fn render_directional_shadow_maps_for_active_pipeline(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        camera: &RenderCamera,
    ) {
        if let Some(shadow_pipeline) = self.get_directional_shadow_pipeline_entry() {
            self.render_shadow_maps(device, queue, encoder, camera, &[shadow_pipeline]);
        }
    }

    pub(crate) fn get_directional_shadow_resources(&self) -> Option<(wgpu::Buffer, RenderTexture)> {
        let pipeline_arc = self.get_directional_shadow_pipeline_entry()?;
        let pipeline_entry = pipeline_arc.read().ok()?;
        if let PipelineEntry::DirectionalShadow(p) = &*pipeline_entry {
            return Some((
                p.directional_shadow_info_buffer.clone(),
                p.shadow_map_array.clone(),
            ));
        }
        None
    }

    pub(super) fn render_directional_shadow_maps(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        _main_camera: &RenderCamera,
        shadow_pipeline: &Arc<RwLock<PipelineEntry>>,
    ) {
        let local_uniform_alignment = {
            let alignment = self.min_uniform_buffer_offset_alignment;
            align_to(
                std::mem::size_of::<LocalUniforms>() as wgpu::BufferAddress,
                alignment,
            )
        };

        let directional_light_shadows = {
            let pipeline_entry = shadow_pipeline.read().unwrap();
            if let PipelineEntry::DirectionalShadow(p) = &*pipeline_entry {
                p.directional_light_shadows.clone()
            } else {
                return;
            }
        };
        if directional_light_shadows.is_empty() {
            return;
        }

        let mut layer: u32 = 0;
        for shadow in directional_light_shadows.iter() {
            for cascade in shadow.cascades.iter() {
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
                let shadow_global_uniform_buffer =
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Shadow Global Uniform Buffer"),
                        contents: bytemuck::bytes_of(&global_uniforms),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
                let shadow_global_bind_group =
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Shadow Global Bind Group"),
                        layout: &self.global_bind_group_layout,
                        entries: &[wgpu::BindGroupEntry {
                            binding: 0,
                            resource: shadow_global_uniform_buffer.as_entire_binding(),
                        }],
                    });

                let shadow_texture = &cascade.texture;
                {
                    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Directional Shadow Render Pass"),
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

                    {
                        let pipeline_entry = shadow_pipeline.read().unwrap();
                        if let PipelineEntry::DirectionalShadow(p) = &*pipeline_entry {
                            render_pass.set_pipeline(&p.pipeline);
                            render_pass.set_bind_group(0, &shadow_global_bind_group, &[]);
                            for item_index in p.mesh_indices.iter().copied() {
                                if let RenderItem::Mesh(mesh_item) =
                                    self.mesh_items[item_index].as_ref()
                                {
                                    let local_uniform_offset = item_index as wgpu::DynamicOffset
                                        * local_uniform_alignment as wgpu::DynamicOffset;
                                    render_pass.set_bind_group(
                                        1,
                                        &self.local_bind_group,
                                        &[local_uniform_offset],
                                    );
                                    render_pass.set_vertex_buffer(
                                        0,
                                        mesh_item.mesh.vertex_buffer.slice(..),
                                    );
                                    render_pass.set_index_buffer(
                                        mesh_item.mesh.index_buffer.slice(..),
                                        wgpu::IndexFormat::Uint32,
                                    );
                                    render_pass.draw_indexed(
                                        0..mesh_item.mesh.index_count,
                                        0,
                                        0..1,
                                    );
                                }
                            }
                        }
                    }
                }

                {
                    let pipeline_entry = shadow_pipeline.read().unwrap();
                    if let PipelineEntry::DirectionalShadow(p) = &*pipeline_entry {
                        encoder.copy_texture_to_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: &shadow_texture.texture,
                                mip_level: 0,
                                origin: wgpu::Origin3d::ZERO,
                                aspect: wgpu::TextureAspect::DepthOnly,
                            },
                            wgpu::TexelCopyTextureInfo {
                                texture: &p.shadow_map_array.texture,
                                mip_level: 0,
                                origin: wgpu::Origin3d {
                                    x: 0,
                                    y: 0,
                                    z: layer as u32,
                                },
                                aspect: wgpu::TextureAspect::DepthOnly,
                            },
                            wgpu::Extent3d {
                                width: shadow_texture
                                    .texture
                                    .width()
                                    .min(p.shadow_map_array.texture.width()),
                                height: shadow_texture
                                    .texture
                                    .height()
                                    .min(p.shadow_map_array.texture.height()),
                                depth_or_array_layers: 1,
                            },
                        );
                    }
                }
                layer += 1;
            }
        }
    }

    pub(super) fn build_directional_shadow_pipeline_entry(
        &mut self,
        device: &wgpu::Device,
        shadow_mesh_indices: &[usize],
    ) -> Option<Arc<RwLock<PipelineEntry>>> {
        if shadow_mesh_indices.is_empty() {
            return None;
        }
        if self.get_directional_shadow_pipeline_entry().is_none() {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Directional Shadow Pipeline Shader"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("../shaders/lighting_z_prepass.wgsl").into(),
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
                label: Some("Directional Shadow Pipeline Layout"),
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

            let depth_texture_format = wgpu::TextureFormat::Depth32Float;
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Directional Shadow Pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &vertex_buffer_layout,
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: None,
                primitive,
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

            self.pipelines.insert(
                DIRECTIONAL_SHADOW_PIPELINE_ID,
                Arc::new(RwLock::new(PipelineEntry::DirectionalShadow(
                    DirectionalShadowPipelineEntry {
                        pipeline,
                        mesh_indices: Vec::new(),
                        directional_light_shadows: Vec::new(),
                        directional_shadow_info_buffer: Self::create_directional_shadow_info_buffer(
                            device,
                        ),
                        shadow_map_array: Self::create_directional_shadow_map_array(
                            device, 1, 1, 1,
                        ),
                    },
                ))),
            );
        }
        let entry = self.get_directional_shadow_pipeline_entry()?;
        {
            let mut entry_mut = entry.write().unwrap();
            if let PipelineEntry::DirectionalShadow(p) = &mut *entry_mut {
                p.mesh_indices = shadow_mesh_indices.to_vec();
            }
        }
        Some(entry.clone())
    }
}
