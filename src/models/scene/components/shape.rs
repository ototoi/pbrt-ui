use super::component::Component;
use crate::models::base::*;

use crate::models::scene::mesh::Mesh;

use std::sync::Arc;
use std::sync::RwLock;

use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ShapeComponent {
    pub mesh: Arc<RwLock<Mesh>>,
}

fn create_mesh(name: &str, props: &PropertyMap) -> Arc<RwLock<Mesh>> {
    let mesh = Mesh::new(name, &props);
    Arc::new(RwLock::new(mesh))
}

impl ShapeComponent {
    pub fn new(shape_type: &str, props: &PropertyMap) -> Self {
        let mut props = props.clone();
        props.insert("string type", Property::from(shape_type));
        let edition_id = Uuid::new_v4();
        props.insert("string edition", Property::from(edition_id.to_string()));
        let name = Self::get_name_from_type(shape_type);
        let mesh = create_mesh(&name, &props);
        ShapeComponent { mesh }
    }

    pub fn get_name_from_type(t: &str) -> String {
        let name = match t {
            "sphere" => "Sphere",
            "disk" => "Disk",
            "cylinder" => "Cylinder",
            "cone" => "Cone",
            "paraboloid" => "Paraboloid",
            "hyperboloid" => "Hyperboloid",
            _ => "Shape",
        };
        name.to_string()
    }
}

impl Component for ShapeComponent {}
