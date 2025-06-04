use super::component::Component;
use crate::models::base::*;

#[derive(Debug, Clone)]
pub struct LightComponent {
    pub props: PropertyMap,
}

fn replace_properties(props: &mut PropertyMap) {
    if let Some(prop) = props.get_mut("string type") {
        if let Property::Strings(v) = prop {
            assert!(v.len() >= 1);
            if v[0] == "exinfinite" {
                v[0] = "infinite".to_string();
                log::warn!("Replaced exinfinite with infinite");
            }
            if v[0] == "area" {
                v[0] = "diffuse".to_string();
                log::warn!("Replaced area with diffuse");
            }
        }
    }

    if let Some((_, key_name, _)) = props.entry_mut("conedelta") {
        *key_name = "conedeltaangle".to_string();
    }
    if let Some((_, key_name, _)) = props.entry_mut("samples") {
        *key_name = "nsamples".to_string();
    }
}

impl LightComponent {
    pub fn new(t: &str, props: &PropertyMap) -> Self {
        let mut props = props.clone();
        props.add_string("string type", &t);
        replace_properties(&mut props);
        LightComponent { props: props }
    }

    pub fn get_keys(&self) -> Vec<(String, String)> {
        return self.props.get_keys();
    }

    pub fn get_type(&self) -> String {
        self.props.find_one_string("string type").unwrap()
    }

    pub fn get_name_from_type(t: &str) -> String {
        let name = match t {
            "point" => "PointLight",
            "spot" => "SpotLight",
            "goniometric" => "GoniometricLight",
            "projection" => "ProjectionLight",
            "distant" => "DistantLight",
            "infinite" | "exinfinite" => "InfiniteLight",
            "diffuse" | "area" => "AreaLight",
            _ => "Light",
        };
        name.to_string()
    }
}

impl Component for LightComponent {}

pub type AreaLightComponent = LightComponent;
