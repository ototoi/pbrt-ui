use super::image_data::ImageData;
use super::image_receiver::ImageReceiver;
use super::render_state::*;
use crate::models::scene::Node;
use crate::models::scene::SceneComponent;
use crate::{error::*, models::config::AppConfig};

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;

use crypto::digest::Digest;
use dirs;
use std::path::PathBuf;
use uuid::Uuid;

fn get_file_path(node: &Arc<RwLock<Node>>) -> Option<String> {
    let node = node.read().unwrap();
    if let Some(scene) = node.get_component::<SceneComponent>() {
        return scene.get_fullpath();
    }
    return None;
}

fn get_digest(path: &str) -> String {
    let mut hasher = crypto::sha1::Sha1::new();
    hasher.input_str(path);
    let digest = hasher.result_str();
    return digest;
}

fn scene_cache_dir(path: Option<String>) -> PathBuf {
    let (name, digest) = if let Some(path) = path {
        let filename = std::path::Path::new(path.as_str()).file_stem().unwrap();
        let filename = filename.to_str().unwrap();
        (filename.to_string(), get_digest(&path))
    } else {
        ("none".to_string(), "none".to_string())
    };
    let mut cache_dir = dirs::cache_dir().unwrap();
    cache_dir.push("pbrt_ui"); //
    cache_dir.push("scenes");
    cache_dir.push(name);
    cache_dir.push(digest);
    cache_dir
}

pub struct RenderSession {
    state: RenderState,
    tasks: HashMap<RenderState, Box<dyn RenderTask>>,
    receiver: Option<ImageReceiver>,
}

impl RenderSession {
    pub fn new(
        node: &Arc<RwLock<Node>>,
        config: &AppConfig,
        session_id: &Uuid,
    ) -> Result<RenderSession, PbrtError> {
        let cache_dir = scene_cache_dir(get_file_path(node));

        let execute_path = config.pbrt_executable_path.clone();
        let pbrt_path = cache_dir.join(format!("{}.pbrt", session_id)); //
        let image_path = cache_dir.join(format!("{}.exr", session_id)); //

        let execute_path = execute_path.to_str().unwrap().to_string();
        let pbrt_path = pbrt_path.to_str().unwrap().to_string();
        let image_path = image_path.to_str().unwrap().to_string();

        let display_server = if config.enable_display_server {
            Some((
                config.display_server_host.clone(),
                config.display_server_port,
            ))
        } else {
            None
        };

        let image_receiver = if let Some((_hostname, port)) = display_server.as_ref() {
            //println!("Using display server: {}", display_server);
            let mut image_receiver = ImageReceiver::new();
            image_receiver.start(*port)?;
            Some(image_receiver)
        } else {
            None
        };

        let mut tasks: HashMap<RenderState, Box<dyn RenderTask>> = HashMap::new();
        {
            // Initialize tasks
            tasks.insert(RenderState::Ready, Box::new(ReadyRenderTask::new()));
            // Saving phase
            tasks.insert(
                RenderState::Saving,
                Box::new(SavingRenderTask::new(node.clone(), &pbrt_path)),
            );
            // Rendering phase

            tasks.insert(
                RenderState::Rendering,
                Box::new(RenderingRenderTask::new(
                    &execute_path,
                    &pbrt_path,
                    &image_path,
                    &display_server,
                )),
            );
            // Finishing phase
            tasks.insert(RenderState::Finishing, Box::new(FinishingRenderTask::new()));
            tasks.insert(RenderState::Finished, Box::new(FinishedRenderTask::new()));
        }

        // Enter the first task
        if let Some(save_task) = tasks.get_mut(&RenderState::Saving) {
            save_task.enter()?;
        }
        return Ok(Self {
            state: RenderState::Saving,
            tasks: tasks,
            receiver: image_receiver,
        });
    }

    pub fn get_state(&self) -> RenderState {
        self.state
    }

    pub fn update(&mut self) -> Result<RenderState, PbrtError> {
        let before_state = self.state;
        if let Some(task) = self.tasks.get_mut(&before_state) {
            let next_state = task.update()?;
            if next_state != before_state {
                task.exit()?;
                //println!("Exited state: {:?}", before_state);
                if let Some(next_task) = self.tasks.get_mut(&next_state) {
                    //println!("Entering next task: {:?}", next_task.get_state());
                    next_task.enter()?;
                    //println!("Task entered: {:?}", next_task.get_state())
                }
                //println!("Entered state: {:?}", self.state);
            }
            self.state = next_state;
        }
        Ok(self.state)
    }

    pub fn cancel(&mut self) -> Result<(), PbrtError> {
        if let Some(task) = self.tasks.get_mut(&self.state) {
            task.cancel()?;
            if let Some(receiver) = self.receiver.as_mut() {
                let _ = receiver.stop();
            }
            //self.receiver = None;
        }
        Ok(())
    }

    pub fn get_image_data(&self) -> Option<Arc<Mutex<ImageData>>> {
        if let Some(receiver) = self.receiver.as_ref() {
            return receiver.get_image_data();
        }
        None
    }
}
