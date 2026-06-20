use super::component::Component;
use crate::model::base::Matrix4x4;

#[derive(Debug, Clone)]
pub struct ComputedTransformComponent {
    local_matrix: Matrix4x4,
    parent_world_matrix: Matrix4x4,
    world_matrix: Matrix4x4,
}

impl Default for ComputedTransformComponent {
    fn default() -> Self {
        Self {
            local_matrix: Matrix4x4::identity(),
            parent_world_matrix: Matrix4x4::identity(),
            world_matrix: Matrix4x4::identity(),
        }
    }
}

impl ComputedTransformComponent {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn matches(&self, local_matrix: &Matrix4x4, parent_world_matrix: &Matrix4x4) -> bool {
        self.local_matrix == *local_matrix && self.parent_world_matrix == *parent_world_matrix
    }

    pub fn world_matrix(&self) -> Matrix4x4 {
        self.world_matrix
    }

    pub fn update(
        &mut self,
        local_matrix: Matrix4x4,
        parent_world_matrix: Matrix4x4,
        world_matrix: Matrix4x4,
    ) {
        self.local_matrix = local_matrix;
        self.parent_world_matrix = parent_world_matrix;
        self.world_matrix = world_matrix;
    }
}

impl Component for ComputedTransformComponent {}
