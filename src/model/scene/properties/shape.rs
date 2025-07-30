use super::value_range::ValueRange;
use crate::model::base::*;
use std::collections::HashMap;

const PARAMETERS: [(&str, &str, &str, &str, &str); 33] = [
    //("trianglemesh", "integer", "indices", "", ""),
    //("trianglemesh", "point", "P", "", ""),
    //("trianglemesh", "normal", "N", "", ""),
    //("trianglemesh", "vector", "S", "", ""),
    //("trianglemesh", "float", "uv", "", ""),
    ("trianglemesh", "bool", "twosided", "true", ""),
    ("trianglemesh", "float", "alpha", "1.0", ""),
    ("trianglemesh", "float", "shadowalpha", "1.0", ""),
    ("plymesh", "string", "filename", "", ""),
    ("plymesh", "float", "alpha", "1.0", ""),
    ("plymesh", "bool", "twosided", "true", ""),
    ("plymesh", "float", "shadowalpha", "1.0", ""),
    ("sphere", "float", "radius", "1.0", "0.0 100.0"),
    ("sphere", "float", "zmin", "-1.0", "-100.0 0.0"),
    ("sphere", "float", "zmax", "1.0", "0.0 100.0"),
    ("sphere", "float", "phimax", "360.0", "0.0 360.0"),
    ("disk", "float", "height", "0.0", "0.0 100.0"),
    ("disk", "float", "radius", "1.0", "0.0 100.0"),
    ("disk", "float", "innerradius", "0.0", "0.0 100.0"),
    ("disk", "float", "phimax", "360.0", "0.0 360.0"),
    ("cylinder", "float", "radius", "1.0", "0.0 100.0"),
    ("cylinder", "float", "zmin", "-1.0", "-100.0 0.0"),
    ("cylinder", "float", "zmax", "1.0", "0.0 100.0"),
    ("cylinder", "float", "phimax", "360.0", "0.0 360.0"),
    ("cone", "float", "height", "1.0", "0.0 100.0"),
    ("cone", "float", "radius", "1.0", "0.0 100.0"),
    ("cone", "float", "phimax", "360.0", "0.0 360.0"),
    ("paraboloid", "float", "radius", "1.0", "0.0 100.0"),
    ("paraboloid", "float", "zmin", "0.0", "0.0 100.0"),
    ("paraboloid", "float", "zmax", "1.0", "0.0 100.0"),
    ("paraboloid", "float", "phimax", "360.0", "0.0 360.0"),
    ("hyperboloid", "point", "p1", "1.0 1.0 1.0", ""),
    ("hyperboloid", "point", "p2", "0.0 0.0 0.0", ""),
    ("hyperboloid", "float", "phimax", "360.0", "0.0 360.0"),
    ("loopsubdiv", "integer", "nlevels", "3", ""),
    ("loopsubdiv", "integer", "indices", "", ""),
    ("loopsubdiv", "point", "P", "", ""),
    ("loopsubdiv", "string", "scheme", "loop", ""),
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
        "point" | "vector" | "normal" | "color" | "float" => Property::from(parse_floats(value)),
        "integer" => Property::from(parse_ints(value)),
        "string" | "spectrum" | "texture" => Property::from(parse_strings(value)),
        "bool" => Property::from(parse_bools(value)),
        _ => panic!("Unknown parameter type: {}", key_type),
    };
    let range = parse_range(&key_type, range);

    (name.to_string(), (key_type, key_name, value, range))
}

#[derive(Debug, Clone)]
pub struct ShapeProperties(
    pub HashMap<String, Vec<(String, String, Property, Option<ValueRange>)>>,
);

impl ShapeProperties {
    pub fn new() -> Self {
        let mut params = HashMap::new();
        for param in PARAMETERS.iter() {
            let (name, (key_type, key_name, value, range)) = parse_parameter(*param);
            params
                .entry(name)
                .or_insert_with(Vec::new)
                .push((key_type, key_name, value, range));
        }
        ShapeProperties(params)
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
}
