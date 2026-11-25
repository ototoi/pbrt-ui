//! Object-space GPU mesh cache entries and checked source conversion.
use super::{cross, norm, sub};
use protocol::geometry::{
    DrawRecord, MeshRecord, check_blob_header, read_f32x2_array, read_f32x3_array, read_indices,
};
use std::collections::{BTreeMap, BTreeSet};
use wgpu::util::DeviceExt;

pub(super) const MAX_UPLOAD_BYTES: u64 = 512 * 1024 * 1024;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    /// Texture coordinates for the lit preview; (0, 0) when the mesh has none.
    uv: [f32; 2],
}

#[derive(Clone)]
pub(super) struct Mesh {
    pub(super) edition: u64,
    pub(super) vertices: wgpu::Buffer,
    pub(super) triangles: wgpu::Buffer,
    pub(super) lines: wgpu::Buffer,
    pub(super) triangle_count: u32,
    pub(super) line_count: u32,
    pub(super) extent: [f64; 3],
    /// Center of the mesh-local bounding box, for sorting glass by depth.
    pub(super) center: [f64; 3],
    pub(super) bytes: u64,
}

pub(super) fn validate_draws(
    draws: &[DrawRecord],
    ids: &BTreeSet<u64>,
    times: usize,
) -> Result<(), String> {
    let mut picks = BTreeSet::new();
    for draw in draws {
        if !ids.contains(&draw.mesh_id) {
            return Err(format!("draw references unknown mesh {}", draw.mesh_id));
        }
        if draw.pick_id == 0 || !picks.insert(draw.pick_id) {
            return Err("preview pick ids must be unique and nonzero".into());
        }
        if draw.world_from_object.len() != 1 && draw.world_from_object.len() != times {
            return Err("invalid preview transform sample count".into());
        }
        for m in &draw.world_from_object {
            if m.iter().any(|n| !n.is_finite()) || m[12..] != [0., 0., 0., 1.] {
                return Err("preview transform must be finite and affine".into());
            }
        }
    }
    Ok(())
}

pub(super) struct PreparedMesh {
    vertices: Vec<Vertex>,
    triangles: Vec<u32>,
    lines: Vec<u32>,
    extent: [f64; 3],
    center: [f64; 3],
    pub(super) bytes: u64,
}

impl PreparedMesh {
    pub(super) fn read(
        record: &MeshRecord,
        blobs: &BTreeMap<String, Vec<u8>>,
        limit: u64,
        budget: u64,
    ) -> Result<Self, String> {
        let vertex_bytes = record
            .positions
            .count
            .checked_mul(std::mem::size_of::<Vertex>() as u64)
            .ok_or("preview vertex size overflow")?;
        let line_bytes = record
            .indices
            .count
            .checked_mul(8)
            .ok_or("preview index size overflow")?;
        let bytes = vertex_bytes
            .max(4)
            .checked_add(line_bytes.max(4))
            .and_then(|size| size.checked_add((record.indices.count * 4).max(4)))
            .ok_or("preview mesh size overflow")?;
        if vertex_bytes > limit
            || line_bytes > limit
            || bytes > MAX_UPLOAD_BYTES
            || bytes > budget
            || record.positions.count > 12_000_000
            || record.indices.count > 60_000_000
        {
            return Err("preview mesh exceeds the GPU upload limit".into());
        }
        let blob = blobs
            .get(&record.blob)
            .ok_or_else(|| format!("missing geometry blob {}", record.blob))?;
        check_blob_header(blob)?;
        let positions = read_f32x3_array(blob, &record.positions)?;
        let triangles = read_indices(blob, &record.indices)?;
        if triangles.len() % 3 != 0 || triangles.iter().any(|i| *i as usize >= positions.len()) {
            return Err(format!(
                "mesh {} has invalid triangle indices",
                record.mesh_id
            ));
        }
        let uvs = if let Some(uvs) = &record.uv {
            if uvs.count != record.positions.count {
                return Err("preview uv count does not match positions".into());
            }
            let uvs = read_f32x2_array(blob, uvs)?;
            if uvs.len() != positions.len() {
                return Err("preview uv count does not match positions".into());
            }
            uvs
        } else {
            vec![[0f32; 2]; positions.len()]
        };
        let normals = if let Some(normals) = &record.normals {
            if normals.count != record.positions.count {
                return Err("preview normal count does not match positions".into());
            }
            let normals = read_f32x3_array(blob, normals)?;
            if normals.len() != positions.len() {
                return Err("preview normal count does not match positions".into());
            }
            normals
        } else {
            let mut normals = vec![[0f64; 3]; positions.len()];
            for tri in triangles.chunks_exact(3) {
                let [a, b, c] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
                let n = cross(
                    sub(positions[b].map(f64::from), positions[a].map(f64::from)),
                    sub(positions[c].map(f64::from), positions[a].map(f64::from)),
                );
                for index in [a, b, c] {
                    for axis in 0..3 {
                        normals[index][axis] += n[axis];
                    }
                }
            }
            normals
                .into_iter()
                .map(|n| norm(n).unwrap_or([0., 0., 1.]).map(|x| x as f32))
                .collect()
        };
        if positions
            .iter()
            .chain(&normals)
            .flatten()
            .any(|n| !n.is_finite())
        {
            return Err("preview mesh contains nonfinite values".into());
        }
        let extent = std::array::from_fn(|axis| {
            positions
                .iter()
                .map(|p| (p[axis] as f64).abs())
                .fold(0f64, f64::max)
        });
        let center = std::array::from_fn(|axis| {
            let (low, high) = positions
                .iter()
                .fold((f64::MAX, f64::MIN), |(low, high), p| {
                    (low.min(p[axis] as f64), high.max(p[axis] as f64))
                });
            if low <= high { (low + high) * 0.5 } else { 0. }
        });
        let lines = triangles
            .chunks_exact(3)
            .flat_map(|t| [t[0], t[1], t[1], t[2], t[2], t[0]])
            .collect();
        let vertices = positions
            .into_iter()
            .zip(normals)
            .zip(uvs)
            .map(|((position, normal), uv)| Vertex {
                position,
                normal,
                uv,
            })
            .collect();
        Ok(Self {
            vertices,
            triangles,
            lines,
            extent,
            center,
            bytes,
        })
    }

    pub(super) fn upload(self, device: &wgpu::Device, edition: u64) -> Mesh {
        let buffer = |label, contents: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: if contents.is_empty() {
                    &[0; 4]
                } else {
                    contents
                },
                usage,
            })
        };
        Mesh {
            edition,
            vertices: buffer(
                "preview-mesh-vertices",
                bytemuck::cast_slice(&self.vertices),
                wgpu::BufferUsages::VERTEX,
            ),
            triangles: buffer(
                "preview-mesh-triangles",
                bytemuck::cast_slice(&self.triangles),
                wgpu::BufferUsages::INDEX,
            ),
            lines: buffer(
                "preview-mesh-lines",
                bytemuck::cast_slice(&self.lines),
                wgpu::BufferUsages::INDEX,
            ),
            triangle_count: self.triangles.len() as u32,
            line_count: self.lines.len() as u32,
            extent: self.extent,
            center: self.center,
            bytes: self.bytes,
        }
    }
}
