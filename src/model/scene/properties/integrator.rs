use super::common::*;
use super::value_range::*;
use crate::model::base::*;
use std::cell::LazyCell;

const PARAMETERS: [(&str, &str, &str, &str, &str); 22] = [
    ("whitted", "integer", "maxdepth", "5", "1 10"), //
    //("whitted", "integer", "pixelbounds", "", ""),   //pixelbounds
    //
    ("directlighting", "integer", "maxdepth", "5", "1 10"),
    ("directlighting", "string", "strategy", "all", ""), //all, one
    //
    ("path", "integer", "maxdepth", "5", "1 10"), //
    ("path", "string", "lightsamplestrategy", "spatial", ""), //uniform, power, spatial,
    ("path", "float", "rrthreshold", "1.0", ""),  //
    //
    ("volpath", "integer", "maxdepth", "5", "1 10"), //
    ("volpath", "string", "lightsamplestrategy", "spatial", ""), //uniform, power, spatial,
    ("volpath", "float", "rrthreshold", "1.0", ""),  //
    //
    ("bdpt", "integer", "maxdepth", "5", "1 10"), //
    ("bdpt", "string", "lightsamplestrategy", "power", ""),
    //("bdpt", "bool", "visualizestrategies", "false", ""),
    //("bdpt", "bool", "visualizeweights", "false", ""),
    //
    ("mlt", "integer", "maxdepth", "5", "1 10"), //
    ("mlt", "integer", "mutationsperpixel", "100", "1 1000000"),
    ("mlt", "integer", "chains", "1000", "1 1000000"),
    ("mlt", "float", "largestepprobability", "0.3", "0.0 1.0"),
    ("mlt", "float", "sigma", "0.01", ""),
    //
    ("ambientocclusion", "integer", "nsamples", "64", "1 1000000"), //
    ("ambientocclusion", "bool", "cossample", "true", ""),          //
    //
    ("sppm", "integer", "maxdepth", "5", "1 10"), //
    ("sppm", "integer", "numiterations", "64", "1 1000000"), //iterations
    ("sppm", "integer", "photonsperiteration", "-1", ""),
    ("sppm", "float", "radius", "1.0", "0.0 1000000"),
    //
    //("aov", "string", "target", "uv", ""), //name
    //("aov", "float", "scale", "1.0", ""), //type
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
            key_name,
            key_type,
            default_value: value,
            value_range: range,
            ..Default::default()
        },
    );
}

#[derive(Debug, Clone)]
pub struct IntegratorProperties;

impl IntegratorProperties {
    fn new() -> Properties {
        let props: Vec<(String, PropertyEntry)> =
            PARAMETERS.iter().map(|p| parse_parameter(*p)).collect();
        Properties::new(&props)
    }
    pub fn get_instance() -> LazyCell<Properties> {
        return LazyCell::new(|| IntegratorProperties::new());
    }
}
