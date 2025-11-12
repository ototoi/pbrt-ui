use super::common::*;
use std::cell::LazyCell;
use std::collections::HashMap;

const PARAMETERS: [(&str, &str, &str, &str, &str); 19] = [
    ("perspective", "float", "fov", "35.0", "0.0 90.0"), //halffov
    (
        "perspective",
        "float",
        "focaldistance",
        "1e6",
        "0.0 100000.0",
    ),
    ("perspective", "float", "lensradius", "0.0", "0.0 100.0"),
    //("perspective", "float", "frameaspectratio", "0.0", ""),
    //("perspective", "float", "screenwindow", "0.0", ""),
    ("perspective", "float", "shutteropen", "0.0", "0.0 1.0"),
    ("perspective", "float", "shutterclose", "1.0", "0.0 1.0"),
    //
    ("perspective", "float", "znear", "0.01", ""),
    ("perspective", "float", "zfar", "10000.0", ""),
    //
    ("realistic", "string", "lensfile", "", ""),
    ("realistic", "float", "aperturediameter", "1.0", ""),
    ("realistic", "float", "focusdistance", "10.0", ""),
    ("realistic", "bool", "simpleweighting", "true", ""),
    ("orthographic", "float", "focaldistance", "1e6", ""),
    ("orthographic", "float", "lensradius", "0.0", ""),
    //("orthographic", "float", "frameaspectratio", "0.0", ""),
    //("orthographic", "float", "screenwindow", "0.0", ""),
    ("orthographic", "float", "shutteropen", "0.0", ""),
    ("orthographic", "float", "shutterclose", "1.0", ""),
    ("environment", "float", "focaldistance", "1e6", ""),
    ("environment", "float", "lensradius", "0.0", ""),
    //("environment", "float", "frameaspectratio", "0.0", ""),
    //("environment", "float", "screenwindow", "0.0", ""),
    ("environment", "float", "shutteropen", "0.0", ""),
    ("environment", "float", "shutterclose", "1.0", ""),
];

#[derive(Debug, Clone)]
pub struct CameraProperties;

impl CameraProperties {
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
        return LazyCell::new(|| CameraProperties::new());
    }
}
