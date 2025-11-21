use super::common::*;
use std::cell::LazyCell;
use std::collections::HashMap;

const PARAMETERS: [(&str, &str, &str, &str, &str); 27] = [
    ("point", "color", "I", "1.0 1.0 1.0", ""),
    ("point", "color", "scale", "1.0 1.0 1.0", ""),
    ("point", "point", "from", "0.0 0.0 0.0", ""),
    //
    ("spot", "color", "I", "1.0 1.0 1.0", ""),
    ("spot", "color", "scale", "1.0 1.0 1.0", ""),
    ("spot", "float", "coneangle", "30.0", "0.0 90.0"),
    ("spot", "float", "conedeltaangle", "5.0", "0.0 90.0"), //if conedelta
    ("spot", "point", "from", "0.0 0.0 0.0", ""),
    ("spot", "point", "to", "0.0 0.0 1.0", ""),
    ("goniometric", "color", "L", "1.0 1.0 1.0", ""),
    ("goniometric", "color", "scale", "1.0 1.0 1.0", ""),
    ("goniometric", "texture", "mapname", "", ""),
    //
    ("projection", "color", "I", "1.0 1.0 1.0", ""),
    ("projection", "color", "scale", "1.0 1.0 1.0", ""),
    ("projection", "float", "fov", "45.0", "0.0 90.0"),
    //
    ("distant", "color", "L", "1.0 1.0 1.0", ""),
    ("distant", "color", "scale", "1.0 1.0 1.0", ""),
    //("distant", "point", "from", "0.0 0.0 0.0", ""),
    //("distant", "point", "to", "0.0 0.0 1.0", ""),

    //
    ("infinite", "color", "L", "1.0 1.0 1.0", ""),
    ("infinite", "color", "scale", "1.0 1.0 1.0", ""),
    ("infinite", "texture", "mapname", "", ""),
    ("infinite", "integer", "nsamples", "1", "1 100000"), //samples
    //
    ("diffuse", "color", "L", "1.0 1.0 1.0", ""),
    ("diffuse", "color", "scale", "1.0 1.0 1.0", ""),
    ("diffuse", "integer", "nsamples", "1", "1 100000"),
    ("diffuse", "bool", "twosided", "false", ""),
    ("diffuse", "float", "coneangle", "90.0", "0.0 90.0"),
    ("diffuse", "float", "conedeltaangle", "90.0", "0.0 90.0"),
];

#[derive(Debug, Clone)]
pub struct LightProperties;

impl LightProperties {
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
        return LazyCell::new(|| LightProperties::new());
    }
}
