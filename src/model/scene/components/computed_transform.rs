use super::component::Component;
use crate::model::base::Matrix4x4;

#[derive(Debug, Clone)]
pub struct ComputedTransformComponent {
    local_edition: String,
    parent_world_edition: String,
    world_edition: String,
    world_matrix: Matrix4x4,
}

impl Default for ComputedTransformComponent {
    fn default() -> Self {
        Self {
            local_edition: String::new(),
            parent_world_edition: String::new(),
            world_edition: String::new(),
            world_matrix: Matrix4x4::identity(),
        }
    }
}

impl ComputedTransformComponent {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn matches(&self, local_edition: &str, parent_world_edition: &str) -> bool {
        self.local_edition == local_edition
            && self.parent_world_edition == parent_world_edition
    }

    pub fn world_matrix(&self) -> Matrix4x4 {
        self.world_matrix
    }

    pub fn world_edition(&self) -> String {
        self.world_edition.clone()
    }

    pub fn update(
        &mut self,
        local_edition: String,
        parent_world_edition: String,
        world_edition: String,
        world_matrix: Matrix4x4,
    ) {
        self.local_edition = local_edition;
        self.parent_world_edition = parent_world_edition;
        self.world_edition = world_edition;
        self.world_matrix = world_matrix;
    }
}

impl Component for ComputedTransformComponent {}
