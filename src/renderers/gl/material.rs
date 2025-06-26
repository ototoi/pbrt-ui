use super::program::RenderProgram;
use std::collections::HashMap;
use std::sync::Arc;

use uuid::Uuid;

//use eframe::egui;
use eframe::{egui_glow, glow::HasContext};
use egui_glow::glow;

#[derive(Debug, Clone)]
pub struct RenderMaterial {
    pub id: Uuid,
    pub program: Arc<RenderProgram>,
}

impl RenderMaterial {
    pub fn destroy(&self, gl: &Arc<glow::Context>) {
        //
    }
}
