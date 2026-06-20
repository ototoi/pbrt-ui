use super::component::Component;
use crate::render::wgpu::light::RenderLight;

use std::sync::Arc;

#[derive(Debug, Clone, Default)]
pub struct RenderLightComponent {
    render_light: Option<Arc<RenderLight>>,
}

impl RenderLightComponent {
    pub fn new() -> Self {
        Self { render_light: None }
    }

    pub fn get_render_light(&self) -> Option<Arc<RenderLight>> {
        self.render_light.clone()
    }

    pub fn set_render_light(&mut self, render_light: Arc<RenderLight>) {
        self.render_light = Some(render_light);
    }

    pub fn clear(&mut self) {
        self.render_light = None;
    }
}

impl Component for RenderLightComponent {}
