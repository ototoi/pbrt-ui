use super::component::Component;
use crate::model::base::*;

#[derive(Debug, Clone)]
pub struct CameraComponent {
    pub props: PropertyMap,
}

impl CameraComponent {
    pub fn new(t: &str, props: &PropertyMap) -> Self {
        let mut props = props.clone();
        props.insert("string type", Property::from(t));
        CameraComponent { props: props }
    }
}

impl Component for CameraComponent {}
