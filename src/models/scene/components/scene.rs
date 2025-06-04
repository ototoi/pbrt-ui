use super::component::Component;
use crate::models::base::*;

#[derive(Debug, Clone)]
pub struct SceneComponent {
    pub props: PropertyMap,
}

impl SceneComponent {
    pub fn new(props: &PropertyMap) -> Self {
        let props = props.clone();
        SceneComponent { props: props }
    }

    pub fn get_filename(&self) -> Option<String> {
        self.props.find_one_string("string filename")
    }

    pub fn get_fullpath(&self) -> Option<String> {
        self.props.find_one_string("string fullpath")
    }
}

impl Component for SceneComponent {}
