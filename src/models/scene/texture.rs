use super::ResourceObject;
use crate::models::base::*;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Texture {
    pub id: Uuid,
    pub props: PropertyMap,
    pub transform: Matrix4x4,
}

impl Texture {
    pub fn new(
        name: &str,
        color_type: &str,
        tex_type: &str,
        fullpath: Option<&str>,
        props: &PropertyMap,
        transform: &Matrix4x4,
    ) -> Self {
        let id = Uuid::new_v4();
        let name = name.to_string();
        let mut props = props.clone();
        props.insert("string id", Property::from(id.to_string()));
        props.insert("string name", Property::from(name));
        props.insert("string color_type", Property::from(color_type)); //float ot spectrum
        props.insert("string type", Property::from(tex_type)); //
        if let Some(fullpath) = fullpath {
            props.insert("string fullpath", Property::from(fullpath));
        }
        let transform = *transform;
        Self {
            id,
            props,
            transform,
        }
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
        return self.props.find_one_string("string name").unwrap();
    }

    pub fn get_type(&self) -> String {
        return self.props.find_one_string("string type").unwrap();
    }

    pub fn get_color_type(&self) -> String {
        return self.props.find_one_string("string color_type").unwrap();
    }

    pub fn get_transform(&self) -> Matrix4x4 {
        return self.transform;
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

    pub fn get_order(&self) -> i32 {
        self.props.find_one_int("integer order").unwrap_or(-1)
    }

    pub fn set_order(&mut self, order: i32) {
        self.props.insert("integer order", Property::from(order));
    }
}

impl ResourceObject for Texture {
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
