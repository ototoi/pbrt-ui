use crate::controller::AppController;
use crate::model::scene::Node as SceneNode;
use crate::panel::Panel;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use eframe::egui;
use uuid::Uuid;

#[derive(Debug)]
pub struct HierarchyPanel {
    pub is_open: bool,
    pub controller: Arc<RwLock<AppController>>,
    pub nodes_info: HashMap<Uuid, bool>,
}

impl HierarchyPanel {
    pub fn new(controller: &Arc<RwLock<AppController>>) -> Self {
        Self {
            is_open: true,
            controller: controller.clone(),
            nodes_info: HashMap::new(),
        }
    }
}

struct SelectedTree {
    name: String,
    id: Uuid,
    selected: bool,
    children: Vec<SelectedTree>,
    has_children: bool,
    is_open: bool,
}

fn convert_node_to_tree(
    node: &Arc<RwLock<SceneNode>>,
    selected_id: Option<Uuid>,
    nodes_info: &mut HashMap<Uuid, bool>,
) -> SelectedTree {
    let node = node.read().unwrap();
    let is_open = *nodes_info.entry(node.get_id()).or_insert(false);
    let mut children = Vec::new();
    if is_open {
        for child in &node.children {
            children.push(convert_node_to_tree(child, selected_id, nodes_info));
        }
    }
    let name = format!("{}", node.get_name());
    SelectedTree {
        name: name,
        id: node.get_id(),
        selected: selected_id == Some(node.get_id()),
        children,
        has_children: !node.children.is_empty(),
        is_open,
    }
}

fn show_tree(
    ui: &mut egui::Ui,
    tree: &SelectedTree,
    nodes_info: &mut HashMap<Uuid, bool>,
) -> Option<Uuid> {
    let mut selected_id = None;
    let id = tree.id.clone();
    let id = ui.make_persistent_id(id);
    if !tree.has_children {
        let name = tree.name.clone();
        if ui.selectable_label(tree.selected, &name).clicked() {
            log::info!("Selected: {}, {}", name, tree.id);
            selected_id = Some(tree.id);
        }
        return selected_id;
    } else {
        let header_resonse = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            id,
            tree.is_open,
        )
        .show_header(ui, |ui| {
            let name = tree.name.clone();
            if ui.selectable_label(tree.selected, &name).clicked() {
                log::info!("Selected: {}, {}", name, tree.id);
                selected_id = Some(tree.id);
            }
        });
        nodes_info.insert(tree.id, header_resonse.is_open());
        header_resonse.body(|ui| {
            for child in &tree.children {
                if let Some(id) = show_tree(ui, child, nodes_info) {
                    selected_id = Some(id);
                }
            }
        });
    }
    return selected_id;
}

impl Panel for HierarchyPanel {
    fn name(&self) -> &str {
        "Hierarchy"
    }
    fn is_open(&self) -> bool {
        self.is_open
    }
    fn toggle_open(&mut self) -> bool {
        self.is_open = !self.is_open;
        self.is_open
    }

    fn show(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("hierarchy")
            .default_width(200.0)
            .resizable(true)
            .show_animated(ctx, self.is_open, |ui| {
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("X").clicked() {
                            self.toggle_open();
                        }
                        ui.vertical_centered(|ui| {
                            ui.label("Hierarchy");
                        });
                    });
                });
                ui.separator();

                let tree = {
                    let controller = self.controller.read().unwrap();
                    let current_node_id = controller.get_current_node_id();
                    let root_node = controller.get_root_node();
                    let root_tree =
                        convert_node_to_tree(&root_node, current_node_id, &mut self.nodes_info);
                    root_tree
                };

                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        if let Some(id) = show_tree(ui, &tree, &mut self.nodes_info) {
                            let mut controller = self.controller.write().unwrap();
                            if let Some(node) = controller.get_node_by_id(id) {
                                controller.set_current_node(&node);
                            }
                        }
                    });
            });
    }
}
