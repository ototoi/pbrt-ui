use crate::model::scene::ResourceManager;

use std::sync::{Arc, Mutex};
use uuid::Uuid;

pub type ResourceSelectionItem = (Uuid, String, String);

#[derive(Debug, Clone, Default)]
pub struct ResourceSelector {
    pub texture_items: Vec<ResourceSelectionItem>,
    pub material_items: Vec<ResourceSelectionItem>,
    pub spd_items: Vec<ResourceSelectionItem>,
    pub bsdffile_items: Vec<ResourceSelectionItem>,
}

impl ResourceSelector {
    pub fn new(resouce_manager: &Arc<Mutex<ResourceManager>>) -> Self {
        let mut texture_items = Vec::new();
        let mut material_items = Vec::new();
        let mut spd_items = Vec::new();
        let mut bsdffile_items = Vec::new();

        let manager = resouce_manager.lock().unwrap();
        for (id, texture) in manager.textures.iter() {
            let texture = texture.read().unwrap();
            //let t = texture.get_type();
            let name = texture.get_name();
            texture_items.push((id.clone(), name.clone(), name.clone()));
        }
        texture_items.sort_by(|a, b| a.1.cmp(&b.1));
        for (id, material) in manager.materials.iter() {
            let material = material.read().unwrap();
            let name = material.get_name();
            material_items.push((id.clone(), name.clone(), name.clone()));
        }
        material_items.sort_by(|a, b| a.1.cmp(&b.1));
        for (id, resource) in manager.other_resources.iter() {
            let resource = resource.read().unwrap();
            if resource.get_type() == "spd" {
                let name = resource.get_name().to_lowercase();
                let filename = resource.get_filename().unwrap();
                spd_items.push((id.clone(), name.clone(), filename.clone(), filename.clone()));
            }
            if resource.get_type() == "bsdffile" {
                let name = resource.get_name().to_lowercase();
                let filename = resource.get_filename().unwrap();
                bsdffile_items.push((id.clone(), name.clone(), filename.clone(), filename.clone()));
            }
        }
        spd_items.sort_by(|a, b| a.1.cmp(&b.1));
        bsdffile_items.sort_by(|a, b| a.1.cmp(&b.1));
        let spd_items = spd_items
            .into_iter()
            .map(|(id, _name1, name2, name3)| (id, name2, name3))
            .collect();
        let bsdffile_items = bsdffile_items
            .into_iter()
            .map(|(id, _name1, name2, name3)| (id, name2, name3))
            .collect();

        Self {
            texture_items: texture_items,
            material_items: material_items,
            spd_items: spd_items,
            bsdffile_items: bsdffile_items,
        }
    }
    //pub fn get_texture_items(&self) -> &[ResourceSelectionItem] {
    //    &self.texture_items
    //}
    pub fn get_texture_items(&self) -> Vec<(Uuid, String, String)> {
        self.texture_items.clone()
    }

    pub fn get_material_items(&self) -> Vec<(Uuid, String, String)> {
        self.material_items.clone()
    }

    pub fn get_spd_items(&self) -> Vec<(Uuid, String, String)> {
        self.spd_items.clone()
    }

    pub fn get_bsdffile_items(&self) -> Vec<(Uuid, String, String)> {
        self.bsdffile_items.clone()
    }
}
