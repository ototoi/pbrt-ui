use super::render_history::RenderHistory;
use super::render_state::RenderState;
//
use crate::controllers::AppController;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;

use eframe::egui;

pub struct RenderPanel {
    app_controller: Arc<RwLock<AppController>>,
    histories: Vec<Box<RenderHistory>>,
    current: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderCommand {
    Render,
    Stop,
    NewHistory,
}

impl RenderPanel {
    pub fn new<'a>(
        _cc: &'a eframe::CreationContext<'a>,
        controller: &Arc<RwLock<AppController>>,
    ) -> Self {
        let config = controller.read().unwrap().get_config();
        let render_output_directory = config.read().unwrap().render_output_directory.clone();
        let render_output_directory = PathBuf::from(render_output_directory);
        if !render_output_directory.exists() {
            std::fs::create_dir_all(&render_output_directory).unwrap();
        }
        let output_image_path = render_output_directory.join("render_image.exr"); //should be configurable
        let mut history = Box::new(RenderHistory::new("1"));
        history.output_image_path = output_image_path.to_str().unwrap().to_string();
        Self {
            app_controller: controller.clone(),
            histories: vec![history],
            current: 0,
        }
    }

    pub fn show(&mut self, _ctx: &egui::Context, ui: &mut egui::Ui) {
        //---------------------------------------------------------------------------
        let current_index = self.current;
        let mut commamds = Vec::new();
        //---------------------------------------------------------------------------
        {
            if let Some(last_history) = self.histories.last_mut() {
                match last_history.update() {
                    Ok(state) => {
                        if state == RenderState::Finished {
                            commamds.push(RenderCommand::NewHistory);
                        }
                        // log::info!("Render state updated: {:?}", last_history.get_state());
                    }
                    Err(e) => {
                        log::error!(
                            "Failed to update render state for session {}: {}",
                            last_history.get_name(),
                            e
                        );
                    }
                }
            }
        }
        //---------------------------------------------------------------------------
        egui::TopBottomPanel::bottom("buttons").show_inside(ui, |ui| {
            {
                ui.add_space(3.0);
                ui.horizontal_wrapped(|ui| {
                    for (i, session) in self.histories.iter().enumerate() {
                        let name = egui::RichText::new(session.get_name())
                            .family(egui::FontFamily::Monospace);
                        if ui.selectable_value(&mut self.current, i, name).clicked() {
                            log::info!("Clicked {}", i);
                        }
                    }
                });
            }
            ui.separator();

            {
                let history = &mut self.histories[current_index];
                let state = history.get_state();
                let is_ready = state == RenderState::Ready;
                let is_stoppable = state == RenderState::Rendering;
                let is_finished = state == RenderState::Finished;
                ui.horizontal(|ui| {
                    //ui.text_edit_singleline(&mut session.output_image_path);
                    if is_finished {
                        ui.label(&history.output_image_path);
                    } else {
                        ui.add_enabled(
                            is_ready,
                            egui::TextEdit::singleline(&mut history.output_image_path),
                        );
                        if ui
                            .add_enabled(is_ready, egui::Button::new("Output Path"))
                            .on_hover_text("Set the output path for the rendered image")
                            .clicked()
                        {
                            let mut dialog = rfd::FileDialog::new();
                            let path = PathBuf::from(&history.output_image_path);

                            if let Some(dir) = path.parent() {
                                dialog = dialog.set_directory(dir);
                            }

                            if let Some(file_name) = path.file_name() {
                                // Use the file name from the existing path
                                // or provide a default name if it doesn't exist
                                let file_name = file_name.to_str().unwrap_or("render_image.exr");
                                dialog = dialog.set_file_name(file_name);
                            } else {
                                // If no file name, use a default one
                                dialog = dialog.set_file_name("render_image.exr");
                            }

                            if let Some(path) = dialog
                                .set_title("Select Output Path")
                                .add_filter("EXR", &["exr"])
                                .save_file()
                            {
                                history.output_image_path = path.to_str().unwrap_or("").to_string();
                                if let Some(parent) = path.parent() {
                                    if parent.exists() {
                                        let config =
                                            self.app_controller.read().unwrap().get_config();
                                        let mut config = config.write().unwrap();
                                        config.render_output_directory =
                                            parent.to_str().unwrap_or("").to_string();
                                    }
                                }
                            }
                        }

                        ui.separator();
                        let (text, cmd) = if is_ready {
                            ("▶ Render", RenderCommand::Render)
                        } else {
                            ("⏹ Stop", RenderCommand::Stop)
                        };
                        if ui
                            .add_enabled(is_ready | is_stoppable, egui::Button::new(text))
                            .clicked()
                        {
                            log::info!("Clicked {}", text);
                            commamds.push(cmd);
                        }
                    }
                });
            }
        });
        //---------------------------------------------------------------------------
        egui::CentralPanel::default().show_inside(ui, |ui| {
            let history = &mut self.histories[current_index];
            let state = history.get_state();
            let is_ready = state == RenderState::Ready;
            let available_rect = ui.available_rect_before_wrap();
            ui.painter()
                .rect_filled(available_rect, 0.0, egui::Color32::BLACK);
            if is_ready {
                ui.label("Ready to render.");
            } else {
                match state {
                    RenderState::Ready => {
                        ui.label("Ready to render.");
                    }
                    RenderState::Rendering => {
                        ui.label("Rendering...");
                    }
                    RenderState::Saving => {
                        ui.label("Saving image...");
                    }
                    RenderState::Finished => {
                        ui.label("Render finished.");
                    }
                    _ => {
                        ui.label(format!("Current state: {:?}", state));
                    }
                }
                //show renderred image
            }
        });
        //---------------------------------------------------------------------------
        {
            for cmd in commamds {
                match cmd {
                    RenderCommand::Render => {
                        assert!(!self.histories.is_empty());
                        let last_history = self.histories.last_mut().unwrap();
                        let node = self.app_controller.read().unwrap().get_root_node();
                        let config = self.app_controller.read().unwrap().get_config();
                        let config = config.read().unwrap();
                        match last_history.render(&node, &config) {
                            Ok(_) => {
                                log::info!(
                                    "Render started for session: {}",
                                    last_history.get_name()
                                );
                            }
                            Err(e) => {
                                log::error!(
                                    "Failed to start render for session {}: {}",
                                    last_history.get_name(),
                                    e
                                );
                            }
                        }
                    }
                    RenderCommand::Stop => {
                        // Handle stop command
                        assert!(!self.histories.is_empty());
                        let last_history = self.histories.last_mut().unwrap();
                        match last_history.cancel() {
                            Ok(_) => {}
                            Err(e) => {
                                log::error!(
                                    "Failed to update render state for session {}: {}",
                                    last_history.get_name(),
                                    e
                                );
                            }
                        }
                    }
                    RenderCommand::NewHistory => {
                        // Create a new history
                        let new_history =
                            Box::new(RenderHistory::new(&(self.histories.len() + 1).to_string()));
                        self.histories.push(new_history);
                        self.current = self.histories.len() - 1;
                        log::info!("New render history created: {}", self.current);
                    }
                }
            }
        }
    }
}
