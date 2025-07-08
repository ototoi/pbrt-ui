use super::component::Component;
use crate::model::base::*;

use crate::model::scene::shape::Shape;

use std::sync::Arc;
use std::sync::RwLock;

use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ShapeComponent {
    shape: Arc<RwLock<Shape>>,
}

fn replace_properties(props: &mut PropertyMap) {
    if let Some((_, key_name, _)) = props.entry_mut("levels") {
        *key_name = "nlevels".to_string();
    }
}

fn create_shape(name: &str, props: &PropertyMap) -> Arc<RwLock<Shape>> {
    let mesh = Shape::new(name, &props);
    Arc::new(RwLock::new(mesh))
}

impl ShapeComponent {
    pub fn new(shape_type: &str, name: &str, props: &PropertyMap) -> Self {
        let mut props = props.clone();
        props.insert("string type", Property::from(shape_type));
        props.insert("string name", Property::from(name));
        replace_properties(&mut props);
        let edition_id = Uuid::new_v4();
        props.insert("string edition", Property::from(edition_id.to_string()));
        let name = Self::get_name_from_type(shape_type);
        let mesh = create_shape(&name, &props);
        ShapeComponent { shape: mesh }
    }

    pub fn with_shape(shape: &Arc<RwLock<Shape>>) -> Self {
        let shape = shape.clone();
        ShapeComponent { shape }
    }

    pub fn get_shape(&self) -> Arc<RwLock<Shape>> {
        self.shape.clone()
    }

    pub fn is_ediable(t: &str) -> bool {
        match t {
            "sphere" => true,
            "disk" => true,
            "cylinder" => true,
            "cone" => true,
            "paraboloid" => true,
            "hyperboloid" => true,
            _ => false,
        }
    }

    pub fn get_name_from_type(t: &str) -> String {
        let name = match t {
            "trianglemesh" => "Mesh",
            "plymesh" => "Mesh",
            "sphere" => "Sphere",
            "disk" => "Disk",
            "cylinder" => "Cylinder",
            "cone" => "Cone",
            "paraboloid" => "Paraboloid",
            "hyperboloid" => "Hyperboloid",
            "loopsubdiv" => "Subdiv",
            _ => "Shape",
        };
        name.to_string()
    }
}

impl Component for ShapeComponent {}
