use super::log::LogPanel;
use super::resources::ResourcesPanel;
use crate::controller::AppController;
use crate::panel::Panel;

use eframe::egui;
use std::sync::Arc;
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq)]
enum ManageTab {
    Resources,
    Log,
}

#[derive(Debug, Clone)]
pub struct ManagePanel {
    pub is_open: bool,
    current_tab: ManageTab,
    pub resources_panel: ResourcesPanel,
    pub log_panel: LogPanel,
}

impl ManagePanel {
    pub fn new(controller: &Arc<RwLock<AppController>>) -> Self {
        Self {
            is_open: true,
            current_tab: ManageTab::Resources,
            resources_panel: ResourcesPanel::new(controller),
            log_panel: LogPanel::new(),
        }
    }

    fn show_resources(&mut self, ui: &mut egui::Ui) {
        self.resources_panel.show(ui);
    }

    fn show_log(&mut self, ui: &mut egui::Ui) {
        self.log_panel.show(ui);
    }
}

impl Panel for ManagePanel {
    fn name(&self) -> &str {
        "Manage"
    }
    fn is_open(&self) -> bool {
        self.is_open
    }
    fn toggle_open(&mut self) -> bool {
        self.is_open = !self.is_open;
        self.is_open
    }
    fn show(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("resources")
            .default_height(200.0)
            .min_height(150.0)
            .resizable(true)
            .show_animated(ctx, self.is_open, |ui| {
                ui.add_space(5.0);
                //ui.horizontal(|ui| {
                //    ui.visuals_mut().button_frame = false;
                //    ui.selectable_value(&mut self.current_tab, ManageTab::Resources, "Resources");
                //    ui.selectable_value(&mut self.current_tab, ManageTab::Log, "Log");
                //});
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.visuals_mut().button_frame = false;
                        ui.selectable_value(
                            &mut self.current_tab,
                            ManageTab::Resources,
                            "Resources",
                        );
                        ui.selectable_value(&mut self.current_tab, ManageTab::Log, "Log");
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("X").clicked() {
                            self.toggle_open();
                        }
                    });
                });
                ui.separator();
                match self.current_tab {
                    ManageTab::Resources => {
                        self.show_resources(ui);
                    }
                    ManageTab::Log => {
                        self.show_log(ui);
                    }
                }
            });
    }
}
