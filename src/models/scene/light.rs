use crate::models::base::*;

use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Light {
    pub id: Uuid,
    pub props: PropertyMap,
}

//

impl Light {
    pub fn new(props: &PropertyMap) -> Self {
        let id = Uuid::new_v4();
        let mut props = props.clone();
        if props.get("string id").is_none() {
            props.insert("string id", Property::from(id.to_string()));
        }
        Light { id, props }
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

    pub fn get_type(&self) -> String {
        return self
            .props
            .find_one_string("string type")
            .unwrap_or_default();
    }

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
}
