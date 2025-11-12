use super::common::*;
use std::cell::LazyCell;
use std::collections::HashMap;

pub const PARAMETERS: [(&str, &str, &str, &str, &str); 30] = [
    ("imagemap", "string", "filename", "", ""),
    ("imagemap", "float", "maxanisotropy", "8.0", ""),
    ("imagemap", "bool", "trilinear", "false", ""),
    ("imagemap", "string", "wrap", "repeat", ""),
    ("imagemap", "float", "scale", "1.0", ""),
    ("imagemap", "bool", "gamma", "false", ""),
    ("constant", "color", "value", "1.0 1.0 1.0", ""),
    ("scale", "texture", "tex1", "", ""),
    ("scale", "texture", "tex2", "", ""),
    ("mix", "texture", "tex1", "", ""),
    ("mix", "texture", "tex2", "", ""),
    ("mix", "float", "amount", "0.5", ""),
    ("bilerp", "float", "v00", "0.0", ""),
    ("bilerp", "float", "v01", "1.0", ""),
    ("bilerp", "float", "v10", "0.0", ""),
    ("bilerp", "float", "v11", "1.0", ""),
    ("checkerboard", "color", "tex1", "1.0 1.0 1.0", ""),
    ("checkerboard", "color", "tex2", "0.0 0.0 0.0", ""),
    ("checkerboard", "integer", "dimension", "2", ""),
    ("checkerboard", "string", "aamode", "closedform", ""),
    ("dots", "color", "tex1", "1.0 1.0 1.0", ""),
    ("dots", "color", "tex2", "0.0 0.0 0.0", ""),
    ("fbm", "integer", "octaves", "8", ""),
    ("fbm", "float", "roughness", "0.5", ""),
    ("wrinkled", "integer", "octaves", "8", ""),
    ("wrinkled", "float", "roughness", "0.5", ""),
    ("marble", "integer", "octaves", "8", ""),
    ("marble", "float", "roughness", "0.5", ""),
    ("marble", "float", "scale", "1.0", ""),
    ("marble", "float", "variation", "0.2", ""),
];

pub const MAPPING_PARAMETERS: [(&str, &str, &str, &str, &str); 1] =
    [("", "string", "mapping", "uv", "")];

#[derive(Debug, Clone)]
pub struct TextureProperties;

impl TextureProperties {
    fn new() -> Properties {
        let mut props: Vec<(String, PropertyEntry)> = PARAMETERS
            .iter()
            .map(|(name, key_type, key_name, default_value, value_range)| {
                let mut param = HashMap::new();
                param.insert(PropetyParseKey::Name, name.to_string());
                param.insert(PropetyParseKey::KeyType, key_type.to_string());
                param.insert(PropetyParseKey::KeyName, key_name.to_string());
                param.insert(PropetyParseKey::DefaultValue, default_value.to_string());
                param.insert(PropetyParseKey::ValueRange, value_range.to_string());
                return parse_property_entry(&param);
            })
            .collect();
        let mut names: Vec<String> = vec![];
        for (name, _) in props.iter() {
            if !names.contains(&name) {
                names.push(name.clone());
            }
        }
        for name in names.iter() {
            for (_, key_type, key_name, default_value, value_range) in MAPPING_PARAMETERS.iter() {
                let mut param = HashMap::new();
                param.insert(PropetyParseKey::Name, name.to_string());
                param.insert(PropetyParseKey::KeyType, key_type.to_string());
                param.insert(PropetyParseKey::KeyName, key_name.to_string());
                param.insert(PropetyParseKey::DefaultValue, default_value.to_string());
                param.insert(PropetyParseKey::ValueRange, value_range.to_string());

                props.push(parse_property_entry(&param));
            }
        }
        Properties::new(&props)
    }
    pub fn get_instance() -> LazyCell<Properties> {
        return LazyCell::new(|| TextureProperties::new());
    }
}
