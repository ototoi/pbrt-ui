use super::parse::pbrt_parse_file;
use super::targets::MultipleTarget;
// use super::targets::PrintTarget;
use super::targets::SceneTarget;
use crate::error::PbrtError;
use crate::model::scene::Node;
use crate::model::scene::optimize_nodes;

use std::sync::Arc;
use std::sync::RwLock;

pub fn load_pbrt(path: &str) -> Result<Arc<RwLock<Node>>, PbrtError> {
    let scene_target = Arc::new(RwLock::new(SceneTarget::default()));
    {
        //let print_target = Arc::new(RwLock::new(PrintTarget::default()));

        let mut target = MultipleTarget::default();
        //target.add_target(print_target.clone());
        target.add_target(scene_target.clone());

        pbrt_parse_file(path, &mut target)?;
    }

    let scene_target = match Arc::try_unwrap(scene_target) {
        Ok(st) => st,
        Err(_) => return Err(PbrtError::error("Failed to unwrap scene_target")),
    };
    let scene_target = match scene_target.into_inner() {
        Ok(st) => st,
        Err(_) => return Err(PbrtError::error("Failed to get inner value")),
    };

    let root = scene_target.create_scene_node();
    let root = optimize_nodes(&root);
    Ok(root)
}
