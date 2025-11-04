use super::lines_renderer::LinesRenderer;
use super::render_item::get_render_items;
use super::solid_mesh_renderer::SolidMeshRenderer;
use crate::model::base::Matrix4x4;
use crate::model::scene::Node;
use crate::render::render_mode::RenderMode;
use crate::render::wgpu::render_item::RenderItem;
use std::sync::Arc;
use std::sync::RwLock;

use eframe::egui;
use eframe::egui_wgpu;
use eframe::wgpu;

pub struct SolidRenderer {
    mesh_renderer: Arc<RwLock<SolidMeshRenderer>>,
    lines_renderer: Arc<RwLock<LinesRenderer>>,
}

#[derive(Debug, Clone)]
struct PerFrameCallback {
    mesh_renderer: Arc<RwLock<SolidMeshRenderer>>,
    lines_renderer: Arc<RwLock<LinesRenderer>>,
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
        let render_items = get_render_items(device, queue, &self.node, RenderMode::Solid);
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

            let mut renderer = self.mesh_renderer.write().unwrap();
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
        if true {
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

            //println!("Preparing lines renderer with {} items", render_items.len());
            let mut renderer = self.lines_renderer.write().unwrap();
            renderer.prepare(
                device,
                queue,
                &render_items,
                &self.world_to_camera,
                &self.camera_to_clip,
            );
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
            let renderer = self.mesh_renderer.read().unwrap();
            renderer.paint(&info, render_pass, resources);
        }
        if true {
            let renderer = self.lines_renderer.read().unwrap();
            renderer.paint(render_pass);
        }
    }
}

impl SolidRenderer {
    pub fn new<'a>(cc: &'a eframe::CreationContext<'a>) -> Option<Self> {
        let render_state = cc.wgpu_render_state.as_ref()?;
        let device = &render_state.device;
        let queue = &render_state.queue;
        let mesh_renderer = SolidMeshRenderer::new(device, queue, render_state.target_format);
        let lines_renderer = LinesRenderer::new(device, queue, render_state.target_format);
        return Some(SolidRenderer {
            mesh_renderer: Arc::new(RwLock::new(mesh_renderer)),
            lines_renderer: Arc::new(RwLock::new(lines_renderer)),
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
                mesh_renderer: self.mesh_renderer.clone(),
                lines_renderer: self.lines_renderer.clone(),
                node: node.clone(),
                world_to_camera: glam::Mat4::from(w2c),
                camera_to_clip: glam::Mat4::from(c2c),
            },
        ));
    }
}
