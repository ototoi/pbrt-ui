use super::common::*;
use super::value_range::*;
use crate::model::base::*;
use std::cell::LazyCell;

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
pub struct TextureProperties;

impl TextureProperties {
    fn new() -> BasicProperties {
        let props: Vec<(String, PropertyEntry)> =
            PARAMETERS.iter().map(|p| parse_parameter(*p)).collect();
        BasicProperties::new(&props)
    }
    pub fn get_instance() -> LazyCell<BasicProperties> {
        return LazyCell::new(|| TextureProperties::new());
    }
}
