use super::scene_view::SceneView;
use crate::controllers::AppController;
use crate::panels::views::scene_view::RenderMode;

use std::sync::Arc;
use std::sync::RwLock;

use eframe::egui;

const RENDER_MODES: [(&str, RenderMode); 3] = [
    ("L", RenderMode::Lighting),
    ("S", RenderMode::Solid),
    ("W", RenderMode::Wireframe),
];

pub struct ScenePanel {
    scene_view: Option<SceneView>,
    render_mode: RenderMode,
}

impl ScenePanel {
    pub fn new<'a>(
        cc: &'a eframe::CreationContext<'a>,
        controller: &Arc<RwLock<AppController>>,
    ) -> Self {
        Self {
            scene_view: SceneView::new(cc, controller),
            render_mode: RenderMode::Wireframe,
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        egui::TopBottomPanel::top("scene_panel").show_inside(ui, |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                if ui.small_button("G").clicked() {
                    log::info!("with gizmo");
                }
                if ui.small_button("W").clicked() {
                    log::info!("with wireframe");
                }
                ui.separator();
                for (mode, render_mode) in RENDER_MODES.iter() {
                    let button = ui.small_button(*mode);
                    let button = if *render_mode == self.render_mode {
                        button.highlight()
                    } else {
                        button
                    };
                    if button.clicked() {
                        self.render_mode = *render_mode;
                        if let Some(scene_view) = &mut self.scene_view {
                            scene_view.set_render_mode(*render_mode);
                        }
                        log::info!("Render mode: {:?}", render_mode);
                    }
                }
            });
        });
        egui::CentralPanel::default().show_inside(ui, |ui| {
            if let Some(scene_view) = &mut self.scene_view {
                scene_view.show(ui);
            }
        });
    }
}
