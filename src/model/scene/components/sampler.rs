use super::component::Component;
use crate::model::base::*;

#[derive(Debug, Clone)]
pub struct SamplerComponent {
    pub props: PropertyMap,
}

fn replace_properties(props: &mut PropertyMap) {
    if let Some(prop) = props.get_mut("string type")
        && let Property::Strings(v) = prop {
            assert!(!v.is_empty());
            if v[0] == "02sequence" {
                v[0] = "lowdiscrepancy".to_string();
                log::warn!("Replaced 02sequence with lowdiscrepancy");
            }
        }
}

impl SamplerComponent {
    pub fn new(t: &str, props: &PropertyMap) -> Self {
        let mut props = props.clone();
        props.insert("string type", Property::from(t));
        replace_properties(&mut props);
        SamplerComponent { props }
    }
    pub fn get_keys(&self) -> Vec<(String, String)> {
        
        self.props.get_keys()
    }
}

impl Component for SamplerComponent {}
