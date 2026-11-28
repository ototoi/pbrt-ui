//! File/process executor for capture and render jobs. No GPU types enter core.
use crate::state::LoadTask;
use protocol::{ErrorCode, ErrorPayload, jobs::*};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};
#[derive(Clone)]
pub struct ServiceConfig {
    pub gui: bool,
    pub capture: Option<PathBuf>,
    pub renderer: Option<PathBuf>,
    pub directory: Arc<tempfile::TempDir>,
    /// Shared content-keyed tessellation cache so repeated project captures
    /// (and capture/pick pairs of one snapshot) skip re-tessellation, exactly
    /// like the desktop preview refresh. Interior-mutable and process-local.
    pub geometry_cache: Arc<std::sync::Mutex<scene_parse::GeometryCache>>,
    /// Resident capture adapter. `None` until the first capture spawns it
    /// (or after a break, when jobs fall back to one-shot adapter runs).
    pub capture_daemon: Arc<std::sync::Mutex<Option<capture_daemon::CaptureDaemon>>>,
    /// One `Library/Jobs` folder per project, shared by every clone so job
    /// artifacts outlive the project; removed when the last clone drops.
    pub project_directories:
        Arc<std::sync::Mutex<std::collections::HashMap<PathBuf, Arc<tempfile::TempDir>>>>,
}
impl ServiceConfig {
    pub(crate) fn for_project(&self, project: Option<&Path>) -> Result<Self, ErrorPayload> {
        let directory = match project {
            Some(folder) => self.project_job_directory(folder).map_err(fail)?,
            None => self.directory.clone(),
        };
        let mut config = self.clone();
        config.directory = directory;
        Ok(config)
    }

    fn project_job_directory(&self, folder: &Path) -> std::io::Result<Arc<tempfile::TempDir>> {
        let mut directories = self
            .project_directories
            .lock()
            .expect("job directory map poisoned");
        if let Some(directory) = directories.get(folder) {
            return Ok(directory.clone());
        }
        let directory = Arc::new(project::jobs_tempdir_in(&folder.join("Library/Jobs"))?);
        directories.insert(folder.to_path_buf(), directory.clone());
        Ok(directory)
    }

    pub fn discover(renderer: Option<PathBuf>) -> Result<Self, ErrorPayload> {
        let renderer = renderer.or_else(|| {
            std::env::current_exe()
                .ok()
                .map(|path| path.with_file_name(format!("pbrt-r4{}", std::env::consts::EXE_SUFFIX)))
                .filter(|path| path.is_file())
        });
        let capture = std::env::current_exe()
            .ok()
            .map(|path| {
                path.with_file_name(format!("pbrt-ui-capture{}", std::env::consts::EXE_SUFFIX))
            })
            .filter(|path| path.is_file());
        if renderer
            .as_ref()
            .is_some_and(|p| !p.is_absolute() || !p.is_file())
        {
            return Err(fail("renderer must be an absolute executable path"));
        }
        Ok(Self {
            gui: false,
            capture,
            renderer,
            directory: Arc::new(
                project::jobs_tempdir_in(&project::cache_directory().map_err(fail)?)
                    .map_err(fail)?,
            ),
            geometry_cache: Arc::new(std::sync::Mutex::new(scene_parse::GeometryCache::new(
                256 * 1024 * 1024,
            ))),
            capture_daemon: Arc::new(std::sync::Mutex::new(None)),
            project_directories: Default::default(),
        })
    }
}

#[derive(Clone)]
pub enum Work {
    ProjectRender {
        root: String,
        project: protocol::project::ProjectRef,
        settings: RenderSettings,
        view: Option<protocol::view::ViewState>,
        ipr: bool,
    },
    ProjectCapture {
        root: String,
        project: protocol::project::ProjectRef,
        view: Option<protocol::view::ViewState>,
        /// View revision pinned with the request, reported in the frame key.
        view_revision: protocol::Revision,
        pick: Option<[f64; 2]>,
        debug_albedo: bool,
        debug_ao: bool,
        /// Capture's camera selection; pick always passes `None`.
        camera: Option<protocol::project::RenderCamera>,
    },
    Export {
        artifact: Artifact,
        path: String,
        overwrite: bool,
    },
}
pub struct ServiceTask {
    pub task: LoadTask,
    pub work: Work,
    pub config: ServiceConfig,
}
fn fail(error: impl ToString) -> ErrorPayload {
    ErrorPayload::new(ErrorCode::IoError, error.to_string())
}
pub struct WorkResult {
    pub result: Result<Value, ErrorPayload>,
    pub artifacts: Vec<Artifact>,
}
/// A temp directory only this user can enter: render jobs and previews hold
/// the scene and hard links to project assets. Windows' `%TEMP%` is already
/// per-user.
pub(crate) fn private_tempdir_in(parent: &Path) -> std::io::Result<tempfile::TempDir> {
    // Only Unix narrows the permissions, so only Unix needs `mut`.
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o700));
    }
    builder.tempdir_in(parent)
}

/// `create_dir_all` whose new directories only this user can enter.
pub(crate) fn create_private_dir_all(path: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

#[cfg(test)]
mod directory_tests {
    use super::*;

    #[test]
    fn project_jobs_are_created_under_library_jobs_and_cache_is_retained_without_project() {
        let cache = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let config = ServiceConfig {
            gui: false,
            capture: None,
            renderer: None,
            directory: Arc::new(private_tempdir_in(cache.path()).unwrap()),
            geometry_cache: Default::default(),
            capture_daemon: Default::default(),
            project_directories: Default::default(),
        };
        let fallback = config.for_project(None).unwrap();
        assert_eq!(fallback.directory.path(), config.directory.path());
        let project_config = config.for_project(Some(project.path())).unwrap();
        assert!(
            project_config
                .directory
                .path()
                .starts_with(project.path().join("Library/Jobs"))
        );
        let job_folder = project_config.directory.path().join("job");
        fs::create_dir_all(&job_folder).unwrap();
        assert!(job_folder.exists());
        drop(project_config);
        let again = config.for_project(Some(project.path())).unwrap();
        assert!(job_folder.exists(), "artifacts outlive the job's config");
        drop((again, fallback, config));
        assert!(!job_folder.exists(), "the last config removes the folder");
    }
}

mod artifact_io;
mod capture;
pub mod capture_daemon;
mod child_group;
mod process;
mod render;
mod view_scene;
use artifact_io::artifact;
use process::run_process;
use view_scene::write_view_scene;
pub fn execute(
    work: ServiceTask,
    progress: std::sync::mpsc::Sender<(protocol::JobId, Artifact)>,
) -> WorkResult {
    let directory = work
        .config
        .directory
        .path()
        .join(work.task.job_id.to_string());
    let mut artifacts = Vec::new();
    let result = (|| {
        fs::create_dir_all(&directory).map_err(fail)?;
        if work.task.is_cancelled() {
            return Err(ErrorPayload::new(ErrorCode::Busy, "job cancelled"));
        }
        match &work.work {
            Work::ProjectCapture { .. } => capture::execute(&work, &directory, &mut artifacts),
            Work::ProjectRender { .. } => {
                render::execute(&work, &directory, &mut artifacts, progress)
            }
            Work::Export { .. } => artifact_io::export(&work),
        }
    })();
    if matches!(&work.work, Work::ProjectRender { ipr: true, .. }) {
        let _ = fs::remove_dir_all(&directory);
    }
    if let Work::ProjectRender { project, .. } | Work::ProjectCapture { project, .. } = &work.work {
        for artifact in &mut artifacts {
            artifact.metadata["project"] = json!(project);
        }
    }
    WorkResult { result, artifacts }
}
