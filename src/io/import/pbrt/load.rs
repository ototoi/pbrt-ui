use super::parse::pbrt_parse_file;
use super::targets::MultipleTarget;
use super::targets::PrintTarget;
use super::targets::SceneTarget;
use crate::error::PbrtError;
use crate::models::scene::Node;
use crate::models::scene::SceneComponent;

use std::sync::Arc;
use std::sync::RwLock;

pub fn load_pbrt(path: &str) -> Result<Arc<RwLock<Node>>, PbrtError> {
    let scene_target = Arc::new(RwLock::new(SceneTarget::default()));
    {
        let print_target = Arc::new(RwLock::new(PrintTarget::default()));

        let mut target = MultipleTarget::default();
        target.add_target(print_target.clone());
        target.add_target(scene_target.clone());
        // Parse the PBRT file
        pbrt_parse_file(path, &mut target)?;
        // Process the parsed data
    }
    {
        let target = scene_target.read().unwrap();
        let node = target.create_scene_node();
        {
            let mut node = node.write().unwrap();
            if let Some(scene) = node.get_component_mut::<SceneComponent>() {
                let fullpath = std::path::Path::new(path);
                let fullpath = fullpath.canonicalize()?;
                let fullpath = fullpath.to_str().unwrap();

                // Add the filename to the scene properties
                let props = &mut scene.props;
                props.add_string("string filename", path);
                props.add_string("string fullpath", fullpath);
            }
        }
        return Ok(node);
    }
}
