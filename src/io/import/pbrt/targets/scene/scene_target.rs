use super::super::super::parse::ParseTarget;
use super::graphics_state::GraphicsState;
use super::render_options::RenderOptions;
use super::transform::Transform;
use super::transform::TransformBit;
use super::transform::TransformSet;
use crate::model::base::Matrix4x4;
use crate::model::base::ParamSet;
use crate::model::base::Property;
use crate::model::base::Vector3;
use crate::model::scene;
use crate::model::scene::AcceleratorComponent;
use crate::model::scene::AnimationComponent;
use crate::model::scene::AreaLightComponent;
use crate::model::scene::CameraComponent;
use crate::model::scene::Component;
use crate::model::scene::CoordinateSystemComponent;
use crate::model::scene::FilmComponent;
use crate::model::scene::IntegratorComponent;
use crate::model::scene::LightComponent;
use crate::model::scene::MaterialComponent;
use crate::model::scene::Node;
use crate::model::scene::OtherResource;
use crate::model::scene::ResourceComponent;
use crate::model::scene::ResourceObject;
use crate::model::scene::SamplerComponent;
use crate::model::scene::ShapeComponent;
use crate::model::scene::TransformComponent;

use crate::model::scene::Material;
use crate::model::scene::Shape;
use crate::model::scene::Texture;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::RwLock;
use std::vec;

use convert_case::{Case, Casing};
use uuid::Uuid;

#[derive(Debug, PartialEq)]
enum APIState {
    OptionsBlock,
    WorldBlock,
}

pub struct SceneTarget {
    api_state: APIState,
    nodes: Vec<Arc<RwLock<Node>>>,
    transforms: Vec<TransformSet>,
    graphics_states: Vec<GraphicsState>,
    render_options: RenderOptions,
    named_coordinate_systems: HashMap<String, TransformSet>,
    meshes: HashMap<String, Arc<RwLock<Shape>>>,
    textures: HashMap<Uuid, Arc<RwLock<Texture>>>,
    image_textures: HashMap<String, Arc<RwLock<Texture>>>,
    materials: HashMap<Uuid, Arc<RwLock<Material>>>,
    resources: HashMap<String, Arc<RwLock<dyn ResourceObject>>>,
    work_dirs: Vec<String>,
}

fn create_default_material() -> Arc<RwLock<Material>> {
    let params = ParamSet::new();
    //params.insert("Kd", Property::Floats(vec![0.5, 0.5, 0.5]));
    Arc::new(RwLock::new(Material::new("Matte", "matte", &params)))
}

fn coordinate_system(d1: &Vector3) -> (Vector3, Vector3, Vector3) {
    let v1 = d1.normalize();
    if f32::abs(v1.x) > f32::abs(v1.y) {
        let v2 = Vector3::new(-v1.z, 0.0, v1.x).normalize();
        let v3 = Vector3::cross(&v1, &v2).normalize();
        return (v1, v2, v3);
    } else {
        let v2 = Vector3::new(0.0, v1.z, -v1.y).normalize();
        let v3 = Vector3::cross(&v1, &v2).normalize();
        return (v1, v2, v3);
    }
}

impl Default for SceneTarget {
    fn default() -> Self {
        let nodes = vec![Node::root_node("Scene")];
        let transforms = vec![TransformSet::new()];
        let mat = create_default_material();
        let mut graphics_states = vec![GraphicsState::default()];
        graphics_states[0].current_material = Some(mat.clone());
        let mut materials = HashMap::new();
        materials.insert(mat.read().unwrap().get_id(), mat.clone());
        SceneTarget {
            api_state: APIState::OptionsBlock,
            nodes: nodes,
            transforms: transforms,
            graphics_states: graphics_states,
            render_options: RenderOptions::default(),
            named_coordinate_systems: HashMap::new(),
            meshes: HashMap::new(),
            textures: HashMap::new(),
            image_textures: HashMap::new(),
            materials: materials,
            resources: HashMap::new(),
            work_dirs: Vec::new(),
        }
    }
}

impl SceneTarget {
    pub fn get_current_transform(&mut self) -> &mut TransformSet {
        self.transforms.last_mut().unwrap()
    }

    fn find_file_path(&self, filename: &str) -> Option<String> {
        if self.work_dirs.len() > 0 {
            for dir in self.work_dirs.iter().rev() {
                let path = Path::new(dir).join(filename);
                if path.exists() {
                    return Some(path.to_str().unwrap().to_string());
                }
            }
        }
        return None;
    }

    fn get_current_local_matrix(&self) -> Matrix4x4 {
        let len = self.transforms.len();
        if len == 0 {
            return Matrix4x4::identity();
        } else if len == 1 {
            return self.transforms[0].get_world_matrix();
        } else {
            let current_world = self.transforms[len - 1].get_world_matrix();
            let parent_inverse_world = self.transforms[len - 2].get_world_inverse_matrix();
            let cuurent_local = parent_inverse_world * current_world;
            return cuurent_local;
        }
    }

    fn create_child_node(&self, name: &str) -> Arc<RwLock<Node>> {
        let node = Node::child_node(name, self.nodes.last().unwrap());
        {
            let mut node = node.write().unwrap();
            if let Some(component) = node.get_component_mut::<TransformComponent>() {
                let local_matrix = self.get_current_local_matrix();
                component.set_local_matrix(local_matrix);
            }
        }
        return node;
    }

    fn register_texture(&mut self, texture: &Arc<RwLock<Texture>>) {
        let (id, name) = {
            let tex = texture.read().unwrap();
            (tex.get_id(), tex.get_name())
        };

        // Set define order of the texture
        {
            let order = self.textures.len(); //
            let mut texture = texture.write().unwrap();
            texture.set_order(order as i32);
        }

        let attr = self.graphics_states.last_mut().unwrap();
        if let Some(_existed) = attr.textures.get(&name) {
            log::warn!("Texture {} already exists", name);
        } else {
            attr.textures.insert(name.to_string(), texture.clone());
            self.textures.insert(id, texture.clone());
        }
    }

    fn register_other_resources(&mut self, params: &ParamSet) {
        if let Some(filename) = params.find_one_string("string bsdffile") {
            if let Some(fullpath) = self.find_file_path(filename.as_str()) {
                match std::path::absolute(fullpath) {
                    Ok(fullpath) => {
                        let name = fullpath
                            .as_path()
                            .file_stem()
                            .unwrap()
                            .to_str()
                            .unwrap()
                            .to_string();
                        let fullpath = fullpath.to_str().unwrap().to_string();
                        if !self.resources.contains_key(&fullpath) {
                            let mut new_params = ParamSet::default();
                            new_params.add_string("string type", "bsdffile"); //
                            new_params.add_string("string filename", &filename);
                            new_params.add_string("string fullpath", &fullpath);
                            let resource =
                                Arc::new(RwLock::new(OtherResource::new(&name, &new_params)));
                            self.resources
                                .insert(fullpath.to_string(), resource.clone());
                        }
                    }
                    Err(e) => {
                        log::warn!("filename error: {}", e);
                    }
                }
            }
        }

        if let Some(filename) = params.find_one_string("string lensfile") {
            if let Some(fullpath) = self.find_file_path(filename.as_str()) {
                match std::path::absolute(fullpath) {
                    Ok(fullpath) => {
                        let name = fullpath
                            .as_path()
                            .file_stem()
                            .unwrap()
                            .to_str()
                            .unwrap()
                            .to_string();
                        let fullpath = fullpath.to_str().unwrap().to_string();
                        if !self.resources.contains_key(&fullpath) {
                            let mut new_params = ParamSet::default();
                            new_params.add_string("string type", "lensfile"); //
                            new_params.add_string("string filename", &filename);
                            new_params.add_string("string fullpath", &fullpath);
                            let resource =
                                Arc::new(RwLock::new(OtherResource::new(&name, &new_params)));
                            self.resources
                                .insert(fullpath.to_string(), resource.clone());
                        }
                    }
                    Err(e) => {
                        log::warn!("filename error: {}", e);
                    }
                }
            }
        }

        {
            for (key_type, key_name) in params.get_keys().iter() {
                if key_type == "spectrum" {
                    if let Some(filename) = params.find_one_string(&key_name) {
                        if let Some(fullpath) = self.find_file_path(filename.as_str()) {
                            match std::path::absolute(fullpath) {
                                Ok(fullpath) => {
                                    let name = fullpath
                                        .as_path()
                                        .file_stem()
                                        .unwrap()
                                        .to_str()
                                        .unwrap()
                                        .to_string();
                                    let fullpath = fullpath.to_str().unwrap().to_string();
                                    if !self.resources.contains_key(&fullpath) {
                                        let mut new_params = ParamSet::default();
                                        new_params.add_string("string type", "spd"); //
                                        new_params.add_string("string filename", &filename);
                                        new_params.add_string("string fullpath", &fullpath);

                                        let resource = Arc::new(RwLock::new(OtherResource::new(
                                            &name,
                                            &new_params,
                                        )));
                                        self.resources
                                            .insert(fullpath.to_string(), resource.clone());
                                    }
                                }
                                Err(e) => {
                                    log::warn!("filename error: {}", e);
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some(filename) = params.find_one_string("string mapname") {
            if let Some(fullpath) = self.find_file_path(filename.as_str()) {
                match std::path::absolute(fullpath) {
                    Ok(fullpath) => {
                        let name = fullpath
                            .as_path()
                            .file_stem()
                            .unwrap()
                            .to_str()
                            .unwrap()
                            .to_string();
                        let fullpath = fullpath.to_str().unwrap().to_string();

                        if !self.image_textures.contains_key(&fullpath) {
                            let transform = Matrix4x4::identity();
                            let mut new_params = ParamSet::default();
                            new_params.add_string("string filename", &filename);
                            new_params.add_string("string fullpath", &fullpath);
                            let texture = Arc::new(RwLock::new(Texture::new(
                                &name,
                                "spectrum",
                                "imagemap",
                                Some(&fullpath),
                                &new_params,
                                &transform,
                            )));
                            self.textures
                                .insert(texture.read().unwrap().get_id(), texture.clone());
                            self.image_textures
                                .insert(fullpath.to_string(), texture.clone());
                        }
                    }
                    Err(e) => {
                        log::warn!("filename error: {}", e);
                    }
                }
            }
        }
    }

    fn add_fullpath_params(&self, params: &mut ParamSet) {
        let mut new_props = vec![];
        for (key_type, key_name) in params.get_keys().iter() {
            if key_type == "spectrum" {
                if let Some(filename) = params.find_one_string(&key_name) {
                    if let Some(fullpath) = self.find_file_path(filename.as_str()) {
                        match std::path::absolute(fullpath) {
                            Ok(fullpath) => {
                                let fullpath = fullpath.to_str().unwrap().to_string();
                                let new_key = format!("{} {}_fullpath", key_type, key_name);
                                new_props.push((new_key.clone(), Property::from(fullpath.clone())));
                            }
                            Err(_) => {
                                //
                            }
                        }
                    }
                }
            }
        }
        for (key, value) in new_props.iter() {
            params.insert(key, value.clone());
        }
    }

    fn make_shape(&mut self, name: &str, params: &ParamSet) -> Option<Arc<RwLock<Node>>> {
        let shape_type = name.to_string();
        match shape_type.as_str() {
            "plymesh" => {
                let title = ShapeComponent::get_name_from_type(name);
                if let Some(filename) = params.find_one_string("filename") {
                    if let Some(fullpath) = self.find_file_path(filename.as_str()) {
                        match std::path::absolute(fullpath) {
                            Ok(fullpath) => {
                                let fullpath = fullpath.to_str().unwrap().to_string();
                                let filename = Path::new(&fullpath)
                                    .file_stem()
                                    .unwrap()
                                    .to_str()
                                    .unwrap()
                                    .to_string();
                                let mut params = params.clone();
                                params.insert("string fullpath", Property::from(fullpath.clone()));
                                let node = self.create_child_node(&title);
                                {
                                    let mut node = node.write().unwrap();
                                    if let Some(mesh) = self.meshes.get(&fullpath) {
                                        let component = ShapeComponent::with_shape(&mesh);
                                        node.add_component(component);
                                    } else {
                                        let component =
                                            ShapeComponent::new(&shape_type, &filename, &params);
                                        let mesh = component.get_shape();
                                        self.meshes.insert(fullpath.clone(), mesh);
                                        node.add_component(component);
                                    }
                                }
                                return Some(node);
                            }
                            Err(e) => {
                                log::warn!("PlyMesh filename error: {}", e);
                            }
                        }
                    }
                }
            }
            "trianglemesh" | "sphere" | "disk" | "cylinder" | "cone" | "paraboloid"
            | "hyperboloid" | "loopsubdiv" => {
                let title = ShapeComponent::get_name_from_type(name);
                let node = self.create_child_node(&title);
                {
                    let mut node = node.write().unwrap();
                    let component = ShapeComponent::new(&shape_type, &title, params);
                    node.add_component(component);
                }
                return Some(node);
            }
            _ => {
                log::warn!("Shape {} not supported", name);
            }
        }
        return None;
    }
}

impl ParseTarget for SceneTarget {
    fn cleanup(&mut self) {}
    fn identity(&mut self) {
        let t = Transform::identity();
        self.get_current_transform().set_transform(&t);
    }

    fn translate(&mut self, dx: f32, dy: f32, dz: f32) {
        let t = Transform::translate(dx, dy, dz);
        self.get_current_transform().mul_transform(&t);
    }

    fn rotate(&mut self, angle: f32, ax: f32, ay: f32, az: f32) {
        let t = Transform::rotate(angle, ax, ay, az);
        self.get_current_transform().mul_transform(&t);
    }

    fn scale(&mut self, sx: f32, sy: f32, sz: f32) {
        let t = Transform::scale(sx, sy, sz);
        self.get_current_transform().mul_transform(&t);
    }

    fn look_at(
        &mut self,
        ex: f32,
        ey: f32,
        ez: f32,
        lx: f32,
        ly: f32,
        lz: f32,
        ux: f32,
        uy: f32,
        uz: f32,
    ) {
        let t = Transform::look_at(ex, ey, ez, lx, ly, lz, ux, uy, uz);
        self.get_current_transform().mul_transform(&t);
    }

    fn concat_transform(&mut self, t: &[f32]) {
        if t.len() < 16 {
            log::warn!("ConcatTransform: invalid transform");
            return;
        }

        #[rustfmt::skip]
        let m = Matrix4x4::from([
            t[0], t[4], t[8], t[12],
            t[1], t[5], t[9], t[13],
            t[2], t[6], t[10], t[14],
            t[3], t[7], t[11], t[15],
        ]);
        if let Some(im) = m.inverse() {
            let t = Transform { m, im };
            self.get_current_transform().mul_transform(&t);
        } else {
            log::warn!("Transform inverse failed");
        }
    }

    fn transform(&mut self, t: &[f32]) {
        if t.len() < 16 {
            log::warn!("ConcatTransform: invalid transform");
            return;
        }

        #[rustfmt::skip]
        let m = Matrix4x4::from([
            t[0], t[4], t[8], t[12],
            t[1], t[5], t[9], t[13],
            t[2], t[6], t[10], t[14],
            t[3], t[7], t[11], t[15],
        ]);
        if let Some(im) = m.inverse() {
            let t = Transform { m, im };
            self.get_current_transform().set_transform(&t);
        } else {
            log::warn!("Transform inverse failed");
        }
    }

    fn coordinate_system(&mut self, name: &str) {
        let t = self.get_current_transform().clone();
        self.named_coordinate_systems.insert(name.to_string(), t);
    }

    fn coord_sys_transform(&mut self, name: &str) {
        if let Some(tcoord) = self.named_coordinate_systems.get(name) {
            let tcoord = tcoord.clone();
            let t = self.get_current_transform();
            *t = tcoord;
        } else {
            log::warn!("Coordinate system {} not found", name);
        }
    }

    fn active_transform_all(&mut self) {
        let ts = self.get_current_transform();
        ts.set_transform_bit(TransformBit::All);
    }

    fn active_transform_end_time(&mut self) {
        let ts = self.get_current_transform();
        ts.set_transform_bit(TransformBit::End);
    }

    fn active_transform_start_time(&mut self) {
        let ts = self.get_current_transform();
        ts.set_transform_bit(TransformBit::Start);
    }

    fn transform_times(&mut self, start: f32, end: f32) {
        let opts = &mut self.render_options;
        opts.transform_start_time = start;
        opts.transform_end_time = end;
    }

    fn pixel_filter(&mut self, name: &str, params: &ParamSet) {
        let opts = &mut self.render_options;
        opts.filter_name = name.to_string();
        opts.filter_params = params.clone();
    }

    fn film(&mut self, name: &str, params: &ParamSet) {
        let opts = &mut self.render_options;
        opts.film_name = name.to_string();
        opts.film_params = params.clone();
    }

    fn sampler(&mut self, name: &str, params: &ParamSet) {
        let opts = &mut self.render_options;
        opts.sampler_name = name.to_string();
        opts.sampler_params = params.clone();
    }

    fn accelerator(&mut self, name: &str, params: &ParamSet) {
        let opts = &mut self.render_options;
        opts.accelerator_name = name.to_string();
        opts.accelerator_params = params.clone();
    }

    fn integrator(&mut self, name: &str, params: &ParamSet) {
        let opts = &mut self.render_options;
        opts.integrator_name = name.to_string();
        opts.integrator_params = params.clone();
    }

    fn camera(&mut self, name: &str, params: &ParamSet) {
        self.register_other_resources(params);
        let opts = &mut self.render_options;
        opts.camera_name = name.to_string();
        opts.camera_params = params.clone();

        let t = self.get_current_transform().clone();
        //let mat = t.get_world_matrix();
        self.named_coordinate_systems
            .insert("camera".to_string(), t);
    }

    fn make_named_medium(&mut self, name: &str, params: &ParamSet) {}
    fn medium_interface(&mut self, inside_name: &str, outside_name: &str) {}

    fn world_begin(&mut self) {
        if self.named_coordinate_systems.get("camera").is_none() {
            let t = self.get_current_transform().clone();
            //let mat = t.get_world_matrix();
            self.named_coordinate_systems
                .insert("camera".to_string(), t);
        }
        self.api_state = APIState::WorldBlock;
        self.nodes.clear();
        self.nodes.push(Node::root_node("Scene"));
        self.transforms.clear();
        self.transforms.push(TransformSet::new());
        if let Some(camara_transform) = self.named_coordinate_systems.get("camera") {
            let w2c = camara_transform.get_world_matrix();
            let c2w = w2c.inverse().unwrap();
            {
                let node = self.create_child_node("Camera");
                let mut node = node.write().unwrap();
                {
                    if let Some(trans) = node.get_component_mut::<TransformComponent>() {
                        trans.set_local_matrix(c2w);
                    }
                }
                {
                    let component = CameraComponent::new(
                        &self.render_options.camera_name,
                        &self.render_options.camera_params,
                    );
                    node.add_component(component);
                }
            }
        }
    }

    fn attribute_begin(&mut self) {
        self.transform_begin();
        let new_graphics_state = self.graphics_states.last().unwrap().clone();
        self.graphics_states.push(new_graphics_state);
    }

    fn attribute_end(&mut self) {
        if self.graphics_states.len() > 1 {
            self.graphics_states.pop();
        } else {
            log::warn!("AttributeEnd without attribute begin");
        }
        self.transform_end();
    }

    fn transform_begin(&mut self) {
        {
            let new_node = self.create_child_node("Transform");
            self.nodes.push(new_node);
        }
        {
            let t = self.get_current_transform().clone();
            self.transforms.push(t);
        }
    }

    fn transform_end(&mut self) {
        if self.transforms.len() > 1 {
            self.transforms.pop();
            self.nodes.pop();
        } else {
            log::warn!("TransformEnd without attribute begin");
        }
    }

    fn texture(&mut self, name: &str, _type: &str, tex_name: &str, params: &ParamSet) {
        let mut params = params.clone();
        self.register_other_resources(&params);
        self.add_fullpath_params(&mut params);
        let t = self.get_current_transform().clone();
        let transform = t.get_world_matrix();
        if tex_name == "imagemap" {
            if let Some(filename) = params.find_one_string("string filename") {
                if let Some(filename) = self.find_file_path(filename.as_str()) {
                    let filepath = Path::new(&filename);
                    assert!(filepath.exists());
                    match std::path::absolute(filepath) {
                        Ok(fullpath) => {
                            let fullpath = fullpath.to_str();
                            let texture = Arc::new(RwLock::new(Texture::new(
                                name, _type, tex_name, fullpath, &params, &transform,
                            )));
                            self.register_texture(&texture);
                        }
                        Err(_) => {
                            log::warn!("Texture file not found");
                        }
                    }
                }
            }
        } else {
            let texture = Arc::new(RwLock::new(Texture::new(
                name, _type, tex_name, None, &params, &transform,
            )));
            self.register_texture(&texture);
        }
    }

    fn material(&mut self, mat_type: &str, params: &ParamSet) {
        self.register_other_resources(params);
        let name = mat_type.to_case(Case::UpperCamel);
        let mut material = Material::new(&name, mat_type, params);
        let id = material.get_id();
        let new_name = format!("{}_{}", name, id.to_string());
        material.set_name(&new_name);
        let material = Arc::new(RwLock::new(material));
        self.materials.insert(id, material.clone());

        let attr = self.graphics_states.last_mut().unwrap();
        attr.current_material = Some(material.clone()); //
    }

    fn make_named_material(&mut self, name: &str, params: &ParamSet) {
        self.register_other_resources(params);
        if let Some(mat_type) = params.find_one_string("string type") {
            let material = Material::new(name, &mat_type, params);
            let id = material.get_id();
            let material = Arc::new(RwLock::new(material));
            self.materials.insert(id, material.clone());

            let attr = self.graphics_states.last_mut().unwrap();
            if let Some(_) = attr.materials.get(name) {
                log::warn!("Material {} already exists", name);
            }
            attr.materials.insert(name.to_string(), material.clone());
        } else {
            log::warn!("Material type not found");
        }
    }

    fn named_material(&mut self, name: &str) {
        let attr = self.graphics_states.last_mut().unwrap();
        if name == "" || name == "none" {
            attr.current_material = None;
        } else if let Some(material) = attr.materials.get(name) {
            attr.current_material = Some(material.clone());
        } else {
            log::warn!("Material {} not found", name);
        }
    }

    fn light_source(&mut self, name: &str, params: &ParamSet) {
        self.register_other_resources(params);
        let title = LightComponent::get_name_from_type(name);
        let node = self.create_child_node(&title);
        let mut params = params.clone();
        if name == "point" {
            let mut has_value = false;
            if let Some(props) = params.get("point from") {
                has_value = true;
                if let Property::Floats(values) = props {
                    if values.len() == 3 {
                        let from = Matrix4x4::translate(values[0], values[1], values[2]);
                        {
                            let mut node = node.write().unwrap();
                            if let Some(component) = node.get_component_mut::<TransformComponent>()
                            {
                                let local_matrix = self.get_current_local_matrix();
                                let local_matrix = from * local_matrix;
                                component.set_local_matrix(local_matrix);
                            }
                        }
                    } else {
                        log::warn!("Light 'from' parameter should have 3 values");
                    }
                }
            }
            if has_value {
                params.remove("point from");
            }
        } else if name == "spot" || name == "distant" {
            let mut remove_keys = Vec::new();
            let mut matrix = Matrix4x4::identity();
            {
                let mut from = Vector3::zero();
                if let Some(props) = params.get("point from") {
                    remove_keys.push("point from".to_string());
                    if let Property::Floats(values) = props {
                        if values.len() == 3 {
                            from = Vector3::new(values[0], values[1], values[2]);
                        } else {
                            log::warn!("Light 'from' parameter should have 3 values");
                        }
                    }
                }
                let mut to = Vector3::new(0.0, 0.0, 1.0);
                if let Some(props) = params.get("point to") {
                    remove_keys.push("point to".to_string());
                    if let Property::Floats(values) = props {
                        if values.len() == 3 {
                            to = Vector3::new(values[0], values[1], values[2]);
                        } else {
                            log::warn!("Light 'to' parameter should have 3 values");
                        }
                    }
                }
                let dir = to - from;
                let (dir, du, dv) = coordinate_system(&dir);
                let dir_to_z = Matrix4x4::new(
                    du.x, du.y, du.z, 0.0, dv.x, dv.y, dv.z, 0., dir.x, dir.y, dir.z, 0.0, 0.0,
                    0.0, 0.0, 1.0,
                );
                matrix = Matrix4x4::translate(from.x, from.y, from.z) * dir_to_z.transpose();
            }
            if !remove_keys.is_empty() {
                {
                    let mut node = node.write().unwrap();
                    if let Some(component) = node.get_component_mut::<TransformComponent>() {
                        let local_matrix = self.get_current_local_matrix();
                        let local_matrix = local_matrix * matrix;
                        component.set_local_matrix(local_matrix);
                    }
                }
                for keys in remove_keys.iter() {
                    params.remove(keys);
                }
            }
        }

        {
            let mut node = node.write().unwrap();
            let component = LightComponent::new(name, &params);
            node.add_component(component);
        }
    }

    fn area_light_source(&mut self, name: &str, params: &ParamSet) {
        self.register_other_resources(params);
        let attr = self.graphics_states.last_mut().unwrap();
        attr.area_light = Some((name.to_string(), params.clone()));
    }

    fn shape(&mut self, name: &str, params: &ParamSet) {
        let ts = self.get_current_transform().clone();
        if !ts.is_animated() {
            if let Some(node) = self.make_shape(name, params) {
                let mut node = node.write().unwrap();
                let attr = self.graphics_states.last().unwrap();
                if let Some(material) = attr.current_material.as_ref() {
                    let component = MaterialComponent::from_material(material);
                    node.add_component(component);
                }
                if let Some((light_type, light_params)) = attr.area_light.as_ref() {
                    node.set_name("AreaLight");
                    let component = AreaLightComponent::new(&light_type, light_params);
                    node.add_component(component);
                }
            }
        } else {
            {
                let attr = self.graphics_states.last().unwrap();
                if let Some(_) = attr.area_light.as_ref() {
                    log::warn!("Area light source cannot be animated");
                }
            }
            if let Some(node) = self.make_shape(name, params) {
                let mut node = node.write().unwrap();
                let attr = self.graphics_states.last().unwrap();
                if let Some(material) = attr.current_material.as_ref() {
                    let component = MaterialComponent::from_material(material);
                    node.add_component(component);
                }
                {
                    let m0 = ts.transforms[0].m; //world todo
                    let m1 = ts.transforms[1].m; //world todo
                    //

                    let start_time = self.render_options.transform_start_time;
                    let end_time = self.render_options.transform_end_time;
                    let component = AnimationComponent::new(&m0, start_time, &m1, end_time);
                    node.add_component(component);
                }
            }
        }
    }
    fn reverse_orientation(&mut self) {}
    fn object_begin(&mut self, name: &str) {}
    fn object_end(&mut self) {}
    fn object_instance(&mut self, name: &str) {}
    fn world_end(&mut self) {}

    fn parse_file(&mut self, filename: &str) {
        //
    }
    fn parse_string(&mut self, s: &str) {
        //
    }

    //----------------------------------------
    fn work_dir_begin(&mut self, path: &str) {
        self.work_dirs.push(path.to_string());
    }
    fn work_dir_end(&mut self) {
        if self.work_dirs.len() > 0 {
            self.work_dirs.pop();
        } else {
            log::warn!("WorkDirEnd without work dir begin");
        }
    }

    fn include(&mut self, _filename: &str, _params: &ParamSet) {
        //
    }
    //----------------------------------------
}

fn find_node_by<T: Component>(node: &Arc<RwLock<Node>>) -> Option<Arc<RwLock<Node>>> {
    let n = node.read().unwrap();
    if let Some(_) = n.get_component::<T>() {
        return Some(node.clone());
    }
    for child in n.children.iter() {
        if let Some(found_node) = find_node_by::<T>(child) {
            return Some(found_node);
        }
    }
    None
}

impl SceneTarget {
    pub fn create_scene_node(&self) -> Arc<RwLock<Node>> {
        let root_node = self.nodes[0].clone();
        {
            let props = ParamSet::new();
            let scene = scene::SceneComponent::new(&props);
            let mut root_node = root_node.write().unwrap();
            root_node.add_component(scene);
        }

        {
            let sampler = SamplerComponent::new(
                &self.render_options.sampler_name,
                &self.render_options.sampler_params,
            );
            let mut root_node = root_node.write().unwrap();
            root_node.add_component(sampler);
        }

        {
            let accelerator = AcceleratorComponent::new(
                &self.render_options.accelerator_name,
                &self.render_options.accelerator_params,
            );
            let mut root_node = root_node.write().unwrap();
            root_node.add_component(accelerator);
        }

        {
            let integrator = IntegratorComponent::new(
                &self.render_options.integrator_name,
                &self.render_options.integrator_params,
            );
            let mut root_node = root_node.write().unwrap();
            root_node.add_component(integrator);
        }

        {
            if let Some(camera_node) = find_node_by::<CameraComponent>(&root_node) {
                {
                    let mut camera_node = camera_node.write().unwrap();
                    {
                        let film = FilmComponent::new(
                            &self.render_options.film_name,
                            &self.render_options.film_params,
                        );
                        camera_node.add_component(film);
                    }
                }
                {
                    let camera_node = camera_node.read().unwrap();
                    if let Some(component) = camera_node.get_component::<TransformComponent>() {
                        let m = component.get_local_matrix(); //local_to_world
                        let up = m.transform_vector(&Vector3::new(0.0, 1.0, 0.0));
                        let up = [up.x, up.y, up.z];
                        let mut index = 0;
                        for i in 1..3 {
                            if up[i].abs() > up[index].abs() {
                                index = i;
                            }
                        }
                        let sign = up[index].signum();
                        let mut up = [0.0, 0.0, 0.0];
                        up[index] = sign;
                        log::info!("Camera up: {:?}", up);
                        let cs = CoordinateSystemComponent::new(&Vector3::new(up[0], up[1], up[2]));
                        let mut root_node = root_node.write().unwrap();
                        root_node.add_component(cs);
                    }
                }
            }
        }

        {
            let resource_component = ResourceComponent::new();
            {
                let resource_manager = resource_component.get_resource_manager();
                let mut resource_manager = resource_manager.write().unwrap();
                for (id, material) in self.materials.iter() {
                    resource_manager
                        .materials
                        .insert(id.clone(), material.clone());
                }
                for (_path, mesh) in self.meshes.iter() {
                    let id = mesh.read().unwrap().get_id();
                    resource_manager.meshes.insert(id, mesh.clone());
                }
                for (id, texture) in self.textures.iter() {
                    resource_manager
                        .textures
                        .insert(id.clone(), texture.clone());
                }
                for (_path, resource) in self.resources.iter() {
                    let id = resource.read().unwrap().get_id();
                    resource_manager
                        .other_resources
                        .insert(id, resource.clone());
                }
            }
            let mut root_node = root_node.write().unwrap();
            root_node.add_component(resource_component);
        }
        return root_node;
    }
}
