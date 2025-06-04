use super::component::Component;
use crate::models::base::*;

#[derive(Debug, Clone)]
pub struct IntegratorComponent {
    pub props: PropertyMap,
}

impl IntegratorComponent {
    pub fn new(t: &str, props: &PropertyMap) -> Self {
        let mut props = props.clone();
        props.insert("string type", Property::from(t));
        IntegratorComponent { props: props }
    }
    pub fn get_keys(&self) -> Vec<(String, String)> {
        let keys = self.props.get_keys();
        let keys = keys
            .iter()
            .filter(|(_key_type, key_name)| key_name != "type")
            .map(|(key_type, key_name)| (key_type.clone(), key_name.clone()))
            .collect::<Vec<(String, String)>>();
        return keys;
    }
}

impl Component for IntegratorComponent {}
