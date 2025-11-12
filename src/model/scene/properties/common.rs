use super::value_range::ValueRange;
use crate::model::base::*;

#[derive(Debug, Clone)]
pub struct PropertyEntry {
    pub key_name: String,
    pub key_type: String,
    pub default_value: Property,
    pub value_range: Option<ValueRange>,
}

pub trait Properties {
    fn get_types(&self) -> Vec<String>;
    fn get_entries(&self, name: &str) -> Vec<PropertyEntry>;
}
