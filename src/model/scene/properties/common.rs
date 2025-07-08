use super::value_range::ValueRange;
use crate::model::base::*;

pub trait Properties {
    fn get_types(&self) -> Vec<String>;
    fn get_entries(&self, name: &str) -> Vec<(String, String, Property, Option<ValueRange>)>;
}
