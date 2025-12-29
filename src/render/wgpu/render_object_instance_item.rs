use super::render_item::RenderItem;
use super::render_item::get_render_items_from_scene_items;
use super::render_resource::RenderResourceManager;
use crate::model::scene::ObjectInstanceComponent;
use crate::model::scene::ResourceCacheManager;
use crate::model::scene::ResourceManager;
use crate::render::render_mode::RenderMode;
use crate::render::scene_item::*;

use std::sync::Arc;

use eframe::wgpu;

pub fn get_render_object_instance_items(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    item: &SceneItem,
    _mode: RenderMode,
    resource_manager: &ResourceManager,
    resource_cache_manager: &mut ResourceCacheManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Vec<Arc<RenderItem>> {
    let node = item.node.read().unwrap();
    let component = node.get_component::<ObjectInstanceComponent>().unwrap();

    let referenced_node = component.get_node();
    let scene_items = get_scene_items(&referenced_node);
    let mut render_items = get_render_items_from_scene_items(
        device,
        queue,
        &scene_items,
        _mode,
        resource_manager,
        resource_cache_manager,
        render_resource_manager,
    );
    let item_matrix = glam::Mat4::from(item.matrix);
    for render_item in render_items.iter_mut() {
        let new_matrix = item_matrix * render_item.get_matrix();
        Arc::make_mut(render_item).set_matrix(new_matrix); //update matrix
    }

    return render_items;
}
