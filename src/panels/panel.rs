use eframe::egui;

pub trait Panel {
    fn name(&self) -> &str;
    fn is_open(&self) -> bool;
    fn toggle_open(&mut self) -> bool;
    fn show(&mut self, ctx: &egui::Context);
}
