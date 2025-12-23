use super::dyna_image::DynaImage;
use crate::model::base::PropertyMap;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::Weak;

use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureSizeType {
    Render,
    Display,
    Icon,
}

#[derive(Debug, Clone)]
pub struct TextureNode {
    pub name: String,
    pub ty: String, //type
    pub id: Uuid,
    pub edition: String,
    pub properties: PropertyMap,
    pub inputs: HashMap<String, Option<Weak<RwLock<TextureNode>>>>,
    pub outputs: HashMap<Uuid, Weak<RwLock<TextureNode>>>,
    pub image_variants: HashMap<TextureSizeType, Arc<RwLock<DynaImage>>>, // key is variant name
}

impl TextureNode {
    pub fn get_name(&self) -> String {
        self.name.clone()
    }
    pub fn get_type(&self) -> String {
        self.ty.clone()
    }
    pub fn get_id(&self) -> Uuid {
        self.id
    }
    pub fn get_edition(&self) -> String {
        self.edition.clone()
    }

    pub fn set_link(key: &str, from: &Arc<RwLock<TextureNode>>, to: &Arc<RwLock<TextureNode>>) {
        {
            let mut from = from.write().unwrap();
            let id = to.read().unwrap().id;
            from.outputs.insert(id, Arc::downgrade(to));
        }
        {
            let mut to = to.write().unwrap();
            to.inputs
                .insert(key.to_string(), Some(Arc::downgrade(from)));
        }
    }
}
