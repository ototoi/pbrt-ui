use super::common::*;
use super::value_range::ValueRange;
use crate::model::base::*;
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

fn parse_floats(value: &str) -> Vec<f32> {
    let mut floats = Vec::new();
    for s in value.split_whitespace() {
        if let Ok(f) = s.parse::<f32>() {
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

fn parse_range(key_type: &str, range: &str) -> Option<ValueRange> {
    if range.is_empty() {
        return None;
    }
    match key_type {
        "float" => {
            let mut range = range.split_whitespace();
            let min = range.next().unwrap_or("0.0").parse::<f32>().unwrap_or(0.0);
            let max = range.next().unwrap_or("1.0").parse::<f32>().unwrap_or(1.0);
            Some(ValueRange::FloatRange(min, max))
        }
        "integer" => {
            let mut range = range.split_whitespace();
            let min = range.next().unwrap_or("0").parse::<i32>().unwrap_or(0);
            let max = range.next().unwrap_or("1").parse::<i32>().unwrap_or(1);
            Some(ValueRange::IntRange(min, max))
        }
        _ => None,
    }
}

fn parse_parameter(param: (&str, &str, &str, &str, &str)) -> (String, PropertyEntry) {
    let (name, key_type, key_name, value, range) = param;
    let key_type = key_type.to_string();
    let key_name = key_name.to_string();
    let value = match key_type.as_str() {
        "point" | "vector" | "normal" | "color" | "float" => Property::from(parse_floats(value)),
        "integer" => Property::from(parse_ints(value)),
        "string" | "spectrum" | "texture" => Property::from(parse_strings(value)),
        "bool" => Property::from(parse_bools(value)),
        _ => panic!("Unknown parameter type"),
    };
    let range = parse_range(&key_type, range);

    (
        name.to_string(),
        PropertyEntry {
            key_type,
            key_name,
            default_value: value,
            value_range: range,
            ..Default::default()
        },
    )
}

#[derive(Debug, Clone)]
pub struct MappingProperties(pub HashMap<String, Vec<PropertyEntry>>);

impl MappingProperties {
    fn new() -> Self {
        let mut params = HashMap::new();
        for param in PARAMETERS.iter() {
            let (name, entry) = parse_parameter(*param);
            params.entry(name).or_insert_with(Vec::new).push(entry);
        }
        MappingProperties(params)
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
        return LazyCell::new(|| MappingProperties::new());
    }
}
