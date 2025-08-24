use super::light::DirectionalRenderLight;
use super::light::DiskRenderLight;
use super::light::RenderLight;
use super::light::RectRenderLight;
use super::light::SphereRenderLight;
use super::lines::RenderLines;
use super::material::RenderMaterial;
use super::material::RenderUniformValue;
use super::render_item::LinesRenderItem;
use super::render_item::RenderItem;
use super::render_item::RenderLightItem;
use super::render_item::get_color;
use super::render_resource::RenderResourceManager;
use crate::conversion::light_shape::create_light_shape;
use crate::conversion::mesh_data::create_mesh_data;
use crate::model::base::Matrix4x4;
use crate::model::base::Vector3;
use crate::model::scene::Light;
use crate::model::scene::LightComponent;
use crate::model::scene::Node;
use crate::model::scene::ResourceManager;
use crate::model::scene::Shape;
use crate::model::scene::ShapeComponent;
use crate::render::render_mode::RenderMode;
use crate::render::scene_item::*;

use std::sync::Arc;
use std::sync::RwLock;

use eframe::wgpu;
use uuid::Uuid;

#[inline]
fn coordinate_system(v1: &Vector3) -> (Vector3, Vector3) {
    let v2 = if f32::abs(v1.x) > f32::abs(v1.y) {
        Vector3::new(-v1.z, 0.0, v1.x) / f32::sqrt(v1.x * v1.x + v1.z * v1.z)
    } else {
        Vector3::new(0.0, v1.z, -v1.y) / f32::sqrt(v1.y * v1.y + v1.z * v1.z)
    };
    let v3 = Vector3::cross(v1, &v2).normalize();
    return (v2, v3);
}

fn get_light_type(node: &Arc<RwLock<Node>>) -> Option<String> {
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<LightComponent>() {
        let light = component.get_light();
        let light = light.read().unwrap();
        return Some(light.get_type());
    }
    return None; // No LightComponent found
}

fn get_light_id_edition(node: &Arc<RwLock<Node>>) -> Option<(Uuid, String)> {
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<LightComponent>() {
        let light = component.get_light();
        let light = light.read().unwrap();
        let id = light.get_id();
        let edition = light.get_edition();
        return Some((id, edition));
    }
    return None; // No LightComponent found
}

fn get_directional_light_item(
    item: &SceneItem,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<RenderItem> {
    let node = &item.node;
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<LightComponent>() {
        let light = component.get_light();
        let light = light.read().unwrap();

        let id = light.get_id();
        let light_type = light.get_type();
        let edition = light.get_edition();
        if let Some(render_light) = render_resource_manager.get_light(id) {
            if render_light.get_edition() == edition {
                let render_item = RenderLightItem {
                    light: render_light.clone(),
                    matrix: glam::Mat4::from(item.matrix),
                };
                return Some(RenderItem::Light(render_item));
            }
        }
        assert!(
            light_type == "distant",
            "Expected light type to be 'distant', found: {}",
            light_type
        );

        let props = light.as_property_map();

        let mut from = props.get_floats("from");
        if from.len() != 3 {
            from = vec![0.0, 0.0, 0.0];
        }
        let mut to = props.get_floats("to");
        if to.len() != 3 {
            to = vec![0.0, 0.0, 1.0];
        }
        let from = Vector3::new(from[0], from[1], from[2]);
        let to = Vector3::new(to[0], to[1], to[2]);
        let dir = to - from;
        let direction = [dir.x, dir.y, dir.z];

        let l = get_color(&props, "L", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
        let scale = get_color(&props, "scale", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);

        let intensity = [l[0] * scale[0], l[1] * scale[1], l[2] * scale[2]];
        let render_light = DirectionalRenderLight {
            id,
            edition: edition.clone(),
            direction: direction,
            intensity: intensity,
            ..Default::default()
        };
        let render_light = Arc::new(RenderLight::Directional(render_light));
        render_resource_manager.add_light(&render_light);
        let render_item = RenderLightItem {
            light: render_light.clone(),
            matrix: glam::Mat4::from(item.matrix),
        };
        return Some(RenderItem::Light(render_item));
    }
    return None;
}

fn get_point_light_item(
    item: &SceneItem,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<RenderItem> {
    let node = &item.node;
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<LightComponent>() {
        let light = component.get_light();
        let light = light.read().unwrap();

        let id = light.get_id();
        let light_type = light.get_type();
        let edition = light.get_edition();
        let props = light.as_property_map();
        if let Some(render_light) = render_resource_manager.get_light(id) {
            if render_light.get_edition() == edition {
                let mut from = props.get_floats("from");
                if from.len() != 3 {
                    from = vec![0.0, 0.0, 0.0];
                }
                let translation = Matrix4x4::translate(from[0], from[1], from[2]);
                let mat = translation * item.matrix;

                let render_item = RenderLightItem {
                    light: render_light.clone(),
                    matrix: glam::Mat4::from(mat),
                };
                return Some(RenderItem::Light(render_item));
            }
        }
        assert!(
            light_type == "point",
            "Expected light type to be 'point', found: {}",
            light_type
        );

        let mut from = props.get_floats("from");
        if from.len() != 3 {
            from = vec![0.0, 0.0, 0.0];
        }

        let translation = Matrix4x4::translate(from[0], from[1], from[2]);
        let mat = translation * item.matrix;

        let l = get_color(&props, "I", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
        let scale = get_color(&props, "scale", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);

        let intensity = [l[0] * scale[0], l[1] * scale[1], l[2] * scale[2]];

        let render_light = SphereRenderLight {
            id,
            edition: edition.clone(),
            intensity: intensity,
            radius: 0.0,
            ..Default::default()
        };
        let render_light = Arc::new(RenderLight::Sphere(render_light));
        render_resource_manager.add_light(&render_light);

        let render_item = RenderLightItem {
            light: render_light.clone(),
            matrix: glam::Mat4::from(mat),
        };
        return Some(RenderItem::Light(render_item));
    }
    return None; // Point lights are not yet supported
}

fn get_spot_light_item(
    item: &SceneItem,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<RenderItem> {
    let node = &item.node;
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<LightComponent>() {
        let light = component.get_light();
        let light = light.read().unwrap();

        let id = light.get_id();
        let light_type = light.get_type();
        let edition = light.get_edition();
        if let Some(render_light) = render_resource_manager.get_light(id) {
            if render_light.get_edition() == edition {
                let render_item = RenderLightItem {
                    light: render_light.clone(),
                    matrix: glam::Mat4::from(item.matrix),
                };
                return Some(RenderItem::Light(render_item));
            }
        }
        assert!(
            light_type == "spot",
            "Expected light type to be 'point', found: {}",
            light_type
        );
        let props = light.as_property_map();

        let mut from = props.get_floats("from");
        if from.len() != 3 {
            from = vec![0.0, 0.0, 0.0];
        }
        let mut to = props.get_floats("to");
        if to.len() != 3 {
            to = vec![0.0, 0.0, 1.0];
        }
        let from = Vector3::new(from[0], from[1], from[2]);
        let to = Vector3::new(to[0], to[1], to[2]);
        let dir = (to - from).normalize();
        let (du, dv) = coordinate_system(&dir);
        let dir_to_z = Matrix4x4::new(
            du.x, du.y, du.z, 0.0, dv.x, dv.y, dv.z, 0., dir.x, dir.y, dir.z, 0.0, 0.0, 0.0, 0.0,
            1.0,
        );
        let mat =
            Matrix4x4::translate(from.x, from.y, from.z) * Matrix4x4::inverse(&dir_to_z).unwrap();

        let position = Vector3::new(0.0, 0.0, 0.0); // Position is not used for spot lights
        let direction = Vector3::new(0.0, 0.0, 1.0); // Direction is not used for spot lights
        let position = mat.transform_point(&position);
        let direction = mat.transform_vector(&direction).normalize();

        let coneangle = props.find_one_float("coneangle").unwrap_or(30.0);
        let conedelta = props.find_one_float("conedelta").unwrap_or(5.0); //5.0
        let conedelta = props.find_one_float("conedeltaangle").unwrap_or(conedelta);
        let conedelta = conedelta.clamp(0.0, coneangle);

        let outer_angle = f32::to_radians(coneangle);
        let inner_angle = f32::to_radians((coneangle - conedelta).max(0.0));

        let l = get_color(&props, "I", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
        let scale = get_color(&props, "scale", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);

        let intensity = [l[0] * scale[0], l[1] * scale[1], l[2] * scale[2]];
        let radius = 10.0; //todo: get radius from properties
        let render_light = DiskRenderLight {
            id,
            edition: edition.clone(),
            position: [position.x, position.y, position.z], // Position is not used for spot lights
            direction: [direction.x, direction.y, direction.z], // Direction is not used for spot lights
            intensity: intensity,
            radius: radius,
            inner_angle, // Inner radius for spot lights
            outer_angle, // Outer radius for spot lights
        };
        let render_light = Arc::new(RenderLight::Disk(render_light));
        render_resource_manager.add_light(&render_light);

        let render_item = RenderLightItem {
            light: render_light.clone(),
            matrix: glam::Mat4::from(item.matrix),
        };
        return Some(RenderItem::Light(render_item));
    }
    return None; // Point lights are not yet supported
}

fn get_sphere_light_item(
    light: &Light,
    shape: &Shape,
    matrix: &Matrix4x4,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<RenderItem> {
    let shape_type = shape.get_type();
    assert!(
        shape_type == "sphere",
        "Expected shape type to be 'sphere' or 'disk', found: {}",
        shape_type
    );

    let id = light.get_id();
    let light_edition = light.get_edition();
    let shape_edition = shape.get_edition();
    let edition = format!("{}-{}", light_edition, shape_edition); // Combine editions of light and shape

    if let Some(render_light) = render_resource_manager.get_light(id) {
        if render_light.get_edition() == edition {
            let render_item = RenderLightItem {
                light: render_light.clone(),
                matrix: glam::Mat4::from(matrix),
            };
            return Some(RenderItem::Light(render_item));
        }
    }

    let radius = shape
        .as_property_map()
        .find_one_float("radius")
        .unwrap_or(1.0);
    /*
    let zmin = shape
        .as_property_map()
        .find_one_float("zmin")
        .unwrap_or(-1.0);
    let zmax = shape
        .as_property_map()
        .find_one_float("zmax")
        .unwrap_or(1.0);
    */

    //self.phi_max * self.radius * (self.z_max - self.z_min)
    let area = if radius > 0.0 {
        //std::f32::consts::PI * radius * (zmax - zmin) // Area of the sphere segment
        2.0 * std::f32::consts::PI * radius // Area of the sphere
    } else {
        1.0 // Default area if radius is not specified
    };

    let props = light.as_property_map();
    let l = get_color(&props, "L", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let scale = get_color(&props, "scale", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);

    let intensity = [
        area * l[0] * scale[0],
        area * l[1] * scale[1],
        area * l[2] * scale[2],
    ];

    let render_light = SphereRenderLight {
        id,
        edition: edition.clone(),
        position: [0.0, 0.0, 0.0], // Position is not used for sphere lights
        intensity: intensity,
        radius: radius,
    };
    let render_light = Arc::new(RenderLight::Sphere(render_light));
    render_resource_manager.add_light(&render_light);

    let render_item = RenderLightItem {
        light: render_light.clone(),
        matrix: glam::Mat4::from(matrix),
    };
    return Some(RenderItem::Light(render_item));
}

fn get_disk_light_item(
    light: &Light,
    shape: &Shape,
    matrix: &Matrix4x4,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<RenderItem> {
    let shape_type = shape.get_type();
    assert!(
        shape_type == "disk",
        "Expected shape type to be 'sphere' or 'disk', found: {}",
        shape_type
    );

    let id = light.get_id();
    let light_edition = light.get_edition();
    let shape_edition = shape.get_edition();
    let edition = format!("{}-{}", light_edition, shape_edition); // Combine editions of light and shape

    if let Some(render_light) = render_resource_manager.get_light(id) {
        if render_light.get_edition() == edition {
            let render_item = RenderLightItem {
                light: render_light.clone(),
                matrix: glam::Mat4::from(matrix),
            };
            return Some(RenderItem::Light(render_item));
        }
    }

    let radius = shape
        .as_property_map()
        .find_one_float("radius")
        .unwrap_or(1.0);

    let area = if radius > 0.0 {
        std::f32::consts::PI * radius * radius // Area of the disk
    } else {
        1.0 // Default area if radius is not specified
    };
    //let area = 1.0;////radius * radius; // Assuming a disk for area calculation

    let props = light.as_property_map();
    let l = get_color(&props, "L", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let scale = get_color(&props, "scale", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);

    let outer_angle = props
        .find_one_float("coneangle")
        .unwrap_or(90.0)
        .to_radians();

    //let area = area * (1.0 - f32::powf(outer_angle/std::f32::consts::PI, 2.0));

    let position = Vector3::new(0.0, 0.0, 0.0); // Center of the disk
    let n = Vector3::new(0.0, 0.0, -1.0); // Normal of the disk

    let intensity = [
        area * l[0] * scale[0],
        area * l[1] * scale[1],
        area * l[2] * scale[2],
    ];

    let render_light = DiskRenderLight {
        id,
        edition: edition.clone(),
        position: [position.x, position.y, position.z], // Position is not used for spot lights
        direction: [n.x, n.y, n.z],                     // Direction is not used for spot lights
        intensity: intensity,
        radius: radius,           // Radius of the disk
        inner_angle: outer_angle, // Inner radius for disk lights
        outer_angle: outer_angle, // Outer radius for disk lights
    };
    let render_light = Arc::new(RenderLight::Disk(render_light));
    render_resource_manager.add_light(&render_light);

    let render_item = RenderLightItem {
        light: render_light.clone(),
        matrix: glam::Mat4::from(matrix),
    };
    return Some(RenderItem::Light(render_item));
}

fn check_square_mesh(indices0: &[i32; 3], indices1: &[i32; 3]) -> Option<[i32; 4]> {
    // Check if the two triangles form a square
    for i in 0..3 {
        for j in 0..3 {
            let i0 = indices0[(i + 0) % 3];
            let i1 = indices0[(i + 1) % 3];
            let i2 = indices0[(i + 2) % 3];
            let j0 = indices1[(j + 0) % 3];
            let j1 = indices1[(j + 1) % 3];
            let j2 = indices1[(j + 2) % 3];

            if i1 == j2 && i2 == j1 && i0 != j0 {
                // Found a square mesh
                //i0, i1 == j2, j0, i2 == j1
                return Some([i0, i1, j0, j1]);
            }
        }
    }
    return None; // Not a square mesh
}

fn get_rectangle_light_item(
    light: &Light,
    shape: &Shape,
    matrix: &Matrix4x4,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<RenderItem> {
    let shape_type = shape.get_type();
    assert!(
        shape_type == "trianglemesh" || shape_type == "plymesh",
        "Expected shape type to be 'trianglemesh' or 'plymesh', found: {}",
        shape_type
    );

    let id = light.get_id();
    let light_edition = light.get_edition();
    let shape_edition = shape.get_edition();
    let edition = format!("{}-{}", light_edition, shape_edition); // Combine editions of light and shape

    return None;
}

/*
if let Some(mesh_data) = create_mesh_data(shape) {
        let num_face = mesh_data.indices.len() / 3;
        if num_face == 2 {
            let indices0: [i32; 3] = mesh_data.indices[0..3].try_into().unwrap();
            let indices1: [i32; 3] = mesh_data.indices[3..6].try_into().unwrap();
            if let Some(indices) = check_square_mesh(&indices0, &indices1) {
                //println!("Found square mesh: {:?}", indices);
                let i0 = indices[0] as usize;
                let i1 = indices[1] as usize;
                let i2 = indices[2] as usize;
                let i3 = indices[3] as usize;
                let p0 = Vector3::new(
                    mesh_data.positions[3 * i0 + 0],
                    mesh_data.positions[3 * i0 + 1],
                    mesh_data.positions[3 * i0 + 2],
                );
                let p1 = Vector3::new(
                    mesh_data.positions[3 * i1 + 0],
                    mesh_data.positions[3 * i1 + 1],
                    mesh_data.positions[3 * i1 + 2],
                );
                let p2 = Vector3::new(
                    mesh_data.positions[3 * i2 + 0],
                    mesh_data.positions[3 * i2 + 1],
                    mesh_data.positions[3 * i2 + 2],
                );
                let p3 = Vector3::new(
                    mesh_data.positions[3 * i3 + 0],
                    mesh_data.positions[3 * i3 + 1],
                    mesh_data.positions[3 * i3 + 2],
                );
                /*
                   p0 - p3
                   |    |
                   p1 - p2
                */
                let na = Vector3::cross(&(p1 - p0), &(p2 - p0)).normalize();
                let nb = Vector3::cross(&(p0 - p3), &(p2 - p3)).normalize();
                if Vector3::dot(&na, &nb) >= 0.9 {
                    let center = (p0 + p1 + p2 + p3) * 0.25;
                    let n = (na + nb).normalize();
                    let mut r: f32 = 0.0;
                    for i in 0..4 {
                        let p = Vector3::new(
                            mesh_data.positions[3 * indices[i] as usize + 0],
                            mesh_data.positions[3 * indices[i] as usize + 1],
                            mesh_data.positions[3 * indices[i] as usize + 2],
                        );
                        r = r.max(Vector3::length_squared(&(p - center)));
                    }
                    r = f32::sqrt(r);

                    let l =
                        get_color(&props, "L", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
                    let scale = get_color(&props, "scale", resource_manager)
                        .unwrap_or([1.0, 1.0, 1.0, 1.0]);

                    let outer_angle = props
                        .find_one_float("coneangle")
                        .unwrap_or(30.0)
                        .to_radians();
                    let delta_angle = props
                        .find_one_float("conedeltaangle")
                        .unwrap_or(5.0)
                        .to_radians();
                    let inner_angle = (outer_angle - delta_angle).max(0.0);

                    let area = 1.0; //

                    let intensity = [
                        area * l[0] * scale[0],
                        area * l[1] * scale[1],
                        area * l[2] * scale[2],
                    ];

                    /*
                    l * sin(outer_angle) = r
                    l * cos(outer_angle) = x
                    l = r / sin(outer_angle)
                    x = r / sin(outer_angle) * cos(outer_angle)
                    x = r * cos(outer_angle) / sin(outer_angle)
                    x = r / tan(outer_angle)
                     */
                    let h = r / f32::tan(outer_angle);
                    let position = center - n * h; //
                    let direction = n;

                    let render_light = RenderLight {
                        id,
                        edition: edition.clone(),
                        light_type: RenderLightType::Spot,
                        position: [position.x, position.y, position.z], // Position is not used for spot lights
                        direction: [direction.x, direction.y, direction.z], // Direction is not used for spot lights
                        intensity: intensity,
                        range: [h, 10.0],
                        angle: [inner_angle, outer_angle],
                        center: [center.x, center.y, center.z],
                        ..Default::default()
                    };
                    let render_light = Arc::new(render_light);
                    render_resource_manager.add_light(&render_light);

                    let render_item = RenderLightItem {
                        light: render_light.clone(),
                        matrix: glam::Mat4::from(matrix),
                    };
                    render_items.push(RenderItem::Light(render_item));

                    if two_sided {
                        let d = center - position; // Direction from position to center
                        let position = center + d; // Position on the other side

                        // Create a second light for the other side
                        let render_light = RenderLight {
                            id,
                            edition: edition.clone(),
                            light_type: RenderLightType::Spot,
                            position: [position.x, position.y, position.z], // Position is not used for spot lights
                            direction: [-direction.x, -direction.y, -direction.z], // Inverted direction for two-sided
                            intensity: intensity,
                            range: [0.0, 10.0],
                            angle: [inner_angle, outer_angle],
                            center: [center.x, center.y, center.z],
                            ..Default::default()
                        };
                        let render_light = Arc::new(render_light);
                        render_resource_manager.add_light(&render_light);

                        let render_item = RenderLightItem {
                            light: render_light.clone(),
                            matrix: glam::Mat4::from(matrix),
                        };
                        render_items.push(RenderItem::Light(render_item));
                    }
                }
            }
        }

*/

fn get_area_light_item_core(
    light: &Light,
    shape: &Shape,
    matrix: &Matrix4x4,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<RenderItem> {
    let light_type = light.get_type();
    let shape_type = shape.get_type();
    assert!(
        light_type == "diffuse" || light_type == "area",
        "Expected light type to be 'diffuse' or 'area', found: {}",
        light_type
    );
    match shape_type.as_str() {
        "sphere" => {
            return get_sphere_light_item(
                light,
                shape,
                matrix,
                resource_manager,
                render_resource_manager,
            );
        }
        "disk" => {
            return get_disk_light_item(
                light,
                shape,
                matrix,
                resource_manager,
                render_resource_manager,
            );
        }
        "trianglemesh" | "plymesh" => {
            return get_rectangle_light_item(
                light,
                shape,
                matrix,
                resource_manager,
                render_resource_manager,
            );
        }
        _ => {
            // Unsupported shape type for area light
        }
    }
    return None; // No area light item created
}

fn get_area_light_item(
    item: &SceneItem,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<RenderItem> {
    let node = &item.node;
    let node = node.read().unwrap();
    let components = (
        node.get_component::<LightComponent>(),
        node.get_component::<ShapeComponent>(),
    );
    match components {
        (Some(light_component), Some(shape_component)) => {
            let light = light_component.get_light();
            let light = light.read().unwrap();

            let shape = shape_component.get_shape();
            let shape = shape.read().unwrap();
            return get_area_light_item_core(
                &light,
                &shape,
                &item.matrix,
                resource_manager,
                render_resource_manager,
            );
        }
        _ => {}
    }
    return None;
}

fn get_lines_material(
    id: Uuid,
    edition: &str,
    render_resource_manager: &mut RenderResourceManager,
    base_color: &[f32; 4],
) -> Option<Arc<RenderMaterial>> {
    if let Some(mat) = render_resource_manager.get_material(id) {
        if mat.edition == edition {
            return Some(mat.clone());
        }
    }
    // Create a default material for the light gizmo
    let mut uniform_values = Vec::new();
    uniform_values.push((
        "base_color".to_string(),
        RenderUniformValue::Vec4(base_color.clone()),
    ));
    let render_material = RenderMaterial {
        id: id,
        edition: edition.to_string(),
        uniform_values,
    };
    let render_material = Arc::new(render_material);
    render_resource_manager.add_material(&render_material);
    return Some(render_material);
}

fn get_light_gizmo_material(
    node: &Arc<RwLock<Node>>,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<Arc<RenderMaterial>> {
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<LightComponent>() {
        let light = component.get_light();
        let light = light.read().unwrap();
        let light_id = light.get_id();
        let edition = light.get_edition();
        let base_color = [1.0, 1.0, 0.0, 1.0]; // Default Yellow color for light gizmo
        return get_lines_material(light_id, &edition, render_resource_manager, &base_color);
    }
    return None;
}

fn get_light_gizmo(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    node: &Arc<RwLock<Node>>,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<Arc<RenderLines>> {
    if let Some((id, edition)) = get_light_id_edition(node) {
        if let Some(lines) = render_resource_manager.get_lines(id) {
            if lines.edition == edition {
                return Some(lines.clone());
            }
        }
        if let Some(light_shape) = create_light_shape(node) {
            let lines = &light_shape.lines;
            let lines = lines
                .iter()
                .map(|line| {
                    line.iter()
                        .map(|point| [point.x, point.y, point.z])
                        .collect::<Vec<[f32; 3]>>()
                })
                .collect::<Vec<Vec<[f32; 3]>>>();
            if let Some(lines) = RenderLines::from_lines(device, queue, id, &edition, &lines) {
                let lines = Arc::new(lines);
                render_resource_manager.add_lines(&lines);
                return Some(lines);
            }
        }
    }
    return None;
}

pub fn get_render_light_item(
    _device: &wgpu::Device,
    _queue: &wgpu::Queue,
    item: &SceneItem,
    _mode: RenderMode,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<RenderItem> {
    if let Some(light_type) = get_light_type(&item.node) {
        match light_type.as_str() {
            "distant" => {
                return get_directional_light_item(item, resource_manager, render_resource_manager);
            }
            "point" => {
                return get_point_light_item(item, resource_manager, render_resource_manager);
            }
            "spot" => {
                return get_spot_light_item(item, resource_manager, render_resource_manager); // Spot lights are not yet supported
            }
            "diffuse" | "area" => {
                return get_area_light_item(item, resource_manager, render_resource_manager); // Area lights are not yet supported
            }
            _ => {
                // Handle unknown or unsupported light types
                return None;
            }
        }
    }
    return None; // Placeholder for light retrieval logic
}

fn get_point_light_offset(node: &Arc<RwLock<Node>>) -> Option<Vector3> {
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<LightComponent>() {
        let light = component.get_light();
        let light = light.read().unwrap();
        if light.get_type() == "point" {
            let props = light.as_property_map();
            let mut from = props.get_floats("from");
            if from.len() != 3 {
                from = vec![0.0, 0.0, 0.0];
            }
            return Some(Vector3::new(from[0], from[1], from[2]));
        }
    }
    return None; // Default offset
}

pub fn get_render_light_gizmo_item(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    item: &SceneItem,
    _mode: RenderMode,
    _resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<RenderItem> {
    if let Some(lines) = get_light_gizmo(device, queue, &item.node, render_resource_manager) {
        let mut matrix = item.matrix;
        if let Some(offset) = get_point_light_offset(&item.node) {
            // Adjust the matrix for point lights
            matrix = Matrix4x4::translate(offset.x, offset.y, offset.z) * matrix;
        }
        let matrix = glam::Mat4::from(matrix);
        let material = get_light_gizmo_material(&item.node, render_resource_manager);
        let render_item = LinesRenderItem {
            lines,
            material,
            matrix,
        };
        return Some(RenderItem::Lines(render_item));
    }
    return None;
}
