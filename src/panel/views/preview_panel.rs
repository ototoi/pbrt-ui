use eframe::egui;

use crate::controller::AppController;
use std::sync::{Arc, RwLock};

pub struct PreviewPanel {}

impl PreviewPanel {
    pub fn new<'a>(
        cc: &'a eframe::CreationContext<'a>,
        controller: &Arc<RwLock<AppController>>,
    ) -> Self {
        Self {}
    }
    pub fn show(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(&egui::Style::default()).inner_margin(0.0))
            .show(ctx, |ui| {
                ui.label("Preview Panel1111");
            });
    }
}
