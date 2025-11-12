use super::common::*;
use crate::model::base::*;
use std::cell::LazyCell;
use std::collections::HashMap;

pub const OPTION_PARAMETERS: [(&str, &str, &str, &str); 6] = [
    ("film", "string", "filename", ""),
    ("film", "integer", "xresolution", "1280"),
    ("film", "integer", "yresolution", "720"),
    ("film", "float", "cropwindow", "0.0 1.0 0.0 1.0"),
    ("film", "float", "scale", "1.0"),
    ("film", "float", "diagonal", "35.0"),
    //("film", "float", "maxsampleluminance", "inf"),
];

fn parse_floats(value: &str) -> Vec<f32> {
    let mut floats = Vec::new();
    for s in value.split_whitespace() {
        if s == "inf" {
            floats.push(f32::INFINITY);
        } else if s == "-inf" {
            floats.push(f32::NEG_INFINITY);
        } else if let Ok(f) = s.parse::<f32>() {
            floats.push(f);
        }
    }
    floats
}

fn parse_ints(value: &str) -> Vec<i32> {
    let mut ints = Vec::new();
    for s in value.split_whitespace() {
        if let Ok(i) = s.parse::<i32>() {
            ints.push(i);
        }
    }
    ints
}

fn parse_strings(value: &str) -> Vec<String> {
    let mut strings = Vec::new();
    for s in value.split_whitespace() {
        strings.push(s.to_string());
    }
    if strings.is_empty() {
        strings.push("".to_string());
    }
    strings
}

fn parse_bools(value: &str) -> Vec<bool> {
    let mut bools = Vec::new();
    for s in value.split_whitespace() {
        if let Ok(b) = s.parse::<bool>() {
            bools.push(b);
        }
    }
    bools
}

fn parse_parameter(param: (&str, &str, &str, &str)) -> (String, PropertyEntry) {
    let (name, key_type, key_name, value) = param;
    let key_type = key_type.to_string();
    let key_name = key_name.to_string();
    let value = match key_type.as_str() {
        "point" | "color" | "float" => Property::from(parse_floats(value)),
        "integer" => Property::from(parse_ints(value)),
        "string" | "spectrum" | "texture" => Property::from(parse_strings(value)),
        "bool" => Property::from(parse_bools(value)),
        _ => panic!("Unknown parameter type"),
    };
    (
        name.to_string(),
        PropertyEntry {
            key_type,
            key_name,
            default_value: value,
            value_range: None,
            ..Default::default()
        },
    )
}

#[derive(Debug, Clone)]
pub struct OptionProperties(pub HashMap<String, Vec<PropertyEntry>>);

impl OptionProperties {
    fn new() -> Self {
        let mut params = HashMap::new();
        for param in OPTION_PARAMETERS.iter() {
            let (name, entry) = parse_parameter(*param);
            params.entry(name).or_insert_with(Vec::new).push(entry);
        }
        OptionProperties(params)
    }

    pub fn get(&self, name: &str) -> Option<&Vec<PropertyEntry>> {
        self.0.get(name)
    }

    pub fn get_keys(&self, name: &str) -> Vec<(String, String)> {
        let mut keys = Vec::new();
        if let Some(params) = self.0.get(name) {
            for entry in params.iter() {
                keys.push((entry.key_type.to_string(), entry.key_name.to_string()));
            }
        }
        keys
    }

    pub fn get_instance() -> LazyCell<Self> {
        return LazyCell::new(|| OptionProperties::new());
    }
}
