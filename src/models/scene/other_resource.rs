use super::resource::ResourceObject;
use crate::models::base::*;

use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct OtherResource {
    pub id: Uuid,
    pub props: PropertyMap,
}

impl OtherResource {
    pub fn new(name: &str, props: &PropertyMap) -> Self {
        let id = Uuid::new_v4();
        let mut props = props.clone();
        props.insert("string id", Property::from(id.to_string()));
        props.insert("string name", Property::from(name));
        OtherResource { id, props }
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
            .find_one_string("string name")
            .unwrap_or_default();
    }

    pub fn get_type(&self) -> String {
        return self
            .props
            .find_one_string("string type")
            .unwrap_or_default();
    }

    pub fn get_filename(&self) -> Option<String> {
        if let Some(filename) = self.props.find_one_string("string filename") {
            return Some(filename);
        }
        None
    }

    pub fn get_fullpath(&self) -> Option<String> {
        if let Some(fullpath) = self.props.find_one_string("string fullpath") {
            return Some(fullpath);
        }
        None
    }
}

impl ResourceObject for OtherResource {
    fn get_id(&self) -> Uuid {
        self.get_id()
    }

    fn get_name(&self) -> String {
        self.get_name()
    }

    fn get_type(&self) -> String {
        self.get_type()
    }

    fn get_filename(&self) -> Option<String> {
        self.get_filename()
    }

    fn get_fullpath(&self) -> Option<String> {
        self.get_fullpath()
    }
}
