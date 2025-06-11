use super::image_data::ImageData;
use super::render_session::RenderSession;
use super::render_state::RenderState;
use crate::error::PbrtError;
use crate::models::config::AppConfig;
use crate::models::scene::Node;

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;
use uuid::Uuid;

use eframe::egui;

pub struct RenderHistory {
    pub id: Uuid,
    pub name: String,
    pub output_image_path: String,
    pub session: Option<RenderSession>,
    pub texture_id: Option<egui::TextureId>,
}

impl RenderHistory {
    pub fn new(name: &str) -> Self {
        let name = name.to_string();
        let id = Uuid::new_v4();
        let output_image_path = String::new();
        RenderHistory {
            id,
            name,
            output_image_path,
            session: None,
            texture_id: None,
        }
    }

    pub fn get_id(&self) -> Uuid {
        return self.id;
    }

    pub fn get_name(&self) -> String {
        return self.name.clone();
    }

    pub fn get_state(&self) -> RenderState {
        if let Some(session) = self.session.as_ref() {
            return session.get_state();
        }
        return RenderState::Ready;
    }

    pub fn update(&mut self) -> Result<RenderState, PbrtError> {
        if let Some(session) = self.session.as_mut() {
            let before_state = session.get_state();
            let next_state = session.update()?;
            if next_state != before_state {
                if next_state == RenderState::Finished {
                    self.session = None;
                }
            }
        }
        return Ok(RenderState::Ready);
    }

    pub fn render(
        &mut self,
        node: &Arc<RwLock<Node>>,
        config: &AppConfig,
    ) -> Result<(), PbrtError> {
        println!("Starting render for history: {}", self.name);
        if self.session.is_some() {
            return Ok(());
        }
        //println!("Creating new render session for history: {}", self.name);
        let session = RenderSession::new(node, config)?;
        //println!("Render session created for history: {}", self.name);
        self.session = Some(session);
        return Ok(());
    }

    pub fn cancel(&mut self) -> Result<(), PbrtError> {
        if let Some(session) = self.session.as_mut() {
            return session.cancel();
        }
        return Ok(());
    }

    pub fn get_image_data(&self) -> Option<Arc<Mutex<ImageData>>> {
        if let Some(session) = self.session.as_ref() {
            return session.get_image_data();
        }
        return None;
    }
}
