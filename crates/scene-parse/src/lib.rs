//! pbrt-ui owned PBRT parser for the realtime preview path.
//!
//! This crate is deliberately independent of `pbrt-r4`: the viewport geometry is
//! produced here so the interactive renderer never waits on the (much slower)
//! renderer scene-build path. Metadata and source locations are also owned by
//! this workspace. Rendering uses an external CLI; parser values are never
//! fed back into the renderer. Core joins preview draws to the UI index by
//! source location and leaves ambiguous joins unresolved.
//!
//! Meshes are generated in **object space**
//! with an identity transform, and every placement (top-level shape or
//! `ObjectInstance`) carries its own `world_from_object` matrices in a
//! `DrawRecord`.
// Vector and matrix code indexes several arrays with one component index;
// iterator chains would read worse there.
#![allow(clippy::needless_range_loop)]

pub mod document;
pub mod error;
pub mod export;
pub mod hdr;
pub mod lexer;
pub mod material_preview;
pub mod mesh_build;
pub mod params;
pub mod ply;
pub mod scene;
pub mod state_graph;
pub mod tessellate;
pub mod texture_eval;
pub mod tonemap;
pub mod transform;

pub use error::{Diagnostic, SceneParseError, Severity};
pub use export::{
    PreviewManifest, PreviewSummary, read_preview_blobs, read_preview_diagnostics,
    read_preview_manifest, read_preview_package, write_preview, write_preview_with_store,
};
pub use params::{Directive, Parameter, Params, Value};
pub use scene::program::{PREVIEW_PATH, PreviewProgram, preview_from_program};
pub use scene::{
    CachedGeometry, GeometryCache, GeometryCacheStats, ParseOptions, PreviewGeometry, parse_scene,
    parse_scene_with_cache,
};
pub use transform::{Mat4, TransformSet};
