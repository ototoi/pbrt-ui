use crate::model::scene::TransformComponent;

use super::node::Node;
use std::sync::Arc;
use std::sync::RwLock;

fn remove_empty_node(node: &Arc<RwLock<Node>>) -> Option<Arc<RwLock<Node>>> {
    {
        let mut node = node.write().unwrap();
        {
            let mut new_children = vec![];
            for child in &node.children {
                if let Some(opt_child) = remove_empty_node(child) {
                    new_children.push(opt_child);
                }
            }
            node.children = new_children;
        }
        if node.children.is_empty()
            && node.components.len() == 1
                && node.get_component::<TransformComponent>().is_some() {
                    return None;
                }
    }
    Some(node.clone())
}

fn remove_empty_nodes(root: &Arc<RwLock<Node>>) -> Arc<RwLock<Node>> {
    {
        let mut root = root.write().unwrap();
        let mut new_children = vec![];
        // Recursively optimize children
        for child in &root.children {
            if let Some(opt_child) = remove_empty_node(child) {
                new_children.push(opt_child);
            }
        }
        root.children = new_children;
    }
    root.clone()
}

fn remove_identity_node(node: &Arc<RwLock<Node>>) -> Option<Arc<RwLock<Node>>> {
    {
        let mut node = node.write().unwrap();
        {
            let mut new_children = vec![];
            for child in &node.children {
                if let Some(opt_child) = remove_identity_node(child) {
                    new_children.push(opt_child);
                }
            }
            node.children = new_children;
        }
        if node.components.len() == 1 && node.children.len() == 1
            && let Some(transform) = node.get_component::<TransformComponent>()
                && transform.is_identity() {
                    // Skip this node and promote its children
                    return Some(node.children[0].clone());
                }
    }
    Some(node.clone())
}

fn remove_identity_nodes(root: &Arc<RwLock<Node>>) -> Arc<RwLock<Node>> {
    {
        let mut root = root.write().unwrap();
        let mut new_children = vec![];
        // Recursively optimize children
        for child in &root.children {
            if let Some(opt_child) = remove_identity_node(child) {
                new_children.push(opt_child);
            }
        }
        root.children = new_children;
    }
    root.clone()
}

pub fn optimize_nodes(node: &Arc<RwLock<Node>>) -> Arc<RwLock<Node>> {
    let node = node.clone();
    let node = remove_empty_nodes(&node);
    
    remove_identity_nodes(&node)
}
