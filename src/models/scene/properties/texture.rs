use super::common::*;
use super::value_range::*;
use crate::models::base::*;
use std::collections::HashMap;

const TYPES: [&str; 11] = [
    "imagemap",
    "constant",
    "scale",
    "mix",
    "bilerp",
    "checkerboard",
    "dots",
    "fbm",
    "wrinkled",
    "marble",
    "windy",
];

pub const PARAMETERS: [(&str, &str, &str, &str, &str); 30] = [
    ("imagemap", "string", "filename", "", ""),
    ("imagemap", "float", "maxanisotropy", "8.0", ""),
    ("imagemap", "bool", "trilinear", "false", ""),
    ("imagemap", "string", "wrap", "repeat", ""),
    ("imagemap", "string", "scale", "1.0", ""),
    ("imagemap", "bool", "gamma", "false", ""),
    ("constant", "color", "value", "1.0 1.0 1.0", ""),
    ("scale", "string", "tex1", "", ""),
    ("scale", "string", "tex2", "", ""),
    ("mix", "string", "tex1", "", ""),
    ("mix", "string", "tex2", "", ""),
    ("mix", "float", "amount", "0.5", ""),
    ("bilerp", "float", "v00", "0.0", ""),
    ("bilerp", "float", "v01", "1.0", ""),
    ("bilerp", "float", "v10", "0.0", ""),
    ("bilerp", "float", "v11", "1.0", ""),
    ("checkerboard", "color", "tex1", "1.0 1.0 1.0", ""),
    ("checkerboard", "color", "tex2", "0.0 0.0 0.0", ""),
    ("checkerboard", "integer", "dimension", "2", ""),
    ("checkerboard", "string", "aamode", "closedform", ""),
    ("dots", "string", "tex1", "", ""),
    ("dots", "string", "tex2", "", ""),
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

fn parse_parameter(
    param: (&str, &str, &str, &str, &str),
) -> (String, (String, String, Property, Option<ValueRange>)) {
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
    (name.to_string(), (key_type, key_name, value, range))
}

#[derive(Debug, Clone)]
pub struct TextureProperties(
    pub HashMap<String, Vec<(String, String, Property, Option<ValueRange>)>>,
);

impl TextureProperties {
    pub fn new() -> Self {
        let mut params = HashMap::new();
        for param in PARAMETERS.iter() {
            let (name, (key_type, key_name, value, range)) = parse_parameter(*param);
            params
                .entry(name)
                .or_insert_with(Vec::new)
                .push((key_type, key_name, value, range));
        }
        for (_key, values) in params.iter_mut() {
            for param in MAPPING_PARAMETERS.iter() {
                let (_name, (key_type, key_name, value, range)) = parse_parameter(*param);
                values.push((key_type, key_name, value, range));
            }
        }
        TextureProperties(params)
    }

    pub fn get(&self, name: &str) -> Option<&Vec<(String, String, Property, Option<ValueRange>)>> {
        self.0.get(name)
    }
}

impl Properties for TextureProperties {
    fn get_types(&self) -> Vec<String> {
        TYPES.iter().map(|s| s.to_string()).collect()
    }
    fn get_entries(&self, name: &str) -> Vec<(String, String, Property, Option<ValueRange>)> {
        let mut entries = Vec::new();
        if let Some(params) = self.0.get(name) {
            for (key_type, key_name, value, range) in params.iter() {
                entries.push((
                    key_type.to_string(),
                    key_name.to_string(),
                    value.clone(),
                    range.clone(),
                ));
            }
        }
        entries
    }
}
