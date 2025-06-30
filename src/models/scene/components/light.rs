use super::super::light::Light;
use std::sync::Arc;
use std::sync::RwLock;
use uuid::Uuid;

use super::component::Component;
use crate::models::base::*;

#[derive(Debug, Clone)]
pub struct LightComponent {
    pub light: Arc<RwLock<Light>>,
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
    pub fn new(light_type: &str, props: &PropertyMap) -> Self {
        let mut props = props.clone();
        props.insert("string type", Property::from(light_type.to_string()));
        replace_properties(&mut props);
        let light = Arc::new(RwLock::new(Light::new(&props)));
        LightComponent { light }
    }

    pub fn get_id(&self) -> Uuid {
        self.light.read().unwrap().get_id()
    }

    pub fn get_keys(&self) -> Vec<(String, String)> {
        let light = self.light.read().unwrap();
        let props = light.as_property_map();
        let keys = props
            .0
            .iter()
            .map(|(key_type, key_value, _prop)| (key_type.clone(), key_value.to_string()))
            .collect();
        return keys;
    }

    pub fn get_type(&self) -> String {
        self.light.read().unwrap().get_type()
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
