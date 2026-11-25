use super::*;

fn emitter_shape(
    kind: &str,
    directive: &crate::params::Directive,
    has_area_light: bool,
) -> Option<protocol::geometry::EmitterShape> {
    use protocol::geometry::EmitterShape;
    if !has_area_light {
        return None;
    }
    let parameter = |name: &str, default: f64| {
        directive
            .parameters
            .iter()
            .find(|parameter| parameter.matches(name))
            .and_then(|parameter| parameter.values.first())
            .and_then(crate::params::Value::as_float)
            .unwrap_or(default)
    };
    match kind {
        "sphere" => {
            let radius = parameter("radius", 1.);
            let zmin = parameter("zmin", -radius);
            let zmax = parameter("zmax", radius);
            let phimax = parameter("phimax", 360.);
            (zmin <= -radius && zmax >= radius && phimax >= 360.)
                .then_some(EmitterShape::Sphere { radius })
        }
        "disk" => {
            let radius = parameter("radius", 1.);
            let height = parameter("height", 0.);
            let inner_radius = parameter("innerradius", 0.);
            let phimax = parameter("phimax", 360.);
            (inner_radius == 0. && phimax >= 360.).then_some(EmitterShape::Disk { radius, height })
        }
        _ => None,
    }
}

pub(super) fn build(
    shapes: Vec<Shape>,
    instances: &[InstanceUse],
    camera: &Option<CameraRecord>,
    root_dir: &Path,
    options: &ParseOptions,
    cache: &mut GeometryCache,
    diagnostics: &mut Vec<Diagnostic>,
    entity_of: Option<&BTreeMap<u32, protocol::EntityId>>,
) -> Result<(GeometryManifest, BTreeMap<String, Vec<u8>>, Option<Bounds>), SceneParseError> {
    let mut instance_uses = BTreeMap::<&str, Vec<&InstanceUse>>::new();
    for instance in instances {
        instance_uses
            .entry(&instance.name)
            .or_default()
            .push(instance);
    }
    let mut manifest = GeometryManifest::new();
    let mut blobs = BTreeMap::new();
    let shutter = camera.as_ref().map_or([0.0, 1.0], |record| record.shutter);
    let has_animation = shapes
        .iter()
        .any(|shape| shape.start_transform != shape.end_transform)
        || instances
            .iter()
            .any(|instance| instance.start_transform != instance.end_transform);
    manifest.supported_times = if has_animation && shutter[0].is_finite() && shutter[1] > shutter[0]
    {
        (0..=60)
            .map(|index| shutter[0] + (shutter[1] - shutter[0]) * index as f64 / 60.0)
            .collect()
    } else if has_animation && shutter[0].is_finite() {
        vec![shutter[0]]
    } else {
        vec![0.0]
    };
    let mut scene_bounds: Option<Bounds> = None;
    let mut geometry_bytes = 0usize;
    for shape in shapes {
        let kind = shape.d.positional_text(0).unwrap_or("unknown").to_string();
        // Unchanged shape directives (with their asset contents) reuse the
        // tessellation from the previous parse; only the placement depends
        // on the surrounding transform state and is rebuilt either way.
        // A plymesh with a model sha256 forms its
        // cache key without opening the file, and its bytes load —
        // verified against the model — only if the cache misses.
        let mut ply = if kind == "plymesh" {
            let filename = shape.d.params().string("filename", "");
            match options.asset_sha256.get(&filename).cloned() {
                Some(model_sha) => PlyAsset::Model(model_sha),
                None => {
                    let path = shape_geometry::resolve_ply_asset(&shape.d, root_dir);
                    match load_ply_asset(&path, options) {
                        Ok((hash, bytes)) => {
                            cache.note_asset_read();
                            PlyAsset::Loaded(hash, bytes)
                        }
                        Err(error) => {
                            diagnostics.push(
                                Diagnostic::warning(
                                    "geometry_failed",
                                    asset_error_message(&error, &path),
                                )
                                .at(&shape.d.path, shape.d.line)
                                .for_shape(kind.as_str()),
                            );
                            continue;
                        }
                    }
                }
            }
        } else {
            PlyAsset::NotPly
        };
        let key = cache_key(
            &shape.d,
            &kind,
            ply.sha(),
            matches!(ply, PlyAsset::Model(_)),
        );
        let cached = cache.get(&key, &options.stored_blobs);
        let placements = match &shape.definition {
            None => placement_transforms(&shape, None, &manifest.supported_times)
                .map(|transforms| vec![(transforms, None)]),
            Some(name) => instance_uses
                .get(name.as_str())
                .into_iter()
                .flat_map(|uses| uses.iter())
                .map(|instance| {
                    placement_transforms(&shape, Some(instance), &manifest.supported_times).map(
                        |transforms| {
                            (
                                transforms,
                                Some(InstanceSource {
                                    name: instance.name.clone(),
                                    source: instance.source.clone(),
                                }),
                            )
                        },
                    )
                })
                .collect::<Option<Vec<_>>>(),
        };
        let Some(placements) = placements else {
            diagnostics.push(
                Diagnostic::warning(
                    "animated_transform_unsupported",
                    "animated transform could not be decomposed into translation, rotation and scale",
                )
                .at(&shape.d.path, shape.d.line)
                .for_shape(&kind),
            );
            continue;
        };
        if placements.is_empty() {
            continue;
        }
        enum Geometry {
            Fresh(Box<(Vec<[f64; 3]>, Vec<u32>)>),
            Cached(Box<CachedGeometry>),
        }
        let mut shape_uvs: Option<Vec<[f32; 2]>> = None;
        let geometry = match cached {
            Some(entry) => Geometry::Cached(Box::new(entry)),
            None => {
                // The bytes of a model-asserted PLY load
                // only on the miss that tessellates them, and must hash
                // back to the model's sha256 when an asset changes.
                let ply_bytes = match &mut ply {
                    PlyAsset::NotPly => None,
                    PlyAsset::Loaded(_, bytes) => Some(std::mem::take(bytes)),
                    PlyAsset::Model(expected) => {
                        let path = shape_geometry::resolve_ply_asset(&shape.d, root_dir);
                        match assets::read_ply_bytes(&path, options.max_asset_bytes) {
                            Ok(bytes) => {
                                cache.note_asset_read();
                                let hashed = format!("{:x}", sha2::Sha256::digest(&bytes));
                                if hashed != expected.as_str() {
                                    return Err(SceneParseError::AssetChanged);
                                }
                                Some(bytes)
                            }
                            Err(error) => {
                                diagnostics.push(
                                    Diagnostic::warning(
                                        "geometry_failed",
                                        asset_error_message(&error, &path),
                                    )
                                    .at(&shape.d.path, shape.d.line)
                                    .for_shape(kind.as_str()),
                                );
                                continue;
                            }
                        }
                    }
                };
                let Some(result) = shape_geometry::build(
                    &shape,
                    &kind,
                    root_dir,
                    options,
                    ply_bytes.as_deref(),
                    &mut shape_uvs,
                ) else {
                    diagnostics.push(
                        Diagnostic::warning(
                            "unsupported_shape",
                            format!("shape {kind} is parsed but has no preview tessellator"),
                        )
                        .at(&shape.d.path, shape.d.line)
                        .for_shape(&kind),
                    );
                    // The package reader joins unsupported entries by source location in a
                    // later phase. Keep the parser diagnostic authoritative in the meantime.
                    continue;
                };
                match result {
                    Ok(v) => Geometry::Fresh(Box::new(v)),
                    Err(e) => {
                        // A project preview reads PLYs from a temporary
                        // staging bundle; keep the file name only, so the
                        // warning stays readable — and dedupable across the
                        // preview that runs after every edit.
                        let message = if kind == "plymesh" {
                            asset_error_message(
                                &e,
                                Path::new(&shape.d.params().string("filename", "")),
                            )
                        } else {
                            e.to_string()
                        };
                        diagnostics.push(
                            Diagnostic::warning("geometry_failed", message)
                                .at(&shape.d.path, shape.d.line)
                                .for_shape(&kind),
                        );
                        continue;
                    }
                }
            }
        };
        let (points, indices, _uvs, normals, blob, po, no, io, uo, pull, blob_sha256) =
            match geometry {
                Geometry::Cached(entry) => (
                    entry.points,
                    entry.indices,
                    None,
                    entry.normals,
                    entry.blob,
                    entry.positions,
                    entry.normals_ref,
                    entry.indices_ref,
                    entry.uv_ref,
                    true,
                    entry.blob_sha256,
                ),
                Geometry::Fresh(boxed) => {
                    let (points, indices) = *boxed;
                    let origin = points.first().copied().unwrap_or([0.; 3]);
                    let positions: Vec<[f32; 3]> = points
                        .iter()
                        .map(|p| {
                            [
                                (p[0] - origin[0]) as f32,
                                (p[1] - origin[1]) as f32,
                                (p[2] - origin[2]) as f32,
                            ]
                        })
                        .collect();
                    if positions.iter().flatten().any(|v| !v.is_finite()) {
                        diagnostics.push(
                            Diagnostic::warning(
                                "geometry_failed",
                                "vertex exceeds finite f32 preview range",
                            )
                            .at(&shape.d.path, shape.d.line),
                        );
                        continue;
                    }
                    let mut blob = blob_header().to_vec();
                    let pb = positions_bytes(&positions);
                    let po = append_aligned_array(&mut blob, &pb);
                    let normals = mesh_build::vertex_normals(&points, &indices);
                    let nb = protocol::geometry::positions_bytes(&normals);
                    let no = append_aligned_array(&mut blob, &nb);
                    let uo = shape_uvs.as_ref().map(|uvs| {
                        let bytes = protocol::geometry::uv_bytes(uvs);
                        append_aligned_array(&mut blob, &bytes)
                    });
                    let ib: Vec<u8> = indices.iter().flat_map(|i| i.to_le_bytes()).collect();
                    let io = append_aligned_array(&mut blob, &ib);
                    let (position_count, normal_count, index_count) = (
                        positions.len() as u64,
                        normals.len() as u64,
                        indices.len() as u64,
                    );
                    // The blob's sha256 is computed once
                    // here, shared by the cache entry and the manifest.
                    let blob_sha256 = format!("{:x}", sha2::Sha256::digest(&blob));
                    (
                        points,
                        indices,
                        shape_uvs,
                        normals,
                        blob,
                        ArrayRef {
                            offset: po,
                            count: position_count,
                            stride: POSITION_STRIDE,
                        },
                        ArrayRef {
                            offset: no,
                            count: normal_count,
                            stride: NORMAL_STRIDE,
                        },
                        ArrayRef {
                            offset: io,
                            count: index_count,
                            stride: INDEX_STRIDE,
                        },
                        uo.map(|offset| ArrayRef {
                            offset,
                            count: position_count,
                            stride: UV_STRIDE,
                        }),
                        false,
                        blob_sha256,
                    )
                }
            };
        let blob_size = options
            .stored_blobs
            .get(&blob_sha256)
            .copied()
            .unwrap_or(blob.len() as u64) as usize;
        let Some(next_geometry_bytes) = geometry_bytes.checked_add(blob_size) else {
            return Err(SceneParseError::ResourceExhausted {
                message: "preview geometry byte count overflowed".into(),
            });
        };
        if next_geometry_bytes > options.max_geometry_bytes {
            diagnostics.push(
                Diagnostic::warning(
                    "preview_geometry_limit",
                    format!(
                        "preview geometry exceeded the {} byte budget; remaining shapes were skipped",
                        options.max_geometry_bytes
                    ),
                )
                .at(&shape.d.path, shape.d.line)
                .for_shape(&kind),
            );
            break;
        }
        geometry_bytes = next_geometry_bytes;
        let origin = points.first().copied().unwrap_or([0.; 3]);
        if !pull {
            cache.put(
                key,
                CachedGeometry {
                    points: points.clone(),
                    normals: normals.clone(),
                    indices: indices.clone(),
                    blob: blob.clone(),
                    blob_sha256: blob_sha256.clone(),
                    positions: po.clone(),
                    normals_ref: no.clone(),
                    indices_ref: io.clone(),
                    uv_ref: uo.clone(),
                },
            );
        }
        // With an origin table the mesh id derives from the shape's
        // originating node: node insertion no longer shifts
        // every downstream mesh id. Unresolved shapes keep the ordinal.
        let mesh_id = entity_of
            .and_then(|table| table.get(&shape.d.line))
            .map(|entity| stable_mesh_id(entity))
            .unwrap_or_else(|| manifest.meshes.len() as u64);
        let filename = format!("geometry/{mesh_id:08}.bin");
        let mut local = Bounds {
            min: points[0],
            max: points[0],
        };
        for p in &points {
            for a in 0..3 {
                local.min[a] = local.min[a].min(p[a]);
                local.max[a] = local.max[a].max(p[a]);
            }
        }
        let mut mesh_bounds: Option<Bounds> = None;
        for (transforms, instance_source) in placements {
            let mut world_bounds: Option<Bounds> = None;
            for transform in &transforms {
                let world = transformed_bounds(&local, *transform);
                scene_bounds =
                    Some(scene_bounds.map_or(world.clone(), |bounds| bounds.union(&world)));
                world_bounds =
                    Some(world_bounds.map_or(world.clone(), |bounds| bounds.union(&world)));
            }
            let world = world_bounds.unwrap();
            mesh_bounds = Some(mesh_bounds.map_or(world.clone(), |bounds| bounds.union(&world)));
            manifest.draws.push(DrawRecord {
                mesh_id,
                shape_index: shape.index,
                source: Some(SourceLocation {
                    path: shape.d.path.clone(),
                    line: shape.d.line,
                    column: shape.d.column,
                }),
                shape_type: kind.clone(),
                pick_id: manifest.draws.len() as u32 + 1,
                entity_id: entity_of.and_then(|table| {
                    instance_source
                        .as_ref()
                        .map(|instance| instance.source.line)
                        .or(Some(shape.d.line))
                        .and_then(|line| table.get(&line).cloned())
                }),
                instance_use_id: None,
                instance_source,
                bounds: Some(world),
                world_from_object: transforms
                    .into_iter()
                    .map(|transform| {
                        transform
                            .mul(Mat4::translation(origin[0], origin[1], origin[2]))
                            .0
                    })
                    .collect(),
                reverse_orientation: shape.reverse,
                material_id: shape
                    .material
                    .as_ref()
                    .map(|material| intern_material(&mut manifest.materials, material)),
                interface: shape.interface_material,
                area_light: shape.area_light.clone(),
                emitter_shape: emitter_shape(&kind, &shape.d, shape.area_light.is_some()),
                alpha: shape.alpha.clone(),
            });
        }
        manifest.meshes.push(MeshRecord {
            mesh_id,
            blob: filename.clone(),
            positions: po,
            normals: Some(no),
            uv: uo,
            indices: io,
            origin_f64: origin,
            bounds: mesh_bounds.unwrap(),
        });
        manifest.blobs.insert(
            filename.clone(),
            protocol::package::PackageFile {
                size: blob_size as u64,
                // Cached entries carry the blob's sha256
                // from put time instead of re-hashing it here.
                sha256: blob_sha256.clone(),
            },
        );
        if !options.stored_blobs.contains_key(&blob_sha256) {
            blobs.insert(filename, blob);
        }
    }
    Ok((manifest, blobs, scene_bounds))
}

/// Stable mesh key from the shape's originating node: SHA-256 of the entity
/// id, upper 64 bits.
fn stable_mesh_id(entity: &protocol::EntityId) -> u64 {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(entity.0.as_bytes());
    u64::from_be_bytes(digest[..8].try_into().expect("8 bytes"))
}

/// Content key of one shape's tessellation input: the directive's parsed
/// tokens plus the SHA-256 of a referenced PLY asset. Placement transforms are
/// deliberately excluded so transform edits reuse meshes. A model-asserted
/// PLY is content-addressed by the model's sha256 alone:
/// the filename path does not feed the tessellation and legitimately
/// changes between staging flavors and payload leases, so it must not
/// change the key. Files read the historical way keep hashing it.
fn cache_key(
    directive: &crate::params::Directive,
    kind: &str,
    asset: Option<&str>,
    asset_from_model: bool,
) -> [u8; 32] {
    use sha2::Digest;
    let mut hash = sha2::Sha256::new();
    // Bump when a tessellation's output changes: this one added analytic UVs
    // for paraboloid, hyperboloid, bilinearmesh, heightfield and nurbs.
    hash.update(b"geometry-3\0");
    hash.update(kind.as_bytes());
    hash.update([0]);
    for positional in &directive.positional {
        hash.update(format!("{positional:?}").as_bytes());
        hash.update([0]);
    }
    for parameter in &directive.parameters {
        if asset_from_model && kind == "plymesh" && parameter.matches("filename") {
            continue;
        }
        hash.update(parameter.value_type.as_bytes());
        hash.update([0]);
        hash.update(parameter.name.as_bytes());
        hash.update([0]);
        for token in &parameter.value_tokens {
            hash.update(token.as_bytes());
            hash.update([0x1f]);
        }
        hash.update([0x1e]);
    }
    if let Some(asset) = asset {
        hash.update(asset.as_bytes());
    }
    hash.finalize().into()
}

/// A plymesh's asset, split into model-asserted and file-read cases: the model's asserted
/// sha256 is [`PlyAsset::Model`]'s table case of this loader — it forms
/// the tessellation cache key without reading the file, and its bytes
/// load only on the miss that tessellates, verified against the model
/// once on the way in. Files no model knows keep the historical eager
/// read-and-hash ([`PlyAsset::Loaded`]).
enum PlyAsset {
    NotPly,
    Model(String),
    Loaded(String, Vec<u8>),
}

impl PlyAsset {
    /// The cache key's asset component: the model's sha256 when known,
    /// else the digest of the bytes actually read.
    fn sha(&self) -> Option<&str> {
        match self {
            PlyAsset::NotPly => None,
            PlyAsset::Model(sha) | PlyAsset::Loaded(sha, _) => Some(sha),
        }
    }
}

fn load_ply_asset(
    path: &Path,
    options: &ParseOptions,
) -> Result<(String, Vec<u8>), SceneParseError> {
    let bytes = assets::read_ply_bytes(path, options.max_asset_bytes)?;
    Ok((format!("{:x}", sha2::Sha256::digest(&bytes)), bytes))
}

/// A PLY failure inside a project preview names a file inside a temporary
/// staging bundle (already gone when anyone reads the warning); the
/// geometry_failed message keeps the file name only.
fn asset_error_message(error: &SceneParseError, path: &Path) -> String {
    let full = path.display().to_string();
    let message = error.to_string();
    match path.file_name().and_then(|name| name.to_str()) {
        Some(name) if name != full => message.replace(&full, name),
        _ => message,
    }
}

/// Materials are interned so identical definitions share one record id; the
/// draw count and the material table stay small when transferred in a package.
fn intern_material(materials: &mut Vec<MaterialRecord>, material: &MaterialRecord) -> u64 {
    if let Some(index) = materials.iter().position(|existing| existing == material) {
        return index as u64;
    }
    materials.push(material.clone());
    (materials.len() - 1) as u64
}
