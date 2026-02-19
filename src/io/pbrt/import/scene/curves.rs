use crate::model::base::Matrix4x4;
use crate::model::base::ParamSet;
use crate::model::base::Property;
use crate::model::base::PropertyMap;
use crate::model::scene::Material;

use std::sync::Arc;
use std::sync::RwLock;

const APPENDABLE_KEYS: [&str; 9] = [
    "P", "N", "width", "width0", "width1", "normal", "u", "v", "uv",
];

#[derive(Default)]
pub struct CurvesState {
    pub curve_edition: String,
    pub curves: Vec<ParamSet>,
    pub transform: Option<Matrix4x4>,
    pub material: Option<Arc<RwLock<Material>>>,
}

impl CurvesState {
    pub fn create_curve_edition(material_id: Option<&str>, transform: &Matrix4x4) -> String {
        let mut transform_text = String::new();
        for value in transform.m.iter() {
            let value = if value.abs() < 1e-8 { 0.0 } else { *value };
            transform_text.push_str(&format!("{:.9},", value));
        }
        let material_id = material_id.unwrap_or("none");
        format!("material={};transform={}", material_id, transform_text)
    }

    pub fn clear(&mut self) {
        self.curve_edition.clear();
        self.curves.clear();
        self.transform = None;
        self.material = None;
    }

    pub fn push(&mut self, params: &ParamSet) {
        self.curves.push(params.clone());
    }

    pub fn merged_params(&self) -> Option<ParamSet> {
        let mut merged = self.curves.first()?.clone();
        for curve in self.curves.iter().skip(1) {
            append_curve_params(&mut merged, curve);
        }
        Some(merged)
    }
}

fn append_curve_params(dst: &mut PropertyMap, src: &ParamSet) {
    for (key_type, key_name, src_prop) in src.0.iter() {
        if !APPENDABLE_KEYS.contains(&key_name.as_str()) {
            continue;
        }
        if let Some(dst_prop) = dst.get_mut(key_name) {
            match (dst_prop, src_prop) {
                (Property::Strings(dst_values), Property::Strings(src_values)) => {
                    dst_values.extend(src_values.clone());
                }
                (Property::Floats(dst_values), Property::Floats(src_values)) => {
                    dst_values.extend(src_values.clone());
                }
                (Property::Ints(dst_values), Property::Ints(src_values)) => {
                    dst_values.extend(src_values.clone());
                }
                (Property::Bools(dst_values), Property::Bools(src_values)) => {
                    dst_values.extend(src_values.clone());
                }
                _ => {
                    // Type mismatch is ignored; keep original property.
                }
            }
        } else {
            let key = PropertyMap::get_key(key_type, key_name);
            dst.insert(&key, src_prop.clone());
        }
    }
}
