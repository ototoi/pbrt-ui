mod from_point;
mod from_spot;
mod light_shape;

use crate::model::scene::Light;
use from_point::create_light_shape_from_point;
use from_spot::create_light_shape_from_spot;
pub use light_shape::LightShape;

pub fn create_light_shape(light: &Light) -> Option<LightShape> {
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
            "goniometric" | "projection" | "distant" | "infinite" => {
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
