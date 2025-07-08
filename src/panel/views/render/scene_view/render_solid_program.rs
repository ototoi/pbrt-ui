use crate::renderer::gl::RenderProgram;

use std::collections::HashMap;
use std::sync::Arc;

use uuid::Uuid;

use eframe::egui_glow;
use eframe::glow;

pub const RENDER_SOLID_SHADER_ID: &str = "812e32bf-8051-42a7-94af-03e4099025da";
pub fn create_render_solid_program(
    gl: &Arc<glow::Context>,
    id: Uuid,
) -> Option<Arc<RenderProgram>> {
    use glow::HasContext as _;

    unsafe {
        // Check if the OpenGL context is current
        // get gl shader version
        let _shader_version = egui_glow::ShaderVersion::get(gl);

        //todo!("Implement create_dunny_program");
        let program = gl.create_program().ok()?;

        let (vertex_shader_source, fragment_shader_source) = (
            r#"
                layout(location = 0) in vec3 position;   //
                //layout(location = 1) in vec3 normal;     //
                layout(location = 2) in vec2 uv;         //

                out vec2 vertexUV;
                out vec4 vertexColor;

                uniform vec4 base_color;
                uniform mat4 local_to_world;
                uniform mat4 world_to_camera;
                uniform mat4 camera_to_clip;
                void main() {
                    //gl_Position = camera_to_clip * world_to_camera * local_to_world * vec4(position, 1);
                    gl_Position = vec4(position, 1) * local_to_world * world_to_camera * camera_to_clip;
                    //float z = abs(gl_Position.z / gl_Position.w) * 0.5;
                    vertexUV = uv;
                    vertexColor = base_color;
                }
            "#,
            r#"
                precision highp float;
                in vec2 vertexUV;
                in vec4 vertexColor;

                out vec4 outColor;
                void main() {
                    //outColor = vec4(vertexUV, 0.0, 1.0);
                    outColor = vertexColor;
                }
            "#,
        );

        let shader_sources = [
            (glow::VERTEX_SHADER, vertex_shader_source),
            (glow::FRAGMENT_SHADER, fragment_shader_source),
        ];

        let shaders: Vec<_> = shader_sources
            .iter()
            .map(|(shader_type, shader_source)| {
                let source = format!(
                    "{}\n{}",
                    "#version 330", //shader_version.version_declaration(),
                    shader_source
                );
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
