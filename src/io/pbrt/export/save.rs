use crate::error::PbrtError;
use crate::io::pbrt::ParseTarget;
use crate::model::base::Matrix4x4;
use crate::model::base::PropertyMap as ParamSet;
use crate::model::base::Property;
use crate::model::base::Vector3;
use crate::model::scene::CameraComponent;
use crate::model::scene::CameraProperties;
use crate::model::scene::FilmComponent;
use crate::model::scene::IntegratorComponent;
use crate::model::scene::IntegratorProperties;
use crate::model::scene::Light;
use crate::model::scene::LightComponent;
use crate::model::scene::LightProperties;
use crate::model::scene::Material;
use crate::model::scene::MaterialComponent;
use crate::model::scene::MaterialProperties;
use crate::model::scene::Node;
use crate::model::scene::OptionProperties;
use crate::model::scene::PropertyEntry;
use crate::model::scene::ResourceComponent;
use crate::model::scene::SamplerComponent;
use crate::model::scene::SamplerProperties;
use crate::model::scene::Shape;
use crate::model::scene::ShapeComponent;
use crate::model::scene::ShapeProperties;
use crate::model::scene::TextureProperties;
use crate::model::scene::TransformComponent;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::RwLock;

pub type Float = f32;

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

pub struct SaveTarget<W: Write> {
    writer: W,
    indent_level: usize,
    pretty_print: bool,
    copy_resources: bool,
}

impl<W: Write> SaveTarget<W> {
    pub fn new(writer: W, options: &SavePbrtOptions) -> Self {
        Self {
            writer,
            indent_level: 0,
            pretty_print: options.pretty_print,
            copy_resources: options.copy_resources,
        }
    }

    fn write_line(&mut self, line: &str) -> std::io::Result<()> {
        if self.pretty_print {
            let indent = "    ".repeat(self.indent_level);
            write!(self.writer, "{}{}\n", indent, line)
        } else {
            write!(self.writer, "{}\n", line)
        }
    }

    fn write_command(&mut self, cmd: &str, args: &str) -> std::io::Result<()> {
        if args.is_empty() {
            self.write_line(cmd)
        } else {
            self.write_line(&format!("{} {}", cmd, args))
        }
    }

    fn format_params(&self, params: &ParamSet) -> String {
        let mut result = String::new();
        for (param_type, key, value) in params.0.iter() {
            if !result.is_empty() {
                result.push(' ');
            }
            result.push_str(&self.format_property(param_type, key, value));
        }
        result
    }

    fn format_property(&self, param_type: &str, key: &str, value: &Property) -> String {
        match value {
            Property::Floats(v) => {
                let values = v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" ");
                format!("\"{}\" [{}]", Self::make_param_key(param_type, key), values)
            }
            Property::Ints(v) => {
                let values = v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" ");
                format!("\"{}\" [{}]", Self::make_param_key(param_type, key), values)
            }
            Property::Strings(v) => {
                if v.len() == 1 {
                    format!("\"{}\" \"{}\"", Self::make_param_key(param_type, key), v[0])
                } else {
                    let values = v.iter().map(|s| format!("\"{}\"", s)).collect::<Vec<_>>().join(" ");
                    format!("\"{}\" [{}]", Self::make_param_key(param_type, key), values)
                }
            }
            Property::Bools(v) => {
                if v.len() == 1 {
                    let value_str = if v[0] { "true" } else { "false" };
                    format!("\"{}\" \"{}\"", Self::make_param_key(param_type, key), value_str)
                } else {
                    // Multiple bools as array
                    let values = v.iter().map(|b| if *b { "true" } else { "false" }).collect::<Vec<_>>().join(" ");
                    format!("\"{}\" [{}]", Self::make_param_key(param_type, key), values)
                }
            }
        }
    }

    fn make_param_key(param_type: &str, key: &str) -> String {
        if param_type.is_empty() {
            key.to_string()
        } else {
            format!("{} {}", param_type, key)
        }
    }

    pub fn finish(mut self) -> std::io::Result<W> {
        self.writer.flush()?;
        Ok(self.writer)
    }
}

impl<W: Write> ParseTarget for SaveTarget<W> {
    fn cleanup(&mut self) {
        // No-op for save target
    }

    fn identity(&mut self) {
        let _ = self.write_line("Identity");
    }

    fn translate(&mut self, dx: Float, dy: Float, dz: Float) {
        let _ = self.write_command("Translate", &format!("{} {} {}", dx, dy, dz));
    }

    fn rotate(&mut self, angle: Float, ax: Float, ay: Float, az: Float) {
        let _ = self.write_command("Rotate", &format!("{} {} {} {}", angle, ax, ay, az));
    }

    fn scale(&mut self, sx: Float, sy: Float, sz: Float) {
        let _ = self.write_command("Scale", &format!("{} {} {}", sx, sy, sz));
    }

    fn look_at(
        &mut self,
        ex: Float,
        ey: Float,
        ez: Float,
        lx: Float,
        ly: Float,
        lz: Float,
        ux: Float,
        uy: Float,
        uz: Float,
    ) {
        let _ = self.write_command(
            "LookAt",
            &format!("{} {} {} {} {} {} {} {} {}", ex, ey, ez, lx, ly, lz, ux, uy, uz),
        );
    }

    fn concat_transform(&mut self, transform: &[Float]) {
        if transform.len() == 16 {
            let values = transform.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" ");
            let _ = self.write_command("ConcatTransform", &format!("[{}]", values));
        }
    }

    fn transform(&mut self, transform: &[Float]) {
        if transform.len() == 16 {
            let values = transform.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" ");
            let _ = self.write_command("Transform", &format!("[{}]", values));
        }
    }

    fn coordinate_system(&mut self, name: &str) {
        let _ = self.write_command("CoordinateSystem", &format!("\"{}\"", name));
    }

    fn coord_sys_transform(&mut self, name: &str) {
        let _ = self.write_command("CoordSysTransform", &format!("\"{}\"", name));
    }

    fn active_transform_all(&mut self) {
        let _ = self.write_line("ActiveTransform All");
    }

    fn active_transform_end_time(&mut self) {
        let _ = self.write_line("ActiveTransform EndTime");
    }

    fn active_transform_start_time(&mut self) {
        let _ = self.write_line("ActiveTransform StartTime");
    }

    fn transform_times(&mut self, start: Float, end: Float) {
        let _ = self.write_command("TransformTimes", &format!("{} {}", start, end));
    }

    fn pixel_filter(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("PixelFilter", &format!("\"{}\" {}", name, params_str));
    }

    fn film(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("Film", &format!("\"{}\" {}", name, params_str));
    }

    fn sampler(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("Sampler", &format!("\"{}\" {}", name, params_str));
    }

    fn accelerator(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("Accelerator", &format!("\"{}\" {}", name, params_str));
    }

    fn integrator(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("Integrator", &format!("\"{}\" {}", name, params_str));
    }

    fn camera(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("Camera", &format!("\"{}\" {}", name, params_str));
    }

    fn make_named_medium(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("MakeNamedMedium", &format!("\"{}\" {}", name, params_str));
    }

    fn medium_interface(&mut self, inside_name: &str, outside_name: &str) {
        let _ = self.write_command("MediumInterface", &format!("\"{}\" \"{}\"", inside_name, outside_name));
    }

    fn world_begin(&mut self) {
        let _ = self.write_line("WorldBegin");
        if self.pretty_print {
            self.indent_level += 1;
        }
    }

    fn attribute_begin(&mut self) {
        let _ = self.write_line("AttributeBegin");
        if self.pretty_print {
            self.indent_level += 1;
        }
    }

    fn attribute_end(&mut self) {
        if self.pretty_print && self.indent_level > 0 {
            self.indent_level -= 1;
        }
        let _ = self.write_line("AttributeEnd");
    }

    fn transform_begin(&mut self) {
        let _ = self.write_line("TransformBegin");
        if self.pretty_print {
            self.indent_level += 1;
        }
    }

    fn transform_end(&mut self) {
        if self.pretty_print && self.indent_level > 0 {
            self.indent_level -= 1;
        }
        let _ = self.write_line("TransformEnd");
    }

    fn texture(&mut self, name: &str, _type: &str, tex_name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("Texture", &format!("\"{}\" \"{}\" \"{}\" {}", name, _type, tex_name, params_str));
    }

    fn material(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("Material", &format!("\"{}\" {}", name, params_str));
    }

    fn make_named_material(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("MakeNamedMaterial", &format!("\"{}\" {}", name, params_str));
    }

    fn named_material(&mut self, name: &str) {
        let _ = self.write_command("NamedMaterial", &format!("\"{}\"", name));
    }

    fn light_source(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("LightSource", &format!("\"{}\" {}", name, params_str));
    }

    fn area_light_source(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("AreaLightSource", &format!("\"{}\" {}", name, params_str));
    }

    fn shape(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("Shape", &format!("\"{}\" {}", name, params_str));
    }

    fn reverse_orientation(&mut self) {
        let _ = self.write_line("ReverseOrientation");
    }

    fn object_begin(&mut self, name: &str) {
        let _ = self.write_command("ObjectBegin", &format!("\"{}\"", name));
        if self.pretty_print {
            self.indent_level += 1;
        }
    }

    fn object_end(&mut self) {
        if self.pretty_print && self.indent_level > 0 {
            self.indent_level -= 1;
        }
        let _ = self.write_line("ObjectEnd");
    }

    fn object_instance(&mut self, name: &str) {
        let _ = self.write_command("ObjectInstance", &format!("\"{}\"", name));
    }

    fn world_end(&mut self) {
        if self.pretty_print && self.indent_level > 0 {
            self.indent_level -= 1;
        }
        let _ = self.write_line("WorldEnd");
    }

    fn parse_file(&mut self, filename: &str) {
        // For save target, we might want to write an Include directive
        let _ = self.write_command("Include", &format!("\"{}\"", filename));
    }

    fn parse_string(&mut self, _s: &str) {
        // No-op for save target
    }
}

// Helper functions
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

fn near_equal(a: f32, b: f32, epsilon: f32) -> bool {
    (a - b).abs() < epsilon
}

fn apply_transform<W: Write>(
    transform: &Matrix4x4,
    target: &mut SaveTarget<W>,
) -> Result<(), PbrtError> {
    let (t, r, s) = transform
        .decompose(0.1)
        .ok_or(PbrtError::error("Decompose failed!"))?;
    
    let epsilon = 1e-3;
    
    // Clean up near-zero values
    let tx = if t.x.abs() < epsilon { 0.0 } else { t.x };
    let ty = if t.y.abs() < epsilon { 0.0 } else { t.y };
    let tz = if t.z.abs() < epsilon { 0.0 } else { t.z };
    
    if tx != 0.0 || ty != 0.0 || tz != 0.0 {
        target.translate(tx, ty, tz);
    }
    
    if !near_equal(r.w.abs(), 1.0, epsilon) {
        let theta = (2.0 * f32::acos(r.w.clamp(-1.0, 1.0))).to_degrees();
        if theta.abs() > 0.01 && (360.0 - theta).abs() > 0.01 {
            let mut axis = Vector3::new(r.x, r.y, r.z).normalize();
            
            // Clean up near-zero axis components
            axis.x = if axis.x.abs() < epsilon { 0.0 } else { axis.x };
            axis.y = if axis.y.abs() < epsilon { 0.0 } else { axis.y };
            axis.z = if axis.z.abs() < epsilon { 0.0 } else { axis.z };
            
            target.rotate(theta, axis.x, axis.y, axis.z);
        }
    }
    
    if !near_equal(s.x, 1.0, epsilon) || !near_equal(s.y, 1.0, epsilon) || !near_equal(s.z, 1.0, epsilon) {
        target.scale(s.x, s.y, s.z);
    }
    Ok(())
}

fn get_material_ignore_keys(material: &Material) -> Vec<String> {
    let mut ignore_keys = Vec::new();
    if material.get_type() == "subsurface"
        && let Some(name_value) = material.as_property_map().find_one_string("string name")
        && !name_value.is_empty()
    {
        ignore_keys.push("sigma_a".to_string());
        ignore_keys.push("sigma_s".to_string());
    }
    ignore_keys
}

fn get_shape_ignore_keys(_shape: &Shape) -> Vec<String> {
    Vec::new()
}

fn get_light_ignore_keys(_light: &Light) -> Vec<String> {
    Vec::new()
}

// High-level write methods
impl<W: Write> SaveTarget<W> {
    fn collect_params_from_entries(
        &self,
        entries: &[PropertyEntry],
        props: &ParamSet,
    ) -> ParamSet {
        let mut params = ParamSet::new();
        for entry in entries.iter() {
            if !entry.output_to_file {
                continue;
            }
            if let Some((_kt, _kn, value)) = props.entry(&entry.key_name) {
                params.insert(&format!("{} {}", entry.key_type, entry.key_name), value.clone());
            }
        }
        params
    }

    fn write_header(&mut self, _node: &Arc<RwLock<Node>>) -> Result<(), PbrtError> {
        writeln!(self.writer, "# Generated by pbrt-ui")?;
        Ok(())
    }

    fn write_camera_options(&mut self, node: &Arc<RwLock<Node>>) -> Result<(), PbrtError> {
        let camera_node = Node::find_node_by_component::<CameraComponent>(node)
            .ok_or(PbrtError::error("Camera is not found!"))?;
        
        // Camera Transform
        {
            let local_to_world = get_world_matrix(&camera_node)?;
            let world_to_local = local_to_world
                .inverse()
                .ok_or(PbrtError::error("Camera transform is not found!"))?;
            apply_transform(&world_to_local, self)?;
        }
        
        // Camera
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
            if let Some(entries) = camera_properties.get_entries(&camera_type) {
                let params = self.collect_params_from_entries(entries, &camera_component.props);
                self.camera(&camera_type, &params);
            }
        }
        
        // Film
        {
            let camera_node = camera_node.read().unwrap();
            let film_component = camera_node
                .get_component::<FilmComponent>()
                .ok_or(PbrtError::error("Film is not found!"))?;
            let film_type = film_component
                .props
                .find_one_string("string type")
                .ok_or(PbrtError::error("Film type is not found!"))?;
            let option_properties = OptionProperties::get_instance();
            if let Some(entries) = option_properties.get_entries("film") {
                let params = self.collect_params_from_entries(entries, &film_component.props);
                self.film(&film_type, &params);
            }
        }
        
        Ok(())
    }

    fn write_sampler_options(&mut self, node: &Arc<RwLock<Node>>) -> Result<(), PbrtError> {
        let node = node.read().unwrap();
        let sampler_component = node
            .get_component::<SamplerComponent>()
            .ok_or(PbrtError::error("Sampler is not found!"))?;
        let sampler_type = sampler_component
            .props
            .find_one_string("string type")
            .ok_or(PbrtError::error("Sampler type is not found!"))?;
        let sampler_properties = SamplerProperties::get_instance();
        if let Some(entries) = sampler_properties.get_entries(&sampler_type) {
            let params = self.collect_params_from_entries(entries, &sampler_component.props);
            self.sampler(&sampler_type, &params);
        }
        Ok(())
    }

    fn write_integrator_options(&mut self, node: &Arc<RwLock<Node>>) -> Result<(), PbrtError> {
        let node = node.read().unwrap();
        let integrator_component = node
            .get_component::<IntegratorComponent>()
            .ok_or(PbrtError::error("Integrator is not found!"))?;
        let integrator_type = integrator_component
            .props
            .find_one_string("string type")
            .ok_or(PbrtError::error("Integrator type is not found!"))?;
        let integrator_properties = IntegratorProperties::get_instance();
        if let Some(entries) = integrator_properties.get_entries(&integrator_type) {
            let params = self.collect_params_from_entries(entries, &integrator_component.props);
            self.integrator(&integrator_type, &params);
        }
        Ok(())
    }

    fn write_options_block(&mut self, node: &Arc<RwLock<Node>>) -> Result<(), PbrtError> {
        self.write_camera_options(node)?;
        if self.pretty_print {
            writeln!(self.writer)?;
        }
        self.write_sampler_options(node)?;
        if self.pretty_print {
            writeln!(self.writer)?;
        }
        self.write_integrator_options(node)?;
        if self.pretty_print {
            writeln!(self.writer)?;
        }
        Ok(())
    }

    fn write_world_block(&mut self, node: &Arc<RwLock<Node>>) -> Result<(), PbrtError> {
        self.world_begin();
        self.write_textures(node)?;
        if self.pretty_print {
            writeln!(self.writer)?;
        }
        self.write_materials(node)?;
        if self.pretty_print {
            writeln!(self.writer)?;
        }
        self.write_geometries(node)?;
        self.world_end();
        Ok(())
    }

    fn write_textures(&mut self, node: &Arc<RwLock<Node>>) -> Result<(), PbrtError> {
        let node = node.read().unwrap();
        if let Some(resources_component) = node.get_component::<ResourceComponent>() {
            let resource_manager = resources_component.get_resource_manager();
            let resource_manager = resource_manager.read().unwrap();
            
            if resource_manager.textures.is_empty() {
                return Ok(());
            }

            if self.pretty_print {
                writeln!(self.writer, "    # Textures")?;
            }

            let mut textures = Vec::new();
            for texture in resource_manager.textures.values() {
                let order = texture.read().unwrap().get_order();
                textures.push((order, texture.clone()));
            }
            textures.sort_by(|a, b| a.0.cmp(&b.0));

            for (_order, texture) in textures.iter() {
                let texture = texture.read().unwrap();
                let texture_name = texture.get_name();
                let texture_type = texture.get_type();
                let class = texture.get_color_type();

                let texture_properties = TextureProperties::get_instance();
                if let Some(entries) = texture_properties.get_entries(&texture_type) {
                    let params = self.collect_params_from_entries(entries, texture.as_property_map());
                    self.texture(&texture_name, &class, &texture_type, &params);
                }
            }
        }
        Ok(())
    }

    fn write_materials(&mut self, node: &Arc<RwLock<Node>>) -> Result<(), PbrtError> {
        let node = node.read().unwrap();
        if let Some(resources_component) = node.get_component::<ResourceComponent>() {
            let resource_manager = resources_component.get_resource_manager();
            let resource_manager = resource_manager.read().unwrap();
            
            if resource_manager.materials.is_empty() {
                return Ok(());
            }

            if self.pretty_print {
                writeln!(self.writer, "    # Materials")?;
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
                let material_type = material.get_type();
                let material_name = material.get_name();

                let ignore_keys = get_material_ignore_keys(&material);
                if let Some(entries) = material_properties.get_entries(&material_type) {
                    let mut params = ParamSet::new();
                    params.insert("string type", material_type.clone().into());
                    
                    let mut has_properties = false;
                    for entry in entries.iter() {
                        if !entry.output_to_file {
                            continue;
                        }
                        if ignore_keys.contains(&entry.key_name) {
                            continue;
                        }
                        if let Some((_kt, _kn, value)) = material.as_property_map().entry(&entry.key_name) {
                            params.insert(&format!("{} {}", entry.key_type, entry.key_name), value.clone());
                            has_properties = true;
                        }
                    }
                    
                    // Only write materials that have properties
                    if has_properties {
                        self.make_named_material(&material_name, &params);
                    }
                }
            }
        }
        Ok(())
    }

    fn write_geometries(&mut self, node: &Arc<RwLock<Node>>) -> Result<(), PbrtError> {
        self.write_node_recursive(node, 0)?;
        Ok(())
    }

    fn write_node_recursive(
        &mut self,
        node: &Arc<RwLock<Node>>,
        _depth: usize,
    ) -> Result<(), PbrtError> {
        let node_guard = node.read().unwrap();
        let children: Vec<_> = node_guard.children.iter().map(|c| c.clone()).collect();
        drop(node_guard);

        for child in children.iter() {
            self.write_node(child)?;
        }
        Ok(())
    }

    fn write_node(&mut self, node: &Arc<RwLock<Node>>) -> Result<(), PbrtError> {
        let node_guard = node.read().unwrap();
        
        // Check if this node has geometry
        let has_shape = node_guard.get_component::<ShapeComponent>().is_some();
        let has_light = node_guard.get_component::<LightComponent>().is_some();
        let _has_material = node_guard.get_component::<MaterialComponent>().is_some();
        let has_children = !node_guard.children.is_empty();
        
        if !has_shape && !has_light && !has_children {
            return Ok(());
        }

        self.attribute_begin();

        // Write transform
        if let Some(transform_component) = node_guard.get_component::<TransformComponent>() {
            let matrix = transform_component.get_local_matrix();
            apply_transform(&matrix, self)?;
        }

        // Write material reference
        if let Some(material_component) = node_guard.get_component::<MaterialComponent>() {
            let material = material_component.get_material();
            let material = material.read().unwrap();
            let material_name = material.get_name();
            self.named_material(&material_name);
        }

        // Write area light
        if let Some(light_component) = node_guard.get_component::<LightComponent>() {
            let light_guard = light_component.get_light();
            let light = light_guard.read().unwrap();
            let light_type = light.get_type();
            if light_type == "diffuse" {
                let light_properties = LightProperties::get_instance();
                if let Some(entries) = light_properties.get_entries("diffuse") {
                    let params = self.collect_params_from_entries(entries, light.as_property_map());
                    self.area_light_source("diffuse", &params);
                }
            }
        }

        // Write shape
        if let Some(shape_component) = node_guard.get_component::<ShapeComponent>() {
            let shape_guard = shape_component.get_shape();
            let shape = shape_guard.read().unwrap();
            let shape_type = shape.get_type();
            let shape_properties = ShapeProperties::get_instance();
            if let Some(entries) = shape_properties.get_entries(&shape_type) {
                let params = self.collect_params_from_entries(entries, shape.as_property_map());
                self.shape(&shape_type, &params);
            }
        }

        // Write children
        let children: Vec<_> = node_guard.children.iter().map(|c| c.clone()).collect();
        drop(node_guard);

        for child in children.iter() {
            self.write_node(child)?;
        }

        self.attribute_end();
        Ok(())
    }

    fn copy_resources(
        &self,
        node: &Arc<RwLock<Node>>,
        path: &str,
    ) -> Result<(), PbrtError> {
        if !self.copy_resources {
            return Ok(());
        }

        let out_dir = Path::new(path).parent().ok_or(PbrtError::error(
            "Failed to get parent directory!",
        ))?;
        let node = node.read().unwrap();
        if let Some(resources_component) = node.get_component::<ResourceComponent>() {
            let resource_manager = resources_component.get_resource_manager();
            let resource_manager = resource_manager.read().unwrap();
            let mut copy_paths = Vec::new();

            // Copy textures
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
                }
            }

            // Copy meshes
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
                }
            }

            // Copy other resources
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
                }
            }

            for (src_path, dst_path) in copy_paths.iter() {
                if let Err(e) = super::copy_utility::copy_file(src_path, dst_path) {
                    log::warn!(
                        "Failed to copy resource from {:?} to {:?}: {}",
                        src_path, dst_path, e
                    );
                }
            }
        }
        Ok(())
    }

    pub fn write(&mut self, node: &Arc<RwLock<Node>>, path: &str) -> Result<(), PbrtError> {
        self.write_header(node)?;
        self.write_options_block(node)?;
        if self.pretty_print {
            writeln!(self.writer)?;
        }
        self.write_world_block(node)?;
        self.copy_resources(node, path)?;
        Ok(())
    }
}

pub fn save_pbrt(
    node: &Arc<RwLock<Node>>,
    path: &str,
    options: &SavePbrtOptions,
) -> Result<(), PbrtError> {
    let file = File::create(path)?;
    let writer = std::io::BufWriter::new(file);
    let mut target = SaveTarget::new(writer, options);
    target.write(node, path)?;
    Ok(())
}
