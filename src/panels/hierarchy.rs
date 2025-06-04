use crate::controllers::AppController;
use crate::models::scene::Node as SceneNode;
use crate::panels::Panel;

use std::sync::Arc;
use std::sync::RwLock;

use eframe::egui;
use uuid::Uuid;

#[derive(Debug)]
pub struct HierarchyPanel {
    pub is_open: bool,
    pub controller: Arc<RwLock<AppController>>,
}

impl HierarchyPanel {
    pub fn new(controller: &Arc<RwLock<AppController>>) -> Self {
        Self {
            is_open: true,
            controller: controller.clone(),
        }
    }
}

struct SelectedTree {
    name: String,
    id: Uuid,
    selected: bool,
    children: Vec<SelectedTree>,
}

fn convert_node_to_tree(node: &Arc<RwLock<SceneNode>>, selected_id: Option<Uuid>) -> SelectedTree {
    let node = node.read().unwrap();
    let mut children = Vec::new();
    for child in &node.children {
        children.push(convert_node_to_tree(child, selected_id));
    }
    let name = format!("{}", node.get_name());
    SelectedTree {
        name: name,
        id: node.get_id(),
        selected: selected_id == Some(node.get_id()),
        children,
    }
}

fn show_tree(ui: &mut egui::Ui, tree: &SelectedTree) -> Option<Uuid> {
    let mut selected_id = None;
    let id = tree.id.clone();
    let id = ui.make_persistent_id(id);
    if tree.children.is_empty() {
        let name = tree.name.clone();
        if ui.selectable_label(tree.selected, &name).clicked() {
            log::info!("Selected: {}, {}", name, tree.id);
            selected_id = Some(tree.id);
        }
        return selected_id;
    } else {
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
            .show_header(ui, |ui| {
                let name = tree.name.clone();
                if ui.selectable_label(tree.selected, &name).clicked() {
                    log::info!("Selected: {}, {}", name, tree.id);
                    selected_id = Some(tree.id);
                }
            })
            .body(|ui| {
                if !tree.children.is_empty() {
                    for child in &tree.children {
                        if let Some(id) = show_tree(ui, child) {
                            selected_id = Some(id);
                        }
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
                    let root_tree = convert_node_to_tree(&root_node, current_node_id);
                    root_tree
                };

                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        if let Some(id) = show_tree(ui, &tree) {
                            let mut controller = self.controller.write().unwrap();
                            if let Some(node) = controller.get_node_by_id(id) {
                                controller.set_current_node(&node);
                            }
                        }
                    });
            });
    }
}
