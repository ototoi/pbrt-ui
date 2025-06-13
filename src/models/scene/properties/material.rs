use super::value_range::ValueRange;
use crate::models::base::*;

use std::collections::HashMap;

pub const V3_MATERIAL_NAMES: [&str; 14] = [
    "matte",
    "plastic",
    "translucent",
    "glass",
    "mirror",
    "hair",
    "mix",
    "metal",
    "substrate",
    "subsurface",
    "kdsubsurface",
    "uber",
    "fourier",
    "disney",
];

//type, key_type, key_name, value
pub const V3_MATERIAL_PARAMETERS: [(&str, &str, &str, &str, &str); 98] = [
    ("matte", "color", "Kd", "0.5 0.5 0.5", ""),
    ("matte", "float", "sigma", "0.0", ""),
    ("matte", "texture", "bumpmap", "", ""),
    //
    ("plastic", "color", "Kd", "0.25 0.25 0.25", ""),
    ("plastic", "color", "Ks", "0.25 0.25 0.25", ""),
    ("plastic", "float", "roughness", "0.0", "0.0 1.0"),
    ("plastic", "texture", "bumpmap", "", ""),
    ("plastic", "bool", "remaproughness", "true", "0.0 1.0"),
    //
    ("translucent", "color", "Kd", "0.25 0.25 0.25", ""),
    ("translucent", "color", "Ks", "0.25 0.25 0.25", ""),
    ("translucent", "color", "reflect", "0.25 0.25 0.25", ""),
    ("translucent", "color", "transmit", "0.25 0.25 0.25", ""),
    ("translucent", "float", "roughness", "0.1", "0.0 1.0"),
    ("translucent", "texture", "bumpmap", "", ""),
    ("translucent", "bool", "remaproughness", "true", ""),
    //
    ("glass", "color", "Kr", "1.0 1.0 1.0", ""),
    ("glass", "color", "Kt", "1.0 1.0 1.0", ""),
    ("glass", "color", "reflect", "0.0 0.0 0.0", ""),
    ("glass", "float", "uroughness", "0.0", "0.0 1.0"),
    ("glass", "float", "vroughness", "0.0", "0.0 1.0"),
    ("glass", "float", "eta", "1.5", "0.0 10.0"),
    ("glass", "texture", "bumpmap", "", ""),
    ("glass", "bool", "remaproughness", "true", ""),
    //
    ("mirror", "color", "Kr", "0.9 0.9 0.9", ""),
    ("mirror", "texture", "bumpmap", "", ""),
    //
    ("hair", "color", "sigma_a", "1.0 1.0 1.0", ""), //
    ("hair", "color", "color", "0.0 0.0 0.0", ""),   //
    ("hair", "color", "eumelanin", "0.0 0.0 0.0", ""), //
    ("hair", "color", "pheomelanin", "0.0 0.0 0.0", ""), //
    ("hair", "float", "eta", "1.55", "0.0 10.0"),    //
    ("hair", "float", "beta_m", "0.3", ""),          //
    ("hair", "float", "beta_n", "0.3", ""),          //
    ("hair", "float", "alpha", "2.0", ""),           //
    //
    ("mix", "color", "amount", "0.5 0.5 0.5", ""),
    ("mix", "string", "namedmaterial1", "", ""),
    ("mix", "string", "namedmaterial2", "", ""),
    //
    ("metal", "spectrum", "eta", "", ""), //
    ("metal", "spectrum", "k", "", ""),
    ("metal", "float", "roughness", "0.01", "0.0 1.0"),
    ("metal", "float", "uroughness", "0.0", "0.0 1.0"),
    ("metal", "float", "vroughness", "0.0", "0.0 1.0"),
    ("metal", "texture", "bumpmap", "", ""),
    ("metal", "bool", "remaproughness", "true", ""),
    //
    ("substrate", "color", "Kd", "0.5 0.5 0.5", ""),
    ("substrate", "color", "Ks", "0.5 0.5 0.5", ""),
    ("substrate", "float", "uroughness", "0.1", "0.0 1.0"),
    ("substrate", "float", "vroughness", "0.1", "0.0 1.0"),
    ("substrate", "texture", "bumpmap", "", ""),
    //
    ("subsurface", "string", "name", "", ""),
    ("subsurface", "float", "scale", "1.0", ""),
    ("subsurface", "color", "Kr", "1.0 1.0 1.0", ""),
    ("subsurface", "color", "sigma_a", "0.0011 0.0024 0.014", ""),
    ("subsurface", "color", "sigma_s", "2.55 3.21 3.77", ""),
    ("subsurface", "float", "g", "0.0", ""),
    ("subsurface", "float", "eta", "1.33", "0.0 10.0"),
    ("subsurface", "float", "uroughness", "0.0", "0.0 1.0"),
    ("subsurface", "float", "vroughness", "0.0", "0.0 1.0"),
    ("subsurface", "texture", "bumpmap", "", ""),
    ("subsurface", "bool", "remaproughness", "true", ""),
    //
    ("kdsubsurface", "float", "scale", "1.0", ""),
    ("kdsubsurface", "color", "Kd", "0.5 0.5 0.5", ""),
    ("kdsubsurface", "color", "Kr", "1.0 1.0 1.0", ""),
    ("kdsubsurface", "color", "Kt", "1.0 1.0 1.0", ""),
    ("kdsubsurface", "color", "mfp", "1.0 1.0 1.0", ""),
    ("kdsubsurface", "float", "g", "0.0", ""),
    ("kdsubsurface", "float", "eta", "1.33", "0.0 10.0"),
    ("kdsubsurface", "float", "uroughness", "0.0", "0.0 10.0"),
    ("kdsubsurface", "float", "vroughness", "0.0", "0.0 10.0"),
    ("kdsubsurface", "texture", "bumpmap", "", ""),
    ("kdsubsurface", "bool", "remaproughness", "true", ""),
    //
    ("uber", "color", "Kd", "0.25 0.25 0.25", ""),
    ("uber", "color", "Ks", "0.25 0.25 0.25", ""),
    ("uber", "color", "Kr", "0.0 0.0 0.0", ""),
    ("uber", "color", "Kt", "0.0 0.0 0.0", ""),
    ("uber", "float", "roughness", "0.1", "0.0 1.0"),
    ("uber", "float", "uroughness", "0.1", "0.0 1.0"),
    ("uber", "float", "vroughness", "0.1", "0.0 1.0"),
    ("uber", "float", "eta", "1.5", "0.0 10.0"),
    ("uber", "texture", "bumpmap", "", ""),
    ("uber", "color", "opacity", "1.0 1.0 1.0", ""),
    //
    ("fourier", "string", "bsdffile", "", ""),
    ("fourier", "texture", "bumpmap", "", ""),
    //
    ("disney", "color", "color", "0.5 0.5 0.5", ""),
    ("disney", "float", "metallic", "0.0", ""),
    ("disney", "float", "eta", "1.5", "0.0 10.0"),
    ("disney", "float", "roughness", "0.5", "0.0 1.0"),
    ("disney", "float", "speculartint", "0.0", "0.0 1.0"),
    ("disney", "float", "anisotropic", "0.0", "0.0 1.0"),
    ("disney", "float", "sheen", "0.0", "0.0 1.0"),
    ("disney", "float", "sheentint", "0.5", "0.0 1.0"),
    ("disney", "float", "clearcoat", "0.0", "0.0 1.0"),
    ("disney", "float", "clearcoatgloss", "1.0", "0.0 1.0"),
    ("disney", "float", "spectrans", "0.0", "0.0 1.0"),
    ("disney", "color", "scatterdistance", "0.0 0.0 0.0", ""),
    ("disney", "bool", "thin", "false", ""),
    ("disney", "float", "flatness", "0.0", "0.0 1.0"),
    ("disney", "float", "difftrans", "1.0", "0.0 1.0"),
    ("disney", "texture", "bumpmap", "", ""),
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
        "color" | "float" => Property::from(parse_floats(value)),
        "integer" => Property::from(parse_ints(value)),
        "string" | "spectrum" | "texture" => Property::from(parse_strings(value)),
        "bool" => Property::from(parse_bools(value)),
        _ => panic!("Unknown parameter type"),
    };
    let range = parse_range(&key_type, range);
    (name.to_string(), (key_type, key_name, value, range))
}

#[derive(Debug, Clone)]
pub struct MaterialProperties(
    pub HashMap<String, Vec<(String, String, Property, Option<ValueRange>)>>,
);

impl MaterialProperties {
    pub fn new() -> Self {
        let mut params = HashMap::new();
        for param in V3_MATERIAL_PARAMETERS.iter() {
            let (name, (key_type, key_name, value, range)) = parse_parameter(*param);
            params
                .entry(name)
                .or_insert_with(Vec::new)
                .push((key_type, key_name, value, range));
        }
        Self(params)
    }

    pub fn get_types(&self) -> Vec<String> {
        /*
        let mut types = Vec::new();
        for (name, _) in self.0.iter() {
            if !types.contains(name) {
                types.push(name.to_string());
            }
        }
        return types;
        */
        V3_MATERIAL_NAMES
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<String>>()
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
