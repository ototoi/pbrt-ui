use crate::conversion::mesh_data::MeshData;
use crate::conversion::mesh_data::create_mesh_data;
use crate::model::scene::Shape;
use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use uuid::Uuid;
use wgpu::util::DeviceExt;

#[derive(Debug, Clone)]
pub struct RenderMesh {
    pub id: Uuid,
    pub edition: String,
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub vertex_count: u32,
    pub index_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct RenderVertex {
    pub position: [f32; 3],
    pub uv: [f32; 3],      // Assuming UVs are needed
    pub normal: [f32; 3],  // Assuming normals are also needed
    pub tangent: [f32; 3], // Assuming tangents are needed
}

fn get_vertices(mesh: &MeshData) -> Vec<RenderVertex> {
    let vertex_count = mesh.positions.len() / 3;
    let mut vertices = Vec::with_capacity(vertex_count);
    for i in 0..vertex_count {
        vertices.push(RenderVertex {
            position: [
                mesh.positions[i * 3],
                mesh.positions[i * 3 + 1],
                mesh.positions[i * 3 + 2],
            ],
            uv: [mesh.uvs[i * 2], mesh.uvs[i * 2 + 1], 0.0], // Assuming UVs are 2D, adding a dummy Z value
            normal: [
                mesh.normals[i * 3],
                mesh.normals[i * 3 + 1],
                mesh.normals[i * 3 + 2],
            ],
            tangent: [
                mesh.tangents[i * 3],
                mesh.tangents[i * 3 + 1],
                mesh.tangents[i * 3 + 2],
            ],
        });
    }
    vertices
}

fn get_indices(mesh: &MeshData) -> Vec<u32> {
    mesh.indices.iter().map(|&i| i as u32).collect()
}

impl RenderMesh {
    pub fn get_id(&self) -> Uuid {
        return self.id;
    }

    pub fn from_shape(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        shape: &Shape,
    ) -> Option<RenderMesh> {
        let mesh_id = shape.get_id();
        let edition = shape.get_edition();
        if let Some(mut mesh_data) = create_mesh_data(shape) {
            let num_vertices = mesh_data.positions.len() / 3;
            if mesh_data.normals.len() < num_vertices * 3 {
                // If positions are not provided, create a default position
                mesh_data.normals.resize(num_vertices * 3, 0.0);
            }
            if mesh_data.uvs.len() < num_vertices * 2 {
                // If UVs are not provided, create a default UV mapping
                mesh_data.uvs.resize(num_vertices * 2, 0.0);
            }
            let vertex_data = get_vertices(&mesh_data);
            let vertex_count = vertex_data.len();
            let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Vertex Buffer"),
                contents: bytemuck::cast_slice(&vertex_data),
                usage: wgpu::BufferUsages::VERTEX,
            });

            let index_data = get_indices(&mesh_data);
            let index_count = index_data.len();
            let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Index Buffer"),
                contents: bytemuck::cast_slice(&index_data),
                usage: wgpu::BufferUsages::INDEX,
            });
            let mesh = RenderMesh {
                id: mesh_id,
                edition,
                vertex_buffer,
                index_buffer,
                vertex_count: vertex_count as u32,
                index_count: index_count as u32,
            };
            return Some(mesh);
        }
        None
    }
}
