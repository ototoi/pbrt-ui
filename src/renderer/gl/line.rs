use std::sync::Arc;

//use eframe::egui;
use eframe::{egui_glow, glow::HasContext};
use egui_glow::glow;

#[derive(Debug, Clone)]
pub struct RenderLine {
    pub postions: glow::Buffer,
    pub indices: glow::Buffer,
    pub count: i32,
    pub vao: glow::VertexArray,
}

impl RenderLine {
    pub fn from_positions(gl: &Arc<glow::Context>, positions: &[f32]) -> Option<Self> {
        unsafe {
            let count = (positions.len() / 3) as i32;
            let indices = (0..count).collect::<Vec<i32>>();
            let indices_buffer = gl.create_buffer().ok()?;
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(indices_buffer));
            gl.buffer_data_u8_slice(
                glow::ELEMENT_ARRAY_BUFFER,
                indices.align_to().1,
                glow::STATIC_DRAW,
            );

            let positions_buffer = gl.create_buffer().ok()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(positions_buffer));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                positions.align_to().1,
                glow::STATIC_DRAW,
            );

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

            return Some(Self {
                postions: positions_buffer,
                indices: indices_buffer,
                count,
                vao,
            });
        }
    }

    pub fn destroy(&self, gl: &Arc<glow::Context>) {
        unsafe {
            gl.delete_buffer(self.postions);
            gl.delete_buffer(self.indices);
            gl.delete_vertex_array(self.vao);
        }
    }
}
