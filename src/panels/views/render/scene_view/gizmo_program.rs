use crate::renderers::gl::RenderProgram;

use std::collections::HashMap;
use std::sync::Arc;

use uuid::Uuid;

use eframe::egui_glow;
use eframe::glow;

pub const GIZMO_SHADER_ID: &str = "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11";
pub fn create_gizmo_program(gl: &Arc<glow::Context>, id: Uuid) -> Option<Arc<RenderProgram>> {
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

                out vec4 vertexColor;

                uniform mat4 local_to_world;
                uniform mat4 world_to_camera;
                uniform mat4 camera_to_clip;
                void main() {
                    //gl_Position = camera_to_clip * world_to_camera * local_to_world * vec4(position, 1);
                    gl_Position = vec4(position, 1) * local_to_world * world_to_camera * camera_to_clip;
                    vertexColor = vec4(0.0, 1.0, 0.0, 1.0);
                }
            "#,
            r#"
                precision highp float;
                in vec4 vertexColor;

                out vec4 outColor;
                void main() {
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

        let mut vertex_attributes = HashMap::new();
        for key in ["position"].iter() {
            if let Some(location) = gl.get_attrib_location(program, *key) {
                vertex_attributes.insert(key.to_string(), location as u32);
            }
        }

        return Some(Arc::new(RenderProgram {
            id: id,
            handle: program,
            vertex_attributes: vertex_attributes,
            gl: gl.clone(),
        }));
    }
}
