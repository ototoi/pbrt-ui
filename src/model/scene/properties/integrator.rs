use super::common::*;
use std::cell::LazyCell;
use std::collections::HashMap;

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

#[derive(Debug, Clone)]
pub struct IntegratorProperties;

impl IntegratorProperties {
    fn new() -> Properties {
        let props: Vec<(String, PropertyEntry)> = PARAMETERS
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
        return LazyCell::new(|| IntegratorProperties::new());
    }
}
