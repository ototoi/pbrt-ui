use super::components::*;

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::Weak;

use uuid::Uuid;

type Components = HashMap<TypeId, Box<dyn Component>>;

#[derive(Debug)]
pub struct Node {
    pub enable: bool,
    pub id: Uuid,
    pub name: String,
    pub parent: Option<Weak<RwLock<Node>>>,
    pub children: Vec<Arc<RwLock<Node>>>,
    pub components: Components,
}

impl Node {
    pub fn root_node(name: &str) -> Arc<RwLock<Node>> {
        let mut components = Components::new();
        components.insert(
            TypeId::of::<TransformComponent>(),
            Box::new(TransformComponent::default()),
        );
        let node = Node {
            enable: true,
            name: name.to_string(),
            id: Uuid::new_v4(),
            parent: None,
            children: Vec::new(),
            components,
        };
        Arc::new(RwLock::new(node))
    }

    pub fn child_node(name: &str, parent: &Arc<RwLock<Node>>) -> Arc<RwLock<Node>> {
        let mut components = Components::new();
        components.insert(
            TypeId::of::<TransformComponent>(),
            Box::new(TransformComponent::default()),
        );
        let node = Node {
            enable: true,
            name: name.to_string(),
            id: Uuid::new_v4(),
            parent: Some(Arc::downgrade(parent)),
            children: Vec::new(),
            components,
        };
        let c = Arc::new(RwLock::new(node));
        Node::add_child(parent, &c);
        return c;
    }

    pub fn is_enabled(&self) -> bool {
        self.enable
    }

    pub fn get_enable(&self) -> bool {
        self.enable
    }

    pub fn set_enable(&mut self, enable: bool) {
        self.enable = enable;
    }

    pub fn get_name(&self) -> String {
        self.name.clone()
    }

    pub fn set_name(&mut self, name: &str) {
        self.name = name.to_string();
    }

    pub fn get_id(&self) -> Uuid {
        self.id
    }

    pub fn add_component<T: Component + 'static>(&mut self, component: T) {
        self.components
            .insert(TypeId::of::<T>(), Box::new(component));
    }

    pub fn get_component<T: Component + 'static>(&self) -> Option<&T> {
        self.components
            .get(&TypeId::of::<T>())
            .and_then(|component| (component.as_ref() as &dyn Any).downcast_ref::<T>())
    }

    pub fn get_component_mut<T: Component + 'static>(&mut self) -> Option<&mut T> {
        self.components
            .get_mut(&TypeId::of::<T>())
            .and_then(|component| (component.as_mut() as &mut dyn Any).downcast_mut::<T>())
    }

    // --------------------------------------------------------------------------------------------- //
    pub fn add_child(parent: &Arc<RwLock<Node>>, child: &Arc<RwLock<Node>>) -> Arc<RwLock<Node>> {
        child.write().unwrap().parent = Some(Arc::downgrade(parent));
        parent.write().unwrap().children.push(child.clone());
        child.clone()
    }

    pub fn find_node_by_id(node: &Arc<RwLock<Node>>, id: Uuid) -> Option<Arc<RwLock<Node>>> {
        if node.read().unwrap().id == id {
            return Some(node.clone());
        }
        for child in &node.read().unwrap().children {
            if let Some(found) = Node::find_node_by_id(child, id) {
                return Some(found);
            }
        }
        None
    }

    pub fn find_node_by_component<T: Component>(
        node: &Arc<RwLock<Node>>,
    ) -> Option<Arc<RwLock<Node>>> {
        if node.read().unwrap().get_component::<T>().is_some() {
            return Some(node.clone());
        }
        for child in &node.read().unwrap().children {
            if let Some(found) = Node::find_node_by_component::<T>(child) {
                return Some(found);
            }
        }
        None
    }

    pub fn update(&mut self) {
        for component in self.components.values_mut() {
            component.update();
        }
    }
}
