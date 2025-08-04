use crate::conversion::light_shape::create_light_shape;
use crate::model::scene::Light;
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

    pub fn from_light(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        light: &Light,
    ) -> Option<RenderLines> {
        let id = light.get_id();
        let edition = light.get_edition();
        let t = light.get_type();
        //println!("Creating RenderLines for light: {} (type: {})", id, t);

        let mut vertices: Vec<RenderLinesVertex> = Vec::new();
        if let Some(light_shape) = create_light_shape(light) {
            //println!("Creating RenderLines for light: {}", id);
            let lines = &light_shape.lines;
            for line in lines {
                if line.len() < 2 {
                    continue; // Skip lines with less than 2 points
                }
                for i in 0..line.len() - 1 {
                    let start = line[i];
                    let end = line[i + 1];
                    vertices.push(RenderLinesVertex {
                        position: [start.x, start.y, start.z],
                    });
                    vertices.push(RenderLinesVertex {
                        position: [end.x, end.y, end.z],
                    });
                }
            }

            //println!("RenderLines: {} vertices", vertices.len());
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
                edition,
                vertex_buffer,
                vertex_count,
            });
        }
        return None;
    }
}
