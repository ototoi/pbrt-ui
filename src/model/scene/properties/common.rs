use super::value_range::ValueRange;
use crate::model::base::*;

#[derive(Debug, Clone)]
pub struct PropertyEntry {
    pub key_name: String,
    pub key_type: String,
    pub default_value: Property,
    pub value_range: Option<ValueRange>,
    pub show_in_ui: bool,     //
    pub output_to_file: bool, // *.pbrt
}

impl Default for PropertyEntry {
    fn default() -> Self {
        PropertyEntry {
            key_name: String::new(),
            key_type: String::new(),
            default_value: Property::default(),
            value_range: None,
            show_in_ui: true,
            output_to_file: true,
        }
    }
}

pub trait Properties {
    fn get_types(&self) -> Vec<String>;
    fn get_entries(&self, name: &str) -> Vec<PropertyEntry>;
}
