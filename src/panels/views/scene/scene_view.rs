use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;

use eframe::egui;
use eframe::egui_glow;
use eframe::glow::HasContext;
use egui::Vec2;
use egui_glow::glow;

use crate::controllers::AppController;
use crate::models::base::Matrix4x4;
use crate::models::base::Property;
use crate::models::base::PropertyMap;
use crate::models::base::Quaternion;
use crate::models::base::Vector3;
use crate::models::scene::CameraComponent;
use crate::models::scene::Component;
use crate::models::scene::CoordinateSystemComponent;
use crate::models::scene::FilmComponent;
use crate::models::scene::Material;
use crate::models::scene::MaterialComponent;
use crate::models::scene::Mesh;
use crate::models::scene::MeshComponent;
use crate::models::scene::Node;

use crate::models::scene::ShapeComponent;
use crate::models::scene::SubdivComponent;
use crate::models::scene::TransformComponent;
use crate::renderers::gl::RenderMesh;
use crate::renderers::gl::RenderProgram;
use crate::renderers::gl::ResourceComponent;
use crate::renderers::gl::ResourceManager;
//use crate::renderers::gl::VAO;

fn get_render_size(camera_node: &Arc<RwLock<Node>>) -> Vec2 {
    let camera_node = camera_node.read().unwrap();
    if let Some(c) = camera_node.get_component::<FilmComponent>() {
        let width = c.props.find_one_int("integer xresolution").unwrap_or(1280);
        let height = c.props.find_one_int("integer yresolution").unwrap_or(720);
        return Vec2::new(width as f32, height as f32);
    }
    Vec2::new(1280.0, 720.0) // Default size if no FilmComponent is found
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RenderMode {
    Wireframe,
    Solid,
    Lighting,
}

struct Renderer {
    pub render_mode: RenderMode,
}

impl Renderer {
    pub fn new() -> Self {
        Self {
            render_mode: RenderMode::Wireframe,
        }
    }

    pub fn render(
        &mut self,
        gl: &glow::Context,
        w2c: &Matrix4x4,
        c2c: &Matrix4x4,
        items: &[RenderItem],
    ) {
        unsafe {
            gl.polygon_mode(glow::FRONT_AND_BACK, glow::LINE);

            gl.use_program(Some(items[0].program.handle));
            gl.enable_vertex_attrib_array(0);

            //let loc = gl.get_uniform_location(items[0].program.handle, "world_to_camera").unwrap().0;
            gl.uniform_matrix_4_f32_slice(
                gl.get_uniform_location(items[0].program.handle, "world_to_camera")
                    .as_ref(),
                false,
                &w2c.m,
            );

            gl.uniform_matrix_4_f32_slice(
                gl.get_uniform_location(items[0].program.handle, "camera_to_clip")
                    .as_ref(),
                false,
                &c2c.m,
            );

            for item in items {
                gl.uniform_matrix_4_f32_slice(
                    gl.get_uniform_location(item.program.handle, "local_to_world")
                        .as_ref(),
                    false,
                    &item.local_to_world.m,
                );

                gl.bind_vertex_array(Some(item.mesh.vao));
                gl.draw_elements(glow::TRIANGLES, item.mesh.count, glow::UNSIGNED_INT, 0);
                gl.bind_vertex_array(None);
            }
            gl.use_program(None);
            gl.polygon_mode(glow::FRONT_AND_BACK, glow::FILL);
        }
        //todo!("Implement render");
    }
}

#[derive(Debug, Clone)]
struct RenderItem {
    pub local_to_world: Matrix4x4,
    pub mesh: Arc<RenderMesh>,
    pub program: Arc<RenderProgram>,
}

pub struct SceneView {
    renderer: Option<Arc<Mutex<Renderer>>>,
    controller: Arc<RwLock<AppController>>,
    gl: Arc<glow::Context>,

    test_program: Arc<RenderProgram>,
}

struct SceneItem(pub (Arc<RwLock<Node>>, Matrix4x4, Arc<RwLock<Material>>));

fn has_component<T: Component>(node: &Arc<RwLock<Node>>) -> bool {
    let node = node.read().unwrap();
    node.get_component::<T>().is_some()
}

fn get_local_matrix(node: &Arc<RwLock<Node>>) -> Matrix4x4 {
    let node = node.read().unwrap();
    let t = node.get_component::<TransformComponent>().unwrap();
    return t.get_local_matrix();
}

fn get_material(node: &Arc<RwLock<Node>>) -> Arc<RwLock<Material>> {
    let node = node.read().unwrap();
    let m = node.get_component::<MaterialComponent>().unwrap();
    return m.material.clone();
}

fn get_mesh_nodes(
    parent_matrix: &Matrix4x4,
    node: &Arc<RwLock<Node>>,
    mesh_nodes: &mut Vec<SceneItem>,
) {
    let local_matrix = get_local_matrix(node);
    let world_matrix = *parent_matrix * local_matrix;

    if has_component::<MeshComponent>(&node) && has_component::<MaterialComponent>(&node) {
        let material = get_material(node);
        let item = SceneItem((node.clone(), world_matrix, material));
        mesh_nodes.push(item);
    }

    if has_component::<SubdivComponent>(&node) && has_component::<MaterialComponent>(&node) {
        let material = get_material(node);
        let item = SceneItem((node.clone(), world_matrix, material));
        mesh_nodes.push(item);
    }

    if has_component::<ShapeComponent>(&node) && has_component::<MaterialComponent>(&node) {
        let material = get_material(node);
        let item = SceneItem((node.clone(), world_matrix, material));
        mesh_nodes.push(item);
    }

    let node = node.read().unwrap();
    for child in &node.children {
        get_mesh_nodes(&world_matrix, child, mesh_nodes);
    }
}

fn create_dummy_program(gl: &Arc<glow::Context>) -> Option<Arc<RenderProgram>> {
    use glow::HasContext as _;

    unsafe {
        let shader_version = egui_glow::ShaderVersion::get(gl);

        //todo!("Implement create_dunny_program");
        let id = uuid::Uuid::new_v4();
        let program = gl.create_program().ok()?;

        let (vertex_shader_source, fragment_shader_source) = (
            r#"
                layout(location = 0) in vec3 position;   //
                //layout(location = 1) in vec3 normal;     //
                //layout(location = 2) in vec2 uv;         //

                out vec4 vertexColor;

                uniform mat4 local_to_world;
                uniform mat4 world_to_camera;
                uniform mat4 camera_to_clip;
                void main() {
                    //gl_Position = camera_to_clip * world_to_camera * local_to_world * vec4(position, 1);
                    gl_Position = vec4(position, 1) * local_to_world * world_to_camera * camera_to_clip;
                    vertexColor = vec4(1.0, 1.0, 1.0, 1.0);
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
        for key in ["position", "normal", "uv"].iter() {
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

fn correct_rotation(r: &Quaternion, up: &Vector3) -> Quaternion {
    let mut rotation = r.clone();
    let yy = up;
    let zz = rotation
        .to_matrix()
        .transform_vector(&Vector3::new(0.0, 0.0, 1.0))
        .normalize(); //forward
    let xx = Vector3::cross(&yy, &zz); //right
    if xx.length() > 0.0001 {
        let x2 = xx.normalize();
        let x1 = rotation
            .to_matrix()
            .transform_vector(&Vector3::new(1.0, 0.0, 0.0))
            .normalize();

        let c = Vector3::cross(&x1, &x2);
        if c.length() > 0.0001 {
            let theta = f32::asin(c.length());
            let axis = c.normalize();
            let q = Quaternion::from_angle_axis(theta, &axis);
            rotation = rotation * q;
            //let x3 = new_rotation.to_matrix().transform_vector(&Vector3::new(1.0, 0.0, 0.0));
            //println!("x1:{:?}, x2:{:?}, x3:{:?}",                x1, x2, x3);
        }
    }
    return rotation;
}

impl SceneView {
    pub fn new<'a>(
        cc: &'a eframe::CreationContext<'a>,
        controller: &Arc<RwLock<AppController>>,
    ) -> Option<Self> {
        let gl = cc.gl.as_ref()?;

        let program = create_dummy_program(gl)?;

        Some(Self {
            renderer: Some(Arc::new(Mutex::new(Renderer::new()))),
            controller: controller.clone(),
            gl: gl.clone(),
            test_program: program,
        })
    }

    fn get_root_node(&self) -> Arc<RwLock<Node>> {
        let controller = self.controller.read().unwrap();
        return controller.get_root_node();
    }

    fn get_render_mesh(
        resource_manager: &mut ResourceManager,
        gl: &Arc<glow::Context>,
        mesh: &Mesh,
    ) -> Option<Arc<RenderMesh>> {
        let id = mesh.get_id();
        if let Some(render_mesh) = resource_manager.get_mesh(id) {
            return Some(render_mesh.clone());
        } else {
            if let Some(render_mesh) = RenderMesh::from_mesh(gl, mesh) {
                let render_mesh = Arc::new(render_mesh);
                resource_manager.add_mesh(&render_mesh);
                return Some(render_mesh);
            }
        }
        return None;
    }

    fn get_render_items(&self) -> Vec<RenderItem> {
        let gl = self.gl.clone();

        let program = self.test_program.clone();

        let mut items = Vec::new();
        let mut mesh_items = Vec::new();
        let root_node = self.get_root_node();
        get_mesh_nodes(&Matrix4x4::identity(), &root_node, &mut mesh_items); //

        //let renderer = renderer.lock().unwrap();
        let mut root_node = root_node.write().unwrap();
        if root_node.get_component_mut::<ResourceComponent>().is_none() {
            root_node.add_component::<ResourceComponent>(ResourceComponent::new());
        }
        if let Some(component) = root_node.get_component::<ResourceComponent>() {
            let resource_manager = component.get_resource_manager();
            let mut resource_manager = resource_manager.lock().unwrap();
            for SceneItem((node, local_to_world, _m)) in mesh_items.iter() {
                let node = node.read().unwrap();

                let mut render_mesh = None;
                if let Some(component) = node.get_component::<MeshComponent>() {
                    if let Some(mesh) = component.mesh.as_ref() {
                        let mesh = mesh.read().unwrap();
                        render_mesh = Self::get_render_mesh(&mut resource_manager, &gl, &mesh);
                    }
                } else if let Some(component) = node.get_component::<SubdivComponent>() {
                    if let Some(mesh) = component.mesh.as_ref() {
                        let mesh = mesh.read().unwrap();
                        render_mesh = Self::get_render_mesh(&mut resource_manager, &gl, &mesh);
                    }
                } else if let Some(component) = node.get_component::<ShapeComponent>() {
                    if let Some(mesh) = component.mesh.as_ref() {
                        let mesh = mesh.read().unwrap();
                        let rm = Self::get_render_mesh(&mut resource_manager, &gl, &mesh);
                        if let Some(rm) = rm.as_ref() {
                            render_mesh = Some(rm.clone());
                            if rm.edition
                                != mesh
                                    .as_property_map()
                                    .find_one_string("edition")
                                    .unwrap_or("".to_string())
                            {
                                if let Some(new_mesh) = RenderMesh::from_mesh(&gl, &mesh) {
                                    let new_mesh = Arc::new(new_mesh);
                                    resource_manager.add_mesh(&new_mesh);
                                    render_mesh = Some(new_mesh.clone());
                                }
                            }
                        }
                    }
                }

                if let Some(render_mesh) = render_mesh.as_ref() {
                    items.push(RenderItem {
                        local_to_world: *local_to_world,
                        mesh: render_mesh.clone(),
                        program: program.clone(),
                    });
                }
            }
        }
        items
    }

    fn get_scene_up(node: &Arc<RwLock<Node>>) -> Vector3 {
        let node = node.read().unwrap();
        if let Some(cs) = node.get_component::<CoordinateSystemComponent>() {
            let up = cs.up;
            return up;
        } else {
            return Vector3::new(0.0, 1.0, 0.0);
        }
    }

    pub fn react_response(&mut self, response: &egui::Response) {
        if response.dragged_by(egui::PointerButton::Primary) {
            let controller = self.controller.clone();
            let controller = controller.read().unwrap();
            let root_node = controller.get_root_node();

            let up = Self::get_scene_up(&root_node);

            if let Some(camera_node) = Node::find_node_by_component::<CameraComponent>(&root_node) {
                let mut camera_node = camera_node.write().unwrap();
                if let Some(component) = camera_node.get_component_mut::<TransformComponent>() {
                    let rotation_y = -response.drag_motion().x * 0.01;
                    let rotation_x = -response.drag_motion().y * 0.01;
                    {
                        let (t, r, s) = component.get_local_trs(); //local_to_world
                        //let rotation = Quaternion::from_angle_axis(rotation_y, &Vector3::new(1.0, 0.0, 0.0)) * Quaternion::from_angle_axis(rotation_x, &Vector3::new(1.0, 0.0, 0.0));
                        let rotation =
                            Quaternion::from_euler_angles(rotation_x, rotation_y, 0.0).normalize();
                        let mut new_rotation = r * rotation;

                        if false {
                            //correct_rotation
                            new_rotation = correct_rotation(&new_rotation, &up);
                        }

                        component.set_local_trs(t, new_rotation, s);
                    }
                }
            }
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        let available_rect = ui.available_rect_before_wrap();
        let available_size = available_rect.size();

        ui.painter()
            .rect_filled(available_rect, 0.0, egui::Color32::from_rgb(0, 0, 64));
        let (rect, response) = ui.allocate_exact_size(available_size, egui::Sense::drag());
        self.react_response(&response);

        let items = self.get_render_items();

        //let msg = format!("Mesh count: {}", items.len());
        // ui.label(&msg);
        let mut fov = 90.0f32.to_radians();
        let mut w2c = Matrix4x4::identity();
        let mut render_size = Vec2::new(1280.0, 720.0);
        {
            let root_node = self.get_root_node();
            if let Some(camera_node) = Node::find_node_by_component::<CameraComponent>(&root_node) {
                let camera_node = camera_node.read().unwrap();
                if let Some(t) = camera_node.get_component::<TransformComponent>() {
                    let local_to_world = t.get_local_matrix();
                    w2c = local_to_world.inverse().unwrap();
                }
                if let Some(camera) = camera_node.get_component::<CameraComponent>() {
                    if let Some(prop) = camera.props.get("fov") {
                        if let Property::Floats(f) = prop {
                            if f.len() > 0 {
                                fov = f[0].to_radians();
                            }
                        }
                    }
                }
                if let Some(film) = camera_node.get_component::<FilmComponent>() {
                    let width = film
                        .props
                        .find_one_int("integer xresolution")
                        .unwrap_or(1280);
                    let height = film
                        .props
                        .find_one_int("integer yresolution")
                        .unwrap_or(720);
                    render_size = Vec2::new(width as f32, height as f32);
                }
            }
        }

        {
            let scale_x = available_size.x / render_size.x;
            let scale_y = available_size.y / render_size.y;
            let scale = scale_x.min(scale_y);
            let scaled_size = Vec2::new(render_size.x * scale, render_size.y * scale);
            let scaled_rect = egui::Rect::from_min_size(
                available_rect.min + (available_size - scaled_size) / 2.0,
                scaled_size,
            );
            ui.painter()
                .rect_filled(scaled_rect, 0.0, egui::Color32::from_rgb(0, 0, 128));
            ui.painter().rect_stroke(
                scaled_rect,
                0.0,
                egui::Stroke::new(1.0, egui::Color32::WHITE),
                egui::StrokeKind::Inside,
            );

            let vertical_fov = if scaled_size.x < scaled_size.y {
                // portrait mode
                let k = scaled_size.x / (fov / 2.0).tan(); //tan = y / x
                2.0 * f32::atan2(available_size.y, k)
            } else {
                // landscape mode
                let k = scaled_size.y / (fov / 2.0).tan(); //tan = y / x
                2.0 * f32::atan2(available_size.y, k)
            };
            fov = vertical_fov;
        }

        let aspect = rect.width() / rect.height();
        let c2c = Matrix4x4::perspective(fov, aspect, 0.1, 1000.0);

        if let Some(renderer) = &self.renderer {
            let renderer = renderer.clone();
            let gl = self.gl.clone();
            let cb = egui_glow::CallbackFn::new(move |_info, painter| {
                let mut renderer = renderer.lock().unwrap();
                if items.len() > 0 {
                    renderer.render(&gl, &w2c, &c2c, &items);
                }
            });

            let callback = egui::PaintCallback {
                rect,
                callback: Arc::new(cb),
            };
            ui.painter().add(callback);
        }
    }

    pub fn set_render_mode(&mut self, mode: RenderMode) {
        if let Some(renderer) = &self.renderer {
            let mut renderer = renderer.lock().unwrap();
            renderer.render_mode = mode;
        }
    }
}
