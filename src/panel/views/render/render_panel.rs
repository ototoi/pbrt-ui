use super::render_history::RenderHistory;
use super::render_state::RenderState;
//
use super::show_render_view::show_render_view;
use super::show_scene_view::show_scene_view;
//
use crate::controller::AppController;
use crate::model::config::AppConfig;
use crate::panel::views::render::scene_view::RenderMode;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;

use eframe::egui;
use eframe::egui::frame;
use eframe::egui_glow;
use egui_glow::glow;

pub struct RenderPanel {
    app_controller: Arc<RwLock<AppController>>,
    histories: Vec<Box<RenderHistory>>,
    current: usize,
    gl: Arc<glow::Context>,
    render_mode: RenderMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderCommand {
    Render,
    Stop,
    NewHistory,
}

fn create_history(name: &str, config: &Arc<RwLock<AppConfig>>) -> Box<RenderHistory> {
    let mut history = Box::new(RenderHistory::new(name));
    let render_output_directory = config.read().unwrap().render_output_directory.clone();
    let render_output_directory = PathBuf::from(render_output_directory);
    if !render_output_directory.exists() {
        std::fs::create_dir_all(&render_output_directory).unwrap();
    }
    let filename = format!("render_image_{}.exr", name); //should be configurable
    let output_image_path = render_output_directory.join(filename);
    history.output_image_path = output_image_path.to_str().unwrap().to_string();
    return history;
}

impl RenderPanel {
    pub fn new<'a>(
        cc: &'a eframe::CreationContext<'a>,
        controller: &Arc<RwLock<AppController>>,
    ) -> Self {
        let config = controller.read().unwrap().get_config();
        let history = create_history("1", &config);
        Self {
            app_controller: controller.clone(),
            histories: vec![history],
            current: 0,
            gl: cc.gl.clone().unwrap(),
            render_mode: RenderMode::Wireframe,
        }
    }

    pub fn show(&mut self, _ctx: &egui::Context, ui: &mut egui::Ui) {
        //---------------------------------------------------------------------------
        let current_index = self.current;
        let mut commamds = Vec::new();
        //---------------------------------------------------------------------------
        {
            if let Some(last_history) = self.histories.last_mut() {
                //let before_state = last_history.get_state();
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
        egui::TopBottomPanel::top("render_mode_panel").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // selectable buttos
                    let render_modes = [
                        ("⬜", RenderMode::Wireframe),
                        ("⬛", RenderMode::Solid),
                        //("☀", RenderMode::Lighting),
                    ];
                    for (label, mode) in render_modes.iter().rev() {
                        ui.selectable_value(
                            &mut self.render_mode,
                            *mode,
                            egui::RichText::new(*label).family(egui::FontFamily::Monospace),
                        );
                    }
                    ui.separator();
                })
            });
        });

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
                        let button_text =
                            egui::RichText::new(text).family(egui::FontFamily::Monospace);
                        if ui
                            .add_enabled(is_ready | is_stoppable, egui::Button::new(button_text))
                            .clicked()
                        {
                            log::info!("Clicked {}", text);
                            commamds.push(cmd);
                        }
                        ui.separator();
                        ui.label(format!("{:?}", state));
                    }
                });
            }
        });
        //---------------------------------------------------------------------------
        let frame = egui::Frame {
            inner_margin: egui::Margin::same(0),
            ..Default::default()
        };
        egui::CentralPanel::default()
            .frame(frame)
            .show_inside(ui, |ui| {
                let history = &mut self.histories[current_index];
                let state = history.get_state();
                let available_rect = ui.available_rect_before_wrap();
                ui.painter()
                    .rect_filled(available_rect, 0.0, egui::Color32::BLACK);
                match state {
                    RenderState::Ready => {
                        let node = self.app_controller.read().unwrap().get_root_node();
                        show_scene_view(ui, &self.gl, &node, self.render_mode, true);
                    }
                    RenderState::Saving | RenderState::Rendering => {
                        if history.get_image_data().is_none() {
                            let node = self.app_controller.read().unwrap().get_root_node();
                            show_scene_view(ui, &self.gl, &node, self.render_mode, false);
                        } else {
                            show_render_view(ui, history);
                        }
                        show_render_view(ui, history);
                    }
                    RenderState::Finishing | RenderState::Finished => {
                        show_render_view(ui, history);
                    }
                }
                //show renderred image
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
                        if last_history.get_state() == RenderState::Ready {
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
                    }
                    RenderCommand::Stop => {
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
                        let config = self.app_controller.read().unwrap().get_config();
                        let history =
                            create_history(&format!("{}", self.histories.len() + 1), &config);
                        self.histories.push(history);
                    }
                }
            }
        }
    }
}
