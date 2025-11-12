use super::common::*;
use super::value_range::ValueRange;
use crate::model::base::*;
use std::cell::LazyCell;
use std::collections::HashMap;

pub const LIGHT_PARAMETERS: [(&str, &str, &str, &str, &str); 27] = [
    ("point", "color", "I", "1.0 1.0 1.0", ""),
    ("point", "color", "scale", "1.0 1.0 1.0", ""),
    ("point", "point", "from", "0.0 0.0 0.0", ""),
    //
    ("spot", "color", "I", "1.0 1.0 1.0", ""),
    ("spot", "color", "scale", "1.0 1.0 1.0", ""),
    ("spot", "float", "coneangle", "30.0", "0.0 90.0"),
    ("spot", "float", "conedeltaangle", "5.0", "0.0 90.0"), //if conedelta
    ("spot", "point", "from", "0.0 0.0 0.0", ""),
    ("spot", "point", "to", "0.0 0.0 1.0", ""),
    ("goniometric", "color", "L", "1.0 1.0 1.0", ""),
    ("goniometric", "color", "scale", "1.0 1.0 1.0", ""),
    ("goniometric", "texture", "mapname", "", ""),
    //
    ("projection", "color", "I", "1.0 1.0 1.0", ""),
    ("projection", "color", "scale", "1.0 1.0 1.0", ""),
    ("projection", "float", "fov", "45.0", "0.0 90.0"),
    //
    ("distant", "color", "L", "1.0 1.0 1.0", ""),
    ("distant", "color", "scale", "1.0 1.0 1.0", ""),
    //("distant", "point", "from", "0.0 0.0 0.0", ""),
    //("distant", "point", "to", "0.0 0.0 1.0", ""),

    //
    ("infinite", "color", "L", "1.0 1.0 1.0", ""),
    ("infinite", "color", "scale", "1.0 1.0 1.0", ""),
    ("infinite", "texture", "mapname", "", ""),
    ("infinite", "integer", "nsamples", "1", "1 100000"), //samples
    //
    ("diffuse", "color", "L", "1.0 1.0 1.0", ""),
    ("diffuse", "color", "scale", "1.0 1.0 1.0", ""),
    ("diffuse", "integer", "nsamples", "1", "1 100000"),
    ("diffuse", "bool", "twosided", "false", ""),
    ("diffuse", "float", "coneangle", "90.0", "0.0 90.0"),
    ("diffuse", "float", "conedeltaangle", "90.0", "0.0 90.0"),
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
pub struct LightProperties(
    pub HashMap<String, Vec<(String, String, Property, Option<ValueRange>)>>,
);

impl LightProperties {
    fn new() -> Self {
        let mut params = HashMap::new();
        for param in LIGHT_PARAMETERS.iter() {
            let (name, (key_type, key_name, value, range)) = parse_parameter(*param);
            params
                .entry(name)
                .or_insert_with(Vec::new)
                .push((key_type, key_name, value, range));
        }
        LightProperties(params)
    }

    pub fn get(&self, name: &str) -> Option<&Vec<(String, String, Property, Option<ValueRange>)>> {
        self.0.get(name)
    }

    pub fn get_keys(&self, name: &str) -> Vec<(String, String)> {
        let mut keys = Vec::new();
        if let Some(params) = self.0.get(name) {
            for (key_type, key_name, _, _) in params.iter() {
                keys.push((key_type.to_string(), key_name.to_string()));
            }
        }
        keys
    }

    pub fn get_instance() -> LazyCell<Self> {
        return LazyCell::new(|| LightProperties::new());
    }
}

impl Properties for LightProperties {
    fn get_types(&self) -> Vec<String> {
        vec![
            "point",
            "spot",
            "goniometric",
            "projection",
            "distant",
            "infinite",
            "diffuse",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
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
