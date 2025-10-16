use super::lighting_mesh_renderer::LightingMeshRenderer;
use super::linear_to_srgb_renderer::LinearToSrgbRenderer;
use super::lines_renderer::LinesRenderer;
use super::render_item::get_render_items;
use crate::model::base::Matrix4x4;
use crate::model::scene::Node;
use crate::render::render_mode::RenderMode;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use eframe::egui;
use eframe::egui_wgpu;
use eframe::wgpu;

const INTERNAL_TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum FrameBufferType {
    FinalRender,
}

type FrameBufferMap = HashMap<FrameBufferType, (wgpu::Texture, wgpu::Texture)>;

pub struct LightingRenderer {
    // surface textures
    mesh_renderer: Arc<RwLock<LightingMeshRenderer>>,
    lines_renderer: Arc<RwLock<LinesRenderer>>,
    copy_texture_renderer: Arc<RwLock<LinearToSrgbRenderer>>,
    frame_buffers: Arc<RwLock<FrameBufferMap>>,
}

#[derive(Debug, Clone)]
struct PerFrameCallback {
    rect: [f32; 4],
    mesh_renderer: Arc<RwLock<LightingMeshRenderer>>,
    lines_renderer: Arc<RwLock<LinesRenderer>>,
    copy_texture_renderer: Arc<RwLock<LinearToSrgbRenderer>>,
    frame_buffers: Arc<RwLock<FrameBufferMap>>,
    node: Arc<RwLock<Node>>,
    world_to_camera: glam::Mat4,
    camera_to_clip: glam::Mat4,
}

unsafe impl Send for PerFrameCallback {}
unsafe impl Sync for PerFrameCallback {}

impl PerFrameCallback {
    pub fn prepare_frame_buffers(
        &self,
        device: &wgpu::Device,
        screen_descriptor: &egui_wgpu::ScreenDescriptor,
        rect: &[f32; 4],
    ) {
        let texture_format = INTERNAL_TEXTURE_FORMAT; // Use a suitable format for your application
        let pixels_per_point = screen_descriptor.pixels_per_point;
        let mut frame_buffers = self.frame_buffers.write().unwrap();
        let width = ((rect[2] - rect[0]) * pixels_per_point) as u32;
        let height = ((rect[3] - rect[1]) * pixels_per_point) as u32;
        if frame_buffers.contains_key(&FrameBufferType::FinalRender) {
            // Check if the existing texture matches the size
            let (color_texture, _depth_texture) =
                frame_buffers.get(&FrameBufferType::FinalRender).unwrap();
            if color_texture.width() == width && color_texture.height() == height {
                return; // No need to recreate the texture
            }
        }
        // Create the final render target texture
        let color_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Final Render Texture"),
            size: wgpu::Extent3d {
                width: width,
                height: height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: texture_format, // Use a suitable format
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[texture_format], //
        });
        let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Final Depth Texture"),
            size: wgpu::Extent3d {
                width: width,
                height: height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float, // Use a suitable depth format
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[wgpu::TextureFormat::Depth32Float],
        });
        frame_buffers.insert(FrameBufferType::FinalRender, (color_texture, depth_texture));
    }
}

impl egui_wgpu::CallbackTrait for PerFrameCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        _resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        //let rect = self.rect.clone();
        //println!("PerFrameCallback::prepare: rect: {:?}", rect);
        let render_items = get_render_items(device, queue, &self.node, RenderMode::Lighting);
        let commands = vec![];
        // Prepare the frame buffers
        self.prepare_frame_buffers(device, &screen_descriptor, &self.rect);
        {
            let frame_buffers = self.frame_buffers.read().unwrap();
            if let Some((color_texture, depth_texture)) =
                frame_buffers.get(&FrameBufferType::FinalRender)
            {
                {
                    // Prepare the mesh renderer with the render items
                    let mut renderer = self.mesh_renderer.write().unwrap();
                    renderer.prepare(
                        device,
                        queue,
                        &render_items,
                        &self.world_to_camera,
                        &self.camera_to_clip,
                    );
                }
                {
                    let mut renderer = self.lines_renderer.write().unwrap();
                    renderer.prepare(
                        device,
                        queue,
                        &render_items,
                        &self.world_to_camera,
                        &self.camera_to_clip,
                    );
                }
                {
                    // Prepare the copy renderer
                    let mut renderer = self.copy_texture_renderer.write().unwrap();
                    renderer.prepare(device, color_texture);
                }

                let color_texture_view =
                    color_texture.create_view(&wgpu::TextureViewDescriptor::default());
                let depth_texture_view =
                    depth_texture.create_view(&wgpu::TextureViewDescriptor::default());
                let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color_texture_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), //
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth_texture_view,
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
                    let renderer = self.mesh_renderer.read().unwrap();
                    renderer.paint(&mut rpass);
                }
                {
                    let renderer = self.lines_renderer.read().unwrap();
                    renderer.paint(&mut rpass);
                }
            }
        }

        return commands;
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        _resources: &egui_wgpu::CallbackResources,
    ) {
        // Set the render pass to copy the final render texture to the screen
        let copy_renderer = self.copy_texture_renderer.read().unwrap();
        copy_renderer.paint(render_pass);
    }
}

impl LightingRenderer {
    pub fn new<'a>(cc: &'a eframe::CreationContext<'a>) -> Option<Self> {
        let render_state = cc.wgpu_render_state.as_ref()?;
        let device = &render_state.device;
        let queue = &render_state.queue;
        let mesh_renderer = LightingMeshRenderer::new(device, queue, INTERNAL_TEXTURE_FORMAT);
        let lines_renderer = LinesRenderer::new(device, queue, INTERNAL_TEXTURE_FORMAT);
        let copy_texture_renderer = LinearToSrgbRenderer::new(device, queue, render_state.target_format);
        // Create the lighting renderer with the mesh and lines renderers
        return Some(LightingRenderer {
            mesh_renderer: Arc::new(RwLock::new(mesh_renderer)),
            lines_renderer: Arc::new(RwLock::new(lines_renderer)),
            copy_texture_renderer: Arc::new(RwLock::new(copy_texture_renderer)),
            frame_buffers: Arc::new(RwLock::new(HashMap::new())),
        });
    }

    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        node: &Arc<RwLock<Node>>,
        w2c: &Matrix4x4,
        c2c: &Matrix4x4,
    ) {
        let c2c = *c2c;
        let c2c = Matrix4x4::OPENGL_TO_WGPU_CLIP * c2c; // Convert to WGPU clip space
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            rect,
            PerFrameCallback {
                rect: [rect.min.x, rect.min.y, rect.max.x, rect.max.y],
                mesh_renderer: self.mesh_renderer.clone(),
                lines_renderer: self.lines_renderer.clone(),
                copy_texture_renderer: self.copy_texture_renderer.clone(),
                frame_buffers: self.frame_buffers.clone(),
                node: node.clone(),
                world_to_camera: glam::Mat4::from(w2c),
                camera_to_clip: glam::Mat4::from(c2c),
            },
        ));
    }
}
