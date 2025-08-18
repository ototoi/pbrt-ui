mod from_distant;
mod from_point;
mod from_spot;
mod light_shape;

use crate::model::scene::LightComponent;
use from_distant::create_light_shape_from_distant;
use from_point::create_light_shape_from_point;
use from_spot::create_light_shape_from_spot;
pub use light_shape::LightShape;

use crate::model::scene::Node;
use std::sync::{Arc, RwLock};

pub fn create_light_shape(node: &Arc<RwLock<Node>>) -> Option<LightShape> {
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<LightComponent>() {
        let light = component.get_light();
        let light = light.read().unwrap();
        let props = light.as_property_map();
        if let Some(light_type) = props.find_one_string("string type") {
            match light_type.as_str() {
                "point" => {
                    return create_light_shape_from_point(props);
                }
                "spot" => {
                    return create_light_shape_from_spot(props);
                }
                "diffuse" | "area" => {
                    // Diffuse and area lights do not have a specific shape, return None
                    return None;
                }
                "distant" => {
                    return create_light_shape_from_distant(props);
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
    }
    return None;
}
