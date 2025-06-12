use crate::error::PbrtError;
use crate::io::export::pbrt::*;
use crate::models::scene::Node;

use std::sync::Arc;
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum RenderState {
    Ready,
    Saving,
    Rendering,
    Finishing,
    Finished,
}

fn save_pbrt_file(node: &Arc<RwLock<Node>>, pbrt_path: &str) -> Result<(), PbrtError> {
    let mut options = SavePbrtOptions::default();
    options.pretty_print = false;
    save_pbrt(node, pbrt_path, &options)?;
    Ok(())
}

pub trait RenderTask {
    fn get_state(&self) -> RenderState;
    fn enter(&mut self) -> Result<(), PbrtError> {
        log::info!("Entering render task state: {:?}", self.get_state());
        Ok(())
    }
    fn update(&mut self) -> Result<RenderState, PbrtError>;
    fn exit(&mut self) -> Result<(), PbrtError> {
        log::info!("Exiting render task state: {:?}", self.get_state());
        Ok(())
    }
    fn cancel(&mut self) -> Result<(), PbrtError> {
        Ok(())
    }
}

pub struct ReadyRenderTask {}
impl ReadyRenderTask {
    pub fn new() -> Self {
        Self {}
    }
}
impl RenderTask for ReadyRenderTask {
    fn get_state(&self) -> RenderState {
        RenderState::Ready
    }
    fn update(&mut self) -> Result<RenderState, PbrtError> {
        Ok(RenderState::Ready)
    }
}
pub struct SavingRenderTask {
    node: Arc<RwLock<Node>>,
    pbrt_path: String,
}
impl SavingRenderTask {
    pub fn new(node: Arc<RwLock<Node>>, pbrt_path: &str) -> Self {
        let pbrt_path = pbrt_path.to_string();
        Self { node, pbrt_path }
    }
}
impl RenderTask for SavingRenderTask {
    fn get_state(&self) -> RenderState {
        RenderState::Saving
    }
    fn enter(&mut self) -> Result<(), PbrtError> {
        log::info!("Entering saving state with PBRT file: {}", self.pbrt_path);
        {
            let pbrt_path = self.pbrt_path.clone();
            let pbrt_path = std::path::Path::new(&pbrt_path);
            std::fs::create_dir_all(pbrt_path.parent().unwrap())?;
        }
        save_pbrt_file(&self.node, &self.pbrt_path)?;
        Ok(())
    }
    fn update(&mut self) -> Result<RenderState, PbrtError> {
        Ok(RenderState::Rendering)
    }
}
pub struct RenderingRenderTask {
    execute_path: String,
    pbrt_path: String,
    output_path: String,
    display_server: Option<(String, u16)>,
    child: Option<std::process::Child>,
}

impl RenderingRenderTask {
    pub fn new(
        execute_path: &str,
        pbrt_path: &str,
        output_path: &str,
        display_server: &Option<(String, u16)>,
    ) -> Self {
        let execute_path = execute_path.to_string();
        let pbrt_path = pbrt_path.to_string();
        let output_path = output_path.to_string();
        Self {
            execute_path,
            pbrt_path,
            output_path,
            display_server: display_server.clone(),
            child: None,
        }
    }
}
impl RenderTask for RenderingRenderTask {
    fn get_state(&self) -> RenderState {
        RenderState::Rendering
    }
    fn enter(&mut self) -> Result<(), PbrtError> {
        // Here you would prepare the rendering process
        log::info!(
            "Entering rendering state with PBRT file: {}",
            self.pbrt_path
        );

        let execute_path = self.execute_path.clone();
        let pbrt_path = self.pbrt_path.clone();
        let output_path = self.output_path.clone();

        let mut command = std::process::Command::new(execute_path);
        //.arg("-v") // Optional: quiet mode
        //.arg("-i")
        command.arg(pbrt_path).arg("--outfile").arg(output_path);
        if let Some((hostname, port)) = &self.display_server {
            let display_server = format!("{}:{}", hostname, port);
            command.arg("--display-server").arg(display_server);
        }
        let child = command.spawn()?;
        self.child = Some(child);
        Ok(())
    }

    fn update(&mut self) -> Result<RenderState, PbrtError> {
        // Here you would call the actual rendering process
        // For now, we just simulate it
        if let Some(child) = &mut self.child {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if status.success() {
                        log::info!("Rendering completed successfully");
                        return Ok(RenderState::Finishing);
                    } else {
                        log::error!("Rendering failed with status: {:?}", status);
                        return Ok(RenderState::Finishing);
                        //return Err(PbrtError::error("Rendering process failed"));
                    }
                }
                Ok(None) => {
                    // Still running
                    return Ok(RenderState::Rendering);
                }
                Err(e) => {
                    log::error!("Error waiting for rendering process: {}", e);
                    return Err(PbrtError::error("Error waiting for rendering process"));
                }
            }
        }
        Ok(RenderState::Finishing)
    }

    fn cancel(&mut self) -> Result<(), PbrtError> {
        if let Some(child) = &mut self.child {
            if let Err(e) = child.kill() {
                log::error!("Failed to kill rendering process: {}", e);
                return Err(PbrtError::error("Failed to kill rendering process"));
            }
            log::info!("Rendering process killed successfully");
        }
        Ok(())
    }

    fn exit(&mut self) -> Result<(), PbrtError> {
        log::info!("Exiting rendering state");
        Ok(())
    }
}

impl Drop for RenderingRenderTask {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            if let Err(e) = child.kill() {
                log::error!("Failed to kill rendering process on drop: {}", e);
            } else {
                log::info!("Rendering process killed on drop");
            }
        }
    }
}

pub struct FinishingRenderTask {
    src_path: String,
    dst_path: String,
}
impl FinishingRenderTask {
    pub fn new(src_path: &str, dst_path: &str) -> Self {
        Self {
            src_path: src_path.to_string(),
            dst_path: dst_path.to_string(),
        }
    }
}
impl RenderTask for FinishingRenderTask {
    fn get_state(&self) -> RenderState {
        RenderState::Finishing
    }
    fn enter(&mut self) -> Result<(), PbrtError> {
        // Here you would finalize the rendering process
        if self.src_path != self.dst_path {
            let src_path = std::path::PathBuf::from(&self.src_path);
            let dst_path = std::path::PathBuf::from(&self.dst_path);
            if src_path.exists() {
                std::fs::copy(src_path, dst_path)?;
            }
        }
        Ok(())
    }
    fn update(&mut self) -> Result<RenderState, PbrtError> {
        // Here you would finalize the rendering process
        // For now, we just simulate it
        Ok(RenderState::Finished)
    }
}

pub struct FinishedRenderTask {}
impl FinishedRenderTask {
    pub fn new() -> Self {
        Self {}
    }
}
impl RenderTask for FinishedRenderTask {
    fn get_state(&self) -> RenderState {
        RenderState::Finished
    }
    fn enter(&mut self) -> Result<(), PbrtError> {
        // Here you would finalize the rendering process
        log::info!("Finalizing rendering process");
        Ok(())
    }
    fn update(&mut self) -> Result<RenderState, PbrtError> {
        // Here you would finalize the rendering process
        // For now, we just simulate it
        Ok(RenderState::Finished)
    }
}
