use super::light::DirectionalRenderLight;
use super::light::DiskRenderLight;
use super::light::InfiniteRenderLight;
use super::light::RectRenderLight;
use super::light::RectsRenderLight;
use super::light::RenderLight;
use super::light::SphereRenderLight;
use super::lines::RenderLines;
use super::material::RenderMaterial;
use super::material::RenderUniformValue;
use super::render_item::LinesRenderItem;
use super::render_item::RenderItem;
use super::render_item::RenderLightItem;
use super::render_item::get_color;
use super::render_resource::RenderResourceManager;
use super::texture::RenderTexture;
use crate::conversion::light_shape::create_light_shape;
use crate::conversion::mesh_data::create_mesh_data;
use crate::conversion::plane_data::create_plane_meshes_from_mesh;
use crate::conversion::plane_data::create_plane_outline_from_plane_mesh;
use crate::conversion::plane_data::create_plane_rect_from_plane_outline;
use crate::conversion::texture_cache::TexturePurpose;
use crate::conversion::texture_cache::create_image_variant;
use crate::model::base::Matrix4x4;
use crate::model::base::Vector3;
use crate::model::scene::Light;
use crate::model::scene::LightComponent;
use crate::model::scene::Node;
use crate::model::scene::ResourceCacheManager;
use crate::model::scene::ResourceManager;
use crate::model::scene::Shape;
use crate::model::scene::ShapeComponent;
use crate::render::render_mode::RenderMode;
use crate::render::scene_item::*;

use std::sync::Arc;
use std::sync::RwLock;

use eframe::wgpu;
use eframe::wgpu::naga::back::msl::sampler;
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

        let p = 4.0; //std::f32::consts::PI;//1.0 / (4.0 * std::f32::consts::PI); // Point light power normalization
        let intensity = [
            p * l[0] * scale[0],
            p * l[1] * scale[1],
            p * l[2] * scale[2],
        ];

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

        let inner_angle = f32::to_radians((coneangle - conedelta).max(0.0));
        let outer_angle = f32::to_radians(coneangle);

        let l = get_color(&props, "I", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
        let scale = get_color(&props, "scale", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);

        let p = 1.0; // / std::f32::consts::PI; // Point light power normalization
        let intensity = [
            p * l[0] * scale[0],
            p * l[1] * scale[1],
            p * l[2] * scale[2],
        ];
        let render_light = DiskRenderLight {
            id,
            edition: edition.clone(),
            position: [position.x, position.y, position.z], // Position is not used for spot lights
            direction: [direction.x, direction.y, direction.z], // Direction is not used for spot lights
            intensity: intensity,
            radius: 0.0,
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
        4.0 // Default area if radius is not specified
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
        radius * radius * std::f32::consts::PI // Area of the disk
    } else {
        1.0 // Default area if radius is not specified
    };
    //let area = 8.0 * area;
    //let area = 1.0;////radius * radius; // Assuming a disk for area calculation

    let props = light.as_property_map();
    let l = get_color(&props, "L", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let scale = get_color(&props, "scale", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);

    let coneangle = props.find_one_float("coneangle").unwrap_or(90.0);
    let conedelta = props.find_one_float("conedeltaangle").unwrap_or(90.0);
    let conedelta = conedelta.clamp(0.0, coneangle);

    let inner_angle = f32::to_radians((coneangle - conedelta).max(0.0));
    let outer_angle = f32::to_radians(coneangle);

    //let area = area * (1.0 - f32::powf(outer_angle/std::f32::consts::PI, 2.0));

    let position = Vector3::new(0.0, 0.0, 0.0); // Center of the disk
    let direction = Vector3::new(0.0, 0.0, 1.0); // Normal of the disk

    let intensity = [
        area * l[0] * scale[0],
        area * l[1] * scale[1],
        area * l[2] * scale[2],
    ];

    let render_light = DiskRenderLight {
        id,
        edition: edition.clone(),
        position: [position.x, position.y, position.z], // Position is not used for spot lights
        direction: [direction.x, direction.y, direction.z], // Direction is not used for spot lights
        intensity,
        radius, // Radius of the disk
        inner_angle,
        outer_angle,
    };
    let render_light = Arc::new(RenderLight::Disk(render_light));
    render_resource_manager.add_light(&render_light);

    let render_item = RenderLightItem {
        light: render_light.clone(),
        matrix: glam::Mat4::from(matrix),
    };
    return Some(RenderItem::Light(render_item));
}

fn get_rects_light_item(
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

    if let Some(render_light) = render_resource_manager.get_light(id) {
        if render_light.get_edition() == edition {
            let render_item = RenderLightItem {
                light: render_light.clone(),
                matrix: glam::Mat4::from(matrix),
            };
            return Some(RenderItem::Light(render_item));
        }
    }

    if let Some(mesh_data) = create_mesh_data(shape) {
        let mut rects = Vec::new();
        let planes = create_plane_meshes_from_mesh(&mesh_data, 0.99);
        for plane in planes {
            if let Some(outline) = create_plane_outline_from_plane_mesh(&plane) {
                if let Some(rect) = create_plane_rect_from_plane_outline(&outline, 0.99) {
                    rects.push(rect);
                }
            }
        }
        if !rects.is_empty() {
            let props = light.as_property_map();
            let l = get_color(&props, "L", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
            let scale =
                get_color(&props, "scale", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);

            let render_rects = rects
                .iter()
                .map(|rect| {
                    let position = rect.position;
                    let direction = rect.normal;
                    let u_axis = rect.u_axis;
                    let v_axis = rect.v_axis;

                    //let area = 1.0; //todo: get area from rect
                    let area = 4.0
                        * Vector3::cross(
                            &Vector3::new(u_axis[0], u_axis[1], u_axis[2]),
                            &Vector3::new(v_axis[0], v_axis[1], v_axis[2]),
                        )
                        .length();

                    let intensity = [
                        area * l[0] * scale[0],
                        area * l[1] * scale[1],
                        area * l[2] * scale[2],
                    ];

                    let light = RectRenderLight {
                        id,
                        edition: edition.clone(),
                        position,
                        direction,
                        u_axis,
                        v_axis,
                        intensity,
                    };
                    Arc::new(RenderLight::Rect(light))
                })
                .collect::<Vec<Arc<RenderLight>>>();

            let render_light = RectsRenderLight {
                id,
                edition: edition.clone(),
                rects: render_rects,
            };
            let render_light = Arc::new(RenderLight::_Rects(render_light));
            render_resource_manager.add_light(&render_light);

            let render_item = RenderLightItem {
                light: render_light.clone(),
                matrix: glam::Mat4::from(matrix),
            };
            return Some(RenderItem::Light(render_item));
        }
    }
    return None;
}

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
            return get_rects_light_item(
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

fn get_image_data(image: &image::DynamicImage) -> image::Rgba32FImage {
    return image.to_rgba32f();
}

fn get_texture_from_image(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &image::Rgba32FImage,
) -> wgpu::Texture {
    let dimensions = image.dimensions();
    let size = wgpu::Extent3d {
        width: dimensions.0,
        height: dimensions.1,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Render Texture"),
        size: size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let image_raw = image.as_raw();
    queue.write_texture(
        texture.as_image_copy(),
        bytemuck::cast_slice(image_raw),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * 4 * dimensions.0),
            rows_per_image: None,
        },
        size,
    );
    return texture;
}

fn get_render_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    resource_manager: &ResourceManager,
    resource_cache_manager: &mut ResourceCacheManager,
    render_resource_manager: &mut RenderResourceManager,
    mapname: &str,
) -> Option<Arc<RenderTexture>> {
    //println!("Searching for texture: {}", mapname);
    if let Some(texture) = resource_manager.find_texture_by_filename(mapname) {
        let texture = texture.read().unwrap();
        let texture_id = texture.get_id();
        let texture_edition = texture.get_edition();
        // println!("Found texture: {} (ID: {})", mapname, texture.get_id());
        if let Some(render_texture) = render_resource_manager.get_texture(texture_id) {
            if render_texture.edition == texture_edition {
                return Some(render_texture.clone());
            }
        }
        if let Some(texture_node) = resource_cache_manager.textures.get(&texture_id) {
            // println!("Loading texture: {} (ID: {})", mapname, texture_id);
            if let Some(image) =
                create_image_variant(texture_node, resource_manager, TexturePurpose::Render)
            {
                // println!("Texture image created: {} (ID: {})", mapname, texture_id);
                let image = image.read().unwrap();
                let image_data = get_image_data(&image);
                let texture = get_texture_from_image(device, queue, &image_data);
                let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("Render Texture Sampler"),
                    address_mode_u: wgpu::AddressMode::Repeat,
                    address_mode_v: wgpu::AddressMode::ClampToEdge,
                    ..Default::default()
                });
                let render_texture = RenderTexture {
                    id: texture_id,
                    edition: texture_edition.clone(),
                    texture,
                    sampler,
                };
                let render_texture = Arc::new(render_texture);
                render_resource_manager.add_texture(&render_texture);
                // println!("Loaded texture: {} (ID: {})", mapname, texture_id);
                return Some(render_texture);
            }
        }
    }
    return None; // Texture not found
}

fn get_infinite_light_item(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    item: &SceneItem,
    resource_manager: &ResourceManager,
    resource_cache_manager: &mut ResourceCacheManager,
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
            light_type == "infinite",
            "Expected light type to be 'infinite', found: {}",
            light_type
        );

        let props = light.as_property_map();

        let l = get_color(&props, "L", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
        let scale = get_color(&props, "scale", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);

        let p = 1.0; // / std::f32::consts::PI; // Point light power normalization
        let intensity = [
            p * l[0] * scale[0],
            p * l[1] * scale[1],
            p * l[2] * scale[2],
        ];

        let mapname = props.find_one_string("mapname").unwrap_or("".to_string());
        if mapname.is_empty() {
            return None; // No texture map specified for infinite light
        }
        if let Some(texture) = get_render_texture(
            device,
            queue,
            resource_manager,
            resource_cache_manager,
            render_resource_manager,
            &mapname,
        ) {
            let render_light = InfiniteRenderLight {
                id,
                edition: edition.clone(),
                intensity,
                texture: Some(texture),
            };
            let render_light = Arc::new(RenderLight::Infinite(render_light));
            render_resource_manager.add_light(&render_light);
            let render_item = RenderLightItem {
                light: render_light.clone(),
                matrix: glam::Mat4::from(item.matrix),
            };
            return Some(RenderItem::Light(render_item));
        }
    }
    return None; // Placeholder for light retrieval logic
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
        ..Default::default()
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

//private
fn get_render_light_item(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    item: &SceneItem,
    _mode: RenderMode,
    resource_manager: &ResourceManager,
    resource_cache_manager: &mut ResourceCacheManager,
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
            "infinite" => {
                return get_infinite_light_item(
                    device,
                    queue,
                    item,
                    resource_manager,
                    resource_cache_manager,
                    render_resource_manager,
                );
            }
            _ => {
                // Handle unknown or unsupported light types
                return None;
            }
        }
    }
    return None; // Placeholder for light retrieval logic
}

pub fn get_render_light_items(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    item: &SceneItem,
    _mode: RenderMode,
    resource_manager: &ResourceManager,
    resource_cache_manager: &mut ResourceCacheManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Vec<Arc<RenderItem>> {
    let mut render_items = Vec::new();
    if let Some(render_item) = get_render_light_item(
        device,
        queue,
        item,
        _mode,
        resource_manager,
        resource_cache_manager,
        render_resource_manager,
    ) {
        if let RenderItem::Light(light_item) = render_item {
            if let RenderLight::_Rects(rects) = light_item.light.as_ref() {
                //println!("Area light with {} rects", rects.rects.len());
                for light in rects.rects.iter() {
                    //
                    let render_item = RenderLightItem {
                        light: light.clone(),
                        matrix: light_item.matrix,
                    };
                    render_items.push(Arc::new(RenderItem::Light(render_item)));
                }
            } else {
                render_items.push(Arc::new(RenderItem::Light(light_item)));
            }
        }
    }
    return render_items;
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
