use super::value_range::ValueRange;
use crate::model::base::*;

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct PropertyEntry {
    pub key_name: String,
    pub key_type: String,
    pub default_value: Property,
    pub value_range: Option<ValueRange>,
    pub show_in_inspector: bool, //
    pub output_to_file: bool,    // *.pbrt
}

impl Default for PropertyEntry {
    fn default() -> Self {
        PropertyEntry {
            key_name: String::new(),
            key_type: String::new(),
            default_value: Property::default(),
            value_range: None,
            show_in_inspector: true,
            output_to_file: true,
        }
    }
}

pub trait Properties {
    fn get_types(&self) -> Vec<String>;
    fn get_entries(&self, name: &str) -> Option<&Vec<PropertyEntry>>;
}

#[derive(Debug, Clone)]
pub struct BasicProperties(pub HashMap<String, Vec<PropertyEntry>>);
impl BasicProperties {
    pub fn new(props: &[(String, PropertyEntry)]) -> Self {
        let mut params = HashMap::new();
        for (name, entry) in props.iter() {
            params
                .entry(name.clone())
                .or_insert_with(Vec::new)
                .push(entry.clone());
        }
        BasicProperties(params)
    }

    pub fn get_entries(&self, name: &str) -> Option<&Vec<PropertyEntry>> {
        self.0.get(name)
    }
}
