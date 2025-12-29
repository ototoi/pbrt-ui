use crate::model::scene::Component;
use crate::model::scene::Node;

use std::sync::Arc;
use std::sync::RwLock;

#[derive(Debug, Clone)]
pub struct ObjectInstanceComponent {
    node: Arc<RwLock<Node>>,
}

impl ObjectInstanceComponent {
    pub fn new(node: &Arc<RwLock<Node>>) -> Self {
        ObjectInstanceComponent { node: node.clone() }
    }
    pub fn get_node(&self) -> Arc<RwLock<Node>> {
        return self.node.clone();
    }
}

impl Component for ObjectInstanceComponent {}
