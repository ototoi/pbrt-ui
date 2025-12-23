use super::super::render_resource::RenderResourceManager;
use super::super::texture::RenderTexture;
use super::textures::*;

use eframe::wgpu;
use std::sync::Arc;
use uuid::Uuid;

pub const LTC_GGX_TEXTURE_ID: Uuid = Uuid::from_u128(0x52eca5d6_c228_4136_8840_f3517bb488a3);
pub const LTC_OREN_NAYAR_TEXTURE_ID: Uuid = Uuid::from_u128(0x3f4d5e6c_7a8b_4c9d_8e0f_1a2b3c4d5e6f);
pub const LTC_MICROFACET_REFLECTION_TEXTURE_ID: Uuid =
    Uuid::from_u128(0xabcdef12_3456_7890_abcd_ef1234567890);

pub fn create_ltc_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    id: Uuid,
    data: &Vec<Vec<f32>>,
) -> RenderTexture {
    let count = data.len();
    let texture_size = wgpu::Extent3d {
        width: LTC_LUT_SIZE as u32,
        height: LTC_LUT_SIZE as u32,
        depth_or_array_layers: count as u32,
    };
    let texture_name = format!("{} Texture", label);
    let texture_descriptor = wgpu::TextureDescriptor {
        label: Some(&texture_name),
        size: texture_size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    };
    let texture = device.create_texture(&texture_descriptor);
    for layer in 0..count {
        let data: &Vec<f32> = &data[layer];
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: layer as u32,
                },
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(data),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * 4 * LTC_LUT_SIZE as u32),
                rows_per_image: Some(LTC_LUT_SIZE as u32),
            },
            wgpu::Extent3d {
                width: LTC_LUT_SIZE as u32,
                height: LTC_LUT_SIZE as u32,
                depth_or_array_layers: 1,
            },
        );
    }
    let view_name = format!("{} View", label);
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some(&view_name),
        format: None,
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        aspect: wgpu::TextureAspect::All,
        base_mip_level: 0,
        mip_level_count: None,
        base_array_layer: 0,
        array_layer_count: None,
        usage: None,
    });
    let sampler_name = format!("{} Sampler", label);
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(&sampler_name),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });
    
    RenderTexture {
        id,
        edition: Uuid::new_v4().to_string(),
        texture,
        view,
        sampler,
        scale: [1.0, 1.0],
        delta: [0.0, 0.0],
    }
}

fn get_id_and_textures(name: &str) -> Option<(Uuid, Vec<Vec<f32>>)> {
    match name.to_lowercase().as_str() {
        "ggx" => Some((
            LTC_GGX_TEXTURE_ID,
            vec![LTC_GGX_1.to_vec(), LTC_GGX_2.to_vec()],
        )),
        "oren_nayar" | "orennayar" => Some((
            LTC_OREN_NAYAR_TEXTURE_ID,
            vec![LTC_OREN_NAYAR_1.to_vec(), LTC_OREN_NAYAR_2.to_vec()],
        )),
        "microfacet_reflection" | "microfacetreflection" => Some((
            LTC_MICROFACET_REFLECTION_TEXTURE_ID,
            vec![
                LTC_MICROFACET_REFLECTION_1.to_vec(),
                LTC_MICROFACET_REFLECTION_2.to_vec(),
            ],
        )),
        _ => None,
    }
}

pub fn get_ltc_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    name: &str,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<Arc<RenderTexture>> {
    if name.is_empty() {
        return None;
    }
    if let Some((id, data)) = get_id_and_textures(name) {
        if let Some(render_texture) = render_resource_manager.get_texture(id) {
            return Some(render_texture.clone());
        }
        let render_texture = create_ltc_texture(device, queue, "LTC", id, &data);
        let render_texture = Arc::new(render_texture);
        render_resource_manager.add_texture(&render_texture);
        return Some(render_texture);
    }
    None
}

/*
pub fn register_ltc_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    resource_manager: &mut RenderResourceManager,
) {
    if resource_manager.get_texture(DEFAULT_LTC_UUID).is_none() {
        let data = vec![LTC1.to_vec(), LTC2.to_vec()];
        let texture = create_ltc_texture(device, queue, "LTC", DEFAULT_LTC_UUID, &data);
        resource_manager.add_texture(&Arc::new(texture));
    }
}
    */
