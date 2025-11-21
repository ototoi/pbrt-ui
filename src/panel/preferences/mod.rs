use crate::controller::AppController;
use crate::model::config::AppConfig;
use crate::panel::Panel;

use eframe::egui;
use std::sync::Arc;
use std::sync::RwLock;

pub struct PreferencesWindow {
    controller: Arc<RwLock<AppController>>,
    is_open: bool,
    id: egui::Id,
    config: AppConfig,
}

fn get_config(controller: &Arc<RwLock<AppController>>) -> Arc<RwLock<AppConfig>> {
    let mut controller = controller.write().unwrap();
    controller.load_config();
    return controller.get_config();
}

impl PreferencesWindow {
    pub fn new(controller: &Arc<RwLock<AppController>>) -> Self {
        let config = get_config(controller);
        let config = config.read().unwrap();
        let config: AppConfig = config.clone();
        Self {
            controller: controller.clone(),
            is_open: false,
            id: egui::Id::new("preferences_window"),
            config: config,
        }
    }

    pub fn open(&mut self) {
        self.is_open = true;
    }

    pub fn toggle_open(&mut self) {
        self.is_open = !self.is_open;
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.is_open {
            return;
        }

        egui::Modal::new(self.id)
            .frame(egui::Frame::window(&ctx.style()))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Preferences");
                });
                ui.separator();
                ui.horizontal(|ui| {
                    let config = self.config.clone();
                    let path = config.pbrt_executable_path;
                    let mut path_str = path.to_string_lossy().to_string();
                    ui.label("PBRT executable path:");
                    //ui.label(&path_str).on_hover_ui(|ui| {
                    //    egui::show_tooltip_text(ui.ctx(), ui.layer_id(), egui::Id::new("PBRT_executable_path"), &path_str);
                    //});;
                    ui.add(egui::TextEdit::singleline(&mut path_str))
                        .on_hover_ui(|ui| {
                            egui::show_tooltip_text(
                                ui.ctx(),
                                ui.layer_id(),
                                egui::Id::new("PBRT_executable_path"),
                                &path_str,
                            );
                        });
                    if ui.button("Browse").clicked() {
                        // Open file dialog to select PBRT executable
                        // Use a library like rfd or native-dialog for file dialog
                        let mut dialog = rfd::FileDialog::new().set_title("Select PBRT Executable");
                        if path.exists() {
                            dialog = dialog
                                .set_directory(path.parent().unwrap_or(std::path::Path::new(".")));
                        }
                        if let Some(new_path) = dialog.pick_file() {
                            if new_path.exists() {
                                self.config.pbrt_executable_path = new_path.clone();
                            }
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.checkbox(
                        &mut self.config.enable_display_server,
                        "Enable display server",
                    );
                    ui.add_enabled(
                        self.config.enable_display_server,
                        egui::DragValue::new(&mut self.config.display_server_port)
                            .speed(1.0)
                            .range(1024..=65535),
                    )
                    .on_hover_ui(|ui| {
                        egui::show_tooltip_text(
                            ui.ctx(),
                            ui.layer_id(),
                            egui::Id::new("display_port"),
                            "Display server port (default: 14158)",
                        );
                    });
                });
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        let controller = self.controller.write().unwrap();
                        {
                            let config = controller.get_config();
                            let mut config = config.write().unwrap();
                            *config = self.config.clone();
                        }
                        controller.save_config();
                        self.is_open = false;
                    }
                    if ui.button("Cancel").clicked() {
                        let controller = self.controller.write().unwrap();
                        let config = controller.get_config();
                        let config = config.read().unwrap();
                        self.config = config.clone();
                        self.is_open = false;
                    }
                });
            });
        //
    }
}

impl Panel for PreferencesWindow {
    fn name(&self) -> &str {
        "Preferences"
    }

    fn is_open(&self) -> bool {
        self.is_open
    }

    fn toggle_open(&mut self) -> bool {
        self.is_open = !self.is_open;
        self.is_open
    }

    fn show(&mut self, ctx: &egui::Context) {
        self.show(ctx);
    }
}
