use super::component::Component;
use crate::models::base::*;
use crate::models::scene::mesh::Mesh;

use std::sync::Arc;
use std::sync::RwLock;

#[derive(Debug, Clone)]
pub struct MeshComponent {
    pub mesh: Option<Arc<RwLock<Mesh>>>,
}

fn create_mesh(name: &str, props: &PropertyMap) -> Arc<RwLock<Mesh>> {
    let mesh = Mesh::new(name, &props);
    Arc::new(RwLock::new(mesh))
}

impl MeshComponent {
    pub fn new(mesh_type: &str, name: &str, props: &PropertyMap) -> Self {
        let mut props = props.clone();
        props.insert("string type", Property::from(mesh_type));
        let mesh = create_mesh(name, &props);
        MeshComponent { mesh: Some(mesh) }
    }
}

impl Component for MeshComponent {}
