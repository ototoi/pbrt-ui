use super::common::*;
use std::cell::LazyCell;
use std::collections::HashMap;

const PARAMETERS: [(&str, &str, &str, &str, &str); 38] = [
    ("trianglemesh", "integer", "indices", "", ""),
    ("trianglemesh", "point", "P", "", ""),
    ("trianglemesh", "normal", "N", "", ""),
    ("trianglemesh", "vector", "S", "", ""),
    ("trianglemesh", "float", "uv", "", ""),
    ("trianglemesh", "bool", "twosided", "true", ""),
    ("trianglemesh", "float", "alpha", "1.0", ""),
    ("trianglemesh", "float", "shadowalpha", "1.0", ""),
    ("plymesh", "string", "filename", "", ""),
    ("plymesh", "float", "alpha", "1.0", ""),
    ("plymesh", "bool", "twosided", "true", ""),
    ("plymesh", "float", "shadowalpha", "1.0", ""),
    ("sphere", "float", "radius", "1.0", "0.0 1000.0"),
    ("sphere", "float", "zmin", "-1.0", "-1000.0 0.0"),
    ("sphere", "float", "zmax", "1.0", "0.0 100.0"),
    ("sphere", "float", "phimax", "360.0", "0.0 360.0"),
    ("disk", "float", "height", "0.0", "0.0 1000.0"),
    ("disk", "float", "radius", "1.0", "0.0 1000.0"),
    ("disk", "float", "innerradius", "0.0", "0.0 100.0"),
    ("disk", "float", "phimax", "360.0", "0.0 360.0"),
    ("cylinder", "float", "radius", "1.0", "0.0 1000.0"),
    ("cylinder", "float", "zmin", "-1.0", "-1000.0 0.0"),
    ("cylinder", "float", "zmax", "1.0", "0.0 1000.0"),
    ("cylinder", "float", "phimax", "360.0", "0.0 360.0"),
    ("cone", "float", "height", "1.0", "0.0 1000.0"),
    ("cone", "float", "radius", "1.0", "0.0 1000.0"),
    ("cone", "float", "phimax", "360.0", "0.0 360.0"),
    ("paraboloid", "float", "radius", "1.0", "0.0 1000.0"),
    ("paraboloid", "float", "zmin", "0.0", "0.0 1000.0"),
    ("paraboloid", "float", "zmax", "1.0", "0.0 1000.0"),
    ("paraboloid", "float", "phimax", "360.0", "0.0 360.0"),
    ("hyperboloid", "point", "p1", "1.0 1.0 1.0", ""),
    ("hyperboloid", "point", "p2", "0.0 0.0 0.0", ""),
    ("hyperboloid", "float", "phimax", "360.0", "0.0 360.0"),
    ("loopsubdiv", "integer", "nlevels", "3", ""),
    ("loopsubdiv", "integer", "indices", "", ""),
    ("loopsubdiv", "point", "P", "", ""),
    ("loopsubdiv", "string", "scheme", "loop", ""),
];

#[derive(Debug, Clone)]
pub struct ShapeProperties;

impl ShapeProperties {
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
        return LazyCell::new(|| ShapeProperties::new());
    }
}
