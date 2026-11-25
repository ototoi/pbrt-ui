//! The program entry: build preview geometry from an in-memory directive
//! program instead of scene text. The text path (`parse_scene_with_cache`)
//! funnels through here, so analysis uses one implementation.
use super::assets;
use super::declarations::{self, Declarations};
use super::geometry;
use super::measured;
use super::{GeometryCache, ParseOptions, PreviewGeometry};
use crate::document::Document;
use crate::error::{Diagnostic, SceneParseError};
use crate::params::Directive;
use sha2::Digest;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Source label the direct emitter stamps into synthetic directives.
pub const PREVIEW_PATH: &str = "preview://project";

/// An emitted scene: directives, their file sources (empty for synthetic
/// programs), and the origin table that pins each directive (by its 1-based
/// emit ordinal) to the project node that emitted it.
#[derive(Debug, Default)]
pub struct PreviewProgram {
    pub directives: Vec<Directive>,
    pub origins: BTreeMap<u32, protocol::EntityId>,
    pub sources: BTreeMap<PathBuf, protocol::package::Dependency>,
}

/// Analyze an emitted program directly. `entity_of` pins draws to their
/// project nodes and re-keys meshes by node identity; text parses pass
/// `None` and keep their historical behavior.
pub fn preview_from_program(
    program: PreviewProgram,
    entity_of: Option<&BTreeMap<u32, protocol::EntityId>>,
    root_dir: &Path,
    options: ParseOptions,
    cache: &mut GeometryCache,
) -> Result<PreviewGeometry, SceneParseError> {
    let PreviewProgram {
        directives,
        origins,
        sources,
    } = program;
    let _ = origins;
    analyze(
        Document {
            directives,
            sources,
        },
        entity_of,
        root_dir,
        &options,
        cache,
    )
}

/// The shared analysis pipeline below both entries; also called by
/// `parse_scene_with_cache` with an empty origin table. The origin table
/// rides along unchanged on the text path.
#[allow(clippy::too_many_arguments)]
pub(super) fn analyze(
    document: Document,
    entity_of: Option<&BTreeMap<u32, protocol::EntityId>>,
    root_dir: &Path,
    options: &ParseOptions,
    cache: &mut GeometryCache,
) -> Result<PreviewGeometry, SceneParseError> {
    let Declarations {
        shapes,
        mut diagnostics,
        mut camera,
        instances,
        lights,
        textures,
        film_resolution,
        film_exposure,
        named_materials,
    } = declarations::collect(document, entity_of)?;
    let (mut manifest, mut blobs, scene_bounds) = geometry::build(
        shapes,
        &instances,
        &camera,
        root_dir,
        options,
        cache,
        &mut diagnostics,
        entity_of,
    )?;
    manifest.bounds = scene_bounds.clone();
    manifest.lights = lights;
    add_mix_children(&mut manifest.materials, &named_materials);
    measured::complete_records(&mut manifest.materials, root_dir, options, &mut diagnostics);
    let mut texture_blobs = BTreeMap::new();
    // Imagemap payloads ride along as package blobs next to the mesh ones;
    // texture-table growth stays bounded by the parse budget.
    manifest.textures = textures;
    for record in &manifest.textures {
        if record.implementation != "imagemap" {
            continue;
        }
        let Some(filename) = record
            .parameters
            .iter()
            .find(|p| p.matches("filename"))
            .and_then(|p| p.texts.first().cloned())
        else {
            diagnostics.push(match record.source.clone() {
                Some(source) => {
                    Diagnostic::warning("texture_failed", "imagemap without a filename")
                        .at(&source.path, source.line)
                }
                None => Diagnostic::warning("texture_failed", "imagemap without a filename"),
            });
            continue;
        };
        let asset_sha = options.asset_sha256.get(&filename);
        let stored_sha = asset_sha.filter(|sha| options.stored_blobs.contains_key(*sha));
        let bytes = if stored_sha.is_some() {
            None
        } else {
            read_referenced_asset(&filename, record.source.as_ref(), root_dir, options).ok()
        };
        if bytes.is_none() && stored_sha.is_none() {
            diagnostics.push(match record.source.clone() {
                Some(source) => Diagnostic::warning(
                    "texture_failed",
                    format!("imagemap {filename} could not be read within the budget"),
                )
                .at(&source.path, source.line),
                None => Diagnostic::warning(
                    "texture_failed",
                    format!("imagemap {filename} could not be read within the budget"),
                ),
            });
            continue;
        }
        let name = format!("texture/{}/{}", record.value_type, record.name);
        let sha256 = asset_sha
            .cloned()
            .unwrap_or_else(|| format!("{:x}", sha2::Sha256::digest(bytes.as_ref().unwrap())));
        let size = stored_sha
            .and_then(|sha| options.stored_blobs.get(sha).copied())
            .unwrap_or_else(|| bytes.as_ref().unwrap().len() as u64);
        manifest.blobs.insert(
            name.clone(),
            protocol::package::PackageFile {
                size,
                // A source that is a project asset takes
                // the model's sha256; the bytes still ride along until
                // the content store is available.
                sha256,
            },
        );
        if let Some(bytes) = bytes {
            texture_blobs.insert(name, bytes);
        }
    }
    blobs.extend(texture_blobs);
    // Image-based lights ride along too, as `light/{index}` of `manifest.lights`:
    // `infinite` maps as radiance, `goniometric` and `projection` as their
    // control images.
    for (index, record) in manifest.lights.iter().enumerate() {
        let kind = record.implementation.to_ascii_lowercase();
        if !matches!(kind.as_str(), "infinite" | "goniometric" | "projection") {
            continue;
        }
        let Some(filename) = ["filename", "mapname"].iter().find_map(|name| {
            record
                .parameters
                .iter()
                .find(|p| p.matches(name))?
                .texts
                .first()
                .filter(|text| !text.is_empty())
                .cloned()
        }) else {
            continue;
        };
        let asset_sha = options.asset_sha256.get(&filename);
        let stored_sha = asset_sha.filter(|sha| options.stored_blobs.contains_key(*sha));
        let loaded = if stored_sha.is_some() {
            Ok(None)
        } else {
            read_referenced_asset(&filename, record.source.as_ref(), root_dir, options).map(Some)
        };
        match loaded {
            Ok(bytes) => {
                let name = format!("light/{index}");
                let size = stored_sha
                    .and_then(|sha| options.stored_blobs.get(sha).copied())
                    .unwrap_or_else(|| bytes.as_ref().unwrap().len() as u64);
                let sha256 = asset_sha.cloned().unwrap_or_else(|| {
                    format!("{:x}", sha2::Sha256::digest(bytes.as_ref().unwrap()))
                });
                manifest.blobs.insert(
                    name.clone(),
                    protocol::package::PackageFile {
                        size,
                        // As with imagemaps, an asset
                        // source takes the model's sha256.
                        sha256,
                    },
                );
                if let Some(bytes) = bytes {
                    blobs.insert(name, bytes);
                }
            }
            Err(_) => {
                let warning = Diagnostic::warning(
                    "light_image_failed",
                    format!("{kind} light map {filename} could not be read within the budget"),
                );
                diagnostics.push(match &record.source {
                    Some(source) => warning.at(&source.path, source.line),
                    None => warning,
                });
            }
        }
    }
    if let Some(record) = camera.as_mut() {
        record.film_resolution = film_resolution;
        record.film_exposure = film_exposure;
        // Navigation orbits about `up`. The parsed up is the camera's own
        // (tilted with the view); a world axis that gives the same zero-roll
        // pose is what the scene declared (LookAt's `0 0 1`), so use it.
        if let Some([eye, target, up]) = record.look_at {
            record.look_at = Some([eye, target, world_up(sub3(target, eye), up)]);
        }
        // Choose a target on the camera's forward ray at the scene's depth.
        // This preserves the parsed pose while giving navigation/clipping a
        // useful distance even for matrix-only camera declarations. The
        // depth is that of the scene's center, not its size: a huge floor
        // must not push the pivot far past what the camera looks at.
        if let (Some([eye, target, up]), Some(bounds)) = (record.look_at, scene_bounds.as_ref()) {
            let forward = [target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]];
            let length = forward.iter().map(|v| v * v).sum::<f64>().sqrt();
            if length > 0. && length.is_finite() {
                let forward = forward.map(|v| v / length);
                let center = bounds.center();
                let depth = (0..3)
                    .map(|i| (center[i] - eye[i]) * forward[i])
                    .sum::<f64>();
                // Behind or at the eye: fall back to the scene's size.
                let distance = if depth > bounds.half_diagonal() * 1e-3 {
                    depth
                } else {
                    bounds.half_diagonal()
                }
                .max(1e-3);
                record.look_at = Some([
                    eye,
                    [
                        eye[0] + forward[0] * distance,
                        eye[1] + forward[1] * distance,
                        eye[2] + forward[2] * distance,
                    ],
                    up,
                ]);
            }
        }
    }
    manifest.camera = camera;
    Ok(PreviewGeometry {
        manifest,
        blobs,
        diagnostics,
    })
}

fn sub3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// The world axis (±X/±Y/±Z) nearest the camera's `up` when it yields the
/// same pose (no roll about `forward`); otherwise `up` unchanged.
fn world_up(forward: [f64; 3], up: [f64; 3]) -> [f64; 3] {
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let length = |a: [f64; 3]| dot(a, a).sqrt();
    let (Some(axis), true) = (
        (0..3).max_by(|&a, &b| up[a].abs().total_cmp(&up[b].abs())),
        length(forward) > 0. && length(up) > 0.,
    ) else {
        return up;
    };
    let mut snapped = [0.; 3];
    snapped[axis] = up[axis].signum();
    // Same pose: the snapped axis lies in the plane of forward and up,
    // i.e. it has no component along the camera's right.
    let right = [
        forward[1] * up[2] - forward[2] * up[1],
        forward[2] * up[0] - forward[0] * up[2],
        forward[0] * up[1] - forward[1] * up[0],
    ];
    let along_forward = dot(snapped, forward) / length(forward);
    if dot(snapped, right).abs() <= 1e-9 * length(right) && along_forward.abs() < 1. - 1e-6 {
        snapped
    } else {
        up
    }
}

/// Append the named materials a `mix` refers to (`"string materials"`),
/// recursively, so the preview can resolve them by name. Draws keep their
/// material ids: children only ever append.
fn add_mix_children(
    materials: &mut Vec<protocol::geometry::MaterialRecord>,
    named: &BTreeMap<String, protocol::geometry::MaterialRecord>,
) {
    let mut index = 0;
    while index < materials.len() {
        if materials[index].implementation == "mix" {
            let children = materials[index]
                .parameters
                .iter()
                .find(|parameter| parameter.matches("materials"))
                .map(|parameter| parameter.texts.clone())
                .unwrap_or_default();
            for child in children {
                let present = materials
                    .iter()
                    .any(|material| material.name.as_deref() == Some(child.as_str()));
                if let (false, Some(record)) = (present, named.get(&child)) {
                    materials.push(record.clone());
                }
            }
        }
        index += 1;
    }
}

/// A file a declaration names, next to that declaration or under the scene root.
fn read_referenced_asset(
    filename: &str,
    source: Option<&protocol::SourceLocation>,
    root_dir: &Path,
    options: &ParseOptions,
) -> Result<Vec<u8>, SceneParseError> {
    let path = resolve_referenced_asset(filename, source, root_dir)?;
    assets::read_asset_bytes(&path, options.max_asset_bytes)
}

pub(super) fn resolve_referenced_asset(
    filename: &str,
    source: Option<&protocol::SourceLocation>,
    root_dir: &Path,
) -> Result<PathBuf, SceneParseError> {
    let source_dir = Path::new(source.map(|s| s.path.as_str()).unwrap_or(""))
        .parent()
        .unwrap_or(Path::new("."));
    let candidates = if Path::new(filename).is_absolute() {
        vec![PathBuf::from(filename)]
    } else {
        vec![source_dir.join(filename), root_dir.join(filename)]
    };
    candidates
        .iter()
        .find(|c| c.is_file())
        .cloned()
        .ok_or_else(|| SceneParseError::unreadable(filename, "file is missing"))
}
