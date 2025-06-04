use super::resource::ResourceObject;
use crate::models::base::*;

use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Material {
    pub id: Uuid,
    pub props: PropertyMap,
}

fn replace_properties(props: &mut PropertyMap) {
    if let Some((_, key_name, _)) = props.entry_mut("index") {
        *key_name = "eta".to_string();
    }
    if let Some((_, key_name, _)) = props.entry_mut("ior") {
        *key_name = "eta".to_string();
    }
}

impl Material {
    pub fn new(name: &str, t: &str, props: &PropertyMap) -> Self {
        let id = Uuid::new_v4();
        let mut props = props.clone();
        props.insert("string id", Property::from(Uuid::new_v4().to_string()));
        props.insert("string name_", Property::from(name));
        props.insert("string type", Property::from(t));
        replace_properties(&mut props);
        Material { id, props }
    }

    pub fn as_property_map(&self) -> &PropertyMap {
        &self.props
    }

    pub fn as_property_map_mut(&mut self) -> &mut PropertyMap {
        &mut self.props
    }

    pub fn get_id(&self) -> Uuid {
        return self.id;
    }

    pub fn get_name(&self) -> String {
        return self
            .props
            .find_one_string("string name_")
            .unwrap_or_default();
    }

    pub fn set_name(&mut self, name: &str) {
        self.props.insert("string name_", Property::from(name));
    }

    pub fn get_type(&self) -> String {
        return self
            .props
            .find_one_string("string type")
            .unwrap_or_default();
    }
}

impl ResourceObject for Material {
    fn get_id(&self) -> Uuid {
        self.id
    }

    fn get_name(&self) -> String {
        self.get_name()
    }

    fn get_type(&self) -> String {
        self.get_type()
    }
}
