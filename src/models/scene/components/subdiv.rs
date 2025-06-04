use super::component::Component;
use crate::models::base::*;

use crate::models::scene::mesh::Mesh;

use std::sync::Arc;
use std::sync::RwLock;

//use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct SubdivComponent {
    pub mesh: Option<Arc<RwLock<Mesh>>>,
}

fn create_mesh(name: &str, props: &PropertyMap) -> Arc<RwLock<Mesh>> {
    let mesh = Mesh::new(name, &props);
    Arc::new(RwLock::new(mesh))
}

impl SubdivComponent {
    pub fn new(t: &str, props: &PropertyMap) -> Self {
        let mut props = props.clone();
        props.insert("string type", Property::from(t));
        //let edition_id = Uuid::new_v4();
        //props.insert("string edition", Property::from(edition_id.to_string()));
        let name = Self::get_name_from_type(t);
        let mesh = create_mesh(&name, &props);
        SubdivComponent { mesh: Some(mesh) }
    }

    pub fn get_name_from_type(t: &str) -> String {
        "Subdiv".to_string() //loop
    }
}

impl Component for SubdivComponent {}
