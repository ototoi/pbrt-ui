use crate::error::PbrtError;
use crate::io::pbrt::PbrtTarget;
use crate::io::pbrt::PrintTarget;
use crate::model::base::Matrix4x4;
use crate::model::base::PropertyMap as ParamSet;
use crate::model::base::Vector3;
use crate::model::scene::CameraComponent;
use crate::model::scene::CameraProperties;
use crate::model::scene::FilmComponent;
use crate::model::scene::IntegratorComponent;
use crate::model::scene::IntegratorProperties;
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
use crate::model::scene::ShapeComponent;
use crate::model::scene::ShapeProperties;
use crate::model::scene::TextureProperties;
use crate::model::scene::TransformComponent;
use std::cell::RefCell;
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

// Helper functions

/// Get the world transformation matrix for a node
///
/// Recursively accumulates transformations from parent nodes to compute the final world matrix
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

/// Check if two floating point values are nearly equal within epsilon
fn near_equal(a: f32, b: f32, epsilon: f32) -> bool {
    (a - b).abs() < epsilon
}

/// Apply a transformation matrix to the print target
///
/// Decomposes the matrix into translation, rotation, and scale components and applies them
fn apply_transform(transform: &Matrix4x4, target: &mut PrintTarget) -> Result<(), PbrtError> {
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

    if !near_equal(s.x, 1.0, epsilon)
        || !near_equal(s.y, 1.0, epsilon)
        || !near_equal(s.z, 1.0, epsilon)
    {
        target.scale(s.x, s.y, s.z);
    }
    Ok(())
}

/// Get list of parameter keys to ignore when exporting a material
///
/// Some material types require special handling where certain parameters should be excluded
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

/// Collect parameters from property entries that should be written to the file
///
/// Filters entries based on output_to_file flag and builds a parameter set
fn collect_params_from_entries(entries: &[PropertyEntry], props: &ParamSet) -> ParamSet {
    let mut params = ParamSet::new();
    for entry in entries.iter() {
        if !entry.output_to_file {
            continue;
        }
        if let Some((key_type, key_name, value)) = props.entry(&entry.key_name) {
            params.insert(&format!("{} {}", key_type, key_name), value.clone());
        }
    }
    params
}

/// Write the PBRT file header
fn write_header(target: &mut PrintTarget) -> Result<(), PbrtError> {
    target.write_str("# Generated by pbrt-ui\n")?;
    Ok(())
}

/// Write camera, transform, and film options to the PBRT file
///
/// Searches for camera component in node hierarchy and outputs camera configuration
fn write_camera_options(
    node: &Arc<RwLock<Node>>,
    target: &mut PrintTarget,
) -> Result<(), PbrtError> {
    let node = Node::find_node_by_component::<CameraComponent>(node)
        .ok_or(PbrtError::error("Camera is not found!"))?;

    // Camera Transform
    {
        let local_to_world = get_world_matrix(&node)?;
        let world_to_local = local_to_world
            .inverse()
            .ok_or(PbrtError::error("Camera transform is not found!"))?;
        apply_transform(&world_to_local, target)?;
    }

    // Camera
    {
        let node = node.read().unwrap();
        let component = node
            .get_component::<CameraComponent>()
            .ok_or(PbrtError::error("Camera is not found!"))?;
        let ty = component
            .props
            .find_one_string("string type")
            .ok_or(PbrtError::error("Camera type is not found!"))?;
        let camera_properties = CameraProperties::get_instance();
        if let Some(entries) = camera_properties.get_entries(&ty) {
            let params = collect_params_from_entries(entries, &component.props);
            target.camera(&ty, &params);
        }
    }

    // Film
    {
        let node = node.read().unwrap();
        let component = node
            .get_component::<FilmComponent>()
            .ok_or(PbrtError::error("Film is not found!"))?;
        let ty = component
            .props
            .find_one_string("string type")
            .ok_or(PbrtError::error("Film type is not found!"))?;
        let option_properties = OptionProperties::get_instance();
        if let Some(entries) = option_properties.get_entries("film") {
            let params = collect_params_from_entries(entries, &component.props);
            target.film(&ty, &params);
        }
    }

    Ok(())
}

/// Write sampler options to the PBRT file
fn write_sampler_options(
    node: &Arc<RwLock<Node>>,
    target: &mut PrintTarget,
) -> Result<(), PbrtError> {
    let node = node.read().unwrap();
    let component = node
        .get_component::<SamplerComponent>()
        .ok_or(PbrtError::error("Sampler is not found!"))?;
    let ty = component
        .props
        .find_one_string("string type")
        .ok_or(PbrtError::error("Sampler type is not found!"))?;
    let sampler_properties = SamplerProperties::get_instance();
    if let Some(entries) = sampler_properties.get_entries(&ty) {
        let params = collect_params_from_entries(entries, &component.props);
        target.sampler(&ty, &params);
    }
    Ok(())
}

/// Write integrator options to the PBRT file
fn write_integrator_options(
    node: &Arc<RwLock<Node>>,
    target: &mut PrintTarget,
) -> Result<(), PbrtError> {
    let node = node.read().unwrap();
    let component = node
        .get_component::<IntegratorComponent>()
        .ok_or(PbrtError::error("Integrator is not found!"))?;
    let ty = component
        .props
        .find_one_string("string type")
        .ok_or(PbrtError::error("Integrator type is not found!"))?;
    let integrator_properties = IntegratorProperties::get_instance();
    if let Some(entries) = integrator_properties.get_entries(&ty) {
        let params = collect_params_from_entries(entries, &component.props);
        target.integrator(&ty, &params);
    }
    Ok(())
}

/// Write the complete options block (camera, sampler, integrator)
fn write_options_block(
    node: &Arc<RwLock<Node>>,
    target: &mut PrintTarget,
) -> Result<(), PbrtError> {
    write_camera_options(node, target)?;
    target.write_str("\n")?;
    write_sampler_options(node, target)?;
    target.write_str("\n")?;
    write_integrator_options(node, target)?;
    target.write_str("\n")?;
    Ok(())
}

/// Write texture definitions to the PBRT file
///
/// Outputs all textures sorted by creation order
fn write_textures(node: &Arc<RwLock<Node>>, target: &mut PrintTarget) -> Result<(), PbrtError> {
    let node = node.read().unwrap();
    if let Some(resources_component) = node.get_component::<ResourceComponent>() {
        let resource_manager = resources_component.get_resource_manager();
        let resource_manager = resource_manager.read().unwrap();

        if resource_manager.textures.is_empty() {
            return Ok(());
        }

        target.write_str("# Textures\n")?;

        let mut textures = Vec::new();
        for texture in resource_manager.textures.values() {
            let order = texture.read().unwrap().get_order();
            textures.push((order, texture.clone()));
        }
        textures.sort_by(|a, b| a.0.cmp(&b.0));

        for (_order, texture) in textures.iter() {
            let texture = texture.read().unwrap();
            let name = texture.get_name();
            let ty = texture.get_type();
            let color_type = texture.get_color_type();

            let texture_properties = TextureProperties::get_instance();
            if let Some(entries) = texture_properties.get_entries(&ty) {
                let params = collect_params_from_entries(entries, texture.as_property_map());
                target.texture(&name, &color_type, &ty, &params);
            }
        }
    }
    Ok(())
}

/// Write material definitions to the PBRT file
///
/// Outputs all materials sorted alphabetically by name
fn write_materials(node: &Arc<RwLock<Node>>, target: &mut PrintTarget) -> Result<(), PbrtError> {
    let node = node.read().unwrap();
    if let Some(resources_component) = node.get_component::<ResourceComponent>() {
        let resource_manager = resources_component.get_resource_manager();
        let resource_manager = resource_manager.read().unwrap();

        if resource_manager.materials.is_empty() {
            return Ok(());
        }

        target.write_str("# Materials\n")?;

        let mut materials = resource_manager
            .materials
            .values()
            .map(|m| (m.read().unwrap().get_name().to_ascii_lowercase(), m))
            .collect::<Vec<_>>();
        materials.sort_by(|a, b| a.0.cmp(&b.0));

        let material_properties = MaterialProperties::get_instance();
        for (_name, material) in materials.iter() {
            let material = material.read().unwrap();
            let ty = material.get_type();
            let name = material.get_name();

            let ignore_keys = get_material_ignore_keys(&material);
            if let Some(entries) = material_properties.get_entries(&ty) {
                let mut params = ParamSet::new();
                params.insert("string type", ty.clone().into());

                let mut has_properties = false;
                for entry in entries.iter() {
                    if !entry.output_to_file {
                        continue;
                    }
                    if ignore_keys.contains(&entry.key_name) {
                        continue;
                    }
                    if let Some((key_type, key_name, value)) =
                        material.as_property_map().entry(&entry.key_name)
                    {
                        params.insert(&format!("{} {}", key_type, key_name), value.clone());
                        has_properties = true;
                    }
                }

                // Only write materials that have properties
                if has_properties {
                    target.make_named_material(&name, &params);
                }
            }
        }
    }
    Ok(())
}

/// Write a node and its children to the PBRT file
///
/// Recursively outputs transforms, materials, lights, and shapes for a scene node
fn write_node(node: &Arc<RwLock<Node>>, target: &mut PrintTarget) -> Result<(), PbrtError> {
    let children = {
        let node = node.read().unwrap();

        // Check if this node has geometry
        let has_shape = node.get_component::<ShapeComponent>().is_some();
        let has_light = node.get_component::<LightComponent>().is_some();
        let has_children = !node.children.is_empty();

        if !has_shape && !has_light && !has_children {
            return Ok(());
        }

        target.attribute_begin();

        // Write transform
        if let Some(transform_component) = node.get_component::<TransformComponent>() {
            let matrix = transform_component.get_local_matrix();
            apply_transform(&matrix, target)?;
        }

        // Write material reference
        if let Some(material_component) = node.get_component::<MaterialComponent>() {
            let material = material_component.get_material();
            let material = material.read().unwrap();
            let material_name = material.get_name();
            target.named_material(&material_name);
        }

        // Write area light
        if let Some(component) = node.get_component::<LightComponent>() {
            let light = component.get_light();
            let light = light.read().unwrap();
            let ty = light.get_type();
            if ty == "diffuse" {
                let light_properties = LightProperties::get_instance();
                if let Some(entries) = light_properties.get_entries("diffuse") {
                    let params = collect_params_from_entries(entries, light.as_property_map());
                    target.area_light_source("diffuse", &params);
                }
            }
        }

        // Write shape
        if let Some(component) = node.get_component::<ShapeComponent>() {
            let shape = component.get_shape();
            let shape = shape.read().unwrap();
            let ty = shape.get_type();
            let shape_properties = ShapeProperties::get_instance();
            if let Some(entries) = shape_properties.get_entries(&ty) {
                let params = collect_params_from_entries(entries, shape.as_property_map());
                target.shape(&ty, &params);
            }
        }

        // Collect children
        let children = node.children.iter().map(|c| c.clone()).collect::<Vec<_>>();
        children
    };

    for child in children.iter() {
        write_node(child, target)?;
    }

    target.attribute_end();
    Ok(())
}

/// Write all geometry nodes (children of the root node)
fn write_geometries(node: &Arc<RwLock<Node>>, target: &mut PrintTarget) -> Result<(), PbrtError> {
    let children = {
        let node = node.read().unwrap();
        let children = node.children.iter().map(|c| c.clone()).collect::<Vec<_>>();
        children
    };

    for child in children.iter() {
        write_node(child, target)?;
    }
    Ok(())
}

/// Write the world block containing textures, materials, and geometry
fn write_world_block(node: &Arc<RwLock<Node>>, target: &mut PrintTarget) -> Result<(), PbrtError> {
    target.world_begin();
    write_textures(node, target)?;
    target.write_str("\n")?;
    write_materials(node, target)?;
    target.write_str("\n")?;
    write_geometries(node, target)?;
    target.world_end();
    Ok(())
}

/// Copy external resources (textures, meshes, etc.) to the output directory
///
/// Copies referenced files like images and PLY meshes to the same directory as the PBRT file
fn copy_resources(node: &Arc<RwLock<Node>>, path: &str) -> Result<(), PbrtError> {
    let out_dir = Path::new(path)
        .parent()
        .ok_or(PbrtError::error("Failed to get parent directory!"))?;
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
                    src_path,
                    dst_path,
                    e
                );
            }
        }
    }
    Ok(())
}

/// Save a scene graph to a PBRT file
///
/// # Arguments
/// * `node` - Root node of the scene to export
/// * `path` - Output file path
/// * `options` - Export options (pretty print, copy resources)
///
/// # Returns
/// * `Result<(), PbrtError>` - Success or error
pub fn save_pbrt(
    node: &Arc<RwLock<Node>>,
    path: &str,
    options: &SavePbrtOptions,
) -> Result<(), PbrtError> {
    let file = File::create(path)?;
    let writer = std::io::BufWriter::new(file);
    let writer: Arc<RefCell<dyn Write>> = Arc::new(RefCell::new(writer));
    let mut target = PrintTarget::new_with_params(writer.clone(), false);

    println!("Saving PBRT file to {}", path);
    write_header(&mut target)?;
    write_options_block(node, &mut target)?;
    target.write_str("\n")?;
    write_world_block(node, &mut target)?;

    if options.copy_resources {
        copy_resources(node, path)?;
    }

    // Flush the writer
    target.flush()?;
    Ok(())
}
