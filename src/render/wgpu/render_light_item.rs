use super::light::DirectionalRenderLight;
use super::light::DirectionalShadowProjection;
use super::light::DiskRenderLight;
use super::light::InfiniteRenderLight;
use super::light::RectRenderLight;
use super::light::RectsRenderLight;
use super::light::RenderLight;
use super::light::SphereRenderLight;
use super::lines::RenderLines;
use super::material::RenderMaterial;
use super::material::RenderUniformValue;
use super::render_item::LightRenderItem;
use super::render_item::LinesRenderItem;
use super::render_item::RenderItem;
use super::render_item::create_render_pass;
use super::render_item::get_color;
use super::render_resource::RenderResourceManager;
use super::texture::RenderTexture;
use crate::conversion::light_shape::create_light_shape;
use crate::conversion::mesh_data::create_mesh_data;
use crate::conversion::plane_data::create_plane_meshes_from_mesh;
use crate::conversion::plane_data::create_plane_outline_from_plane_mesh;
use crate::conversion::plane_data::create_plane_rect_from_plane_outline;
use crate::conversion::texture_node::DynaImage;
use crate::conversion::texture_node::TextureSizeType;
use crate::conversion::texture_node::create_image_variant;
use crate::model::base::Matrix4x4;
use crate::model::base::Vector3;
use crate::model::scene::Light;
use crate::model::scene::LightComponent;
use crate::model::scene::Node;
use crate::model::scene::RenderLightComponent;
use crate::model::scene::ResourceCacheManager;
use crate::model::scene::ResourceManager;
use crate::model::scene::Shape;
use crate::model::scene::ShapeComponent;
use crate::render::render_mode::RenderMode;
use crate::render::scene_item::*;
use crate::render::wgpu::material::RenderCategory;

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

fn get_cached_render_light(node: &Arc<RwLock<Node>>, edition: &str) -> Option<Arc<RenderLight>> {
    let node = node.read().unwrap();
    let component = node.get_component::<RenderLightComponent>()?;
    let render_light = component.get_render_light()?;
    if render_light.get_edition() == edition {
        Some(render_light)
    } else {
        None
    }
}

fn set_cached_render_light(node: &Arc<RwLock<Node>>, render_light: Arc<RenderLight>) {
    let mut node = node.write().unwrap();
    if let Some(component) = node.get_component_mut::<RenderLightComponent>() {
        component.set_render_light(render_light);
    } else {
        let mut component = RenderLightComponent::new();
        component.set_render_light(render_light);
        node.add_component(component);
    }
}

struct DirectionalLightData {
    id: Uuid,
    edition: String,
    direction: [f32; 3],
    intensity: [f32; 3],
    source_angle: f32,
    cast_shadow: bool,
    shadow_bias: f32,
    shadow_slope_bias: f32,
    cascade_count: u32,
    shadow_projection: DirectionalShadowProjection,
}

struct PointLightData {
    id: Uuid,
    edition: String,
    matrix: glam::Mat4,
    intensity: [f32; 3],
}

struct SpotLightData {
    id: Uuid,
    edition: String,
    position: [f32; 3],
    direction: [f32; 3],
    intensity: [f32; 3],
    inner_angle: f32,
    outer_angle: f32,
}

struct InfiniteLightData {
    id: Uuid,
    edition: String,
    intensity: [f32; 3],
    mapname: String,
    matrix: glam::Mat4,
}

fn read_directional_light_data(
    node: &Arc<RwLock<Node>>,
    resource_manager: &ResourceManager,
) -> Option<DirectionalLightData> {
    let light = {
        let node_ref = node.read().unwrap();
        node_ref.get_component::<LightComponent>()?.get_light()
    };
    let light = light.read().unwrap();
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

    let l = get_color(props, "L", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let scale = get_color(props, "scale", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let source_angle = props.find_one_float("sourceangle").unwrap_or(0.5357).max(0.2);
    let shadow_projection = props
        .find_one_string("shadowprojection")
        .unwrap_or_else(|| "csm".to_string())
        .to_lowercase();

    Some(DirectionalLightData {
        id: light.get_id(),
        edition: light.get_edition(),
        direction: [dir.x, dir.y, dir.z],
        intensity: [l[0] * scale[0], l[1] * scale[1], l[2] * scale[2]],
        source_angle: source_angle.to_radians(),
        cast_shadow: props
            .find_one_bool("castshadow")
            .or_else(|| props.find_one_bool("castshadows"))
            .unwrap_or(true),
        shadow_bias: props.find_one_float("shadowbias").unwrap_or(0.001).max(0.0),
        shadow_slope_bias: props
            .find_one_float("shadowslopebias")
            .unwrap_or(0.01)
            .max(0.0),
        cascade_count: props.find_one_int("cascadecount").unwrap_or(4).clamp(1, 4) as u32,
        shadow_projection: if shadow_projection == "lspsm" {
            DirectionalShadowProjection::Lspsm
        } else {
            DirectionalShadowProjection::Csm
        },
    })
}

fn read_point_light_data(
    item: &SceneItem,
    node: &Arc<RwLock<Node>>,
    resource_manager: &ResourceManager,
) -> Option<PointLightData> {
    let light = {
        let node_ref = node.read().unwrap();
        node_ref.get_component::<LightComponent>()?.get_light()
    };
    let light = light.read().unwrap();
    let props = light.as_property_map();

    let mut from = props.get_floats("from");
    if from.len() != 3 {
        from = vec![0.0, 0.0, 0.0];
    }
    let translation = Matrix4x4::translate(from[0], from[1], from[2]);
    let l = get_color(props, "I", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let scale = get_color(props, "scale", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);

    Some(PointLightData {
        id: light.get_id(),
        edition: light.get_edition(),
        matrix: glam::Mat4::from(translation * item.matrix),
        intensity: [4.0 * l[0] * scale[0], 4.0 * l[1] * scale[1], 4.0 * l[2] * scale[2]],
    })
}

fn read_spot_light_data(
    item: &SceneItem,
    node: &Arc<RwLock<Node>>,
    resource_manager: &ResourceManager,
) -> Option<SpotLightData> {
    let light = {
        let node_ref = node.read().unwrap();
        node_ref.get_component::<LightComponent>()?.get_light()
    };
    let light = light.read().unwrap();
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
    let position = mat.transform_point(&Vector3::new(0.0, 0.0, 0.0));
    let direction = mat.transform_vector(&Vector3::new(0.0, 0.0, 1.0)).normalize();

    let coneangle = props.find_one_float("coneangle").unwrap_or(30.0);
    let conedelta = props
        .find_one_float("conedeltaangle")
        .unwrap_or(props.find_one_float("conedelta").unwrap_or(5.0))
        .clamp(0.0, coneangle);
    let l = get_color(props, "I", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let scale = get_color(props, "scale", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);

    Some(SpotLightData {
        id: light.get_id(),
        edition: light.get_edition(),
        position: [position.x, position.y, position.z],
        direction: [direction.x, direction.y, direction.z],
        intensity: [l[0] * scale[0], l[1] * scale[1], l[2] * scale[2]],
        inner_angle: f32::to_radians((coneangle - conedelta).max(0.0)),
        outer_angle: f32::to_radians(coneangle),
    })
}

fn read_infinite_light_data(
    item: &SceneItem,
    node: &Arc<RwLock<Node>>,
    resource_manager: &ResourceManager,
) -> Option<InfiniteLightData> {
    let light = {
        let node_ref = node.read().unwrap();
        node_ref.get_component::<LightComponent>()?.get_light()
    };
    let light = light.read().unwrap();
    let props = light.as_property_map();
    let l = get_color(props, "L", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let scale = get_color(props, "scale", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let mapname = props.find_one_string("mapname").unwrap_or_default();

    Some(InfiniteLightData {
        id: light.get_id(),
        edition: light.get_edition(),
        intensity: [l[0] * scale[0], l[1] * scale[1], l[2] * scale[2]],
        mapname,
        matrix: glam::Mat4::from(get_rotation_matrix(&item.matrix)),
    })
}

fn get_directional_light_item(
    item: &SceneItem,
    resource_manager: &ResourceManager,
    _render_resource_manager: &mut RenderResourceManager,
) -> Option<RenderItem> {
    let node = &item.node;
    let light_type = get_light_type(node)?;
    assert!(
        light_type == "distant",
        "Expected light type to be 'distant', found: {}",
        light_type
    );
    let data = read_directional_light_data(node, resource_manager)?;
    if let Some(render_light) = get_cached_render_light(node, &data.edition) {
        let render_item = LightRenderItem {
            light: render_light,
            matrix: glam::Mat4::from(item.matrix),
        };
        return Some(RenderItem::Light(render_item));
    }
    let render_light = Arc::new(RenderLight::Directional(DirectionalRenderLight {
        id: data.id,
        edition: data.edition.clone(),
        direction: data.direction,
        intensity: data.intensity,
        source_angle: data.source_angle,
        cast_shadow: data.cast_shadow,
        shadow_bias: data.shadow_bias,
        shadow_slope_bias: data.shadow_slope_bias,
        cascade_count: data.cascade_count,
        shadow_projection: data.shadow_projection,
        ..Default::default()
    }));
    set_cached_render_light(node, render_light.clone());
    let render_item = LightRenderItem {
        light: render_light.clone(),
        matrix: glam::Mat4::from(item.matrix),
    };
    Some(RenderItem::Light(render_item))
}

fn get_point_light_item(
    item: &SceneItem,
    resource_manager: &ResourceManager,
    _render_resource_manager: &mut RenderResourceManager,
) -> Option<RenderItem> {
    let node = &item.node;
    let light_type = get_light_type(node)?;
    assert!(
        light_type == "point",
        "Expected light type to be 'point', found: {}",
        light_type
    );
    let data = read_point_light_data(item, node, resource_manager)?;
    if let Some(render_light) = get_cached_render_light(node, &data.edition) {
            let render_item = LightRenderItem {
                light: render_light,
                matrix: data.matrix,
            };
            return Some(RenderItem::Light(render_item));
    }
    let render_light = Arc::new(RenderLight::Sphere(SphereRenderLight {
        id: data.id,
        edition: data.edition.clone(),
        intensity: data.intensity,
        radius: 0.0,
        ..Default::default()
    }));
    set_cached_render_light(node, render_light.clone());

    let render_item = LightRenderItem {
        light: render_light.clone(),
        matrix: data.matrix,
    };
    Some(RenderItem::Light(render_item))
}

fn get_spot_light_item(
    item: &SceneItem,
    resource_manager: &ResourceManager,
    _render_resource_manager: &mut RenderResourceManager,
) -> Option<RenderItem> {
    let node = &item.node;
    let light_type = get_light_type(node)?;
    assert!(
        light_type == "spot",
        "Expected light type to be 'point', found: {}",
        light_type
    );
    let data = read_spot_light_data(item, node, resource_manager)?;
    if let Some(render_light) = get_cached_render_light(node, &data.edition) {
            let render_item = LightRenderItem {
                light: render_light,
                matrix: glam::Mat4::from(item.matrix),
            };
            return Some(RenderItem::Light(render_item));
    }
    let render_light = Arc::new(RenderLight::Disk(DiskRenderLight {
        id: data.id,
        edition: data.edition.clone(),
        position: data.position,
        direction: data.direction,
        intensity: data.intensity,
        radius: 0.0,
        inner_angle: data.inner_angle,
        outer_angle: data.outer_angle,
        ..Default::default()
    }));
    set_cached_render_light(node, render_light.clone());

    let render_item = LightRenderItem {
        light: render_light.clone(),
        matrix: glam::Mat4::from(item.matrix),
    };
    Some(RenderItem::Light(render_item))
}

fn get_sphere_light_item(
    node: &Arc<RwLock<Node>>,
    light: &Light,
    shape: &Shape,
    matrix: &Matrix4x4,
    resource_manager: &ResourceManager,
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

    if let Some(render_light) = get_cached_render_light(node, &edition) {
        let render_item = LightRenderItem {
            light: render_light,
            matrix: glam::Mat4::from(matrix),
        };
        return Some(RenderItem::Light(render_item));
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
        //std::f32::consts::PI * radius * radius // Area of the disk
        //std::f32::consts::PI * std::f32::consts::PI * std::f32::consts::PI * std::f32::consts::PI
        1.0
    } else {
        4.0 // Default area if radius is not specified
    };

    let props = light.as_property_map();
    let l = get_color(props, "L", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let scale = get_color(props, "scale", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);

    let intensity = [
        area * l[0] * scale[0],
        area * l[1] * scale[1],
        area * l[2] * scale[2],
    ];

    let render_light = SphereRenderLight {
        id,
        edition: edition.clone(),
        position: [0.0, 0.0, 0.0], // Position is not used for sphere lights
        intensity,
        radius,
    };
    let render_light = Arc::new(RenderLight::Sphere(render_light));
    set_cached_render_light(node, render_light.clone());

    let render_item = LightRenderItem {
        light: render_light.clone(),
        matrix: glam::Mat4::from(matrix),
    };
    return Some(RenderItem::Light(render_item));
}

fn get_disk_light_item(
    node: &Arc<RwLock<Node>>,
    light: &Light,
    shape: &Shape,
    matrix: &Matrix4x4,
    resource_manager: &ResourceManager,
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

    if let Some(render_light) = get_cached_render_light(node, &edition) {
        let render_item = LightRenderItem {
            light: render_light,
            matrix: glam::Mat4::from(matrix),
        };
        return Some(RenderItem::Light(render_item));
    }

    let radius = shape
        .as_property_map()
        .find_one_float("radius")
        .unwrap_or(1.0);

    let area = if radius > 0.0 {
        1.0 // Area of the disk
    } else {
        1.0 // Default area if radius is not specified
    };
    //let area = 8.0 * area;
    //let area = 1.0;////radius * radius; // Assuming a disk for area calculation

    let props = light.as_property_map();
    let l = get_color(props, "L", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let scale = get_color(props, "scale", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);

    let coneangle = props.find_one_float("coneangle").unwrap_or(90.0);
    let conedelta = props.find_one_float("conedeltaangle").unwrap_or(90.0);
    let conedelta = conedelta.clamp(0.0, coneangle);

    let inner_angle = f32::to_radians((coneangle - conedelta).max(0.0));
    let outer_angle = f32::to_radians(coneangle);

    let twosided = props.find_one_bool("twosided").unwrap_or(false);

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
        twosided,
    };
    let render_light = Arc::new(RenderLight::Disk(render_light));
    set_cached_render_light(node, render_light.clone());

    let render_item = LightRenderItem {
        light: render_light.clone(),
        matrix: glam::Mat4::from(matrix),
    };
    return Some(RenderItem::Light(render_item));
}

fn get_rects_light_item(
    node: &Arc<RwLock<Node>>,
    light: &Light,
    shape: &Shape,
    matrix: &Matrix4x4,
    resource_manager: &ResourceManager,
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

    if let Some(render_light) = get_cached_render_light(node, &edition) {
        let render_item = LightRenderItem {
            light: render_light,
            matrix: glam::Mat4::from(matrix),
        };
        return Some(RenderItem::Light(render_item));
    }

    if let Some(mesh_data) = create_mesh_data(shape) {
        let mut rects = Vec::new();
        let planes = create_plane_meshes_from_mesh(&mesh_data, 0.99);
        for plane in planes {
            if let Some(outline) = create_plane_outline_from_plane_mesh(&plane)
                && let Some(rect) = create_plane_rect_from_plane_outline(&outline, 0.99)
            {
                rects.push(rect);
            }
        }
        if !rects.is_empty() {
            let props = light.as_property_map();
            let l = get_color(props, "L", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
            let scale = get_color(props, "scale", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);

            let twosided = props.find_one_bool("twosided").unwrap_or(false);

            let render_rects = rects
                .iter()
                .map(|rect| {
                    let position = rect.position;
                    let direction = rect.normal;
                    let u_axis = rect.u_axis;
                    let v_axis = rect.v_axis;

                    //let area = 1.0; //todo: get area from rect
                    let area = 8.0
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
                        twosided,
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
            set_cached_render_light(node, render_light.clone());

            let render_item = LightRenderItem {
                light: render_light.clone(),
                matrix: glam::Mat4::from(matrix),
            };
            return Some(RenderItem::Light(render_item));
        }
    }
    return None;
}

fn get_area_light_item_core(
    node: &Arc<RwLock<Node>>,
    light: &Light,
    shape: &Shape,
    matrix: &Matrix4x4,
    resource_manager: &ResourceManager,
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
                node,
                light,
                shape,
                matrix,
                resource_manager,
            );
        }
        "disk" => {
            return get_disk_light_item(
                node,
                light,
                shape,
                matrix,
                resource_manager,
            );
        }
        "trianglemesh" | "plymesh" => {
            return get_rects_light_item(
                node,
                light,
                shape,
                matrix,
                resource_manager,
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
) -> Option<RenderItem> {
    let node = &item.node;
    let (light, shape) = {
        let node_ref = node.read().unwrap();
        let light = node_ref.get_component::<LightComponent>()?.get_light();
        let shape = node_ref.get_component::<ShapeComponent>()?.get_shape();
        (light, shape)
    };
    let light = light.read().unwrap();
    let shape = shape.read().unwrap();
    get_area_light_item_core(&item.node, &light, &shape, &item.matrix, resource_manager)
}

fn get_image_data(image: &DynaImage) -> image::Rgba32FImage {
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
        size,
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
        if let Some(render_texture) = render_resource_manager.get_texture(texture_id)
            && render_texture.edition == texture_edition
        {
            return Some(render_texture.clone());
        }
        if let Some(texture_node) = resource_cache_manager.textures.get(&texture_id) {
            // println!("Loading texture: {} (ID: {})", mapname, texture_id);
            if let Some(image) =
                create_image_variant(texture_node, resource_manager, TextureSizeType::Render)
            {
                // println!("Texture image created: {} (ID: {})", mapname, texture_id);
                let image = image.read().unwrap();
                let image_data = get_image_data(&image);
                let texture = get_texture_from_image(device, queue, &image_data);
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("Render Texture Sampler"),
                    address_mode_u: wgpu::AddressMode::Repeat,
                    address_mode_v: wgpu::AddressMode::ClampToEdge,
                    min_filter: wgpu::FilterMode::Linear,
                    mag_filter: wgpu::FilterMode::Linear,
                    ..Default::default()
                });
                let render_texture = RenderTexture {
                    id: texture_id,
                    edition: texture_edition.clone(),
                    texture,
                    view,
                    sampler,
                    scale: [1.0, 1.0],
                    delta: [0.0, 0.0],
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

fn get_rotation_matrix(matrix: &Matrix4x4) -> Matrix4x4 {
    let mut rot = *matrix;
    rot.m[3] = 0.0;
    rot.m[7] = 0.0;
    rot.m[11] = 0.0;
    return rot;
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
    let light_type = get_light_type(node)?;
    assert!(
        light_type == "infinite",
        "Expected light type to be 'infinite', found: {}",
        light_type
    );
    let data = read_infinite_light_data(item, node, resource_manager)?;
    if let Some(render_light) = get_cached_render_light(node, &data.edition) {
            let render_item = LightRenderItem {
                light: render_light,
                matrix: glam::Mat4::from(item.matrix),
            };
            return Some(RenderItem::Light(render_item));
    }
    if data.mapname.is_empty() {
        return None;
    }
    if let Some(texture) = get_render_texture(
        device,
        queue,
        resource_manager,
        resource_cache_manager,
        render_resource_manager,
        &data.mapname,
    ) {
        let render_light = Arc::new(RenderLight::Infinite(InfiniteRenderLight {
            id: data.id,
            edition: data.edition.clone(),
            intensity: data.intensity,
            texture: Some(texture.clone()),
        }));
        set_cached_render_light(node, render_light.clone());
        let render_item = LightRenderItem {
            light: render_light.clone(),
            matrix: data.matrix,
        };
        return Some(RenderItem::Light(render_item));
    }
    None
}

fn get_lines_material(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    id: Uuid,
    edition: &str,
    render_resource_manager: &mut RenderResourceManager,
    base_color: &[f32; 4],
) -> Option<Arc<RenderMaterial>> {
    if let Some(mat) = render_resource_manager.get_material(id)
        && mat.edition == edition
    {
        return Some(mat.clone());
    }
    // Create a default material for the light gizmo
    let mut uniform_values = Vec::new();
    uniform_values.push((
        "base_color".to_string(),
        RenderUniformValue::Vec4(*base_color),
    ));
    let passes = vec![create_render_pass(
        device,
        queue,
        "lines",
        RenderCategory::Opaque,
        &uniform_values,
        "",
        render_resource_manager,
    )];
    let render_material = RenderMaterial {
        id,
        edition: edition.to_string(),
        passes,
        ..Default::default()
    };
    let render_material = Arc::new(render_material);
    render_resource_manager.add_material(&render_material);
    return Some(render_material);
}

fn get_light_gizmo_material(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    node: &Arc<RwLock<Node>>,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<Arc<RenderMaterial>> {
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<LightComponent>() {
        let light = component.get_light();
        let light = light.read().unwrap();
        //let light_id = light.get_id();
        let light_type = light.get_type();
        let light_id = Uuid::new_v3(&Uuid::NAMESPACE_OID, light_type.as_bytes());
        let edition = "".to_string();
        let base_color = [1.0, 1.0, 0.0, 1.0]; // Default Yellow color for light gizmo
        return get_lines_material(
            device,
            queue,
            light_id,
            &edition,
            render_resource_manager,
            &base_color,
        );
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
        if let Some(lines) = render_resource_manager.get_lines(id)
            && lines.edition == edition
        {
            return Some(lines.clone());
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
                return get_area_light_item(item, resource_manager); // Area lights are not yet supported
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
    ) && let RenderItem::Light(light_item) = render_item
    {
        if let RenderLight::_Rects(rects) = light_item.light.as_ref() {
            //println!("Area light with {} rects", rects.rects.len());
            for light in rects.rects.iter() {
                //
                let render_item = LightRenderItem {
                    light: light.clone(),
                    matrix: light_item.matrix,
                };
                render_items.push(Arc::new(RenderItem::Light(render_item)));
            }
        } else {
            render_items.push(Arc::new(RenderItem::Light(light_item)));
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
        let material = get_light_gizmo_material(device, queue, &item.node, render_resource_manager);
        let render_item = LinesRenderItem {
            lines,
            material,
            matrix,
        };
        return Some(RenderItem::Lines(render_item));
    }
    return None;
}
