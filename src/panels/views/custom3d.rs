use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

use eframe::egui;
use eframe::egui_glow;
use eframe::glow::HasContext;
use egui::mutex::Mutex;
use egui_glow::glow;

use crate::models::base::Matrix4x4;
use crate::models::base::Quaternion;
use crate::models::base::Vector3;

pub struct Custom3d {
    /// Behind an `Arc<Mutex<…>>` so we can pass it to [`egui::PaintCallback`] and paint later.
    rotating_triangle: Arc<Mutex<RotatingTriangle>>,
    angle_x: f32,
    angle_y: f32,
}

impl Custom3d {
    pub fn new<'a>(cc: &'a eframe::CreationContext<'a>) -> Option<Self> {
        let gl = cc.gl.as_ref()?;
        Some(Self {
            rotating_triangle: Arc::new(Mutex::new(RotatingTriangle::new(gl)?)),
            angle_x: 0.0,
            angle_y: 0.0,
        })
    }
}

impl Custom3d {
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let (rect, response) =
            ui.allocate_exact_size(ui.available_size_before_wrap(), egui::Sense::drag());
        
        self.angle_x += response.drag_motion().x * 0.01;
        self.angle_y += response.drag_motion().y * 0.01;

        // Clone locals so we can move them into the paint callback:
        let angle_x = self.angle_x;
        let angle_y = self.angle_y;
        let qx = Quaternion::from_angle_axis(angle_x, &Vector3::new(0.0, 1.0, 0.0));
        let qy = Quaternion::from_angle_axis(angle_y, &Vector3::new(1.0, 0.0, 0.0));
        let q = qx * qy;
        let m = q.to_matrix();
        //ui.label(format!("angle_x: {:.2}", angle_x));
        //ui.label(format!("angle_y: {:.2}", angle_y));

        let rotating_triangle = self.rotating_triangle.clone();

        let cb = egui_glow::CallbackFn::new(move |_info, painter| {
            rotating_triangle.lock().paint(painter.gl(), &m);
        });

        let callback = egui::PaintCallback {
            rect,
            callback: Arc::new(cb),
        };
        ui.painter().add(callback);
    }
}
//---------------------------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct ShaderProgram {
    pub handle: glow::Program,
    pub gl: Arc<glow::Context>,
}

impl ShaderProgram {
    pub fn create(gl: &Arc<glow::Context>) -> Result<Self, String> {
        use glow::HasContext as _;
        unsafe {
            let handle = gl.create_program().expect("Cannot create program");
            return Ok(Self {
                handle: handle,
                gl: gl.clone(),
            });
        }
    }

    pub fn attach_shader(&self, shader: &Shader) {
        use glow::HasContext as _;
        unsafe {
            self.gl.attach_shader(self.handle, shader.handle);
        }
    }
}
impl Drop for ShaderProgram {
    fn drop(&mut self) {
        use glow::HasContext as _;
        unsafe {
            self.gl.delete_program(self.handle);
        }
    }
}
//---------------------------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct Shader {
    pub handle: glow::Shader,
    pub gl: Arc<glow::Context>,
}

impl Shader {
    pub fn create(gl: &Arc<glow::Context>, shader_type: u32, source: &str) -> Result<Self, String> {
        use glow::HasContext as _;
        unsafe {
            let handle = gl.create_shader(shader_type).expect("Cannot create shader");
            gl.shader_source(handle, source);
            gl.compile_shader(handle);
            if !gl.get_shader_compile_status(handle) {
                return Err(gl.get_shader_info_log(handle));
            }
            Ok(Self {
                handle: handle,
                gl: gl.clone(),
            })
        }
    }
}
impl Drop for Shader {
    fn drop(&mut self) {
        use glow::HasContext as _;
        unsafe {
            self.gl.delete_shader(self.handle);
        }
    }
}
//---------------------------------------------------------------------------------------------
pub struct Buffer {
    pub handle: glow::Buffer,
    pub gl: Arc<glow::Context>,
}

impl Buffer {
    pub fn create(gl: &Arc<glow::Context>) -> Result<Self, String> {
        use glow::HasContext as _;
        unsafe {
            let handle = gl.create_buffer().expect("Cannot create buffer");
            Ok(Self {
                handle: handle,
                gl: gl.clone(),
            })
        }
    }
}

//---------------------------------------------------------------------------------------------
pub struct VertexArray {
    pub handle: glow::VertexArray,
    pub gl: Arc<glow::Context>,
}

impl VertexArray {
    pub fn create(gl: &Arc<glow::Context>) -> Result<Self, String> {
        use glow::HasContext as _;
        unsafe {
            let handle = gl
                .create_vertex_array()
                .expect("Cannot create vertex array");
            Ok(Self {
                handle: handle,
                gl: gl.clone(),
            })
        }
    }
}
impl Drop for VertexArray {
    fn drop(&mut self) {
        use glow::HasContext as _;
        unsafe {
            self.gl.delete_vertex_array(self.handle);
        }
    }
}

//---------------------------------------------------------------------------------------------

struct RotatingTriangle {
    program: ShaderProgram,
    //vertex_array: glow::VertexArray,
    vao: VertexArray,
    vtx: Buffer,
}

#[allow(unsafe_code)] // we need unsafe code to use glow
impl RotatingTriangle {
    fn new(gl: &Arc<glow::Context>) -> Option<Self> {
        use glow::HasContext as _;

        let shader_version = egui_glow::ShaderVersion::get(gl);

        unsafe {
            let program = ShaderProgram::create(gl).ok()?;

            if !shader_version.is_new_shader_interface() {
                log::warn!(
                    "Custom 3D painting hasn't been ported to {:?}",
                    shader_version
                );
                return None;
            }

            let (vertex_shader_source, fragment_shader_source) = (
                r#"
                    in vec3 vertex;//
                    in vec4 color; //

                    out vec4 vertexColor;

                    uniform mat4 local_to_world;
                    void main() {
                        gl_Position = local_to_world * vec4(vertex, 1);
                        vertexColor = color;
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

            //println!("gl version: {}", gl.get_parameter_string(glow::VERSION));

            //println!("{}", shader_version.version_declaration());

            let shaders: Vec<_> = shader_sources
                .iter()
                .map(|(shader_type, shader_source)| {
                    let source = format!(
                        "{}\n{}",
                        shader_version.version_declaration(),
                        shader_source
                    );
                    let shader = Shader::create(gl, *shader_type, &source)
                        .map_err(|e| {
                            log::error!("Shader error: {}", e);
                            println!("Shader error: {}", e);
                            e
                        })
                        .ok()
                        .unwrap();

                    gl.attach_shader(program.handle, shader.handle);
                    shader
                })
                .collect();

            gl.link_program(program.handle);
            assert!(
                gl.get_program_link_status(program.handle),
                "{}",
                gl.get_program_info_log(program.handle)
            );
            {}
            for shader in shaders.iter() {
                gl.detach_shader(program.handle, shader.handle);
            }

            //ignore format skip

            let vertex: [f32; 7 * 3] = [
                0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, //red
                -1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, //green
                1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0, //blue
            ];

            let indices: [u32; 3] = [0, 1, 2];

            let vertex_buffer = Buffer::create(gl).ok()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vertex_buffer.handle));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, vertex.align_to().1, glow::STATIC_DRAW);

            let indices_buffer = Buffer::create(gl).ok()?;
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(indices_buffer.handle));
            gl.buffer_data_u8_slice(
                glow::ELEMENT_ARRAY_BUFFER,
                indices.align_to().1,
                glow::STATIC_DRAW,
            );

            gl.use_program(Some(program.handle));
            let vertex_location = gl.get_attrib_location(program.handle, "vertex").unwrap();
            let color_location = gl.get_attrib_location(program.handle, "color").unwrap();

            //println!("vertex_location: {}", vertex_location);
            //println!("color_location: {}", color_location);

            gl.use_program(None);

            let vertex_array = VertexArray::create(gl).ok()?;
            gl.bind_vertex_array(Some(vertex_array.handle));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vertex_buffer.handle));
            gl.enable_vertex_attrib_array(vertex_location);
            gl.enable_vertex_attrib_array(color_location);
            gl.vertex_attrib_pointer_f32(
                vertex_location,
                3,
                glow::FLOAT,
                false,
                (std::mem::size_of::<f32>() * 7) as i32,
                0,
            );
            gl.vertex_attrib_pointer_f32(
                color_location,
                4,
                glow::FLOAT,
                false,
                (std::mem::size_of::<f32>() * 7) as i32,
                (std::mem::size_of::<f32>() * 3) as i32,
            ); //4 * 3

            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(indices_buffer.handle));

            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, None);

            Some(Self {
                program,
                //vertex_array
                vao: vertex_array,
                vtx: vertex_buffer,
            })
        }
    }

    fn paint(&self, gl: &glow::Context, local_to_world: &Matrix4x4) {
        use glow::HasContext as _;
        unsafe {
            //gl.clear_color(1.0, 1.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.use_program(Some(self.program.handle));
            //gl.uniform_1_f32(
            //    gl.get_uniform_location(self.program.handle, "u_angle")
            //        .as_ref(),
            //    angle,
            //);
            gl.uniform_matrix_4_f32_slice(
                gl.get_uniform_location(self.program.handle, "local_to_world")
                    .as_ref(),
                false,
                &local_to_world.m,
            );
            gl.bind_vertex_array(Some(self.vao.handle));
            //gl.draw_arrays(glow::TRIANGLES, 0, 3);
            //gl.polygon_mode(glow::FRONT_AND_BACK, glow::LINE);
            gl.draw_elements(glow::TRIANGLES, 3, glow::UNSIGNED_INT, 0);
            //.polygon_mode(glow::FRONT_AND_BACK, glow::FILL);
            gl.bind_vertex_array(None);
            gl.use_program(None);
        }
    }
}
