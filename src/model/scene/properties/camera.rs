use super::common::*;
use super::value_range::*;
use crate::model::base::*;
use std::cell::LazyCell;

const TYPES: [&str; 4] = ["perspective", "realistic", "orthographic", "environment"];

const PARAMETERS: [(&str, &str, &str, &str, &str); 19] = [
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
    ("perspective", "float", "znear", "0.01", ""),
    ("perspective", "float", "zfar", "10000.0", ""),
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

fn parse_parameter(param: (&str, &str, &str, &str, &str)) -> (String, PropertyEntry) {
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
    return (
        name.to_string(),
        PropertyEntry {
            key_type,
            key_name,
            default_value: value,
            value_range: range,
            ..Default::default()
        },
    );
}

#[derive(Debug, Clone)]
pub struct CameraProperties(BasicProperties);

impl CameraProperties {
    fn new() -> Self {
        let props: Vec<(String, PropertyEntry)> =
            PARAMETERS.iter().map(|p| parse_parameter(*p)).collect();
        CameraProperties(BasicProperties::new(&props))
    }
    pub fn get_instance() -> LazyCell<Self> {
        return LazyCell::new(|| CameraProperties::new());
    }
}

impl Properties for CameraProperties {
    fn get_types(&self) -> Vec<String> {
        TYPES.iter().map(|s| s.to_string()).collect()
    }
    fn get_entries(&self, name: &str) -> Option<&Vec<PropertyEntry>> {
        self.0.get_entries(name)
    }
}
