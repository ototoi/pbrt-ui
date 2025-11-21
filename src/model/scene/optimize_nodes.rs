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
        if node.children.is_empty() {
            if node.components.len() == 1 {
                if let Some(_) = node.get_component::<TransformComponent>() {
                    return None;
                }
            }
        }
    }
    return Some(node.clone());
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
    return root.clone();
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
        if node.components.len() == 1 && node.children.len() == 1 {
            if let Some(transform) = node.get_component::<TransformComponent>() {
                if transform.is_identity() {
                    // Skip this node and promote its children
                    return Some(node.children[0].clone());
                }
            }
        }
    }
    return Some(node.clone());
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
    return root.clone();
}

pub fn optimize_nodes(node: &Arc<RwLock<Node>>) -> Arc<RwLock<Node>> {
    let node = node.clone();
    let node = remove_empty_nodes(&node);
    let node = remove_identity_nodes(&node);
    return node;
}
