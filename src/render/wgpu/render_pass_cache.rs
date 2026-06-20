use super::ltc::get_ltc_texture;
use super::material::RenderCategory;
use super::material::RenderPass;
use super::material::RenderUniformValue;
use super::render_resource::RenderResourceManager;
use super::shader::RenderShader;
use crate::preprocessor::Preprocessor;

use std::sync::Arc;

use eframe::wgpu;
use uuid::Uuid;

pub fn get_shader_type(
    shader_type: &str,
    uniform_values: &[(String, RenderUniformValue)],
    render_category: RenderCategory,
) -> String {
    let mut s = String::new();
    for (key, val) in uniform_values {
        match val {
            RenderUniformValue::Float(_) => s.push_str(&format!("_{}@F", key)),
            RenderUniformValue::Vec4(_) => s.push_str(&format!("_{}@V", key)),
            RenderUniformValue::Int(_) => s.push_str(&format!("_{}@I", key)),
            RenderUniformValue::Bool(_) => s.push_str(&format!("_{}@B", key)),
            RenderUniformValue::Mat4(_) => s.push_str(&format!("_{}@M", key)),
            RenderUniformValue::Texture(_) => s.push_str(&format!("_{}@T", key)),
        }
    }
    if uses_directional_light_shadow(render_category) {
        s.push_str("_ENABLE_DIRECTIONAL_LIGHT_SHADOW@D");
    }
    format!("{}{}", shader_type, s)
}

fn uses_directional_light_shadow(render_category: RenderCategory) -> bool {
    render_category != RenderCategory::Emissive
}

fn create_uniform_value_bytes(
    uniform_values: &[(String, RenderUniformValue)],
) -> (Vec<(String, String)>, Vec<u8>) {
    let mut type_variables: Vec<(String, String)> = Vec::new();
    let mut bytes: Vec<u8> = Vec::new();
    let mut padding_count = 1;
    let mut remain = 0;
    for (name, value) in uniform_values {
        match value {
            RenderUniformValue::Float(v) => {
                bytes.extend_from_slice(bytemuck::bytes_of(v));
                if remain == 0 {
                    remain = 4;
                }
                remain -= 1;
                type_variables.push(("f32".to_string(), name.clone()));
            }
            RenderUniformValue::Vec4(v) => {
                if remain != 0 {
                    for _ in 0..remain {
                        bytes.extend_from_slice(bytemuck::bytes_of(&0.0f32));
                        type_variables.push(("f32".to_string(), format!("_pad{}", padding_count)));
                        padding_count += 1;
                    }
                    remain = 0;
                }
                bytes.extend_from_slice(bytemuck::bytes_of(v));
                type_variables.push(("vec4<f32>".to_string(), name.clone()));
            }
            RenderUniformValue::Int(v) => {
                bytes.extend_from_slice(bytemuck::bytes_of(v));
                if remain == 0 {
                    remain = 4;
                }
                remain -= 1;
                type_variables.push(("i32".to_string(), name.clone()));
            }
            RenderUniformValue::Bool(v) => {
                let int_value: u32 = if *v { 1 } else { 0 };
                bytes.extend_from_slice(bytemuck::bytes_of(&int_value));
                if remain == 0 {
                    remain = 4;
                }
                remain -= 1;
                type_variables.push(("u32".to_string(), name.clone()));
            }
            RenderUniformValue::Mat4(v) => {
                if remain != 0 {
                    for _ in 0..remain {
                        bytes.extend_from_slice(bytemuck::bytes_of(&0.0f32));
                        type_variables.push(("f32".to_string(), format!("_pad{}", padding_count)));
                        padding_count += 1;
                    }
                    remain = 0;
                }
                bytes.extend_from_slice(bytemuck::bytes_of(v));
                type_variables.push(("mat4x4<f32>".to_string(), name.clone()));
            }
            RenderUniformValue::Texture(v) => {
                if remain != 0 {
                    for _ in 0..remain {
                        bytes.extend_from_slice(bytemuck::bytes_of(&0.0f32));
                        type_variables.push(("f32".to_string(), format!("_pad{}", padding_count)));
                        padding_count += 1;
                    }
                    remain = 0;
                }
                let scale_offset: [f32; 4] = [v.scale[0], v.scale[1], v.delta[0], v.delta[1]];
                bytes.extend_from_slice(bytemuck::bytes_of(&scale_offset));
                type_variables.push(("vec4<f32>".to_string(), format!("{}_uv_factor", name)));
            }
        }
    }
    if remain != 0 {
        for _ in 0..remain {
            bytes.extend_from_slice(bytemuck::bytes_of(&0.0f32));
            type_variables.push(("f32".to_string(), format!("_pad{}", padding_count)));
            padding_count += 1;
        }
    }
    (type_variables, bytes)
}

fn get_shader_id_from_type(shader_type: &str) -> Uuid {
    Uuid::new_v3(&Uuid::NAMESPACE_OID, shader_type.as_bytes())
}

fn get_fallback_shader_source() -> String {
    include_str!("shaders/basic_material.wgsl").to_string()
}

fn create_defines(
    uniform_values: &[(String, RenderUniformValue)],
    render_category: RenderCategory,
) -> Vec<(String, String)> {
    let mut defines = Vec::new();
    for (name, value) in uniform_values {
        if let RenderUniformValue::Texture(_) = value {
            defines.push((format!("USE_TEXTURE_{}", name.to_uppercase()), "1".to_string()));
        }
    }
    if uses_directional_light_shadow(render_category) {
        defines.push(("ENABLE_DIRECTIONAL_LIGHT_SHADOW".to_string(), "1".to_string()));
    }
    defines
}

fn generate_shader_source(
    shader_type: &str,
    uniform_values: &[(String, RenderUniformValue)],
    render_category: RenderCategory,
) -> Option<String> {
    let modified_shader_type = get_shader_type(shader_type, uniform_values, render_category);
    let cache_dir = dirs::cache_dir()
        .unwrap()
        .join("pbrt_ui")
        .join("assets")
        .join("shaders");
    let prebuilt_shader_path = cache_dir.join(format!("{}.wgsl", shader_type));

    let defines = create_defines(uniform_values, render_category);
    if prebuilt_shader_path.exists() {
        let base_path = dirs::cache_dir()
            .unwrap()
            .join("pbrt_ui")
            .join("assets")
            .join("shaders")
            .join("include");
        let mut pp = Preprocessor::with_base_path(base_path);
        for (key, value) in &defines {
            pp.define(key, value);
        }
        let input_data = std::fs::read_to_string(prebuilt_shader_path).unwrap();
        match pp.process(&input_data) {
            Ok(processed_data) => {
                let output_path = cache_dir
                    .join("generated")
                    .join(format!("{}.wgsl", modified_shader_type));
                std::fs::create_dir_all(output_path.parent().unwrap()).unwrap();
                std::fs::write(&output_path, &processed_data).unwrap();
                Some(processed_data)
            }
            Err(e) => {
                log::error!("Shader preprocessor error: {}", e);
                None
            }
        }
    } else {
        None
    }
}

fn get_shader_source(
    shader_type: &str,
    uniform_values: &[(String, RenderUniformValue)],
    render_category: RenderCategory,
) -> String {
    let cache_dir = dirs::cache_dir().unwrap().join("pbrt_ui").join("shaders");
    let shader_path = cache_dir.join(format!("{}.wgsl", shader_type));
    if shader_path.exists()
        && let Ok(code) = std::fs::read_to_string(shader_path)
    {
        return code;
    }
    if let Some(generated_source) =
        generate_shader_source(shader_type, uniform_values, render_category)
    {
        return generated_source;
    }
    get_fallback_shader_source()
}

fn get_shader_module(
    device: &wgpu::Device,
    shader_type: &str,
    uniform_values: &[(String, RenderUniformValue)],
    render_category: RenderCategory,
) -> wgpu::ShaderModule {
    let source = get_shader_source(shader_type, uniform_values, render_category);
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(format!("Shader : {}", shader_type).as_str()),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

pub fn create_render_shader(
    device: &wgpu::Device,
    _queue: &wgpu::Queue,
    shader_type: &str,
    uniform_values: &[(String, RenderUniformValue)],
    render_category: RenderCategory,
    render_resource_manager: &mut RenderResourceManager,
) -> Arc<RenderShader> {
    let modified_shader_type = get_shader_type(shader_type, uniform_values, render_category);
    let shader_id = get_shader_id_from_type(&modified_shader_type);
    if let Some(shader) = render_resource_manager.get_shader(shader_id) {
        return shader.clone();
    }
    let shader_module = get_shader_module(device, shader_type, uniform_values, render_category);
    let render_shader = Arc::new(RenderShader {
        id: shader_id,
        name: modified_shader_type,
        shader: Arc::new(shader_module),
    });
    render_resource_manager.add_shader(&render_shader);
    render_shader
}

pub fn create_render_pass(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    shader_type: &str,
    render_category: RenderCategory,
    uniform_values: &[(String, RenderUniformValue)],
    ltc_type: &str,
    render_resource_manager: &mut RenderResourceManager,
) -> Arc<RenderPass> {
    let shader = create_render_shader(
        device,
        queue,
        shader_type,
        uniform_values,
        render_category,
        render_resource_manager,
    );
    let (_uniform_values_types, uniform_values_bytes) = create_uniform_value_bytes(uniform_values);
    let mut textures = vec![];
    for (_name, value) in uniform_values {
        if let RenderUniformValue::Texture(texture) = value {
            textures.push(Some(texture.clone()));
        } else {
            textures.push(None);
        }
    }
    let ltc_texture = get_ltc_texture(device, queue, ltc_type, render_resource_manager);
    Arc::new(RenderPass {
        id: Uuid::new_v4(),
        shader,
        render_category,
        uniform_values: Arc::new(uniform_values_bytes),
        textures,
        ltc_texture,
    })
}
