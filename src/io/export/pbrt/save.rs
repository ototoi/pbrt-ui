use super::super::copy_utility;
use crate::error::PbrtError;
use crate::model::base::Matrix4x4;
use crate::model::base::ParamSet;
use crate::model::base::Property;
use crate::model::base::Vector3;
//use crate::model::scene::AcceleratorProperties;
use crate::model::scene::AreaLightComponent;
use crate::model::scene::CameraComponent;
use crate::model::scene::CameraProperties;
use crate::model::scene::FilmComponent;
use crate::model::scene::IntegratorComponent;
use crate::model::scene::IntegratorProperties;
use crate::model::scene::LightComponent;
use crate::model::scene::LightProperties;
use crate::model::scene::MappingProperties;
use crate::model::scene::Material;
use crate::model::scene::MaterialComponent;
use crate::model::scene::MaterialProperties;
use crate::model::scene::Node;
use crate::model::scene::OptionProperties;
use crate::model::scene::ResourceComponent;
use crate::model::scene::SamplerComponent;
use crate::model::scene::SamplerProperties;
use crate::model::scene::ShapeComponent;
use crate::model::scene::ShapeProperties;
use crate::model::scene::TextureProperties;
use crate::model::scene::TransformComponent;

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::RwLock;

#[derive(Debug, Clone)]
pub struct SavePbrtOptions {
    pub pretty_print: bool,
    pub copy_resources: bool,
}

impl Default for SavePbrtOptions {
    fn default() -> Self {
        Self {
            pretty_print: true,
            copy_resources: true,
        }
    }
}

struct PbrtSaver {
    options: SavePbrtOptions,
}

fn get_world_matrix(node: &Arc<RwLock<Node>>) -> Result<Matrix4x4, PbrtError> {
    let node = node.read().unwrap();
    if let Some(parent) = node.parent.as_ref() {
        let parent = parent.upgrade().unwrap();
        let parent_matrix = get_world_matrix(&parent)?;
        let local_matrix = node
            .get_component::<TransformComponent>()
            .ok_or(PbrtError::error("Transform is not found!"))?;
        return Ok(parent_matrix * local_matrix.get_local_matrix());
    } else {
        let local_matrix = node
            .get_component::<TransformComponent>()
            .ok_or(PbrtError::error("Transform is not found!"))?;
        return Ok(local_matrix.get_local_matrix());
    }
}

fn make_indent(indent: usize) -> String {
    let mut indent_str = String::new();
    for _ in 0..indent {
        indent_str.push_str("    ");
    }
    indent_str
}

fn near_equal(a: f32, b: f32, epsilon: f32) -> bool {
    (a - b).abs() < epsilon
}

fn write_transform(
    indent: usize,
    transform: &Matrix4x4,
    writer: &mut dyn Write,
) -> Result<(), PbrtError> {
    let indent_str = make_indent(indent);
    let (t, r, s) = transform
        .decompose(0.1)
        .ok_or(PbrtError::error("Decompose failed!"))?;
    if !near_equal(t.x, 0.0, 1e-6) || !near_equal(t.y, 0.0, 1e-6) || !near_equal(t.z, 0.0, 1e-6) {
        writer.write(format!("{}Translate {} {} {}\n", indent_str, t.x, t.y, t.z).as_bytes())?;
    }
    if !near_equal(r.w.abs(), 1.0, 1e-6) {
        let theta = (2.0 * f32::acos(r.w)).to_degrees();
        let axis = Vector3::new(r.x, r.y, r.z).normalize();
        writer.write(
            format!(
                "{}Rotate {} {} {} {}\n",
                indent_str, theta, axis.x, axis.y, axis.z
            )
            .as_bytes(),
        )?;
    }
    if !near_equal(s.x, 1.0, 1e-6) || !near_equal(s.y, 1.0, 1e-6) || !near_equal(s.z, 1.0, 1e-6) {
        writer.write(format!("{}Scale {} {} {}\n", indent_str, s.x, s.y, s.z).as_bytes())?;
    }
    Ok(())
}

fn get_material_ignore_keys(material: &Material) -> Vec<String> {
    let mut ignore_keys = Vec::new();
    if material.get_type() == "subsurface" {
        if let Some(name_value) = material.props.find_one_string("string name") {
            if !name_value.is_empty() {
                ignore_keys.push("sigma_a".to_string());
                ignore_keys.push("sigma_s".to_string());
            }
        }
    }
    ignore_keys
}

impl PbrtSaver {
    pub fn new(options: &SavePbrtOptions) -> Self {
        PbrtSaver {
            options: options.clone(),
        }
    }

    fn write_property(
        &self,
        indent: usize,
        key_type: &str,
        key_name: &str,
        init: &Property,
        props: &ParamSet,
        writer: &mut dyn Write,
    ) -> Result<(), PbrtError> {
        if let Some((kt, kn, value)) = props.entry(key_name) {
            if let Property::Floats(v) = value {
                let values = v
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                writer.write(format!(" \"{} {}\" [{}]", kt, kn, values).as_bytes())?;
            } else if let Property::Ints(v) = value {
                let values = v
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                writer.write(format!(" \"{} {}\" [{}]", kt, kn, values).as_bytes())?;
            } else if let Property::Strings(v) = value {
                let values = v
                    .iter()
                    .map(|v| format!("\"{}\"", v))
                    .collect::<Vec<_>>()
                    .join(" ");
                writer.write(format!(" \"{} {}\" [{}]", kt, kn, values).as_bytes())?;
            } else if let Property::Bools(v) = value {
                let values = v
                    .iter()
                    .map(|v| format!("\"{}\"", v.to_string()))
                    .collect::<Vec<_>>()
                    .join(" ");
                writer.write(format!(" \"{} {}\" [{}]", kt, kn, values).as_bytes())?;
            }
        }
        Ok(())
    }

    fn write_header(
        &mut self,
        _node: &Arc<RwLock<Node>>,
        writer: &mut dyn Write,
    ) -> Result<(), PbrtError> {
        writer.write("# Generated by pbrt-ui\n".as_bytes())?;

        Ok(())
    }

    fn write_camera_options(
        &self,
        node: &Arc<RwLock<Node>>,
        writer: &mut dyn Write,
    ) -> Result<(), PbrtError> {
        // Camera Transform
        // Camera
        // Film
        // PixelFilter
        let camera_node = Node::find_node_by_component::<CameraComponent>(node)
            .ok_or(PbrtError::error("Camera is not found!"))?;
        {
            let local_to_world = get_world_matrix(&camera_node)?;
            let world_to_local = local_to_world
                .inverse()
                .ok_or(PbrtError::error("Camera transform is not found!"))?;
            //let (t, r, s) = world_to_local
            //    .decompose(0.01)
            //    .ok_or(PbrtError::error("Camera transform is not found!"))?;
            write_transform(0, &world_to_local, writer)?;
        }
        {
            let camera_node = camera_node.read().unwrap();
            let camera_component = camera_node
                .get_component::<CameraComponent>()
                .ok_or(PbrtError::error("Camera is not found!"))?;
            let camera_type = camera_component
                .props
                .find_one_string("string type")
                .ok_or(PbrtError::error("Camera type is not found!"))?;
            let camera_properties = CameraProperties::get_instance();
            if let Some(props) = camera_properties.get(&camera_type) {
                writer.write(format!("Camera \"{}\"", camera_type).as_bytes())?;
                for (key_type, key_name, init, _range) in props.iter() {
                    self.write_property(
                        0,
                        key_type,
                        key_name,
                        init,
                        &camera_component.props,
                        writer,
                    )?;
                }
                writer.write("\n".as_bytes())?;
            }
        }
        {
            let camera_node = camera_node.read().unwrap();
            let film_component = camera_node
                .get_component::<FilmComponent>()
                .ok_or(PbrtError::error("Film is not found!"))?;
            let film_type = film_component
                .props
                .find_one_string("string type")
                .ok_or(PbrtError::error("Film type is not found!"))?;
            //println!("Film type: {}", film_type);
            let option_properties = OptionProperties::get_instance();
            if let Some(props) = option_properties.get("film") {
                writer.write(format!("Film \"{}\"", film_type).as_bytes())?;
                for (key_type, key_name, init) in props.iter() {
                    self.write_property(
                        0,
                        key_type,
                        key_name,
                        init,
                        &film_component.props,
                        writer,
                    )?;
                }
                writer.write("\n".as_bytes())?;
            }
        }
        {
            //let camera_node = camera_node.read().unwrap();
            //pixel filter
        }
        Ok(())
    }

    fn write_sampler_options(
        &self,
        node: &Arc<RwLock<Node>>,
        writer: &mut dyn Write,
    ) -> Result<(), PbrtError> {
        let node = node.read().unwrap();
        let sampler_component = node
            .get_component::<SamplerComponent>()
            .ok_or(PbrtError::error("Sampler is not found!"))?;
        let sampler_type = sampler_component
            .props
            .find_one_string("string type")
            .ok_or(PbrtError::error("Sampler type is not found!"))?;
        let sampler_properties = SamplerProperties::get_instance();
        if let Some(props) = sampler_properties.get(&sampler_type) {
            writer.write(format!("Sampler \"{}\"", sampler_type).as_bytes())?;
            for (key_type, key_name, init, _range) in props.iter() {
                self.write_property(
                    0,
                    key_type,
                    key_name,
                    init,
                    &sampler_component.props,
                    writer,
                )?;
            }
            writer.write("\n".as_bytes())?;
        }
        Ok(())
    }

    fn write_integrator_options(
        &self,
        node: &Arc<RwLock<Node>>,
        writer: &mut dyn Write,
    ) -> Result<(), PbrtError> {
        let node = node.read().unwrap();
        let integrator_component = node
            .get_component::<IntegratorComponent>()
            .ok_or(PbrtError::error("Integrator is not found!"))?;
        let integrator_type = integrator_component
            .props
            .find_one_string("string type")
            .ok_or(PbrtError::error("Integrator type is not found!"))?;
        let integrator_properties = IntegratorProperties::get_instance();
        if let Some(props) = integrator_properties.get(&integrator_type) {
            writer.write(format!("Integrator \"{}\"", integrator_type).as_bytes())?;
            for (key_type, key_name, init, _range) in props.iter() {
                self.write_property(
                    0,
                    key_type,
                    key_name,
                    init,
                    &integrator_component.props,
                    writer,
                )?;
            }
            writer.write("\n".as_bytes())?;
        }
        Ok(())
    }

    fn write_options_block(
        &mut self,
        node: &Arc<RwLock<Node>>,
        writer: &mut dyn Write,
    ) -> Result<(), PbrtError> {
        // Accelerator
        self.write_camera_options(node, writer)?;
        if self.options.pretty_print {
            writer.write("\n".as_bytes())?;
        }
        self.write_sampler_options(node, writer)?;
        if self.options.pretty_print {
            writer.write("\n".as_bytes())?;
        }
        self.write_integrator_options(node, writer)?;
        if self.options.pretty_print {
            writer.write("\n".as_bytes())?;
        }

        Ok(())
    }

    fn write_materials(
        &self,
        node: &Arc<RwLock<Node>>,
        writer: &mut dyn Write,
    ) -> Result<(), PbrtError> {
        let indent = 1;
        let node = node.read().unwrap();
        if let Some(resouces_component) = node.get_component::<ResourceComponent>() {
            let resource_manager = resouces_component.get_resource_manager();
            let resource_manager = resource_manager.read().unwrap();
            if resource_manager.materials.is_empty() {
                return Ok(());
            }
            if self.options.pretty_print {
                writer.write(format!("{}# Materials\n", make_indent(indent)).as_bytes())?;
            }
            let mut materials = resource_manager
                .materials
                .values()
                .map(|m| (m.read().unwrap().get_name().to_ascii_lowercase(), m))
                .collect::<Vec<_>>();
            materials.sort_by(|a, b| a.0.cmp(&b.0));
            let material_properties = MaterialProperties::get_instance();
            for (_name, material) in materials.iter() {
                let material = material.read().unwrap();
                //let id = material.get_id();
                let t = material.get_type();
                let name = material.get_name();

                let ignore_keys = get_material_ignore_keys(&material);
                if let Some(props) = material_properties.get(&t) {
                    writer.write(
                        format!("{}MakeNamedMaterial \"{}\"", make_indent(indent), name).as_bytes(),
                    )?;
                    writer.write(format!(" \"string type\" [\"{}\"]", t).as_bytes())?;
                    //writer.write(format!(" \"string id\" [\"{}\"]", id.to_string()).as_bytes())?;
                    for (key_type, key_name, init, _range) in props.iter() {
                        if ignore_keys.contains(key_name) {
                            continue;
                        }
                        self.write_property(0, key_type, key_name, init, &material.props, writer)?;
                    }
                    writer.write("\n".as_bytes())?;
                }
            }
        }
        Ok(())
    }

    fn write_textures(
        &self,
        node: &Arc<RwLock<Node>>,
        writer: &mut dyn Write,
    ) -> Result<(), PbrtError> {
        let mut indent = 1;
        let node = node.read().unwrap();
        if let Some(resouces_component) = node.get_component::<ResourceComponent>() {
            let resource_manager = resouces_component.get_resource_manager();
            let resource_manager = resource_manager.read().unwrap();
            if resource_manager.textures.is_empty() {
                return Ok(());
            }
            if self.options.pretty_print {
                writer.write(format!("{}# Textures\n", make_indent(indent)).as_bytes())?;
            }
            let mut textures = Vec::new();
            for texture in resource_manager.textures.values() {
                let order = texture.read().unwrap().get_order();
                textures.push((order, texture.clone()));
            }
            textures.sort_by(|a, b| a.0.cmp(&b.0));
            for (_order, texture) in textures.iter() {
                let texture = texture.read().unwrap();
                let texture_type = texture.get_type();
                let texture_name = texture.get_name();
                let color_type = texture.get_color_type();
                let transform = texture.get_transform();
                writer.write(format!("{}TransformBegin\n", make_indent(indent)).as_bytes())?;
                indent += 1;
                write_transform(indent, &transform, writer)?;
                writer.write(
                    format!(
                        "{}Texture \"{}\" \"{}\" \"{}\"",
                        make_indent(indent),
                        texture_name,
                        color_type,
                        texture_type
                    )
                    .as_bytes(),
                )?;
                //writer.write(format!(" \"string id\" [\"{}\"]", id.to_string()).as_bytes())?;
                /*
                writer.write("\n".as_bytes())?;
                for (key_type, key_name, init) in texture.props.0.iter() {
                    self.write_property(
                        indent,
                        key_type,
                        key_name,
                        init,
                        &texture.props,
                        writer,
                    )?;
                }
                writer.write("\n".as_bytes())?;
                */
                let texture_properties = TextureProperties::get_instance();
                if let Some(props) = texture_properties.get(&texture_type) {
                    for (key_type, key_name, init, _range) in props.iter() {
                        self.write_property(
                            indent,
                            key_type,
                            key_name,
                            init,
                            &texture.props,
                            writer,
                        )?;
                    }
                }
                let mapping_type = texture
                    .as_property_map()
                    .find_one_string("string mapping")
                    .unwrap_or("uv".to_string());
                {
                    let mapping_properties = MappingProperties::get_instance();
                    if let Some(props) = mapping_properties.get(&mapping_type) {
                        for (key_type, key_name, init, _range) in props.iter() {
                            self.write_property(
                                indent,
                                key_type,
                                key_name,
                                init,
                                &texture.props,
                                writer,
                            )?;
                        }
                    }
                }
                writer.write("\n".as_bytes())?;
                indent -= 1;
                writer.write(format!("{}TransformEnd\n", make_indent(indent)).as_bytes())?;
            }
        }
        Ok(())
    }

    fn write_geomtry(
        &self,
        indent: usize,
        node: &Arc<RwLock<Node>>,
        writer: &mut dyn Write,
    ) -> Result<(), PbrtError> {
        let node = node.read().unwrap();
        if let Some(m) = node.get_component::<MaterialComponent>() {
            let material_name = m.get_name();
            writer.write(
                format!(
                    "{}NamedMaterial \"{}\"\n",
                    make_indent(indent),
                    material_name
                )
                .as_bytes(),
            )?;
        }
        let shape_properties = ShapeProperties::get_instance();
        let light_properties = LightProperties::get_instance();
        if let Some(component) = node.get_component::<ShapeComponent>() {
            if let Some(light_component) = node.get_component::<AreaLightComponent>() {
                let light = light_component.get_light();
                let light = light.read().unwrap();
                let light = light.as_property_map();
                let t = light.find_one_string("string type").unwrap();
                if let Some(props) = light_properties.get(&t) {
                    writer.write(
                        format!("{}AreaLightSource \"{}\"", make_indent(indent), t).as_bytes(),
                    )?;
                    for (key_type, key_name, init, _range) in props.iter() {
                        self.write_property(indent, key_type, key_name, init, light, writer)?;
                    }
                    writer.write("\n".as_bytes())?;
                }
            }
            let shape = component.get_shape();
            let shape = shape.read().unwrap();
            let t = shape.get_type(); //
            if let Some(props) = shape_properties.get(&t) {
                writer.write(format!("{}Shape \"{}\"", make_indent(indent), t).as_bytes())?;
                for (key_type, key_name, init, _range) in props.iter() {
                    self.write_property(indent, key_type, key_name, init, &shape.props, writer)?;
                }
                writer.write("\n".as_bytes())?;
            }
        } else if let Some(light_component) = node.get_component::<LightComponent>() {
            let light = light_component.get_light();
            let light = light.read().unwrap();
            let light = light.as_property_map();
            let t = light.find_one_string("string type").unwrap();
            if let Some(props) = light_properties.get(&t) {
                writer.write(format!("{}LightSource \"{}\"", make_indent(indent), t).as_bytes())?;
                for (key_type, key_name, init, _range) in props.iter() {
                    self.write_property(indent, key_type, key_name, init, light, writer)?;
                }
                writer.write("\n".as_bytes())?;
            }
        }
        Ok(())
    }

    fn write_node(
        &self,
        indent: usize,
        node: &Arc<RwLock<Node>>,
        writer: &mut dyn Write,
    ) -> Result<(), PbrtError> {
        if node.read().unwrap().is_enabled() {
            writer.write(format!("{}AttributeBegin\n", make_indent(indent)).as_bytes())?;
            {
                let node = node.read().unwrap();
                let t = node
                    .get_component::<TransformComponent>()
                    .ok_or(PbrtError::error("Transform is not found!"))?;
                write_transform(indent + 1, &t.get_local_matrix(), writer)?;
            }
            {
                self.write_geomtry(indent + 1, node, writer)?;
            }

            let node = node.read().unwrap();
            for child in node.children.iter() {
                self.write_node(indent + 1, child, writer)?;
            }
            writer.write(format!("{}AttributeEnd\n", make_indent(indent)).as_bytes())?;
        }
        Ok(())
    }

    fn write_geomtries(
        &self,
        node: &Arc<RwLock<Node>>,
        writer: &mut dyn Write,
    ) -> Result<(), PbrtError> {
        let indent = 1;
        if self.options.pretty_print {
            writer.write(format!("{}# Geometries\n", make_indent(indent)).as_bytes())?;
        }
        let node = node.read().unwrap(); //world
        if node.is_enabled() {
            let t = node
                .get_component::<TransformComponent>()
                .ok_or(PbrtError::error("Transform is not found!"))?;
            write_transform(0, &t.get_local_matrix(), writer)?;
            for child in node.children.iter() {
                {
                    let node = child.read().unwrap();
                    if node.get_component::<CameraComponent>().is_some() {
                        continue;
                    }
                }
                self.write_node(1, child, writer)?;
            }
        }
        Ok(())
    }

    fn write_world_black(
        &mut self,
        node: &Arc<RwLock<Node>>,
        writer: &mut dyn Write,
    ) -> Result<(), PbrtError> {
        writer.write("WorldBegin\n".as_bytes())?;
        self.write_textures(node, writer)?;
        if self.options.pretty_print {
            writer.write("\n".as_bytes())?;
        }
        self.write_materials(node, writer)?;
        if self.options.pretty_print {
            writer.write("\n".as_bytes())?;
        }
        self.write_geomtries(node, writer)?;
        writer.write("WorldEnd\n".as_bytes())?;
        Ok(())
    }

    fn copy_resources(&mut self, node: &Arc<RwLock<Node>>, path: &str) -> Result<(), PbrtError> {
        if !self.options.copy_resources {
            return Ok(());
        }
        let node = node.read().unwrap();
        if let Some(resouces_component) = node.get_component::<ResourceComponent>() {
            let resource_manager = resouces_component.get_resource_manager();
            let resource_manager = resource_manager.read().unwrap();

            let out_dir = std::path::Path::new(path)
                .parent()
                .ok_or(PbrtError::error("Invalid path!"))?;
            std::fs::create_dir_all(out_dir)?;
            let mut copy_paths = Vec::new();
            //
            for (_id, texture) in resource_manager.textures.iter() {
                let texture = texture.read().unwrap();
                let texture_type = texture.get_type();
                if texture_type != "imagemap" {
                    continue;
                }
                let filename = texture.get_filename();
                let fullpath = texture.get_fullpath();
                if let (Some(filename), Some(fullpath)) = (filename, fullpath) {
                    let src_path = Path::new(&fullpath).to_path_buf();
                    let dst_path = out_dir.join(filename);
                    if src_path != dst_path && src_path.exists() {
                        copy_paths.push((src_path, dst_path));
                    }
                } else {
                    log::warn!(
                        "Texture {} does not have filename or fullpath!",
                        texture.get_name()
                    );
                }
            }
            //
            for (_id, mesh) in resource_manager.meshes.iter() {
                let mesh = mesh.read().unwrap();
                let mesh_type = mesh.get_type();
                if mesh_type != "plymesh" {
                    continue;
                }
                let filename = mesh.get_filename();
                let fullpath = mesh.get_fullpath();
                if let (Some(filename), Some(fullpath)) = (filename, fullpath) {
                    let src_path = Path::new(&fullpath).to_path_buf();
                    let dst_path = out_dir.join(filename);
                    if src_path != dst_path && src_path.exists() {
                        copy_paths.push((src_path, dst_path));
                    }
                } else {
                    log::warn!(
                        "Mesh {} does not have filename or fullpath!",
                        mesh.get_name()
                    );
                }
            }

            for (_id, other_resource) in resource_manager.other_resources.iter() {
                let other_resource = other_resource.read().unwrap();
                let filename = other_resource.get_filename();
                let fullpath = other_resource.get_fullpath();
                if let (Some(filename), Some(fullpath)) = (filename, fullpath) {
                    let src_path = Path::new(&fullpath).to_path_buf();
                    let dst_path = out_dir.join(filename);
                    if src_path != dst_path && src_path.exists() {
                        copy_paths.push((src_path, dst_path));
                    }
                } else {
                    log::warn!(
                        "Other resource {} does not have filename or fullpath!",
                        other_resource.get_name()
                    );
                }
            }

            for (src_path, dst_path) in copy_paths.iter() {
                if let Err(e) = copy_utility::copy_file(src_path, dst_path) {
                    println!(
                        "Failed to copy resource from {:?} to {:?}: {}",
                        src_path, dst_path, e
                    );
                }
            }
        }
        Ok(())
    }

    pub fn write(&mut self, node: &Arc<RwLock<Node>>, path: &str) -> Result<(), PbrtError> {
        let file = File::create(path)?;
        let mut writer = std::io::BufWriter::new(file);
        self.write_header(node, &mut writer)?;
        self.write_options_block(node, &mut writer)?;
        if self.options.pretty_print {
            writer.write("\n".as_bytes())?;
        }
        self.write_world_black(node, &mut writer)?;
        writer.flush()?;
        self.copy_resources(node, path)?;
        return Ok(());
    }
}

pub fn save_pbrt(
    node: &Arc<RwLock<Node>>,
    path: &str,
    options: &SavePbrtOptions,
) -> Result<(), PbrtError> {
    let mut pbrt_writer = PbrtSaver::new(options);
    pbrt_writer.write(node, path)?;

    return Ok(());
}
