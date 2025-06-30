use super::resource::ResourceObject;
use crate::models::base::*;

use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Shape {
    pub id: Uuid,
    pub props: PropertyMap,
}

//

impl Shape {
    pub fn new(name: &str, props: &PropertyMap) -> Self {
        let id = Uuid::new_v4();
        let mut props = props.clone();
        props.insert("string id", Property::from(id.to_string()));
        props.insert("string name", Property::from(name));
        Shape { id, props }
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

    pub fn get_edition(&self) -> String {
        return self
            .props
            .find_one_string("string edition")
            .unwrap_or_default();
    }

    //------------------------------------------------------------

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

    //------------------------------------------------------------

    pub fn get_floats(&self, key: &str) -> Option<&[f32]> {
        if let Some(prop) = self.props.get(key) {
            if let Property::Floats(arr) = prop {
                return Some(arr);
            }
        }
        None
    }

    pub fn get_property(&self, key: &str) -> Option<&Property> {
        self.props.get(key)
    }

    pub fn get_indices(&self) -> Option<&[i32]> {
        if let Some(prop) = self.props.get("indices") {
            if let Property::Ints(arr) = prop {
                return Some(arr);
            }
        }
        None
    }

    pub fn get_positions(&self) -> Option<&[f32]> {
        if let Some(prop) = self.props.get("P") {
            if let Property::Floats(arr) = prop {
                return Some(arr);
            }
        }
        None
    }

    pub fn get_normals(&self) -> Option<&[f32]> {
        if let Some(prop) = self.props.get("N") {
            if let Property::Floats(arr) = prop {
                return Some(arr);
            }
        }
        None
    }

    pub fn get_uvs(&self) -> Option<&[f32]> {
        if let Some(prop) = self.props.get("uv") {
            if let Property::Floats(arr) = prop {
                return Some(arr);
            }
        }
        if let Some(prop) = self.props.get("st") {
            if let Property::Floats(arr) = prop {
                return Some(arr);
            }
        }
        None
    }
}

impl ResourceObject for Shape {
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
