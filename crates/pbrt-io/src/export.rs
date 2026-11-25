use crate::Result;
use crate::emit::{Emitter, TextSink};
use crate::*;
use project::*;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
};

/// Generate a fixed input bundle. Inputs never refer back to imported sources.
/// Film output is redirected to render.exr; original declarations stay in the project.
pub fn export_project(
    snapshot: &ProjectSnapshot,
    store: &ProjectStore,
    destination: &Path,
    cancelled: impl Fn() -> bool,
) -> Result<PathBuf> {
    check(&cancelled)?;
    let document = snapshot.document();
    let view = snapshot.scene_view()?;
    let evaluated = evaluate(&view)?;
    crate::schema::exportable(document)?;
    crate::schema::missing_required_resources(document)?;
    let names = crate::emit::resource_names(document);
    let destination = crate::import::absolute_destination(destination)?;
    let temporary = tempfile::tempdir_in(destination.parent().unwrap())?;
    let staged = temporary.path().join("bundle");
    fs::create_dir(&staged)?;
    let assets = stage_snapshot_assets(snapshot, store, &staged, &cancelled)?;
    let spectra = stage_spectrum_files(snapshot, store, &staged, &cancelled)?;
    // Hash while writing: scene.pbrt stays write-only and the previous
    // full re-read of the finished bundle drops out of the export path.
    let mut hash = Sha256::new();
    let mut node_sources = BTreeMap::new();
    // Every handle into the bundle closes before `publish` renames it:
    // Windows refuses to rename a directory with open files.
    {
        let mut file = LineWriter {
            inner: HashingWriter {
                inner: BufWriter::new(File::create(staged.join("scene.pbrt"))?),
                hash: &mut hash,
            },
            line: 1,
        };
        {
            let emitter = Emitter {
                snapshot: Some(snapshot),
                view,
                store,
                assets: &assets,
                names: &names,
                spectra: &spectra,
                cancelled: &cancelled,
                evaluated: &evaluated,
            };
            let mut sink = TextSink::new(file, &mut node_sources);
            emitter.whole_scene(&mut sink)?;
            file = sink.file;
        }
        file.flush()?;
        file.inner.inner.get_ref().sync_all()?;
    }
    let manifest = serde_json::json!({
        "format": "pbrt-ui-render-input", "format_version": 1,
        "project_id": document.id, "project_revision": snapshot.revision(),
        "exporter_version": env!("CARGO_PKG_VERSION"), "dialect": "pbrt_r4", "renderer_semantic_target": crate::R4_SEMANTIC_TARGET,
        "scene_sha256": format!("{:x}", hash.finalize()),
        "assets": document.assets.iter().map(|(id,a)| serde_json::json!({"id":id,"path":assets[id],"object":a.object()})).collect::<Vec<_>>(),
        "film_output": "render.exr",
        "node_sources": node_sources
    });
    {
        let mut manifest_file = File::create(staged.join("export.json"))?;
        serde_json::to_writer_pretty(&mut manifest_file, &manifest).map_err(ProjectError::from)?;
        manifest_file.sync_all()?;
    }
    project::sync_path(staged.join("assets"))?;
    project::sync_path(&staged)?;
    check(&cancelled)?;
    publish(&staged, &destination)?;
    Ok(destination.join("scene.pbrt"))
}
/// Record directive line numbers without changing the exported PBRT bytes.
/// Write-through SHA-256: the scene digest is computed on the same write
/// path that serializes the text, so nothing is read back.
struct HashingWriter<'a, W> {
    inner: W,
    hash: &'a mut Sha256,
}
impl<W: Write> Write for HashingWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let count = self.inner.write(bytes)?;
        self.hash.update(&bytes[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
pub(crate) struct LineWriter<W> {
    inner: W,
    pub(crate) line: u64,
}
impl<W: Write> Write for LineWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let count = self.inner.write(bytes)?;
        self.line += bytes[..count].iter().filter(|b| **b == b'\n').count() as u64;
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Stage every project asset into `staged/assets` and return the name table
/// (`AssetId` → bundle-relative path). Shared by the text export and the
/// direct-preview program so PLY references resolve identically.
pub fn stage_assets(
    document: &ProjectDocument,
    store: &ProjectStore,
    staged: &Path,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeMap<AssetId, String>> {
    stage_assets_resolved(document, staged, cancelled, |id| store.asset_path(id))
}

/// Snapshot-aware staging also resolves bytes restored from Undo history.
pub fn stage_snapshot_assets(
    snapshot: &ProjectSnapshot,
    store: &ProjectStore,
    staged: &Path,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeMap<AssetId, String>> {
    stage_assets_resolved(snapshot.document(), staged, cancelled, |id| {
        snapshot.asset_path(store, id)
    })
}

/// Resolve every asset to its real file in the project store (design
/// instead of staging a verified copy: no read, no sha256, no
/// fsync. The result is keyed like [`stage_snapshot_assets`]'s table but
/// the values are the store (or Undo payload lease) paths themselves, so
/// the program's directives reference the source files directly and
/// `snapshot.asset_path`'s verification stays where it is — the cache
/// miss that reads a PLY checks the model's sha256 on the way in.
pub fn asset_store_paths(
    snapshot: &ProjectSnapshot,
    store: &ProjectStore,
) -> Result<BTreeMap<AssetId, String>> {
    let mut assets = BTreeMap::new();
    for (id, asset) in &snapshot.document().assets {
        // Folder projects expose assets by stable AssetId location.
        // Legacy format-1 projects resolve the same API to their
        // read-only object.
        let missing = || Error::AssetMissing {
            asset: *id,
            location: asset.location.clone(),
        };
        let path = snapshot
            .asset_path(store, *id)
            .map_err(|error| match error {
                ProjectError::AssetFileMissing(_) => missing(),
                error => error.into(),
            })?;
        assets.insert(*id, path.to_string_lossy().into_owned());
    }
    Ok(assets)
}

fn stage_assets_resolved(
    document: &ProjectDocument,
    staged: &Path,
    cancelled: &impl Fn() -> bool,
    resolve: impl Fn(AssetId) -> project::Result<PathBuf>,
) -> Result<BTreeMap<AssetId, String>> {
    check(cancelled)?;
    fs::create_dir(staged.join("assets"))?;
    let mut assets = BTreeMap::new();
    for (id, asset) in &document.assets {
        check(cancelled)?;
        let basename = Path::new(&asset.location)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("asset.bin");
        let safe: String = basename
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let name = format!("assets/{id}-{safe}");
        // Folder projects expose assets by stable AssetId location. Legacy
        // format-1 projects resolve the same API to their read-only object.
        let missing = || Error::AssetMissing {
            asset: *id,
            location: asset.location.clone(),
        };
        let source = resolve(*id).map_err(|error| match error {
            ProjectError::AssetFileMissing(_) => missing(),
            error => error.into(),
        })?;
        let destination = staged.join(&name);
        let mut input = File::open(&source).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => missing(),
            _ => error.into(),
        })?;
        let mut output = File::create(&destination)?;
        let mut buffer = [0; 65536];
        let mut hash = Sha256::new();
        let mut size = 0u64;
        loop {
            check(cancelled)?;
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            output.write_all(&buffer[..count])?;
            hash.update(&buffer[..count]);
            size += count as u64;
        }
        if size != asset.size || format!("{:x}", hash.finalize()) != asset.sha256 {
            return Err(invalid("asset changed during export"));
        }
        output.sync_all()?;
        assets.insert(*id, name);
    }
    Ok(assets)
}

/// Write one `spds/<name>.spd` per SpectrumData resource into the bundle
/// and return the paths (`ResourceId` → bundle-relative), so
/// `spectrum` parameters can reference the files like any other dependency.
/// Names come from the resource labels with the bundle's dependency-file
/// character rule, numbered `_N` after the first collision.
pub fn stage_spectrum_files(
    snapshot: &ProjectSnapshot,
    store: &ProjectStore,
    staged: &std::path::Path,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeMap<ResourceId, String>> {
    check(cancelled)?;
    let mut names = BTreeMap::new();
    let mut resources = snapshot
        .document()
        .resources
        .values()
        .filter(|resource| resource.kind == ResourceKind::SpectrumData)
        .collect::<Vec<_>>();
    resources.sort_by(|a, b| a.location.cmp(&b.location).then(a.id.cmp(&b.id)));
    if resources.is_empty() {
        return Ok(names);
    }
    fs::create_dir(staged.join("spds"))?;
    // Placement names compare case-insensitively everywhere else (B.3);
    // the bundle numbers the same way so `Gold`/`gold` from two resources
    // cannot overwrite one file on case-insensitive filesystems.
    let mut used = BTreeSet::<String>::new();
    for resource in resources {
        check(cancelled)?;
        let (wavelengths, values) =
            crate::emit::spectrum_samples(resource, |id| Ok(snapshot.asset_path(store, id)?))?;
        let mut base = resource
            .label
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        if base.is_empty() {
            base = "spectrum".into();
        }
        let mut candidate = base.clone();
        let mut number = 1;
        while !used.insert(candidate.to_lowercase()) {
            candidate = format!("{base}_{number}");
            number += 1;
        }
        let path = format!("spds/{candidate}.spd");
        let mut text = String::new();
        for (wavelength, value) in wavelengths.into_iter().zip(values) {
            text.push_str(&wavelength.to_string());
            text.push(' ');
            text.push_str(&value.to_string());
            text.push('\n');
        }
        let mut file = File::create(staged.join(&path))?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        names.insert(resource.id, path);
    }
    Ok(names)
}

/// Compose the direct-preview program for a snapshot: the same emitter the
/// text export uses, captured as directives plus the origin table instead
/// of scene.pbrt bytes. Runs the same `evaluate` / `schema::exportable`
/// gates as the text path.
pub fn program_from_project(
    snapshot: &ProjectSnapshot,
    store: &ProjectStore,
    assets: &BTreeMap<AssetId, String>,
    cancelled: impl Fn() -> bool,
) -> Result<scene_parse::PreviewProgram> {
    check(&cancelled)?;
    let document = snapshot.document();
    let view = snapshot.scene_view()?;
    let evaluated = evaluate(&view)?;
    crate::schema::exportable(document)?;
    // No missing-reference check: the preview never reaches pbrt-r4, and a
    // fresh mix must not blank the viewport before its materials are set.
    let names = crate::emit::resource_names(document);
    // The program expands SpectrumData references itself; until that
    // lands, no staged files exist and the emitter keeps its resource-name
    // fallback for the parameter's text.
    let spectra = BTreeMap::new();
    let emitter = Emitter {
        snapshot: Some(snapshot),
        view,
        evaluated: &evaluated,
        store,
        assets,
        names: &names,
        spectra: &spectra,
        cancelled: &cancelled,
    };
    let sink = {
        let mut sink = crate::emit::ProgramSink::new();
        emitter.whole_scene(&mut sink)?;
        sink
    };
    Ok(sink.finish())
}

/// Something an export that succeeded left out (the wire's `warnings`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportWarning {
    pub code: &'static str,
    pub resource: ResourceId,
    pub message: String,
}

/// The warnings exporting `snapshot`'s active scene raises: today, the
/// imagemaps without an image it leaves out. Errs as the export would when
/// one of them is written or referred to.
pub fn export_warnings(snapshot: &ProjectSnapshot) -> Result<Vec<ExportWarning>> {
    let view = snapshot.scene_view()?;
    Ok(
        crate::emit::unset_imagemaps(&view, &snapshot.reference_index())?
            .into_iter()
            .map(|resource| ExportWarning {
                code: "imagemap_without_image_skipped",
                resource: resource.id,
                message: format!(
                    "imagemap texture \"{}\" has no image and was left out",
                    resource.label
                ),
            })
            .collect(),
    )
}
