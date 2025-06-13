use crate::error::PbrtError;
use crate::models::scene::CameraComponent;
use crate::models::scene::Node;
use crate::models::scene::ResourceComponent;
use crate::models::scene::ResourceObject;

use crate::models::config::AppConfig;

use std::sync::Arc;
use std::sync::RwLock;

use uuid::Uuid;

#[derive(Debug)]
pub struct AppController {
    root_node: Arc<RwLock<Node>>,
    current_node: Option<Arc<RwLock<Node>>>,
    current_resource: Option<Arc<RwLock<dyn ResourceObject>>>,
    config: Arc<RwLock<AppConfig>>,
}

impl AppController {
    pub fn new() -> Self {
        let root_node = Node::root_node("Scene");
        Self {
            root_node: root_node.clone(),
            current_node: Some(root_node.clone()),
            current_resource: None,
            config: Arc::new(RwLock::new(AppConfig::default())),
        }
    }

    pub fn get_root_node(&self) -> Arc<RwLock<Node>> {
        self.root_node.clone()
    }

    pub fn set_root_node(&mut self, node: &Arc<RwLock<Node>>) {
        self.root_node = node.clone();
        self.current_node = Some(node.clone());
    }

    pub fn get_current_node(&self) -> Option<Arc<RwLock<Node>>> {
        self.current_node.clone()
    }

    pub fn get_current_node_id(&self) -> Option<Uuid> {
        if let Some(node) = &self.current_node {
            Some(node.read().unwrap().get_id())
        } else {
            None
        }
    }

    pub fn get_node_by_id(&self, id: Uuid) -> Option<Arc<RwLock<Node>>> {
        Node::find_node_by_id(&self.root_node, id)
    }

    pub fn set_current_node(&mut self, node: &Arc<RwLock<Node>>) {
        self.current_node = Some(node.clone());
        self.current_resource = None;
    }

    //-------------------------------------------------
    pub fn get_current_resource(&self) -> Option<Arc<RwLock<dyn ResourceObject>>> {
        self.current_resource.clone()
    }

    pub fn get_current_resource_id(&self) -> Option<Uuid> {
        if let Some(resource) = &self.current_resource {
            Some(resource.read().unwrap().get_id())
        } else {
            None
        }
    }

    pub fn set_current_resource(&mut self, resource: &Arc<RwLock<dyn ResourceObject>>) {
        self.current_resource = Some(resource.clone());
        self.current_node = None;
    }

    pub fn set_current_resource_by_id(&mut self, id: Uuid) {
        if let Some(resource) = self.get_resource_by_id(id) {
            self.current_resource = Some(resource.clone());
            self.current_node = None;
        }
    }

    pub fn get_resource_by_id(&self, id: Uuid) -> Option<Arc<RwLock<dyn ResourceObject>>> {
        let root_node = self.root_node.read().unwrap();
        if let Some(c) = root_node.get_component::<ResourceComponent>() {
            for (material_id, material) in c.materials.iter() {
                if *material_id == id {
                    return Some(material.clone());
                }
            }
            for (texture_id, texture) in c.textures.iter() {
                if *texture_id == id {
                    return Some(texture.clone());
                }
            }
            for (mesh_id, mesh) in c.meshes.iter() {
                if *mesh_id == id {
                    return Some(mesh.clone());
                }
            }
            for (other_id, other) in c.other_resources.iter() {
                if *other_id == id {
                    return Some(other.clone());
                }
            }
        }
        None
    }

    //-------------------------------------------------
    pub fn load_config(&mut self) {
        let path = dirs::config_dir()
            .unwrap()
            .join("pbrt_ui")
            .join("config.json");
        if path.exists() {
            match AppConfig::load_from_file(&path) {
                Ok(new_config) => {
                    let mut config = self.config.write().unwrap();
                    *config = new_config;
                }
                Err(e) => {
                    log::error!("Error loading config: {}", e);
                }
            }
        }
    }

    pub fn save_config(&self) {
        let path = dirs::config_dir()
            .unwrap()
            .join("pbrt_ui")
            .join("config.json");
        let config = self.config.read().unwrap();
        match serde_json::to_string_pretty(&*config) {
            Ok(json) => {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, json).unwrap();
            }
            Err(e) => {
                log::error!("Error saving config: {}", e);
            }
        }
    }

    pub fn get_config(&self) -> Arc<RwLock<AppConfig>> {
        self.config.clone()
    }
    //-------------------------------------------------
    pub fn update_nodes(&self) {
        let mut node = self.root_node.write().unwrap();
        node.update();
    }

    pub fn get_camera_node(&self) -> Option<Arc<RwLock<Node>>> {
        return Node::find_node_by_component::<CameraComponent>(&self.root_node);
    }
}

impl Drop for AppController {
    fn drop(&mut self) {
        self.save_config();
    }
}
