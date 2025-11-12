use super::common::*;
use super::value_range::*;
use crate::model::base::*;
use std::cell::LazyCell;
use std::collections::HashMap;

const TYPES: [&str; 3] = [
    "bvh",
    "kdtree",
    "exhaustive", //
];

const PARAMETERS: [(&str, &str, &str, &str, &str); 7] = [
    ("bvh", "string", "splitmethod", "middle", ""),
    ("bvh", "integer", "maxnodeprims", "4", "1 100"),
    ("kdtree", "integer", "intersectcost", "80", "0 100"),
    ("kdtree", "integer", "traversalcost", "1", "0 100"),
    ("kdtree", "float", "emptybonus", "0.5", "0.0 1.0"),
    ("kdtree", "integer", "maxprims", "1", "1 100"),
    ("kdtree", "integer", "maxdepth", "-1", ""),
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
        },
    );
}

#[derive(Debug, Clone)]
pub struct AcceleratorProperties(pub HashMap<String, Vec<PropertyEntry>>);

impl AcceleratorProperties {
    fn new() -> Self {
        let mut params = HashMap::new();
        for param in PARAMETERS.iter() {
            let (name, entry) = parse_parameter(*param);
            params.entry(name).or_insert_with(Vec::new).push(entry);
        }
        AcceleratorProperties(params)
    }

    pub fn get(&self, name: &str) -> Option<&Vec<PropertyEntry>> {
        self.0.get(name)
    }

    pub fn get_instance() -> LazyCell<Self> {
        return LazyCell::new(|| AcceleratorProperties::new());
    }
}

impl Properties for AcceleratorProperties {
    fn get_types(&self) -> Vec<String> {
        TYPES.iter().map(|s| s.to_string()).collect()
    }
    fn get_entries(&self, name: &str) -> Vec<PropertyEntry> {
        return self.get(name).cloned().unwrap_or_else(|| Vec::new());
    }
}
