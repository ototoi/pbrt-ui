use super::image_data::ImageData;
use super::render_session::RenderSession;
use super::render_state::RenderState;
use crate::error::PbrtError;
use crate::model::config::AppConfig;
use crate::model::scene::Node;

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
    pub state: RenderState,
    pub image_data: Option<Arc<Mutex<ImageData>>>,
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
            state: RenderState::Ready,
            image_data: None,
        }
    }

    pub fn get_id(&self) -> Uuid {
        return self.id;
    }

    pub fn get_name(&self) -> String {
        return self.name.clone();
    }

    pub fn get_state(&self) -> RenderState {
        return self.state;
    }

    pub fn update(&mut self) -> Result<RenderState, PbrtError> {
        if let Some(session) = self.session.as_mut() {
            let next_state = session.update()?;
            self.state = next_state;

            if self.image_data.is_none() {
                if let Some(image_data) = session.get_image_data() {
                    self.image_data = Some(image_data);
                }
            }
        }

        if self.state == RenderState::Finished {
            self.session = None;
        }
        return Ok(self.state);
    }

    pub fn render(
        &mut self,
        node: &Arc<RwLock<Node>>,
        config: &AppConfig,
    ) -> Result<(), PbrtError> {
        //println!("Starting render for history: {}", self.name);
        if self.session.is_some() {
            return Ok(());
        }
        //println!("Creating new render session for history: {}", self.name);
        let session = RenderSession::new(node, config, self.id, &self.output_image_path)?;
        self.state = session.get_state();
        //println!("Render session created for history: {}", self.name);
        self.session = Some(session);
        return Ok(());
    }

    pub fn cancel(&mut self) -> Result<(), PbrtError> {
        if let Some(session) = self.session.as_mut() {
            session.cancel()?;
            self.state = session.get_state();
        }
        return Ok(());
    }

    pub fn get_image_data(&self) -> Option<Arc<Mutex<ImageData>>> {
        return self.image_data.clone();
    }
}
