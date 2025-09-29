use crate::model::base::Property;
use crate::model::base::PropertyMap;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use image::DynamicImage;
use uuid::Uuid;

pub enum TextureType {
    Color,
    Bump,
    Normal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TexturePurpose {
    Render,
    Display,
    DisplaySrgb,
    Icon,
    IconSrgb,
}

impl TexturePurpose {
    pub fn add_srgb(&self) -> TexturePurpose {
        match self {
            TexturePurpose::Display => TexturePurpose::DisplaySrgb,
            TexturePurpose::Icon => TexturePurpose::IconSrgb,
            _ => *self,
        }
    }
}

#[derive(Debug, Clone)]
pub enum TextureDependent {
    Node(Arc<RwLock<TextureNode>>),
    Value(Property),
}

#[derive(Debug, Clone)]
pub struct TextureNode {
    pub name: String,
    pub ty: String, //type
    pub id: Uuid,
    pub edition: String,
    pub properties: PropertyMap,
    pub dependencies: HashMap<String, TextureDependent>,
    pub image_variants: HashMap<TexturePurpose, Arc<RwLock<DynamicImage>>>, // key is variant name
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
}
