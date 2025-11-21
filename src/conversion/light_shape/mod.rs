mod from_area;
mod from_distant;
mod from_point;
mod from_spot;
mod light_shape;

use crate::model::scene::LightComponent;
use crate::model::scene::ShapeComponent;
use from_area::create_light_shape_from_area;
use from_distant::create_light_shape_from_distant;
use from_point::create_light_shape_from_point;
use from_spot::create_light_shape_from_spot;
pub use light_shape::LightShape;

use crate::model::scene::Node;
use std::sync::{Arc, RwLock};

pub fn create_light_shape(node: &Arc<RwLock<Node>>) -> Option<LightShape> {
    let node = node.read().unwrap();
    if let Some(light_component) = node.get_component::<LightComponent>() {
        let light = light_component.get_light();
        let light = light.read().unwrap();
        let light_type = light.get_type();
        match light_type.as_str() {
            "point" => {
                return create_light_shape_from_point(&light);
            }
            "spot" => {
                return create_light_shape_from_spot(&light);
            }
            "diffuse" | "area" => {
                if let Some(shape_component) = node.get_component::<ShapeComponent>() {
                    let shape = shape_component.get_shape();
                    let shape = shape.read().unwrap();
                    return create_light_shape_from_area(&light, &shape);
                }
            }
            "distant" => {
                return create_light_shape_from_distant(&light);
            }
            "goniometric" | "projection" | "infinite" => {
                // These light types are not implemented yet
                //log::warn!("Light type '{}' is not implemented yet", light_type);
                return None;
            }
            _ => {
                log::warn!("Unknown light type: {}", light_type);
            }
        }
    }
    return None;
}
