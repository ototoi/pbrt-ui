use crate::models::base::Matrix4x4;
use crate::renderers::gl::RenderMesh;
use crate::renderers::gl::RenderProgram;

use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct RenderItem {
    pub local_to_world: Matrix4x4,
    pub mesh: Arc<RenderMesh>,
    pub program: Arc<RenderProgram>,
}
