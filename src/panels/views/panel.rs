use super::render_panel::RenderPanel;
use super::scene_panel::ScenePanel;
use crate::controllers::AppController;
use crate::panels::Panel;

use eframe::egui;
use std::sync::Arc;
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq)]
enum ViewsTab {
    Scene,
    Render,
}

pub struct ViewsPanel {
    pub is_open: bool,
    current_tab: ViewsTab,
    pub scene_panel: ScenePanel,
    pub render_panel: RenderPanel,
}

impl ViewsPanel {
    pub fn new<'a>(
        cc: &'a eframe::CreationContext<'a>,
        controller: &Arc<RwLock<AppController>>,
    ) -> Self {
        Self {
            is_open: true,
            current_tab: ViewsTab::Scene,
            scene_panel: ScenePanel::new(cc, controller),
            render_panel: RenderPanel::new(cc, controller),
        }
    }
}

impl Panel for ViewsPanel {
    fn name(&self) -> &str {
        "Views"
    }
    fn is_open(&self) -> bool {
        self.is_open
    }
    fn toggle_open(&mut self) -> bool {
        self.is_open = !self.is_open;
        self.is_open
    }
    fn show(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(&egui::Style::default()).inner_margin(1.0))
            .show(ctx, |ui| {
                ctx.request_repaint(); //continuously repaint
                self.render_panel.show(ui);
            });
    }
}
