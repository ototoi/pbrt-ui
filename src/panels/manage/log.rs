use eframe::egui;

use std::sync::Arc;
use std::sync::RwLock;

#[derive(Debug, Default)]
struct Logger {
    data: Arc<RwLock<Vec<(log::Level, String)>>>,
}

impl log::Log for Logger {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            let mut data = self.data.write().unwrap();
            data.push((record.level(), format!("{}", record.args()))); //should be a string
        }
    }

    fn flush(&self) {}
}

#[derive(Debug, Clone)]
pub struct LogPanel {
    pub data: Arc<RwLock<Vec<(log::Level, String)>>>,
}

impl LogPanel {
    pub fn new() -> Self {
        let logger = Logger::default();
        let data = logger.data.clone();

        log::set_boxed_logger(Box::new(logger))
            .map(|()| log::set_max_level(log::LevelFilter::Info))
            .unwrap_or_else(|_| {
                eprintln!("Failed to set logger");
            });
        log::set_max_level(log::LevelFilter::Info);

        Self { data: data }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        let data = self.data.read().unwrap();

        let text_style = egui::TextStyle::Body;
        let row_height = ui.text_style_height(&text_style);
        let size = text_style.resolve(&ui.style()).size;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .stick_to_bottom(true)
            .show_rows(ui, row_height, data.len(), |ui, row_range| {
                for row in row_range {
                    let log = &data[row];

                    let (text, color) = match log.0 {
                        log::Level::Error => ("ERROR", egui::Color32::from_rgb(255, 0, 0)), //red
                        log::Level::Warn => ("WARN ", egui::Color32::from_rgb(255, 255, 0)), //yellow
                        log::Level::Info => ("INFO ", egui::Color32::from_rgb(0, 255, 0)),   //green
                        log::Level::Debug => ("DEBUG", egui::Color32::from_rgb(0, 0, 255)),  //blue
                        log::Level::Trace => ("TRACE", egui::Color32::from_rgb(255, 255, 255)), //white
                    };
                    let level_text = egui::RichText::new(text)
                        .font(egui::FontId::monospace(size))
                        .color(color);
                    ui.horizontal(|ui| {
                        ui.label(level_text);
                        ui.label(":");
                        ui.label(format!("{}", log.1));
                    });
                }
            });
    }
}
