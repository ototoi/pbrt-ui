use super::common::*;
use std::cell::LazyCell;
use std::collections::HashMap;

const PARAMETERS: [(&str, &str, &str, &str, &str); 12] = [
    (
        "lowdiscrepancy",
        "integer",
        "pixelsamples",
        "16",
        "1 1000000",
    ), //
    ("lowdiscrepancy", "integer", "dimensions", "4", ""), //
    //
    ("maxmindist", "integer", "pixelsamples", "16", "1 1000000"), //
    ("maxmindist", "integer", "dimensions", "4", ""),             //
    //
    ("halton", "integer", "pixelsamples", "16", "1 1000000"), //
    ("halton", "bool", "samplepixelcenter", "false", ""),     //
    //
    ("sobol", "integer", "pixelsamples", "16", "1 1000000"), //
    //
    ("random", "integer", "pixelsamples", "4", "1 1000000"), //
    //
    ("stratified", "bool", "jitter", "true", ""), //
    ("stratified", "integer", "xsamples", "4", "1 1000000"), //
    ("stratified", "integer", "ysamples", "4", "1 1000000"), //
    ("stratified", "integer", "dimensions", "4", ""), //
];

#[derive(Debug, Clone)]
pub struct SamplerProperties;

impl SamplerProperties {
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
        return LazyCell::new(|| SamplerProperties::new());
    }
}
