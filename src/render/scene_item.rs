use crate::model::base::Matrix4x4;
use crate::model::scene::CameraComponent;
use crate::model::scene::Component;
use crate::model::scene::LightComponent;
use crate::model::scene::MaterialComponent;
use crate::model::scene::Node;
use crate::model::scene::ShapeComponent;
use crate::model::scene::TransformComponent;

use std::sync::Arc;
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneItemType {
    Mesh,
    Light,
    Camera,
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

/*
fn get_material(node: &Arc<RwLock<Node>>) -> Arc<RwLock<Material>> {
    let node = node.read().unwrap();
    let m = node.get_component::<MaterialComponent>().unwrap();
    return m.get_material();
}
*/

fn get_scene_item(parent_matrix: &Matrix4x4, node: &Arc<RwLock<Node>>, items: &mut Vec<SceneItem>) {
    if !node.read().unwrap().is_enabled() {
        return;
    }

    let local_matrix = get_local_matrix(node);
    let world_matrix = *parent_matrix * local_matrix;

    if has_component::<ShapeComponent>(&node) && has_component::<MaterialComponent>(&node) {
        let item = SceneItem::new(node.clone(), SceneItemType::Mesh, world_matrix);
        items.push(item);
    }

    if has_component::<LightComponent>(&node) {
        let item = SceneItem::new(node.clone(), SceneItemType::Light, world_matrix);
        items.push(item);
    }

    if has_component::<CameraComponent>(&node) {
        let item = SceneItem::new(node.clone(), SceneItemType::Camera, world_matrix);
        items.push(item);
    }

    let node = node.read().unwrap();
    for child in &node.children {
        get_scene_item(&world_matrix, child, items);
    }
}

pub fn get_scene_items(node: &Arc<RwLock<Node>>) -> Vec<SceneItem> {
    let mut items = Vec::new();
    let parent_matrix = Matrix4x4::identity();
    get_scene_item(&parent_matrix, node, &mut items);
    return items;
}
