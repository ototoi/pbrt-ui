use crate::controller::AppController;
use crate::conversion::texture_node::TexturePurpose;
use crate::conversion::texture_node::create_image_variants;
use crate::conversion::texture_node::create_texture_nodes;
use crate::model::scene::ResourceCacheComponent;
use crate::model::scene::ResourceCacheManager;
use crate::model::scene::ResourceComponent;
use crate::model::scene::ResourceManager;

use eframe::egui;

use std::sync::Arc;
use std::sync::RwLock;

use uuid::Uuid;

struct TextureView {
    name: String,
    ty: String, //type
    edition: String,
    node_edition: String,
    dependencies: Vec<(String, String)>, // (key, texture name)
    id: Uuid,
    selected: bool,
    children: Vec<TextureView>,
}

fn create_texture_views(
    resource_manager: &ResourceManager,
    resource_cache_manager: &ResourceCacheManager,
) -> Vec<TextureView> {
    let mut nodes = Vec::new();
    for (_id, texture) in resource_manager.textures.iter() {
        let texture = texture.read().unwrap();
        let id = texture.get_id();
        let name = texture.get_name();
        let ty = texture.get_type();
        let edition = texture.get_edition();
        let cache = resource_cache_manager.textures.get(&id).unwrap();
        let cache = cache.read().unwrap();
        let cache_edition = cache.get_edition();
        let mut dependencies = Vec::new();
        for (key, dep) in cache.inputs.iter() {
            if let Some(dep) = dep {
                if let Some(dep) = dep.upgrade() {
                    let dep = dep.read().unwrap();
                    dependencies.push((key.clone(), dep.get_name()));
                }
            } else {
                dependencies.push((key.clone(), "<none>".to_string()));
            }
        }

        let node = TextureView {
            name: name,
            ty: ty,
            edition: edition,
            node_edition: cache_edition,
            dependencies: dependencies,
            id: id,
            selected: false,
            children: Vec::new(),
        };
        nodes.push(node);
    }

    return nodes;
}

fn show_texture_view(ui: &mut egui::Ui, node: &TextureView) -> Option<Uuid> {
    let mut selected_id = None;
    let id = node.id.clone();
    let id = ui.make_persistent_id(id);

    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
        .show_header(ui, |ui| {
            let name = format!("{} ({})", node.name, node.ty);
            if ui.selectable_label(node.selected, &name).clicked() {
                log::info!("Selected: {}, {}", name, node.id);
                selected_id = Some(node.id);
            }
        })
        .body(|ui| {
            ui.label(format!("Edition: {}", node.edition));
            ui.label(format!("Cache Edition: {}", node.node_edition));
            if !node.dependencies.is_empty() {
                ui.label("Dependencies:");
                for (key, name) in &node.dependencies {
                    ui.label(format!("  {}: {}", key, name));
                }
            }
            for child in &node.children {
                if let Some(id) = show_texture_view(ui, child) {
                    selected_id = Some(id);
                }
            }
        });

    selected_id
}

fn show_texture_views(ui: &mut egui::Ui, nodes: &Vec<TextureView>) -> Option<Uuid> {
    let mut selected_id = None;
    for node in nodes {
        if let Some(id) = show_texture_view(ui, node) {
            selected_id = Some(id);
        }
    }
    return selected_id;
}

#[derive(Debug, Clone)]
pub struct DebugTexturesPanel {
    pub app_controller: Arc<RwLock<AppController>>,
}

impl DebugTexturesPanel {
    pub fn new(controller: &Arc<RwLock<AppController>>) -> Self {
        Self {
            app_controller: Arc::clone(controller),
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let controller = self.app_controller.read().unwrap();
        let root_node = controller.get_root_node();

        {
            let mut root_node = root_node.write().unwrap();
            if root_node
                .get_component::<ResourceCacheComponent>()
                .is_none()
            {
                root_node.add_component(ResourceCacheComponent::new());
            }
        }

        let root_node = root_node.read().unwrap();
        let resource_component = root_node
            .get_component::<ResourceComponent>()
            .expect("ResourceComponent not found");

        let resource_cache_component = root_node
            .get_component::<ResourceCacheComponent>()
            .expect("ResourceCacheComponent not found");
        let resource_manager = resource_component.get_resource_manager();
        let resource_manager = resource_manager.read().unwrap();
        let resource_cache_manager = resource_cache_component.get_resource_cache_manager();
        let mut resource_cache_manager = resource_cache_manager.write().unwrap();
        // Update texture caches
        create_texture_nodes(&resource_manager, &mut resource_cache_manager);
        create_image_variants(
            &resource_manager,
            &mut resource_cache_manager,
            TexturePurpose::Render,
        );
        let texture_views = create_texture_views(&resource_manager, &resource_cache_manager);

        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                show_texture_views(ui, &texture_views);
            });
    }
}
