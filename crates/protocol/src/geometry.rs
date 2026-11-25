//! GPU-facing geometry package DTOs.
//!
//! The worker writes one `geometry.json` plus binary blob files into the scene
//! package; `scene-view` reads them. Everything here is plain serde DTOs plus
//! byte-level helpers so the writer and the reader agree on the binary layout
//! without either depending on r4 or wgpu.

use serde::{Deserialize, Serialize};

use crate::ids::EntityId;
use crate::package::{PackageFile, SourceLocation};

/// First 8 bytes of every geometry blob: ASCII `PBRTUI01`.
pub const GEOMETRY_BLOB_MAGIC: [u8; 8] = *b"PBRTUI01";
/// u32 written after the magic.
pub const GEOMETRY_BLOB_VERSION: u32 = 1;
/// `geometry.json` manifest version. Version 2 adds the appearance records
/// (materials, per-draw materials/area lights, lights) for the lit preview;
/// version 1 manifests remain readable and simply carry no appearance data.
pub const GEOMETRY_MANIFEST_VERSION: u32 = 2;
/// Transfer limits for appearance parameters: the preview
/// renderer consumes compact shading values, not full r4 parameter payloads.
pub const MAX_PREVIEW_PARAMETERS: usize = 32;
pub const MAX_PREVIEW_PARAMETER_VALUES: usize = 64;
pub const MAX_PREVIEW_PARAMETER_TEXT_CHARS: usize = 256;
/// u32 written after the version; always zero.
pub const GEOMETRY_BLOB_RESERVED: u32 = 0;
/// Header length, and the alignment every array offset must satisfy.
pub const GEOMETRY_BLOB_HEADER_BYTES: usize = 16;
pub const GEOMETRY_ARRAY_ALIGNMENT: u64 = 16;

/// Strides in bytes for the position, normal, UV, and index arrays.
pub const POSITION_STRIDE: u32 = 12;
pub const NORMAL_STRIDE: u32 = 12;
pub const UV_STRIDE: u32 = 8;
pub const INDEX_STRIDE: u32 = 4;

/// A mesh record names a blob and where each array lives inside it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArrayRef {
    pub offset: u64,
    pub count: u64,
    pub stride: u32,
}

impl ArrayRef {
    /// Last byte (exclusive) of this array inside the blob.
    pub fn end(&self) -> u64 {
        self.offset
            .saturating_add(self.count.saturating_mul(self.stride as u64))
    }
}

/// Axis-aligned bounds in world space, kept in f64 so large
/// coordinates straight into f32).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Bounds {
    pub fn empty() -> Self {
        Self {
            min: [0.0; 3],
            max: [0.0; 3],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.min == self.max
    }

    pub fn center(&self) -> [f64; 3] {
        [
            (self.min[0] + self.max[0]) / 2.0,
            (self.min[1] + self.max[1]) / 2.0,
            (self.min[2] + self.max[2]) / 2.0,
        ]
    }

    /// Longest diagonal half-length; zero for a point-like bounds.
    pub fn half_diagonal(&self) -> f64 {
        let mut sum = 0.0;
        for axis in 0..3 {
            let extent = (self.max[axis] - self.min[axis]) / 2.0;
            sum += extent * extent;
        }
        sum.sqrt()
    }

    pub fn union(&self, other: &Bounds) -> Bounds {
        let mut out = self.clone();
        for axis in 0..3 {
            out.min[axis] = out.min[axis].min(other.min[axis]);
            out.max[axis] = out.max[axis].max(other.max[axis]);
        }
        out
    }
}

/// One mesh in the package. Attributes that a shape does not provide are
/// `None`, not an empty array.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeshRecord {
    pub mesh_id: u64,
    pub blob: String,
    pub positions: ArrayRef,
    #[serde(default)]
    pub normals: Option<ArrayRef>,
    #[serde(default)]
    pub uv: Option<ArrayRef>,
    pub indices: ArrayRef,
    /// Origin subtracted from the f32 positions so large world coordinates keep
    /// precision. Draw transforms are expressed relative to this origin.
    pub origin_f64: [f64; 3],
    /// World-space union of this mesh's placements at the first supported time.
    pub bounds: Bounds,
}

/// One drawable instance of a mesh. `pick_id` 0 is background and never assigned.
///
/// The preview parser cannot know the entity ids the UI metadata package will
/// issue, so a draw carries its *source location* instead. `core` resolves
/// `entity_id` / `instance_use_id` by matching that location against
/// `entities.jsonl` and reports unresolved draws as diagnostics.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrawRecord {
    pub mesh_id: u64,
    /// Ordinal among the shapes the preview parser read, in file order.
    pub shape_index: usize,
    /// Where the `Shape` directive (or the `Instance` use) appeared.
    pub source: Option<SourceLocation>,
    pub shape_type: String,
    pub pick_id: u32,
    /// Resolved by core; empty in the package the parser writes.
    #[serde(default)]
    pub entity_id: Option<EntityId>,
    /// Set when the draw comes from an instance use rather than a direct shape.
    #[serde(default)]
    pub instance_use_id: Option<EntityId>,
    /// Source occurrence used to bind a placed instance independently of its shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_source: Option<InstanceSource>,
    /// Bounds of this placement; old packages may only provide mesh bounds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<Bounds>,
    /// Row-major 4x4 `world_from_object`, already including the mesh's
    /// `origin_f64` translation so the GPU only multiplies by this matrix.
    /// Either one matrix (applies at every supported time) or exactly one per
    /// entry of `GeometryManifest::supported_times`.
    pub world_from_object: Vec<[f64; 16]>,
    pub reverse_orientation: bool,
    /// Index into `GeometryManifest::materials` for the material state that was
    /// active at this `Shape` directive. `None` means no declared material.
    #[serde(default)]
    pub material_id: Option<u64>,
    /// PBRT's explicit interface binding has no visible surface. `false` is
    /// omitted so manifests written before this field remain unchanged.
    #[serde(default, skip_serializing_if = "is_false")]
    pub interface: bool,
    /// An `AreaLightSource` attached to this shape; emissive in the lit preview.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area_light: Option<AreaLightRecord>,
    /// Analytic object-space shape for a full sphere or disk area emitter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emitter_shape: Option<EmitterShape>,
    /// Shape alpha input, carried independently from the material for later
    /// cutout evaluation. Older packages omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha: Option<PreviewParameter>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EmitterShape {
    /// Full sphere in object space, centered at the origin.
    Sphere { radius: f64 },
    /// Full disk at z = height facing +z, with no inner radius.
    Disk { radius: f64, height: f64 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceSource {
    pub name: String,
    pub source: SourceLocation,
}

/// One compact appearance parameter: the parser keeps shading-relevant values
/// only shading-relevant values. Mixed payloads land in `floats` / `texts` in source order;
/// values dropped by the transfer limits set `truncated`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewParameter {
    pub name: String,
    pub value_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub floats: Vec<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub texts: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl PreviewParameter {
    pub fn float(&self, name: &str) -> Option<f64> {
        if !self.matches(name) {
            return None;
        }
        self.floats.first().copied()
    }

    pub fn matches(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name)
    }
}

/// A material as the preview parser saw it. `resource_id` is set when the
/// material name follows the project export convention `resource_{id}`, which
/// joins the draw directly to a project ResourceId.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialRecord {
    pub implementation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<EntityId>,
    /// The declared named-material name, when the draw bound one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceLocation>,
    pub parameters: Vec<PreviewParameter>,
}

/// An `AreaLightSource` declaration attached to a draw.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AreaLightRecord {
    pub implementation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceLocation>,
    pub parameters: Vec<PreviewParameter>,
}

/// A `Texture` declaration. `imagemap` textures carry their raw image bytes in
/// the package blobs as `texture/{value_type}/{name}`. Other implementations
/// carry their parameter graph for the Lit texture compiler.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextureRecord {
    pub name: String,
    /// pbrt's separate float and spectrum texture namespaces.
    #[serde(default = "default_texture_value_type")]
    pub value_type: String,
    pub implementation: String,
    /// Transform active at the declaration. Kept in f64 to preserve large
    /// translations until texture coordinates are made eye-relative.
    #[serde(default = "identity16")]
    pub world_from_texture: [f64; 16],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceLocation>,
    pub parameters: Vec<PreviewParameter>,
}

fn default_texture_value_type() -> String {
    "spectrum".to_owned()
}

/// A `LightSource` declaration in world space; positions arrive through the
/// parameters (`from` / `to`), which is how r4 declares analytic lights.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightRecord {
    pub implementation: String,
    /// Row-major transform active when the light was declared.
    #[serde(default = "identity16")]
    pub world_from_light: [f64; 16],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceLocation>,
    /// Resolved by scene-parse from the program's origin table, the same
    /// join the draws use: `project.preview_transforms`
    /// keys a moved light's delta by it. Text parses carry no origin table
    /// and leave it unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<EntityId>,
    pub parameters: Vec<PreviewParameter>,
}

impl DrawRecord {
    /// Matrix for one supported time. `None` means the draw is unavailable at
    /// that time; matrices are never interpolated.
    pub fn matrix_at(&self, time_index: usize) -> Option<[f64; 16]> {
        match self.world_from_object.len() {
            1 => Some(self.world_from_object[0]),
            len if time_index < len => Some(self.world_from_object[time_index]),
            _ => None,
        }
    }

    pub fn is_animated(&self) -> bool {
        self.world_from_object.len() > 1
    }
}

/// The scene's own camera, used to initialize the viewer camera. Values
/// are kept in world space; the viewer may re-frame them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraRecord {
    pub kind: String,
    /// Row-major camera-to-world from `LookAt` or the current transform.
    pub camera_from_world: [f64; 16],
    #[serde(default)]
    pub look_at: Option<[[f64; 3]; 3]>,
    /// Field of view in degrees along the film's shorter axis (r4 binds the
    /// perspective `fov` to the ±1 screen axis), so portrait films record a
    /// horizontal angle.
    #[serde(default)]
    pub fov_degrees: Option<f64>,
    /// left, right, bottom, top.
    #[serde(default)]
    pub screenwindow: Option<[f64; 4]>,
    /// Explicit `frameaspectratio`; absent means the film's own aspect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_aspect_ratio: Option<f64>,
    pub shutter: [f64; 2],
    #[serde(default)]
    pub film_resolution: Option<[u32; 2]>,
    /// Film scale multiplied by nonnegative ISO / 100; absent means 1.
    #[serde(default)]
    pub film_exposure: Option<f64>,
}

fn identity16() -> [f64; 16] {
    [
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ]
}

/// Display-only tessellation quality. It is not fed back into r4 numerics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TessellationSettings {
    pub angular_segments: u32,
    pub bilinear_segments: u32,
    pub curve_segments: u32,
    pub tube_sides: u32,
}

impl Default for TessellationSettings {
    fn default() -> Self {
        Self {
            angular_segments: 64,
            bilinear_segments: 8,
            curve_segments: 16,
            tube_sides: 8,
        }
    }
}

/// A shape the exporter could not turn into preview geometry. This forbids
/// silently substituting a sphere or an empty mesh.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnsupportedRecord {
    pub entity_id: EntityId,
    pub shape_type: String,
    pub reason: String,
}

/// Contents of `geometry.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeometryManifest {
    pub version: u32,
    pub tessellation: TessellationSettings,
    /// Times available for animated draws, always including start / mid / end.
    pub supported_times: Vec<f64>,
    /// World bounds over all draws at the first supported time.
    pub bounds: Option<Bounds>,
    pub meshes: Vec<MeshRecord>,
    pub draws: Vec<DrawRecord>,
    pub unsupported: Vec<UnsupportedRecord>,
    /// The scene camera, when the parser found one, for initializing the viewer.
    #[serde(default)]
    pub camera: Option<CameraRecord>,
    /// Blob files carried by the package manifest, keyed by the name used in
    /// `MeshRecord::blob`.
    pub blobs: std::collections::BTreeMap<String, PackageFile>,
    /// Materials referenced by `DrawRecord::material_id` (index = id). Absent
    /// in version 1 manifests.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<MaterialRecord>,
    /// `LightSource` declarations the preview parser read. Absent in version 1.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lights: Vec<LightRecord>,
    /// `Texture` declarations the preview parser read; imagemap payloads ride
    /// along in `blobs` as `texture/{name}`. Absent in versions before this.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub textures: Vec<TextureRecord>,
}

impl GeometryManifest {
    pub fn new() -> Self {
        Self {
            version: GEOMETRY_MANIFEST_VERSION,
            tessellation: TessellationSettings::default(),
            supported_times: Vec::new(),
            bounds: None,
            meshes: Vec::new(),
            draws: Vec::new(),
            unsupported: Vec::new(),
            camera: None,
            blobs: std::collections::BTreeMap::new(),
            materials: Vec::new(),
            lights: Vec::new(),
            textures: Vec::new(),
        }
    }

    /// Time index for a requested time, or `None` when it does not exactly
    /// match an exported sample.
    pub fn time_index(&self, time: f64) -> Option<usize> {
        self.supported_times
            .iter()
            .position(|sample| *sample == time)
    }
}

impl Default for GeometryManifest {
    fn default() -> Self {
        Self::new()
    }
}

/// The 16-byte header every geometry blob starts with.
pub fn blob_header() -> [u8; GEOMETRY_BLOB_HEADER_BYTES] {
    let mut header = [0u8; GEOMETRY_BLOB_HEADER_BYTES];
    header[..8].copy_from_slice(&GEOMETRY_BLOB_MAGIC);
    header[8..12].copy_from_slice(&GEOMETRY_BLOB_VERSION.to_le_bytes());
    header[12..16].copy_from_slice(&GEOMETRY_BLOB_RESERVED.to_le_bytes());
    header
}

/// Validate a blob header. Returns the declared format version.
pub fn check_blob_header(bytes: &[u8]) -> Result<u32, String> {
    if bytes.len() < GEOMETRY_BLOB_HEADER_BYTES {
        return Err("geometry blob is shorter than its header".to_string());
    }
    if bytes[..8] != GEOMETRY_BLOB_MAGIC {
        return Err("geometry blob magic is not PBRTUI01".to_string());
    }
    let version = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    if version != GEOMETRY_BLOB_VERSION {
        return Err(format!("unsupported geometry blob version {version}"));
    }
    let reserved = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    if reserved != GEOMETRY_BLOB_RESERVED {
        return Err(format!(
            "geometry blob reserved field is {reserved}, expected 0"
        ));
    }
    Ok(version)
}

/// Append `bytes` to a blob, padding so the appended array starts on a 16-byte
/// boundary. Returns the offset the array starts at.
pub fn append_aligned_array(blob: &mut Vec<u8>, bytes: &[u8]) -> u64 {
    let padding = (GEOMETRY_ARRAY_ALIGNMENT - (blob.len() as u64 % GEOMETRY_ARRAY_ALIGNMENT))
        % GEOMETRY_ARRAY_ALIGNMENT;
    blob.resize(blob.len() + padding as usize, 0);
    let offset = blob.len() as u64;
    blob.extend_from_slice(bytes);
    offset
}

/// Little-endian f32x3 array bytes.
pub fn positions_bytes(values: &[[f32; 3]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * POSITION_STRIDE as usize);
    for value in values {
        for component in value {
            out.extend_from_slice(&component.to_le_bytes());
        }
    }
    out
}

/// Little-endian f32x2 array bytes.
pub fn uv_bytes(values: &[[f32; 2]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * UV_STRIDE as usize);
    for value in values {
        for component in value {
            out.extend_from_slice(&component.to_le_bytes());
        }
    }
    out
}

/// Little-endian u32 index array bytes.
pub fn indices_bytes(values: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * INDEX_STRIDE as usize);
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

/// Read one array out of a blob using its record. The reader uses this instead
/// of trusting offsets blindly.
pub fn read_f32x3_array(blob: &[u8], array: &ArrayRef) -> Result<Vec<[f32; 3]>, String> {
    if array.stride != POSITION_STRIDE {
        return Err(format!(
            "positions stride {} is not {POSITION_STRIDE}",
            array.stride
        ));
    }
    let bytes = array_bytes(blob, array)?;
    let mut out = Vec::with_capacity(array.count as usize);
    for chunk in bytes.chunks_exact(POSITION_STRIDE as usize) {
        out.push([
            f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]),
            f32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]),
            f32::from_le_bytes([chunk[8], chunk[9], chunk[10], chunk[11]]),
        ]);
    }
    Ok(out)
}

/// Read the u32 index array out of a blob.
pub fn read_indices(blob: &[u8], array: &ArrayRef) -> Result<Vec<u32>, String> {
    if array.stride != INDEX_STRIDE {
        return Err(format!(
            "indices stride {} is not {INDEX_STRIDE}",
            array.stride
        ));
    }
    let bytes = array_bytes(blob, array)?;
    let mut out = Vec::with_capacity(array.count as usize);
    for chunk in bytes.chunks_exact(INDEX_STRIDE as usize) {
        out.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Ok(out)
}

fn array_bytes<'a>(blob: &'a [u8], array: &ArrayRef) -> Result<&'a [u8], String> {
    let end = array.end();
    let start = usize::try_from(array.offset).map_err(|_| "array offset is too large")?;
    let end = usize::try_from(end).map_err(|_| "array length is too large")?;
    if array.offset % GEOMETRY_ARRAY_ALIGNMENT != 0 {
        return Err(format!(
            "array offset {} is not {GEOMETRY_ARRAY_ALIGNMENT}-byte aligned",
            array.offset
        ));
    }
    if end > blob.len() {
        return Err(format!(
            "array at {}..{end} exceeds blob length {}",
            array.offset,
            blob.len()
        ));
    }
    Ok(&blob[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_header_roundtrips_and_rejects_bad_magic() {
        let header = blob_header();
        assert_eq!(&header[..8], b"PBRTUI01");
        assert_eq!(check_blob_header(&header), Ok(1));

        let mut broken = header;
        broken[0] = b'X';
        assert!(check_blob_header(&broken).is_err());

        let mut bad_version = blob_header().to_vec();
        bad_version[8..12].copy_from_slice(&2u32.to_le_bytes());
        assert!(check_blob_header(&bad_version).is_err());

        assert!(check_blob_header(&header[..12]).is_err());
    }

    #[test]
    fn arrays_start_on_16_byte_boundaries() {
        let mut blob = blob_header().to_vec();
        let positions = append_aligned_array(&mut blob, &positions_bytes(&[[1.0, 2.0, 3.0]]));
        let indices = append_aligned_array(&mut blob, &indices_bytes(&[0]));
        assert_eq!(positions, 16);
        assert_eq!(indices % 16, 0);
        // 12 bytes of positions do not fill a 16-byte block, so the next array
        // starts at the following boundary rather than at offset 28.
        assert_eq!(indices, 32);
        assert!(indices >= positions + POSITION_STRIDE as u64);
    }

    #[test]
    fn arrays_read_back_little_endian() {
        let mut blob = blob_header().to_vec();
        let offset = append_aligned_array(&mut blob, &positions_bytes(&[[0.5, -1.5, 2.0]]));
        let index_offset = append_aligned_array(&mut blob, &indices_bytes(&[7, 8]));
        let positions = read_f32x3_array(
            &blob,
            &ArrayRef {
                offset,
                count: 1,
                stride: POSITION_STRIDE,
            },
        )
        .unwrap();
        let indices = read_indices(
            &blob,
            &ArrayRef {
                offset: index_offset,
                count: 2,
                stride: INDEX_STRIDE,
            },
        )
        .unwrap();
        assert_eq!(positions, vec![[0.5, -1.5, 2.0]]);
        assert_eq!(indices, vec![7, 8]);
    }

    #[test]
    fn reader_rejects_out_of_range_and_misaligned_arrays() {
        let blob = blob_header().to_vec();
        let error = read_indices(
            &blob,
            &ArrayRef {
                offset: 16,
                count: 4,
                stride: INDEX_STRIDE,
            },
        )
        .unwrap_err();
        assert!(error.contains("exceeds blob length"));

        let mut blob = blob_header().to_vec();
        let offset = append_aligned_array(&mut blob, &indices_bytes(&[1]));
        let error = read_indices(
            &blob,
            &ArrayRef {
                offset: offset + 4,
                count: 1,
                stride: INDEX_STRIDE,
            },
        )
        .unwrap_err();
        assert!(error.contains("aligned"));
    }

    #[test]
    fn supported_time_lookup_is_exact_not_rounded() {
        let manifest = GeometryManifest {
            supported_times: vec![0.0, 0.5, 1.0],
            ..GeometryManifest::new()
        };
        assert_eq!(manifest.time_index(0.5), Some(1));
        assert_eq!(manifest.time_index(0.4999), None);
    }

    #[test]
    fn bounds_union_keeps_f64_extent() {
        let a = Bounds {
            min: [-1.0, 0.0, 2.0],
            max: [1.0, 3.0, 4.0],
        };
        let b = Bounds {
            min: [0.0, -2.0, 2.5],
            max: [5.0, 1.0, 9.0],
        };
        let joined = a.union(&b);
        assert_eq!(joined.min, [-1.0, -2.0, 2.0]);
        assert_eq!(joined.max, [5.0, 3.0, 9.0]);
        assert_eq!(joined.center(), [2.0, 0.5, 5.5]);
    }

    #[test]
    fn manifest_rejects_unknown_fields_on_the_wire() {
        let json = serde_json::to_string(&GeometryManifest::new()).unwrap();
        let with_extra = json[..json.len() - 1].to_string() + ",\"extra\":1}";
        assert!(serde_json::from_str::<GeometryManifest>(&with_extra).is_err());
    }

    #[test]
    fn draw_interface_defaults_false_and_is_omitted_when_false() {
        let mut draw = serde_json::to_value(DrawRecord {
            mesh_id: 1,
            shape_index: 0,
            source: None,
            shape_type: "sphere".into(),
            pick_id: 1,
            entity_id: None,
            instance_use_id: None,
            instance_source: None,
            bounds: None,
            world_from_object: vec![[1.0; 16]],
            reverse_orientation: false,
            material_id: None,
            interface: false,
            area_light: None,
            emitter_shape: None,
            alpha: None,
        })
        .unwrap();
        assert!(draw.get("interface").is_none());
        assert!(
            !serde_json::from_value::<DrawRecord>(draw.clone())
                .unwrap()
                .interface
        );
        draw["interface"] = serde_json::Value::Bool(true);
        assert!(
            serde_json::from_value::<DrawRecord>(draw)
                .unwrap()
                .interface
        );
    }

    #[test]
    fn static_draw_reuses_one_matrix_for_every_time() {
        let draw = DrawRecord {
            mesh_id: 1,
            shape_index: 0,
            source: None,
            shape_type: "trianglemesh".into(),
            pick_id: 1,
            entity_id: None,
            instance_use_id: None,
            instance_source: None,
            bounds: None,
            world_from_object: vec![[1.0; 16]],
            reverse_orientation: false,
            material_id: None,
            interface: false,
            area_light: None,
            emitter_shape: None,
            alpha: None,
        };
        assert!(!draw.is_animated());
        assert_eq!(draw.matrix_at(0), Some([1.0; 16]));
        assert_eq!(draw.matrix_at(7), Some([1.0; 16]));
    }

    #[test]
    fn animated_draw_refuses_times_it_does_not_cover() {
        let draw = DrawRecord {
            mesh_id: 1,
            shape_index: 0,
            source: None,
            shape_type: "trianglemesh".into(),
            pick_id: 1,
            entity_id: None,
            instance_use_id: None,
            instance_source: None,
            bounds: None,
            world_from_object: vec![[0.0; 16], [1.0; 16]],
            reverse_orientation: false,
            material_id: None,
            interface: false,
            area_light: None,
            emitter_shape: None,
            alpha: None,
        };
        assert!(draw.is_animated());
        assert_eq!(draw.matrix_at(1), Some([1.0; 16]));
        assert_eq!(draw.matrix_at(2), None);
    }

    #[test]
    fn camera_record_roundtrips_without_look_at() {
        let camera = CameraRecord {
            kind: "perspective".into(),
            camera_from_world: [1.0; 16],
            look_at: None,
            fov_degrees: Some(47.0),
            screenwindow: None,
            frame_aspect_ratio: None,
            shutter: [0.0, 1.0],
            film_resolution: Some([1000, 1400]),
            film_exposure: None,
        };
        let json = serde_json::to_string(&camera).unwrap();
        assert_eq!(serde_json::from_str::<CameraRecord>(&json).unwrap(), camera);
    }

    #[test]
    fn manifest_version_two_carries_appearance_records() {
        let manifest = GeometryManifest {
            version: GEOMETRY_MANIFEST_VERSION,
            materials: vec![MaterialRecord {
                implementation: "diffuse".into(),
                resource_id: Some(EntityId::new("res-1")),
                name: Some("resource_res-1".into()),
                source: Some(SourceLocation {
                    path: "scene.pbrt".into(),
                    line: 3,
                    column: 1,
                }),
                parameters: vec![PreviewParameter {
                    name: "reflectance".into(),
                    value_type: "rgb".into(),
                    floats: vec![0.8, 0.2, 0.1],
                    texts: Vec::new(),
                    truncated: false,
                }],
            }],
            lights: vec![LightRecord {
                implementation: "point".into(),
                world_from_light: identity16(),
                source: None,
                // the light's entity pin rides along when
                // present and stays absent when not (see the v1 manifest read).
                entity_id: Some(EntityId::new("light-1")),
                parameters: vec![PreviewParameter {
                    name: "from".into(),
                    value_type: "point".into(),
                    floats: vec![0.0, 1.0, 0.0],
                    texts: Vec::new(),
                    truncated: true,
                }],
            }],
            ..GeometryManifest::new()
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let read_back: GeometryManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(read_back, manifest);
        assert_eq!(
            read_back.materials[0].parameters[0].float("reflectance"),
            Some(0.8)
        );
        assert!(read_back.lights[0].parameters[0].truncated);
        assert_eq!(
            read_back.lights[0].entity_id,
            Some(EntityId::new("light-1")),
            "the light's entity pin round-trips: {json}"
        );
    }

    #[test]
    fn version_one_manifest_reads_back_with_no_appearance_data() {
        let json = r#"{"version":1,"tessellation":{"angular_segments":64,"bilinear_segments":8,"curve_segments":16,"tube_sides":8},"supported_times":[0.0],"bounds":null,"meshes":[],"draws":[{"mesh_id":0,"shape_index":0,"source":null,"shape_type":"sphere","pick_id":1,"entity_id":null,"instance_use_id":null,"world_from_object":[[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]],"reverse_orientation":false}],"unsupported":[],"camera":null,"blobs":{}}"#;
        let manifest: GeometryManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.version, 1);
        assert!(manifest.materials.is_empty());
        assert!(manifest.lights.is_empty());
        assert_eq!(manifest.draws[0].material_id, None);
        assert_eq!(manifest.draws[0].area_light, None);
    }
}

/// Read a `uv` array (two f32 per element) from a geometry blob.
pub fn read_f32x2_array(blob: &[u8], array: &ArrayRef) -> Result<Vec<[f32; 2]>, String> {
    if array.stride != UV_STRIDE {
        return Err(format!("uv stride {} is not {UV_STRIDE}", array.stride));
    }
    let bytes = array_bytes(blob, array)?;
    let mut out = Vec::with_capacity(array.count as usize);
    for chunk in bytes.chunks_exact(UV_STRIDE as usize) {
        out.push([
            f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]),
            f32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]),
        ]);
    }
    Ok(out)
}
