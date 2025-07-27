use super::render_item::RenderItem;
use super::render_item::get_render_items;
use crate::model::scene::Node;
use crate::render::render_mode::RenderMode;

use std::num::NonZeroU64;
use std::sync::Arc;
use std::sync::RwLock;

use eframe::egui;
use eframe::egui_wgpu;
use eframe::wgpu;
use eframe::wgpu::util::DeviceExt;

pub struct WireframeRenderer {
    angle: f32,
}

#[derive(Debug, Clone)]
struct InitResources {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
}

#[derive(Debug, Clone)]
struct PerFrameResources {
    node: Arc<RwLock<Node>>,
    is_playing: bool,
    angle: f32,
}

unsafe impl Send for PerFrameResources {}
unsafe impl Sync for PerFrameResources {}

impl egui_wgpu::CallbackTrait for PerFrameResources {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        //let resources: &InitResources = resources.get().unwrap();
        //resources.prepare(device, queue, self.angle);
        if false {
            let render_items = get_render_items(device, queue, &self.node, RenderMode::Wireframe);
            if !render_items.is_empty() {
                // Create a render pass for the wireframe items
                {
                    //wireframe pass
                    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Wireframe Render Pass"),
                        color_attachments: &[],
                        depth_stencil_attachment: None,
                        occlusion_query_set: None,
                        timestamp_writes: None,
                    });

                    // Set the pipeline and bind group
                    //render_pass.set_pipeline(&resources.pipeline);
                    //render_pass.set_bind_group(0, &resources.bind_group, &[]);
                    // Draw the wireframe items
                    for item in render_items {
                        if let RenderItem::Mesh(item) = item.as_ref() {
                            let mesh = item.mesh.as_ref();
                            render_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                            render_pass.set_index_buffer(
                                mesh.index_buffer.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            render_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                        }
                    }
                }
            }
        }

        return vec![];
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let resources: &InitResources = resources.get().unwrap();
        resources.paint(render_pass);
    }
}

impl InitResources {
    fn prepare(&self, _device: &wgpu::Device, queue: &wgpu::Queue, angle: f32) {
        // Update our uniform buffer with the angle from the UI
        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&[angle, 0.0, 0.0, 0.0]),
        );
    }

    fn paint(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        // Draw our triangle!
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

impl WireframeRenderer {
    pub fn new<'a>(cc: &'a eframe::CreationContext<'a>) -> Option<Self> {
        let wgpu_render_state = cc.wgpu_render_state.as_ref()?;
        let device = &wgpu_render_state.device;
        //let queue = &wgpu_render_state.queue;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("custom3d"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("shaders/custom3dv_wgpu_shader.wgsl").into(),
            ),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("custom3d"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(16),
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("custom3d"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let primitive = wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            polygon_mode: wgpu::PolygonMode::Line,
            ..Default::default()
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("custom3d"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: None,
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu_render_state.target_format.into())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: primitive,
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("custom3d"),
            contents: bytemuck::cast_slice(&[0.0_f32; 4]), // 16 bytes aligned!
            // Mapping at creation (as done by the create_buffer_init utility) doesn't require us to to add the MAP_WRITE usage
            // (this *happens* to workaround this bug )
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("custom3d"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        {
            wgpu_render_state
                .renderer
                .write()
                .callback_resources
                .insert(InitResources {
                    pipeline,
                    bind_group,
                    uniform_buffer,
                });
        }

        let renderer = WireframeRenderer { angle: 0.0 };
        return Some(renderer);
    }

    pub fn show(&mut self, ui: &mut egui::Ui, node: &Arc<RwLock<Node>>, is_playing: bool) {
        let available_rect = ui.available_rect_before_wrap();
        let available_size = available_rect.size();

        let (rect, response) = ui.allocate_exact_size(available_size, egui::Sense::drag());

        ui.painter().rect_filled(rect, 0.0, egui::Color32::RED);

        self.angle += response.drag_motion().x * 0.01;
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            rect,
            PerFrameResources {
                node: node.clone(),
                is_playing,
                angle: self.angle,
            },
        ));
    }
}
