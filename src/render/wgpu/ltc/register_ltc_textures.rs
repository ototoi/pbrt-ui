use super::super::render_resource::RenderResourceManager;
use super::super::texture::RenderTexture;
use super::textures::*;

use eframe::wgpu;
use std::sync::Arc;
use uuid::Uuid;

pub const LTC1_UUID: Uuid = Uuid::from_bytes([0x1; 16]);
pub const LTC2_UUID: Uuid = Uuid::from_u128(0x2);

fn create_ltc_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    id: Uuid,
    name: &str,
    data: &[f32],
) -> RenderTexture {
    let texture_size = wgpu::Extent3d {
        width: LTC_LUT_SIZE as u32,
        height: LTC_LUT_SIZE as u32,
        depth_or_array_layers: 1,
    };

    let texture_name = format!("{} Texture", name);
    let texture_descriptor = wgpu::TextureDescriptor {
        label: Some(texture_name.as_str()),
        size: texture_size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    };
    let ltc_texture = device.create_texture(&texture_descriptor);
    queue.write_texture(
        ltc_texture.as_image_copy(),
        bytemuck::cast_slice(&data),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * 4 * LTC_LUT_SIZE as u32),
            rows_per_image: None,
        },
        texture_size,
    );

    let sampler_name = format!("{} Sampler", name);
    let ltc_view = ltc_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let ltc_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(sampler_name.as_str()),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });
    let ltc1 = RenderTexture {
        id: id,
        edition: Uuid::new_v4().to_string(),
        texture: ltc_texture,
        view: ltc_view,
        sampler: ltc_sampler,
    };
    return ltc1;
}

pub fn register_ltc_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    resource_manager: &mut RenderResourceManager,
) {
    if resource_manager.get_texture(LTC1_UUID).is_none() {
        let ltc1 = create_ltc_texture(device, queue, LTC1_UUID, "LTC1", &LTC1);
        let ltc1 = Arc::new(ltc1);
        resource_manager.add_texture(&ltc1);
    }
    if resource_manager.get_texture(LTC2_UUID).is_none() {
        let ltc2 = create_ltc_texture(device, queue, LTC2_UUID, "LTC2", &LTC2);
        let ltc2 = Arc::new(ltc2);
        resource_manager.add_texture(&ltc2);
    }
}
