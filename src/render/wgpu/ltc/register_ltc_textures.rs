use super::super::render_resource::RenderResourceManager;
use super::super::texture::RenderTexture;
use super::textures::*;

use eframe::wgpu;
use std::sync::Arc;
use uuid::Uuid;


pub const DEFAULT_LTC_UUID: Uuid = Uuid::from_u128(0x123e4567_e89b_12d3_a456_426614174000);

fn create_ltc_texture(
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
    let render_texture = RenderTexture {
        id,
        edition: Uuid::new_v4().to_string(),
        texture,
        view,
        sampler,
    };
    return render_texture;
}

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
