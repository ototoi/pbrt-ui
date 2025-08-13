use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use uuid::Uuid;
use wgpu::util::DeviceExt;

#[derive(Debug, Clone)]
pub struct RenderLines {
    pub id: Uuid,
    pub edition: String,
    pub vertex_buffer: wgpu::Buffer,
    pub vertex_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct RenderLinesVertex {
    pub position: [f32; 3],
}

impl RenderLines {
    pub fn get_id(&self) -> Uuid {
        self.id
    }

    pub fn from_lines(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        id: Uuid,
        edition: &str,
        lines: &[Vec<[f32; 3]>],
    ) -> Option<RenderLines> {
        let mut vertices: Vec<RenderLinesVertex> = Vec::new();
        for line in lines {
            if line.len() < 2 {
                continue; // Skip lines with less than 2 points
            }
            for i in 0..line.len() - 1 {
                let start = line[i];
                let end = line[i + 1];
                vertices.push(RenderLinesVertex {
                    position: [start[0], start[1], start[2]],
                });
                vertices.push(RenderLinesVertex {
                    position: [end[0], end[1], end[2]],
                });
            }
        }

        let vertex_count = vertices.len() as u32;
        if vertex_count == 0 {
            return None; // No vertices to render
        }
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("RenderLines Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        return Some(RenderLines {
            id,
            edition: edition.to_string(),
            vertex_buffer,
            vertex_count,
        });
    }
}
