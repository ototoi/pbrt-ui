use super::super::render_resource::RenderResourceManager;
use super::super::texture::RenderTexture;
use super::textures::*;

use eframe::wgpu;
use std::sync::Arc;
use uuid::Uuid;

pub const LTC_UUID: Uuid = Uuid::from_bytes([0x1; 16]);

fn create_ltc_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    data: &Vec<Vec<f32>>,
) -> wgpu::Texture {
    let count = data.len();
    let texture_size = wgpu::Extent3d {
        width: LTC_LUT_SIZE as u32,
        height: LTC_LUT_SIZE as u32,
        depth_or_array_layers: count as u32,
    };
    let texture_descriptor = wgpu::TextureDescriptor {
        label: Some(label),
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
    return texture;
}

pub fn register_ltc_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    resource_manager: &mut RenderResourceManager,
) {
    if resource_manager.get_texture(LTC_UUID).is_none() {
        let data = vec![LTC1.to_vec(), LTC2.to_vec()];
        let texture = create_ltc_texture(device, queue, "LTC", &data);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("LTC_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let render_texture = RenderTexture {
            id: LTC_UUID,
            edition: Uuid::new_v4().to_string(),
            texture,
            view,
            sampler,
        };
        resource_manager.add_texture(&Arc::new(render_texture));
    }
}
