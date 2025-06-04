use crate::models::base::ParamSet;
use crate::models::scene::Material;
use crate::models::scene::Texture;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

#[derive(Debug, Clone)]
pub struct GraphicsState {
    pub materials: HashMap<String, Arc<RwLock<Material>>>,
    pub current_material: Option<Arc<RwLock<Material>>>,
    pub textures: HashMap<String, Arc<RwLock<Texture>>>,
    pub area_light: Option<(String, ParamSet)>,
}

impl Default for GraphicsState {
    fn default() -> Self {
        Self {
            materials: HashMap::new(),
            current_material: None,
            textures: HashMap::new(),
            area_light: None,
        }
    }
}
