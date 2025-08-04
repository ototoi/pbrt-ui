use super::lines_renderer::LinesRenderer;
use super::render_item::get_render_items;
use super::solid_mesh_renderer::SolidframeMeshRenderer;
use crate::model::base::Matrix4x4;
use crate::model::scene::Node;
use crate::render::render_mode::RenderMode;
use crate::render::wgpu::render_item::RenderItem;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;

use eframe::egui;
use eframe::egui_wgpu;
use eframe::wgpu;

pub struct SolidRenderer {
    mesh_renderer: Arc<Mutex<SolidframeMeshRenderer>>,
    lines_renderer: Arc<Mutex<LinesRenderer>>,
}

#[derive(Debug, Clone)]
struct PerFrameCallback {
    mesh_renderer: Arc<Mutex<SolidframeMeshRenderer>>,
    lines_renderer: Arc<Mutex<LinesRenderer>>,
    node: Arc<RwLock<Node>>,
    world_to_camera: glam::Mat4,
    camera_to_clip: glam::Mat4,
}

unsafe impl Send for PerFrameCallback {}
unsafe impl Sync for PerFrameCallback {}

impl egui_wgpu::CallbackTrait for PerFrameCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let render_items = get_render_items(device, queue, &self.node, RenderMode::Wireframe);
        let num_items = render_items.len();
        if num_items == 0 {
            return vec![];
        }
        let mut command_buffers = vec![];
        {
            let render_items = render_items
                .iter()
                .filter(|item| {
                    if let RenderItem::Mesh(_) = item.as_ref() {
                        true
                    } else {
                        false
                    }
                })
                .cloned()
                .collect::<Vec<_>>();
            if !render_items.is_empty() {
                let mut renderer = self.mesh_renderer.lock().unwrap();
                let cmds = renderer.prepare(
                    device,
                    queue,
                    screen_descriptor,
                    encoder,
                    resources,
                    &render_items,
                    &self.world_to_camera,
                    &self.camera_to_clip,
                );
                command_buffers.extend(cmds);
            }
        }
        {
            let render_items = render_items
                .iter()
                .filter(|item| {
                    if let RenderItem::Lines(_) = item.as_ref() {
                        true
                    } else {
                        false
                    }
                })
                .cloned()
                .collect::<Vec<_>>();
            if !render_items.is_empty() {
                //println!("Preparing lines renderer with {} items", render_items.len());
                let mut renderer = self.lines_renderer.lock().unwrap();
                let cmds = renderer.prepare(
                    device,
                    queue,
                    screen_descriptor,
                    encoder,
                    resources,
                    &render_items,
                    &self.world_to_camera,
                    &self.camera_to_clip,
                );
                command_buffers.extend(cmds);
            }
        }
        return command_buffers;
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        {
            let renderer = self.mesh_renderer.lock().unwrap();
            renderer.paint(&info, render_pass, resources);
        }
        {
            let renderer = self.lines_renderer.lock().unwrap();
            renderer.paint(&info, render_pass, resources);
        }
    }
}

impl SolidRenderer {
    pub fn new<'a>(cc: &'a eframe::CreationContext<'a>) -> Option<Self> {
        if let Some(mesh_renderer) = SolidframeMeshRenderer::new(cc) {
            if let Some(lines_renderer) = LinesRenderer::new(cc) {
                return Some(SolidRenderer {
                    mesh_renderer: Arc::new(Mutex::new(mesh_renderer)),
                    lines_renderer: Arc::new(Mutex::new(lines_renderer)),
                });
            }
        }
        return None;
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
                mesh_renderer: self.mesh_renderer.clone(),
                lines_renderer: self.lines_renderer.clone(),
                node: node.clone(),
                world_to_camera: glam::Mat4::from(w2c),
                camera_to_clip: glam::Mat4::from(c2c),
            },
        ));
    }
}
