use super::common::*;
use std::cell::LazyCell;
use std::collections::HashMap;

const PARAMETERS: [(&str, &str, &str, &str, &str); 7] = [
    ("bvh", "string", "splitmethod", "middle", ""),
    ("bvh", "integer", "maxnodeprims", "4", "1 100"),
    ("kdtree", "integer", "intersectcost", "80", "0 100"),
    ("kdtree", "integer", "traversalcost", "1", "0 100"),
    ("kdtree", "float", "emptybonus", "0.5", "0.0 1.0"),
    ("kdtree", "integer", "maxprims", "1", "1 100"),
    ("kdtree", "integer", "maxdepth", "-1", ""),
];

#[derive(Debug, Clone)]
pub struct AcceleratorProperties;

impl AcceleratorProperties {
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
        return LazyCell::new(|| AcceleratorProperties::new());
    }
}
