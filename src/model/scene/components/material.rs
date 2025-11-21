use super::component::Component;
use crate::model::base::*;
use crate::model::scene::Material;
use std::sync::Arc;
use std::sync::RwLock;

#[derive(Debug, Clone)]
pub struct MaterialComponent {
    material: Arc<RwLock<Material>>,
}

impl MaterialComponent {
    pub fn new(name: &str, t: &str, props: &PropertyMap) -> Self {
        let material = Arc::new(RwLock::new(Material::new(name, t, props)));
        MaterialComponent { material }
    }

    pub fn from_material(material: &Arc<RwLock<Material>>) -> Self {
        MaterialComponent {
            material: material.clone(),
        }
    }

    pub fn get_keys(&self) -> Vec<(String, String)> {
        let material = self.material.read().unwrap();
        let props = material.as_property_map();
        return props.get_keys();
    }

    pub fn get_type(&self) -> String {
        let material = self.material.read().unwrap();
        material.get_type()
    }

    pub fn get_name(&self) -> String {
        let material = self.material.read().unwrap();
        material.get_name()
    }

    pub fn get_material(&self) -> Arc<RwLock<Material>> {
        self.material.clone()
    }
}

impl Component for MaterialComponent {}
