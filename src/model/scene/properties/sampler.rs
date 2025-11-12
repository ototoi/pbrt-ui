use super::common::*;
use super::value_range::*;
use crate::model::base::*;
use std::cell::LazyCell;
use std::collections::HashMap;

const TYPES: [&str; 6] = [
    "lowdiscrepancy", //  "02sequence",
    "maxmindist",
    "halton",
    "sobol",
    "random",
    "stratified",
];

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
pub struct SamplerProperties(BasicProperties);

impl SamplerProperties {
    fn new() -> Self {
        let props: Vec<(String, PropertyEntry)> =
            PARAMETERS.iter().map(|p| parse_parameter(*p)).collect();
        SamplerProperties(BasicProperties::new(&props))
    }
    pub fn get_instance() -> LazyCell<Self> {
        return LazyCell::new(|| SamplerProperties::new());
    }
}

impl Properties for SamplerProperties {
    fn get_types(&self) -> Vec<String> {
        TYPES.iter().map(|s| s.to_string()).collect()
    }
    fn get_entries(&self, name: &str) -> Option<&Vec<PropertyEntry>> {
        self.0.get_entries(name)
    }
}
