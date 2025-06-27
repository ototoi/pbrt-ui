use super::component::Component;
use crate::models::base::*;

use crate::models::scene::mesh::Mesh;

use std::sync::Arc;
use std::sync::RwLock;

//use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct SubdivComponent {
    pub mesh: Arc<RwLock<Mesh>>,
}

fn create_mesh(name: &str, props: &PropertyMap) -> Arc<RwLock<Mesh>> {
    let mesh = Mesh::new(name, &props);
    Arc::new(RwLock::new(mesh))
}

fn replace_properties(props: &mut PropertyMap) {
    if let Some((_, key_name, _)) = props.entry_mut("levels") {
        *key_name = "nlevels".to_string();
    }
}

impl SubdivComponent {
    pub fn new(t: &str, props: &PropertyMap) -> Self {
        let mut props = props.clone();
        props.add_string("string type", &t);
        replace_properties(&mut props);
        let name = Self::get_name_from_type(t);
        let mesh = create_mesh(&name, &props);
        SubdivComponent { mesh: mesh }
    }

    pub fn get_name_from_type(t: &str) -> String {
        "Subdiv".to_string() //loop
    }
}

impl Component for SubdivComponent {}
