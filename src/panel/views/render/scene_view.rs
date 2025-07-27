use crate::model::scene::Node;
use crate::render::RenderMode;
use crate::render::WireframeRenderer;
use crate::render::Custom3dv;

use std::sync::Arc;
use std::sync::RwLock;

use eframe::egui;


pub struct SceneView {
    custom3dv: Option<Custom3dv>,
    //wireframe: Option<WireframeRenderer>,
}

impl SceneView {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        //let wireframe = WireframeRenderer::new(cc);
        //Self { wireframe }
        let custom3dv = Custom3dv::new(cc);
        Self {
            custom3dv,
            //wireframe: Some(wireframe),
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        node: &Arc<RwLock<Node>>,
        render_mode: RenderMode,
        is_playing: bool,
    ) {
        //if let Some(custom3d) = &mut self.custom3d {
        //    custom3d.custom_painting(ui);
        //}
        match render_mode {
            RenderMode::Wireframe => {
                if let Some(renderer) = &mut self.custom3dv {
                    renderer.show(ui, node, is_playing);
                }
            }
            _ => {
                ui.label("Unsupported render mode");
            }
            
        }
    }
}
