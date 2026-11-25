//! The Mesh Viewer's one-mesh scene: the PLY
//! asset as the only shape of a synthesized scene, parsed into the same
//! geometry package the Scene panel draws. Pure apart from the temporary
//! scene file, so it is tested without a GPU.
use scene_parse::{ParseOptions, PreviewGeometry};
use scene_view::{PreviewFrame, PreviewMode, PreviewRenderer, ViewerCamera};
use std::sync::mpsc;

/// What the viewer shows next to the image.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshStats {
    pub vertices: u64,
    pub triangles: u64,
    pub bounds: Option<([f64; 3], [f64; 3])>,
}

/// Counts and bounds of the loaded geometry (all meshes of the one shape).
pub fn stats(geometry: &PreviewGeometry) -> MeshStats {
    let manifest = &geometry.manifest;
    MeshStats {
        vertices: manifest
            .meshes
            .iter()
            .map(|mesh| mesh.positions.count)
            .sum(),
        triangles: manifest
            .meshes
            .iter()
            .map(|mesh| mesh.indices.count / 3)
            .sum(),
        bounds: manifest
            .bounds
            .as_ref()
            .map(|bounds| (bounds.min, bounds.max)),
    }
}

/// One render request; the worker keeps only the newest pending one.
/// `generation` comes back with the frame, so the viewer knows whether the
/// newest request has landed.
pub struct Request {
    pub generation: u64,
    pub camera: ViewerCamera,
    pub size: [u32; 2],
    pub mode: PreviewMode,
}

/// Which requests are out: the viewer waits (and keeps redrawing) until
/// the newest one has landed, however many older frames arrive first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    requested: u64,
    landed: u64,
}

impl Progress {
    /// The generation for a new request.
    pub fn next(&mut self) -> u64 {
        self.requested += 1;
        self.requested
    }

    pub fn land(&mut self, generation: u64) {
        self.landed = self.landed.max(generation);
    }

    /// A request that will not be answered (the worker stopped).
    pub fn abandon(&mut self) {
        self.landed = self.requested;
    }

    pub fn waiting(&self) -> bool {
        self.landed < self.requested
    }
}

/// A rendered frame (or why not) and the request generation it answers.
pub type Landed = (u64, Result<PreviewFrame, String>);

/// A Mesh Viewer tab's own renderer on its own thread (the user's ruling
/// C: never the Scene panel's GPU resources). Dropping it ends the thread
/// and waits for it, so the renderer is released before the tab (or the
/// process) goes away.
pub struct Worker {
    requests: Option<mpsc::Sender<Request>>,
    pub frames: mpsc::Receiver<Landed>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.requests.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Worker {
    pub fn start(geometry: PreviewGeometry) -> Self {
        let (requests, inbox) = mpsc::channel::<Request>();
        let (outbox, frames) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let mut renderer = match PreviewRenderer::new().and_then(|mut renderer| {
                renderer.upload_scene(&geometry.manifest, &geometry.blobs)?;
                Ok(renderer)
            }) {
                Ok(renderer) => renderer,
                Err(error) => {
                    // Answer the first request with the reason, then stop.
                    if let Ok(request) = inbox.recv() {
                        let _ = outbox.send((request.generation, Err(error)));
                    }
                    return;
                }
            };
            let time = geometry
                .manifest
                .supported_times
                .first()
                .copied()
                .unwrap_or(0.);
            while let Ok(mut request) = inbox.recv() {
                // Skip to the newest request: a drag sends many.
                while let Ok(newer) = inbox.try_recv() {
                    request = newer;
                }
                let [width, height] = request.size;
                let frame = renderer.render(request.camera, width, height, request.mode, time);
                if outbox.send((request.generation, frame)).is_err() {
                    return;
                }
            }
        });
        Self {
            requests: Some(requests),
            frames,
            thread: Some(thread),
        }
    }

    /// Queue a request; false when the worker has stopped (its renderer
    /// failed), so the caller does not wait for a frame that never comes.
    pub fn render(&self, request: Request) -> bool {
        self.requests
            .as_ref()
            .is_some_and(|requests| requests.send(request).is_ok())
    }
}

/// The synthesized scene text: just the mesh, at the origin.
pub fn scene_text(ply_path: &str) -> String {
    let escaped = ply_path.replace('\\', "\\\\").replace('"', "\\\"");
    format!("WorldBegin\nShape \"plymesh\" \"string filename\" \"{escaped}\"\n")
}

/// Parse the one-mesh scene for `ply_path`. The scene file lives in a
/// private fresh directory under the user's cache (never next to the asset,
/// which belongs to the project) and is removed again.
pub fn load(ply_path: &str) -> Result<PreviewGeometry, String> {
    let cache = project::cache_directory().map_err(|error| error.to_string())?;
    load_in_cache(ply_path, &cache)
}

fn load_in_cache(ply_path: &str, cache: &std::path::Path) -> Result<PreviewGeometry, String> {
    let directory = cache.join(format!("pbrt-ui-mesh-{}", uuid::Uuid::new_v4()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = std::fs::DirBuilder::new();
        builder
            .mode(0o700)
            .create(&directory)
            .map_err(|error| error.to_string())?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir(&directory).map_err(|error| error.to_string())?;
    let scene = directory.join("mesh.pbrt");
    let result = std::fs::write(&scene, scene_text(ply_path))
        .map_err(|error| error.to_string())
        .and_then(|()| {
            scene_parse::parse_scene(&scene, ParseOptions::default())
                .map_err(|error| error.to_string())
        });
    let _ = std::fs::remove_dir_all(&directory);
    // The parser skips what it cannot read and warns; with one shape, a
    // skipped mesh is the viewer's error.
    let geometry = result?;
    if geometry.manifest.draws.is_empty() {
        let reasons = geometry
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect::<Vec<_>>();
        return Err(if reasons.is_empty() {
            "the mesh has no triangles".into()
        } else {
            reasons.join("; ")
        });
    }
    Ok(geometry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_viewer_waits_for_the_newest_request() {
        let mut progress = Progress::default();
        assert!(!progress.waiting());
        let first = progress.next();
        let second = progress.next();
        // The older frame lands while the newer is still being drawn.
        progress.land(first);
        assert!(progress.waiting(), "the newest frame is still out");
        progress.land(second);
        assert!(!progress.waiting());
        // A stopped worker never answers: stop waiting.
        progress.next();
        progress.abandon();
        assert!(!progress.waiting());
    }

    #[test]
    fn the_scene_holds_only_the_mesh() {
        assert_eq!(
            scene_text("/tmp/a \"b\".ply"),
            "WorldBegin\nShape \"plymesh\" \"string filename\" \"/tmp/a \\\"b\\\".ply\"\n"
        );
    }

    #[test]
    fn a_ply_loads_as_one_draw_with_bounds() {
        let directory = tempfile::tempdir().unwrap();
        let ply = directory.path().join("tri.ply");
        std::fs::write(
            &ply,
            "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\n\
             property float z\nelement face 1\nproperty list uchar int vertex_indices\n\
             end_header\n0 0 0\n2 0 0\n0 1 0\n3 0 1 2\n",
        )
        .unwrap();
        let geometry = load_in_cache(ply.to_str().unwrap(), directory.path()).unwrap();
        assert_eq!(geometry.manifest.draws.len(), 1);
        let stats = stats(&geometry);
        assert_eq!((stats.vertices, stats.triangles), (3, 1));
        assert_eq!(stats.bounds.unwrap().1, [2.0, 1.0, 0.0]);
        let bounds = geometry.manifest.bounds.as_ref().unwrap();
        assert_eq!(bounds.max[0], 2.0);
        assert_eq!(bounds.max[1], 1.0);
        // A missing file is an error, not an empty scene.
        assert!(
            load_in_cache(
                directory.path().join("missing.ply").to_str().unwrap(),
                directory.path()
            )
            .is_err()
        );
    }

    #[test]
    fn staged_gzip_payload_loads_in_the_mesh_viewer_with_header_stats() {
        use sha2::Digest as _;
        use std::io::Write as _;

        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("tri.ply.gz");
        let ply = b"ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n2 0 0\n0 1 0\n3 0 1 2\n";
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(ply).unwrap();
        let compressed = encoder.finish().unwrap();
        std::fs::write(&source, &compressed).unwrap();

        let asset = project::Asset {
            id: project::AssetId::new(),
            location: "Meshes/tri.ply.gz".into(),
            media_type: "application/x-ply".into(),
            sha256: format!("{:x}", sha2::Sha256::digest(&compressed)),
            size: compressed.len() as u64,
            origin: None,
        };
        let mut staging = project::ImportStaging::new_in(directory.path()).unwrap();
        let payload = staging.stage_file(&asset, &source).unwrap();
        assert!(
            payload
                .path()
                .extension()
                .is_some_and(|ext| ext == "payload")
        );

        let info = scene_parse::ply::ply_info_from_file(payload.path()).unwrap();
        assert_eq!((info.vertices, info.faces), (3, 1));
        let geometry = load_in_cache(payload.path().to_str().unwrap(), directory.path()).unwrap();
        assert_eq!(geometry.manifest.draws.len(), 1);
        assert_eq!(
            (stats(&geometry).vertices, stats(&geometry).triangles),
            (3, 1)
        );
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn the_worker_renders_a_frame_on_its_own_renderer() {
        let directory = tempfile::tempdir().unwrap();
        let ply = directory.path().join("tri.ply");
        std::fs::write(
            &ply,
            "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\n\
             property float z\nelement face 1\nproperty list uchar int vertex_indices\n\
             end_header\n0 0 0\n2 0 0\n0 1 0\n3 0 1 2\n",
        )
        .unwrap();
        let geometry = load_in_cache(ply.to_str().unwrap(), directory.path()).unwrap();
        let bounds = geometry.manifest.bounds.clone();
        let worker = Worker::start(geometry);
        assert!(worker.render(Request {
            generation: 7,
            camera: ViewerCamera::frame(bounds.as_ref(), 1.),
            size: [64, 64],
            mode: PreviewMode::Solid,
        }));
        let (generation, frame) = worker
            .frames
            .recv_timeout(std::time::Duration::from_secs(30))
            .unwrap();
        assert_eq!(generation, 7, "the frame answers its request");
        let frame = frame.unwrap();
        assert_eq!((frame.width, frame.height), (64, 64));
        assert_eq!(frame.rgba.len(), 64 * 64 * 4);
    }
}
