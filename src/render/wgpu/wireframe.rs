use super::mesh::RenderVertex;
use super::render_item::RenderItem;
use super::render_item::get_render_items;
use crate::model::base::Matrix4x4;
use crate::model::base::Property;
use crate::model::scene::CameraComponent;
use crate::model::scene::FilmComponent;
use crate::model::scene::Node;
use crate::model::scene::TransformComponent;
use crate::render::render_mode::RenderMode;
use std::sync::Arc;
use std::sync::RwLock;

use eframe::egui;
use eframe::egui_wgpu;
use eframe::wgpu;
use eframe::wgpu::util::DeviceExt;
use egui::Vec2;
use wgpu::util::align_to;

use bytemuck::{Pod, Zeroable};

const MIN_LOCAL_BUFFER_NUM: usize = 512;

pub fn convert_matrix(m: &Matrix4x4) -> glam::Mat4 {
    return glam::Mat4::from_cols_array(&m.m);
}

pub struct WireframeRenderer {}

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
struct InitResources {
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

#[derive(Debug, Clone)]
struct PerFrameCallback {
    node: Arc<RwLock<Node>>,
    world_to_camera: glam::Mat4,
    camera_to_clip: glam::Mat4,
}

unsafe impl Send for PerFrameCallback {}
unsafe impl Sync for PerFrameCallback {}

fn create_local_uniform_buffer(device: &wgpu::Device, num_items: usize) -> wgpu::Buffer {
    let local_uniform_size = std::mem::size_of::<LocalUniforms>() as wgpu::BufferAddress; // 4x4 matrix
    let uniform_alignment = {
        let alignment = device.limits().min_uniform_buffer_offset_alignment as wgpu::BufferAddress;
        //let alignment = 64;
        align_to(local_uniform_size, alignment)
    };
    //let alignment = device.limits().min_uniform_buffer_offset_alignment as wgpu::BufferAddress;
    //println!(
    //    "Local uniform size: {}, alignment; {}, uniform_alignment: {}, num_items: {}",
    //    local_uniform_size, alignment, uniform_alignment, num_items
    //);
    let required_size = uniform_alignment * num_items as wgpu::BufferAddress;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Item Matrices Buffer"),
        size: required_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
        mapped_at_creation: false,
    });
    return buffer;
}

impl egui_wgpu::CallbackTrait for PerFrameCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let render_items = get_render_items(device, queue, &self.node, RenderMode::Wireframe);
        let num_items = render_items.len();
        if num_items == 0 {
            return vec![];
        }

        // Local uniforms buffer
        {
            let init_resources: &mut InitResources = resources.get_mut().unwrap();
            let local_uniform_alignment = init_resources.local_uniform_alignment;
            if init_resources.local_uniform_buffer.size()
                < (num_items as wgpu::BufferAddress * local_uniform_alignment)
            {
                let new_buffer = create_local_uniform_buffer(device, num_items);
                let new_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Wireframe Local Bind Group"),
                    layout: &init_resources.local_bind_group_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &new_buffer,
                            offset: 0,
                            size: wgpu::BufferSize::new(size_of::<LocalUniforms>() as _),
                        }),
                    }],
                });
                init_resources.local_uniform_buffer = new_buffer;
                init_resources.local_bind_group = new_bind_group;
            }
            for (i, item) in render_items.iter().enumerate() {
                let matrix = item.get_matrix();
                let base_color = [1.0, 1.0, 1.0, 1.0]; // Default color for wireframe
                let uniform = LocalUniforms {
                    local_to_world: matrix.to_cols_array_2d(),
                    base_color,
                };
                let offset = i as wgpu::BufferAddress * local_uniform_alignment;
                queue.write_buffer(
                    &init_resources.local_uniform_buffer,
                    offset,
                    bytemuck::bytes_of(&uniform),
                );
            }
        }

        {
            let init_resources: &InitResources = resources.get().unwrap();
            let global_uniforms = GlobalUniforms {
                world_to_camera: self.world_to_camera.to_cols_array_2d(), // Identity matrix for now
                camera_to_clip: self.camera_to_clip.to_cols_array_2d(),   // Identity matrix for now
            };
            let global_unifrom_buffer = &init_resources.global_uniform_buffer;
            queue.write_buffer(
                global_unifrom_buffer,
                0,
                bytemuck::bytes_of(&global_uniforms),
            );
        }

        let per_frame_resources = PerFrameResources { render_items };
        resources.insert(per_frame_resources);

        return vec![];
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let init_resources: &InitResources = resources.get().unwrap();
        if let Some(per_frame_resources) = resources.get::<PerFrameResources>() {
            let local_uniform_alignment = init_resources.local_uniform_alignment;
            render_pass.set_pipeline(&init_resources.pipeline); //
            render_pass.set_bind_group(0, &init_resources.global_bind_group, &[]);
            for (i, item) in per_frame_resources.render_items.iter().enumerate() {
                let i = i as wgpu::DynamicOffset;
                if let RenderItem::Mesh(mesh_item) = item.as_ref() {
                    let local_uniform_offset = i * local_uniform_alignment as wgpu::DynamicOffset;
                    render_pass.set_bind_group(
                        1,
                        &init_resources.local_bind_group,
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

impl WireframeRenderer {
    pub fn new<'a>(cc: &'a eframe::CreationContext<'a>) -> Option<Self> {
        let wgpu_render_state = cc.wgpu_render_state.as_ref()?;
        let device = &wgpu_render_state.device;
        //let queue = &wgpu_render_state.queue;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Wireframe Shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("shaders/wireframe_mesh_shader.wgsl").into(),
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
                    format: wgpu::VertexFormat::Float32x2,
                    offset: std::mem::size_of::<f32>() as u64 * 3,
                    shader_location: 1,
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
            label: Some("Wireframe Pipeline Layout"),
            bind_group_layouts: &[&global_bind_group_layout, &local_bind_group_layout],
            push_constant_ranges: &[],
        });

        let primitive = wgpu::PrimitiveState {
            cull_mode: None,
            topology: wgpu::PrimitiveTopology::TriangleList,
            polygon_mode: wgpu::PolygonMode::Line,
            ..Default::default()
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Wireframe Pipeline"),
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
                targets: &[Some(wgpu_render_state.target_format.into())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: primitive,
            depth_stencil: None,
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
            label: Some("Wireframe Global Bind Group"),
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
            label: Some("Wireframe Local Bind Group"),
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

        {
            wgpu_render_state
                .renderer
                .write()
                .callback_resources
                .insert(InitResources {
                    pipeline,
                    global_bind_group_layout,
                    global_bind_group,
                    global_uniform_buffer,
                    local_bind_group_layout,
                    local_bind_group,
                    local_uniform_buffer,
                    local_uniform_alignment,
                });
        }

        let renderer = WireframeRenderer {};
        return Some(renderer);
    }

    pub fn show(&mut self, ui: &mut egui::Ui, node: &Arc<RwLock<Node>>, is_playing: bool) {
        let available_rect = ui.available_rect_before_wrap();
        let available_size = available_rect.size();

        let mut fov = 90.0f32.to_radians();
        let mut w2c = Matrix4x4::identity();
        let mut render_size = Vec2::new(1280.0, 720.0);
        {
            let root_node = node.clone();
            if let Some(camera_node) = Node::find_node_by_component::<CameraComponent>(&root_node) {
                let camera_node = camera_node.read().unwrap();
                if let Some(t) = camera_node.get_component::<TransformComponent>() {
                    let local_to_world = t.get_local_matrix();
                    w2c = local_to_world.inverse().unwrap();
                }
                if let Some(camera) = camera_node.get_component::<CameraComponent>() {
                    if let Some(prop) = camera.props.get("fov") {
                        if let Property::Floats(f) = prop {
                            if f.len() > 0 {
                                fov = f[0].to_radians();
                            }
                        }
                    }
                }
                if let Some(film) = camera_node.get_component::<FilmComponent>() {
                    let width = film
                        .props
                        .find_one_int("integer xresolution")
                        .unwrap_or(1280);
                    let height = film
                        .props
                        .find_one_int("integer yresolution")
                        .unwrap_or(720);
                    render_size = Vec2::new(width as f32, height as f32);
                }
            }

            {
                let scale_x = available_size.x / render_size.x;
                let scale_y = available_size.y / render_size.y;
                let scale = scale_x.min(scale_y);
                let scaled_size = Vec2::new(render_size.x * scale, render_size.y * scale);

                let vertical_fov = if scaled_size.x < scaled_size.y {
                    // portrait mode
                    let k = scaled_size.x / (fov / 2.0).tan(); //tan = y / x
                    2.0 * f32::atan2(available_size.y, k)
                } else {
                    // landscape mode
                    let k = scaled_size.y / (fov / 2.0).tan(); //tan = y / x
                    2.0 * f32::atan2(available_size.y, k)
                };
                fov = vertical_fov;
            }

            let (rect, response) = ui.allocate_exact_size(available_size, egui::Sense::drag());
            if is_playing {}

            let aspect = rect.width() / rect.height();
            let c2c = Matrix4x4::perspective(fov, aspect, 0.1, 1000.0);

            ui.painter().rect_filled(rect, 0.0, egui::Color32::RED);
            ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                rect,
                PerFrameCallback {
                    node: node.clone(),
                    world_to_camera: convert_matrix(&w2c),
                    camera_to_clip: convert_matrix(&c2c),
                },
            ));
        }
    }
}
