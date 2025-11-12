use super::common::*;
use std::cell::LazyCell;
use std::collections::HashMap;

//type, key_type, key_name, value
pub const V3_MATERIAL_PARAMETERS: [(&str, &str, &str, &str, &str); 100] = [
    ("matte", "color", "Kd", "0.5 0.5 0.5", ""),
    ("matte", "float", "sigma", "0.0", ""),
    ("matte", "texture", "bumpmap", "", ""),
    //
    ("plastic", "color", "Kd", "0.25 0.25 0.25", ""),
    ("plastic", "color", "Ks", "0.25 0.25 0.25", ""),
    ("plastic", "float", "roughness", "0.0", "0.0 1.0"),
    ("plastic", "texture", "bumpmap", "", ""),
    ("plastic", "bool", "remaproughness", "true", ""),
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
    ("substrate", "bool", "remaproughness", "true", ""),
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
    ("uber", "bool", "remaproughness", "true", ""),
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

#[derive(Debug, Clone)]
pub struct MaterialProperties;

impl MaterialProperties {
    fn new() -> Properties {
        let props: Vec<(String, PropertyEntry)> = V3_MATERIAL_PARAMETERS
            .iter()
            .map(|(name, key_type, key_name, default_value, value_range)| {
                let mut param = HashMap::new();
                param.insert(PropetyParseKey::Name, name.to_string());
                param.insert(PropetyParseKey::KeyType, key_type.to_string());
                param.insert(PropetyParseKey::KeyName, key_name.to_string());
                param.insert(PropetyParseKey::DefaultValue, default_value.to_string());
                param.insert(PropetyParseKey::ValueRange, value_range.to_string());
                return parse_property_entry(&param);
            })
            .collect();
        Properties::new(&props)
    }
    pub fn get_instance() -> LazyCell<Properties> {
        return LazyCell::new(|| MaterialProperties::new());
    }
}
