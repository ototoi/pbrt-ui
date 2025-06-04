use super::common::*;
use super::value_range::*;
use crate::models::base::*;
use std::collections::HashMap;

const TYPES: [&str; 4] = ["perspective", "realistic", "orthographic", "environment"];

const PARAMETERS: [(&str, &str, &str, &str, &str); 17] = [
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
        "point" | "color" | "float" => Property::from(parse_floats(value)),
        "integer" => Property::from(parse_ints(value)),
        "string" | "spectrum" | "texture" => Property::from(parse_strings(value)),
        "bool" => Property::from(parse_bools(value)),
        _ => panic!("Unknown parameter type"),
    };
    let range = parse_range(&key_type, range);
    (name.to_string(), (key_type, key_name, value, range))
}

#[derive(Debug, Clone)]
pub struct CameraProperties(
    pub HashMap<String, Vec<(String, String, Property, Option<ValueRange>)>>,
);

impl CameraProperties {
    pub fn new() -> Self {
        let mut params = HashMap::new();
        for param in PARAMETERS.iter() {
            let (name, (key_type, key_name, value, range)) = parse_parameter(*param);
            params
                .entry(name)
                .or_insert_with(Vec::new)
                .push((key_type, key_name, value, range));
        }
        CameraProperties(params)
    }

    pub fn get(&self, name: &str) -> Option<&Vec<(String, String, Property, Option<ValueRange>)>> {
        self.0.get(name)
    }
}

impl Properties for CameraProperties {
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
