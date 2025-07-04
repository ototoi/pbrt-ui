use crate::geometry::mesh_data::*;
use crate::model::scene::Shape;

use std::sync::Arc;
use uuid::Uuid;

//use eframe::egui;
use eframe::{egui_glow, glow::HasContext};
use egui_glow::glow;

#[derive(Debug, Clone)]
pub struct RenderMesh {
    pub id: Uuid,
    pub edition: String,
    pub postions: glow::Buffer,
    //pub normals: glow::Buffer,
    //pub uvs: glow::Buffer,
    pub indices: glow::Buffer,
    pub count: i32,
    pub vao: glow::VertexArray,
    pub gl: Arc<glow::Context>,
}

fn get_normals(indices: &[i32], positions: &[f32]) -> Vec<f32> {
    let mut normals = Vec::new();
    for _ in 0..positions.len() / 3 {
        normals.push(0.0);
        normals.push(1.0);
        normals.push(0.0);
    }
    return normals;
}

fn get_uvs(indices: &[i32], positions: &[f32]) -> Vec<f32> {
    let mut uvs = Vec::new();
    for _ in 0..positions.len() / 3 {
        uvs.push(0.0);
        uvs.push(1.0);
    }
    return uvs;
}

impl RenderMesh {
    pub fn get_id(&self) -> Uuid {
        self.id
    }

    pub fn from_mesh_data(
        gl: &Arc<glow::Context>,
        id: Uuid,
        edition: &str,
        indices: &[i32],
        positions: &[f32],
        //normals: &[f32],
        //uvs: &[f32],
    ) -> Option<Self> {
        unsafe {
            let indices_buffer = gl.create_buffer().ok()?;
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(indices_buffer));
            gl.buffer_data_u8_slice(
                glow::ELEMENT_ARRAY_BUFFER,
                indices.align_to().1,
                glow::STATIC_DRAW,
            );

            let count = indices.len() as i32;

            //let vao = gl.create_vertex_array().ok()?;
            //gl.bind_vertex_array(Some(vao));

            let positions_buffer = gl.create_buffer().ok()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(positions_buffer));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                positions.align_to().1,
                glow::STATIC_DRAW,
            );

            /*
            let normals_buffer = gl.create_buffer().ok()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(normals_buffer));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, normals.align_to().1, glow::STATIC_DRAW);
            let uvs_buffer = gl.create_buffer().ok()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(uvs_buffer));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, uvs.align_to().1, glow::STATIC_DRAW);
            */
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, None);

            let vao = gl.create_vertex_array().ok()?;
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(indices_buffer));

            gl.bind_buffer(glow::ARRAY_BUFFER, Some(positions_buffer));
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(
                0, //location
                3, //element count
                glow::FLOAT,
                false,
                (std::mem::size_of::<f32>() * 3) as i32,
                0,
            );

            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, None);

            let shape = RenderMesh {
                id,
                edition: edition.to_string(),
                postions: positions_buffer,
                //normals: normals_buffer,
                //uvs: uvs_buffer,
                indices: indices_buffer,
                count: count,
                vao: vao,
                gl: gl.clone(),
            };
            return Some(shape);
        }
    }

    pub fn from_mesh(gl: &Arc<glow::Context>, shape: &Shape) -> Option<Self> {
        //println!("from_sphere");
        let gl = gl.clone();
        let id = shape.get_id();
        let edition = shape.get_edition();
        //let edition = Uuid::parse_str(&edition).unwrap_or(Uuid::new_v4());
        if let Some(mesh_data) = create_mesh_data(shape) {
            return Self::from_mesh_data(
                &gl,
                id,
                &edition,
                &mesh_data.indices,
                &mesh_data.positions,
            );
        }
        None
    }

    pub fn from_shape_core(gl: &Arc<glow::Context>, shape: &Shape) -> Option<Self> {
        //println!("from_sphere");
        let gl = gl.clone();
        let id = shape.get_id();
        let edition = shape.get_edition();
        //let edition = Uuid::parse_str(&edition).unwrap_or(Uuid::new_v4());
        if let Some(mesh_data) = create_mesh_data(shape) {
            return Self::from_mesh_data(
                &gl,
                id,
                &edition,
                &mesh_data.indices,
                &mesh_data.positions,
            );
        }
        None
    }
}

impl Drop for RenderMesh {
    fn drop(&mut self) {
        let gl = &self.gl;
        unsafe {
            gl.delete_buffer(self.postions);
            //gl.delete_buffer(self.normals);
            //gl.delete_buffer(self.uvs);
            gl.delete_buffer(self.indices);
            gl.delete_vertex_array(self.vao);
        }
    }
}
