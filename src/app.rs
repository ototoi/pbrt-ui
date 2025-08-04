use crate::controller::AppController;
use crate::io::export;
use crate::io::export::pbrt::*;
use crate::io::import::pbrt::*;
use crate::model::scene::SceneComponent;
use crate::panel::HierarchyPanel;
use crate::panel::InspectorPanel;
use crate::panel::ManagePanel;
use crate::panel::Panel;
use crate::panel::PreferencesWindow;
use crate::panel::ViewsPanel;

use eframe::egui;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;

#[allow(dead_code)]
pub struct PbrtUIApp {
    controller: Arc<RwLock<AppController>>,
    panels: Vec<Box<dyn Panel>>,
    windows: Vec<Box<dyn Panel>>,
    opening_modal: Option<String>,
}

impl PbrtUIApp {
    pub fn new<'a>(cc: &'a eframe::CreationContext<'a>) -> Self {
        //let max_storage_buffer_binding_size = cc.wgpu_render_state.as_ref().unwrap().device.limits().max_storage_buffer_binding_size;
        //println!("Max storage buffer binding size: {}", max_storage_buffer_binding_size);
        let mut controller = AppController::new();
        controller.load_config();
        let controller = Arc::new(RwLock::new(controller));
        let panels: Vec<Box<dyn Panel>> = vec![
            Box::new(InspectorPanel::new(&controller)),
            Box::new(ManagePanel::new(&controller)),
            Box::new(HierarchyPanel::new(&controller)),
            Box::new(ViewsPanel::new(cc, &controller)),
        ];

        let windows: Vec<Box<dyn Panel>> = vec![Box::new(PreferencesWindow::new(&controller))];

        Self {
            controller,
            panels,
            windows,
            opening_modal: None,
        }
    }

    fn get_current_scene_path(&self) -> Option<String> {
        let controller = self.controller.read().unwrap();
        let node = controller.get_root_node();
        let node = node.read().unwrap();
        if let Some(scene) = node.get_component::<SceneComponent>() {
            let fullpath = scene.get_fullpath();
            return fullpath;
        }
        return None;
    }
}

#[derive(Debug, Clone)]
enum MenuCommand {
    Import(String),
    Export(String),
    Quit,
}

impl PbrtUIApp {
    pub fn show_top_menu(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                self.show_top_menu_file(ctx, ui);
                //self.show_top_menu_edit(ui);
                self.show_top_menu_panels(ui);
            });
        });
    }

    pub fn show_top_menu_file(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let mut commands = Vec::new();
        ui.menu_button("File", |ui| {
            if ui.button("Import").clicked() {
                let config = self.controller.read().unwrap().get_config();
                let import_directory = config.read().unwrap().import_file_directory.clone();
                let import_directory = PathBuf::from(import_directory);
                if !import_directory.exists() {
                    let _ = std::fs::create_dir_all(&import_directory);
                }

                let mut dialog = rfd::FileDialog::new()
                    .set_title("Import PBRT File")
                    .add_filter("PBRT", &["pbrt", "pbrt.gz"]);

                if import_directory.exists() {
                    dialog = dialog.set_directory(import_directory);
                }

                if let Some(path) = dialog.pick_file() {
                    if path.exists() {
                        if let Some(parent) = path.parent() {
                            let mut config = config.write().unwrap();
                            config.import_file_directory = parent.to_str().unwrap().to_string();
                        }
                        let path = path.to_str().unwrap().to_string();
                        commands.push(MenuCommand::Import(path));
                    }
                }
                ui.close_menu();
            }
            if ui.button("Export").clicked() {
                let config = self.controller.read().unwrap().get_config();
                let export_directory = config.read().unwrap().export_file_directory.clone();
                let export_directory = PathBuf::from(export_directory);

                if !export_directory.exists() {
                    let _ = std::fs::create_dir_all(&export_directory);
                }

                let mut dialog = rfd::FileDialog::new()
                    .set_title("Export PBRT File")
                    .add_filter("PBRT", &["pbrt", "pbrt.gz"]);

                if export_directory.exists() {
                    dialog = dialog.set_directory(export_directory);
                }

                if let Some(path) = dialog.save_file() {
                    if let Some(parent) = path.parent() {
                        if parent.exists() {
                            let mut config = config.write().unwrap();
                            config.export_file_directory = parent.to_str().unwrap().to_string();
                        }
                    }

                    let path = path.to_str().unwrap().to_string();
                    commands.push(MenuCommand::Export(path));
                }
                ui.close_menu();
            }
            //
            ui.separator();
            if ui.button("Preferences").clicked() {
                // Open preferences window
                ui.close_menu();
                if let Some(preferences) =
                    self.windows.iter_mut().find(|w| w.name() == "Preferences")
                {
                    preferences.toggle_open();
                }
                //log::info!("Preferences window opened");
            }
            ui.separator();
            if ui.button("Quit").clicked() {
                commands.push(MenuCommand::Quit);
                ui.close_menu();
            }
        });

        if let Some(msg) = self.opening_modal.as_mut() {
            let modal = egui::Modal::new(egui::Id::new(&msg)).show(ctx, |ui| {
                ui.heading("Quit");
                ui.separator();
                ui.label("Are you sure you want to quit?");

                ui.horizontal(|ui| {
                    if ui.button("Yes").clicked() {
                        ctx.request_repaint();
                        std::process::exit(0);
                    }
                    if ui.button("No").clicked() {
                        self.opening_modal = None;
                    }
                });
            });
            if modal.should_close() {
                self.opening_modal = None;
            }
        }

        if commands.is_empty() {
            return;
        }

        for command in commands.iter() {
            match command {
                MenuCommand::Import(path) => {
                    match load_pbrt(&path) {
                        Ok(node) => {
                            // Handle successful load
                            let controller = self.controller.clone();
                            let mut controller = controller.write().unwrap();
                            controller.set_root_node(&node);
                            {
                                let node = node.read().unwrap();
                                if let Some(scene) = node.get_component::<SceneComponent>() {
                                    if let Some(fullpath) = scene.get_fullpath() {
                                        let title = format!("PBRT UI - {}", fullpath);
                                        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
                                    }
                                }
                            }
                            log::info!("Loaded PBRT file: {}", path);
                        }
                        Err(e) => {
                            // Handle error
                            log::error!("Error loading PBRT file: {}", e);
                        }
                    }
                }
                MenuCommand::Export(path) => {
                    let controller = self.controller.clone();
                    let controller = controller.read().unwrap();
                    let node = controller.get_root_node();

                    let mut options = SavePbrtOptions::default();
                    options.copy_resources = true;
                    match save_pbrt(&node, path, &options) {
                        Ok(_) => {
                            // Handle successful save
                            log::info!("Saved PBRT file: {}", path);
                        }
                        Err(e) => {
                            // Handle error
                            log::error!("Error saving PBRT file: {}", e);
                        }
                    }
                }
                MenuCommand::Quit => {
                    self.opening_modal = Some("quit_modal".to_string());
                }
            }
        }
    }

    pub fn show_top_menu_edit(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Edit", |ui| {
            //if ui.button("Dummy").clicked() {
            // Open a file
            //    ui.close_menu();
            //}
        });
    }

    pub fn show_top_menu_panels(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Panels", |ui| {
            for panel in self.panels.iter_mut() {
                let name = panel.name();
                if name == "Views" {
                    continue; // Skip view panels
                }
                let title = format!("{} {}", name, if panel.is_open() { "✅" } else { " " },);
                if ui.button(title).clicked() {
                    panel.toggle_open();
                    ui.close_menu();
                }
            }
        });
    }

    pub fn show_footer(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("This is the footer.");
            });
        });
    }

    pub fn show_windows(&mut self, ctx: &egui::Context) {
        for window in self.panels.iter_mut() {
            window.show(ctx);
        }
        for window in self.windows.iter_mut() {
            window.show(ctx);
        }
    }
}

impl eframe::App for PbrtUIApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.show_top_menu(ctx);
        self.show_footer(ctx);
        self.show_windows(ctx);
    }
}
