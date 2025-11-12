use super::common::*;
use std::cell::LazyCell;
use std::collections::HashMap;

const PARAMETERS: [(&str, &str, &str, &str, &str); 8] = [
    ("uv", "float", "uscale", "1.0", "0.0 100.0"),
    ("uv", "float", "vscale", "1.0", "0.0 100.0"),
    ("uv", "float", "udelta", "0.0", "0.0 100.0"),
    ("uv", "float", "vdelta", "0.0", "0.0 100.0"),
    ("planar", "vector", "v1", "1.0 0.0 0.0", ""),
    ("planar", "vector", "v2", "0.0 0.0 0.0", ""),
    ("planar", "float", "udelta", "0.0", "0.0 100.0"),
    ("planar", "float", "vdelta", "0.0", "0.0 100.0"),
];

#[derive(Debug, Clone)]
pub struct MappingProperties;

impl MappingProperties {
    fn new() -> Properties {
        let props: Vec<(String, PropertyEntry)> = PARAMETERS
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
        Properties::new(&props)
    }
    pub fn get_instance() -> LazyCell<Properties> {
        return LazyCell::new(|| MappingProperties::new());
    }
}
