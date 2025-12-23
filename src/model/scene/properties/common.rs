use super::value_range::ValueRange;
use crate::model::base::*;

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct PropertyEntry {
    pub key_name: String,
    pub key_type: String,
    pub default_value: Property,
    pub value_range: Option<ValueRange>,
    pub show_in_inspector: bool, //
    pub output_to_file: bool,    // *.pbrt
}

impl Default for PropertyEntry {
    fn default() -> Self {
        PropertyEntry {
            key_name: String::new(),
            key_type: String::new(),
            default_value: Property::default(),
            value_range: None,
            show_in_inspector: true,
            output_to_file: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Properties {
    pub params: HashMap<String, Vec<PropertyEntry>>,
    pub keys: Vec<String>,
}

impl Properties {
    pub fn new(props: &[(String, PropertyEntry)]) -> Self {
        let mut params = HashMap::new();
        let mut keys = Vec::new();
        for (name, entry) in props.iter() {
            params
                .entry(name.clone())
                .or_insert_with(Vec::new)
                .push(entry.clone());
            if !keys.contains(name) {
                keys.push(name.clone());
            }
        }
        Properties { params, keys }
    }

    pub fn get_entries(&self, name: &str) -> Option<&Vec<PropertyEntry>> {
        self.params.get(name)
    }

    pub fn get_types(&self) -> &Vec<String> {
        &self.keys
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropetyParseKey {
    Name,
    KeyType,
    KeyName,
    DefaultValue,
    ValueRange,
}

pub fn parse_property_entry(param: &HashMap<PropetyParseKey, String>) -> (String, PropertyEntry) {
    let name = param
        .get(&PropetyParseKey::Name)
        .unwrap_or(&"".to_string())
        .clone();
    let key_type = param
        .get(&PropetyParseKey::KeyType)
        .unwrap_or(&"".to_string())
        .clone();
    let key_name = param
        .get(&PropetyParseKey::KeyName)
        .unwrap_or(&"".to_string())
        .clone();
    let default_value = param
        .get(&PropetyParseKey::DefaultValue)
        .unwrap_or(&"".to_string())
        .clone();
    let value_range = param
        .get(&PropetyParseKey::ValueRange)
        .unwrap_or(&"".to_string())
        .clone();
    let default_value = match key_type.as_str() {
        "point" | "color" | "vector" | "normal" | "float" => {
            Property::from(parse_floats(&default_value))
        }
        "integer" => Property::from(parse_ints(&default_value)),
        "string" | "spectrum" | "texture" => Property::from(parse_strings(&default_value)),
        "bool" => Property::from(parse_bools(&default_value)),
        _ => panic!("Unknown parameter type: {}", key_type),
    };
    let value_range = parse_range(&key_type, &value_range);
    return (
        name.to_string(),
        PropertyEntry {
            key_type,
            key_name,
            default_value,
            value_range,
            ..Default::default()
        },
    );
}
