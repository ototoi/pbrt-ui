//! PBRT preview parser for a deliberately explicit initial subset.
use crate::{
    document::{ReadOptions, read_document_with_options},
    error::{Diagnostic, SceneParseError},
    mesh_build,
    params::Directive,
    state_graph::parse_state_graph,
    tessellate,
    transform::Mat4,
};
use protocol::{geometry::*, package::SourceLocation};
use sha2::Digest;
use std::{collections::BTreeMap, fs, io::Read, path::Path};

const DEFAULT_MAX_GEOMETRY_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ParseOptions {
    pub angular_segments: u32,
    pub include_depth: u32,
    pub max_geometry_bytes: usize,
    pub max_asset_bytes: usize,
    /// Program filename → the model's recorded sha256 for that asset
    /// A plymesh with a table entry forms its cache key
    /// without opening the file, and imagemap / image-light blob records
    /// reuse the entry instead of hashing the payload they read. Empty on
    /// the text-parse paths, where assets are read and hashed as before.
    pub asset_sha256: BTreeMap<String, String>,
    /// Existing content-store entries, keyed by sha256 and carrying their
    /// metadata size. A hit lets parsing omit payload reads and mesh clones.
    pub stored_blobs: BTreeMap<String, u64>,
}
impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            angular_segments: 64,
            include_depth: 64,
            max_geometry_bytes: DEFAULT_MAX_GEOMETRY_BYTES,
            max_asset_bytes: 128 * 1024 * 1024,
            asset_sha256: BTreeMap::new(),
            stored_blobs: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct PreviewGeometry {
    pub manifest: GeometryManifest,
    pub blobs: BTreeMap<String, Vec<u8>>,
    pub diagnostics: Vec<Diagnostic>,
}
impl PreviewGeometry {
    /// Durable package write for callers whose output users keep; the
    /// disposable preview slot writes through `write_preview` with
    /// `durable = false` instead.
    pub fn write_to(&self, dir: &Path) -> Result<(), SceneParseError> {
        crate::export::write_preview(self, dir, true)
    }
    /// Warnings the package carries (e.g. a mesh dropped for exceeding
    /// the preview asset budget) — the count surfaces to callers as the
    /// `unsupported` number.
    pub fn warning_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.is_warning())
            .count()
    }
}

/// One cached tessellation, keyed by the shape directive's parsed content and
/// the content hash of any PLY asset it references.
#[derive(Clone, Debug)]
pub struct CachedGeometry {
    pub points: Vec<[f64; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub blob: Vec<u8>,
    /// The blob's sha256, computed once when the entry is put. Cache hits
    /// reuse it for the manifest instead of re-hashing
    /// the blob.
    pub blob_sha256: String,
    pub positions: ArrayRef,
    pub normals_ref: ArrayRef,
    pub indices_ref: ArrayRef,
    /// Serialized uv span, when the tessellator produced texcoords.
    pub uv_ref: Option<ArrayRef>,
}

/// Content-addressed tessellation cache across parses of edited exports:
/// unchanged shape directives (including their asset bytes) skip
/// tessellation, vertex normal generation and blob assembly entirely.
#[derive(Clone, Debug, Default)]
pub struct GeometryCache {
    entries: BTreeMap<[u8; 32], CachedGeometry>,
    entries_bytes: usize,
    max_bytes: usize,
    hits: usize,
    misses: usize,
    /// How many plymesh asset files a build actually opened. Cache hits
    /// with a model sha256 add zero, so this counter shows whether a cached
    /// preview opened a PLY file.
    asset_reads: usize,
}

impl GeometryCache {
    /// Zero means no bound; with a bound the cache clears itself wholesale
    /// once the retained geometry exceeds it.
    pub fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            ..Self::default()
        }
    }

    /// Record one plymesh asset file opened for its bytes.
    pub(crate) fn note_asset_read(&mut self) {
        self.asset_reads += 1;
    }

    pub(crate) fn get(
        &mut self,
        key: &[u8; 32],
        stored_blobs: &BTreeMap<String, u64>,
    ) -> Option<CachedGeometry> {
        if let Some(entry) = self.entries.get(key) {
            self.hits += 1;
            Some(CachedGeometry {
                points: entry.points.clone(),
                normals: entry.normals.clone(),
                indices: entry.indices.clone(),
                blob: if stored_blobs.contains_key(&entry.blob_sha256) {
                    Vec::new()
                } else {
                    entry.blob.clone()
                },
                blob_sha256: entry.blob_sha256.clone(),
                positions: entry.positions.clone(),
                normals_ref: entry.normals_ref.clone(),
                indices_ref: entry.indices_ref.clone(),
                uv_ref: entry.uv_ref.clone(),
            })
        } else {
            self.misses += 1;
            None
        }
    }

    pub(crate) fn put(&mut self, key: [u8; 32], entry: CachedGeometry) {
        let bytes = entry.blob.len() + entry.points.len() * 24;
        if self.max_bytes > 0 && self.entries_bytes + bytes > self.max_bytes {
            self.entries.clear();
            self.entries_bytes = 0;
        }
        if self.entries.insert(key, entry).is_none() {
            self.entries_bytes += bytes;
        }
    }

    pub fn stats(&self) -> GeometryCacheStats {
        GeometryCacheStats {
            entries: self.entries.len(),
            hits: self.hits,
            misses: self.misses,
            bytes: self.entries_bytes,
            asset_reads: self.asset_reads,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeometryCacheStats {
    pub entries: usize,
    pub hits: usize,
    pub misses: usize,
    pub bytes: usize,
    pub asset_reads: usize,
}

#[derive(Clone)]
struct Shape {
    d: Directive,
    start_transform: Mat4,
    end_transform: Mat4,
    times: [f64; 2],
    reverse: bool,
    index: usize,
    definition: Option<String>,
    material: Option<MaterialRecord>,
    interface_material: bool,
    alpha: Option<PreviewParameter>,
    area_light: Option<AreaLightRecord>,
}

struct InstanceUse {
    name: String,
    source: SourceLocation,
    start_transform: Mat4,
    end_transform: Mat4,
    times: [f64; 2],
}

mod assets;
mod declarations;
mod geometry;
mod measured;
mod placement;
pub(crate) mod program;
mod shape_geometry;
use assets::read_ply_asset;
use placement::{placement_transforms, transformed_bounds};

pub fn parse_scene(
    path: impl AsRef<Path>,
    options: ParseOptions,
) -> Result<PreviewGeometry, SceneParseError> {
    parse_scene_with_cache(path, options, &mut GeometryCache::default())
}

/// Same as [`parse_scene`], but reuses tessellations of shapes whose parsed
/// directives (and PLY asset contents) are unchanged since a previous parse,
/// e.g. the preview refresh of an edited project export.
pub fn parse_scene_with_cache(
    path: impl AsRef<Path>,
    options: ParseOptions,
    cache: &mut GeometryCache,
) -> Result<PreviewGeometry, SceneParseError> {
    let root = protocol::path::canonicalize(path.as_ref()).map_err(|e| {
        SceneParseError::unreadable(&path.as_ref().display().to_string(), e.to_string())
    })?;
    let document = read_document_with_options(
        &root,
        ReadOptions {
            include_depth: options.include_depth,
            expand_imports: true,
            ..ReadOptions::default()
        },
    )?;
    let root_dir = root.parent().unwrap_or(Path::new(".")).to_path_buf();
    program::analyze(document, None, &root_dir, &options, cache)
}

#[cfg(test)]
mod tests;
