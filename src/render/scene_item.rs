use crate::model::base::Matrix4x4;
use crate::model::scene::CameraComponent;
use crate::model::scene::ComputedTransformComponent;
use crate::model::scene::Component;
use crate::model::scene::LightComponent;
use crate::model::scene::MaterialComponent;
use crate::model::scene::Node;
use crate::model::scene::ObjectInstanceComponent;
use crate::model::scene::ShapeComponent;
use crate::model::scene::TransformComponent;

use std::sync::Arc;
use std::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneItemType {
    Mesh,
    Light,
    Camera,
    ObjectInstance,
}

pub struct SceneItem {
    pub node: Arc<RwLock<Node>>,
    pub category: SceneItemType, //type of the item (Mesh, Light, etc.)
    pub matrix: Matrix4x4,       //world matrix of the item
}

impl SceneItem {
    pub fn new(node: Arc<RwLock<Node>>, category: SceneItemType, matrix: Matrix4x4) -> Self {
        SceneItem {
            node,
            category,
            matrix,
        }
    }
}

fn has_component<T: Component>(node: &Arc<RwLock<Node>>) -> bool {
    let node = node.read().unwrap();
    node.get_component::<T>().is_some()
}

fn get_local_matrix(node: &Arc<RwLock<Node>>) -> Matrix4x4 {
    let node = node.read().unwrap();
    let t = node.get_component::<TransformComponent>().unwrap();
    return t.get_local_matrix();
}

fn get_local_edition(node: &Arc<RwLock<Node>>) -> String {
    let node = node.read().unwrap();
    let t = node.get_component::<TransformComponent>().unwrap();
    t.get_edition()
}

fn get_world_matrix(
    node: &Arc<RwLock<Node>>,
    parent_matrix: &Matrix4x4,
    parent_world_edition: &str,
) -> (Matrix4x4, String) {
    let local_matrix = get_local_matrix(node);
    let local_edition = get_local_edition(node);

    {
        let node_ref = node.read().unwrap();
        if let Some(component) = node_ref.get_component::<ComputedTransformComponent>()
            && component.matches(&local_edition, parent_world_edition)
        {
            return (component.world_matrix(), component.world_edition());
        }
    }

    let world_matrix = *parent_matrix * local_matrix;
    let world_edition = Uuid::new_v4().to_string();
    let mut node_ref = node.write().unwrap();
    if let Some(component) = node_ref.get_component_mut::<ComputedTransformComponent>() {
        component.update(
            local_edition,
            parent_world_edition.to_string(),
            world_edition.clone(),
            world_matrix,
        );
    } else {
        let mut component = ComputedTransformComponent::new();
        component.update(
            local_edition,
            parent_world_edition.to_string(),
            world_edition.clone(),
            world_matrix,
        );
        node_ref.add_component(component);
    }
    (world_matrix, world_edition)
}

/*
fn get_material(node: &Arc<RwLock<Node>>) -> Arc<RwLock<Material>> {
    let node = node.read().unwrap();
    let m = node.get_component::<MaterialComponent>().unwrap();
    return m.get_material();
}
*/

fn get_scene_item(
    parent_matrix: &Matrix4x4,
    parent_world_edition: &str,
    node: &Arc<RwLock<Node>>,
    items: &mut Vec<SceneItem>,
) {
    if !node.read().unwrap().is_enabled() {
        return;
    }

    let (world_matrix, world_edition) = get_world_matrix(node, parent_matrix, parent_world_edition);

    if has_component::<ShapeComponent>(node) && has_component::<MaterialComponent>(node) {
        let item = SceneItem::new(node.clone(), SceneItemType::Mesh, world_matrix);
        items.push(item);
    }

    if has_component::<LightComponent>(node) {
        let item = SceneItem::new(node.clone(), SceneItemType::Light, world_matrix);
        items.push(item);
    }

    if has_component::<CameraComponent>(node) {
        let item = SceneItem::new(node.clone(), SceneItemType::Camera, world_matrix);
        items.push(item);
    }

    if has_component::<ObjectInstanceComponent>(node) {
        let item = SceneItem::new(node.clone(), SceneItemType::ObjectInstance, world_matrix);
        items.push(item);
    }

    let node = node.read().unwrap();
    for child in &node.children {
        get_scene_item(&world_matrix, &world_edition, child, items);
    }
}

pub fn get_scene_items(node: &Arc<RwLock<Node>>) -> Vec<SceneItem> {
    let mut items = Vec::new();
    let parent_matrix = Matrix4x4::identity();
    get_scene_item(&parent_matrix, "", node, &mut items);
    return items;
}
