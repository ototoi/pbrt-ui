use super::render_version::get_glsl_version_line;
use crate::renderer::gl::RenderProgram;

use std::collections::HashMap;
use std::sync::Arc;

use uuid::Uuid;

use eframe::glow;

pub const GIZMO_SHADER_ID: &str = "c80398e9-a45b-4783-96a9-03ccd15ced40";

pub fn create_render_gizmo_program(
    gl: &Arc<glow::Context>,
    id: Uuid,
) -> Option<Arc<RenderProgram>> {
    use glow::HasContext as _;

    unsafe {
        let version_string = get_glsl_version_line(gl)?;
        let program = gl.create_program().ok()?;

        let (vertex_shader_source, fragment_shader_source) = (
            include_str!("shaders/gizmo/gizmo.vs"),
            include_str!("shaders/gizmo/gizmo.fs"),
        );

        let shader_sources = [
            (glow::VERTEX_SHADER, vertex_shader_source),
            (glow::FRAGMENT_SHADER, fragment_shader_source),
        ];

        let shaders: Vec<_> = shader_sources
            .iter()
            .map(|(shader_type, shader_source)| {
                let source = format!("{}\n{}", version_string, shader_source);
                let shader = gl.create_shader(*shader_type).ok().unwrap();
                gl.shader_source(shader, &source);
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    log::error!("{:?}", gl.get_shader_info_log(shader));
                }

                gl.attach_shader(program, shader);
                shader
            })
            .collect();

        gl.link_program(program);
        for shader in shaders.iter() {
            gl.detach_shader(program, *shader);
            gl.delete_shader(*shader);
        }

        let mut uniform_locations = HashMap::new();
        for key in [
            "local_to_world",
            "world_to_camera",
            "camera_to_clip",
            "base_color",
        ]
        .iter()
        {
            if let Some(location) = gl.get_uniform_location(program, *key) {
                let location = location.0 as u32;
                uniform_locations.insert(key.to_string(), location as u32);
            }
        }

        let mut vertex_locations = HashMap::new();
        for key in ["position"].iter() {
            if let Some(location) = gl.get_attrib_location(program, *key) {
                vertex_locations.insert(key.to_string(), location as u32);
            }
        }

        return Some(Arc::new(RenderProgram {
            id: id,
            handle: program,
            uniform_locations,
            vertex_locations,
            gl: gl.clone(),
        }));
    }
}
