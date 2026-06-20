use super::render_item::get_float;
use super::render_item::get_string;
use super::render_resource::RenderResourceManager;
use super::texture::RenderTexture;
use crate::conversion::normal_map::convert_luma8_to_normal_map;
use crate::conversion::normal_map::convert_luma32f_to_normal_map;
use crate::conversion::texture_node::DynaImage;
use crate::conversion::texture_node::TextureSizeType;
use crate::model::scene::ResourceCacheManager;
use crate::model::scene::ResourceManager;

use std::collections::HashSet;
use std::sync::Arc;

use eframe::wgpu;
use uuid::Uuid;

pub fn get_texture(
    props: &crate::model::base::PropertyMap,
    key: &str,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<Arc<RenderTexture>> {
    if let Some((key_type, _key_name, value)) = props.entry(key)
        && key_type == "texture"
        && let crate::model::base::Property::Strings(v) = value
        && !v.is_empty()
    {
        let name = v[0].clone();
        if let Some(texture) = resource_manager.find_texture_by_name(&name) {
            let texture = texture.read().unwrap();
            let texture_id = texture.get_id();
            if let Some(render_texture) = render_resource_manager.get_texture(texture_id) {
                return Some(render_texture.clone());
            }
        }
    }
    None
}

fn convert_to_f16(data: &[f32]) -> Vec<half::f16> {
    let mut f16_data = Vec::with_capacity(data.len());
    for &value in data {
        f16_data.push(half::f16::from_f32(value));
    }
    f16_data
}

fn get_texture_from_rgba_image(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &image::RgbaImage,
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
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let image = image::imageops::flip_vertical(image);
    let image_raw = image.as_raw();
    queue.write_texture(
        texture.as_image_copy(),
        bytemuck::cast_slice(image_raw),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * dimensions.0),
            rows_per_image: None,
        },
        size,
    );
    texture
}

fn get_normal_map_texture_from_rgba_image(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &image::RgbaImage,
) -> wgpu::Texture {
    let dimensions = image.dimensions();
    let size = wgpu::Extent3d {
        width: dimensions.0,
        height: dimensions.1,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Normal Map Texture"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let image = image::imageops::flip_vertical(image);
    let image_raw = image.as_raw();
    queue.write_texture(
        texture.as_image_copy(),
        bytemuck::cast_slice(image_raw),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * dimensions.0),
            rows_per_image: None,
        },
        size,
    );
    texture
}

fn get_texture_from_rgba32f_image(
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
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let image = image::imageops::flip_vertical(image);
    let image_raw = convert_to_f16(image.as_raw());
    queue.write_texture(
        texture.as_image_copy(),
        bytemuck::cast_slice(&image_raw),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(2 * 4 * dimensions.0),
            rows_per_image: None,
        },
        size,
    );
    texture
}

fn get_color_texture_from_image(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture_image: &DynaImage,
) -> Option<wgpu::Texture> {
    match texture_image {
        DynaImage::ImageRgb32F(_) => {
            let img = texture_image.to_rgba32f();
            Some(get_texture_from_rgba32f_image(device, queue, &img))
        }
        DynaImage::ImageRgb8(_) => {
            let img = texture_image.to_rgba8();
            Some(get_texture_from_rgba_image(device, queue, &img))
        }
        _ => {
            let img = texture_image.to_rgba32f();
            Some(get_texture_from_rgba32f_image(device, queue, &img))
        }
    }
}

fn covert_rgb32f_to_luma32f(
    image: &image::Rgb32FImage,
) -> image::ImageBuffer<image::Luma<f32>, Vec<f32>> {
    let (width, height) = image.dimensions();
    let mut luma_image = image::ImageBuffer::<image::Luma<f32>, Vec<f32>>::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let pixel = image.get_pixel(x, y);
            let r = pixel[0];
            let g = pixel[1];
            let b = pixel[2];
            let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            luma_image.put_pixel(x, y, image::Luma([luma]));
        }
    }
    luma_image
}

fn convert_rgb8_to_luma8(image: &image::RgbImage) -> image::ImageBuffer<image::Luma<u8>, Vec<u8>> {
    let (width, height) = image.dimensions();
    let mut luma_image = image::ImageBuffer::<image::Luma<u8>, Vec<u8>>::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let pixel = image.get_pixel(x, y);
            let r = pixel[0] as f32;
            let g = pixel[1] as f32;
            let b = pixel[2] as f32;
            let luma = (0.2126 * r + 0.7152 * g + 0.0722 * b).clamp(0.0, 255.0) as u8;
            luma_image.put_pixel(x, y, image::Luma([luma]));
        }
    }
    luma_image
}

fn get_normal_texture_from_image(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture_image: &DynaImage,
) -> Option<wgpu::Texture> {
    match texture_image {
        DynaImage::ImageLuma8(img) => {
            let normal_map = convert_luma8_to_normal_map(img);
            Some(get_normal_map_texture_from_rgba_image(
                device,
                queue,
                &normal_map,
            ))
        }
        DynaImage::ImageLuma32F(img) => {
            let normal_map = convert_luma32f_to_normal_map(img);
            Some(get_normal_map_texture_from_rgba_image(
                device,
                queue,
                &normal_map,
            ))
        }
        DynaImage::ImageRgb8(img) => {
            let img = convert_rgb8_to_luma8(img);
            let normal_map = convert_luma8_to_normal_map(&img);
            Some(get_normal_map_texture_from_rgba_image(
                device,
                queue,
                &normal_map,
            ))
        }
        DynaImage::ImageRgb32F(img) => {
            let img = covert_rgb32f_to_luma32f(img);
            let normal_map = convert_luma32f_to_normal_map(&img);
            Some(get_normal_map_texture_from_rgba_image(
                device,
                queue,
                &normal_map,
            ))
        }
    }
}

fn convert_address_mode(wrap: &str) -> wgpu::AddressMode {
    match wrap {
        "repeat" => wgpu::AddressMode::Repeat,
        "black" => wgpu::AddressMode::ClampToBorder,
        "clamp" => wgpu::AddressMode::ClampToEdge,
        _ => wgpu::AddressMode::Repeat,
    }
}

pub fn create_render_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    resource_manager: &ResourceManager,
    resource_cache_manager: &ResourceCacheManager,
    render_resource_manager: &mut RenderResourceManager,
    size_type: TextureSizeType,
) {
    let mut is_bump_map: HashSet<Uuid> = HashSet::new();
    for material in resource_manager.materials.values() {
        let material = material.read().unwrap();
        if let Some(bump) = get_string(material.as_property_map(), "bumpmap")
            && let Some(texture) = resource_manager.find_texture_by_name(&bump)
        {
            let texture = texture.read().unwrap();
            is_bump_map.insert(texture.get_id());
        }
    }

    for texture in resource_manager.textures.values() {
        let texture = texture.read().unwrap();
        let texture_id = texture.get_id();
        let texture_edition = texture.get_edition();
        if let Some(render_texture) = render_resource_manager.get_texture(texture_id)
            && texture_edition == render_texture.edition
        {
            continue;
        }
        let wrap = get_string(texture.as_property_map(), "wrap").unwrap_or("repeat".to_string());
        let swrap = get_string(texture.as_property_map(), "swrap").unwrap_or(wrap.clone());
        let twrap = get_string(texture.as_property_map(), "twrap").unwrap_or(wrap.clone());
        let address_mode_u = convert_address_mode(&swrap);
        let address_mode_v = convert_address_mode(&twrap);
        let uscale = get_float(texture.as_property_map(), "uscale").unwrap_or(1.0);
        let vscale = get_float(texture.as_property_map(), "vscale").unwrap_or(1.0);
        let udelta = get_float(texture.as_property_map(), "udelta").unwrap_or(0.0);
        let vdelta = get_float(texture.as_property_map(), "vdelta").unwrap_or(0.0);

        if let Some(texture_node) = resource_cache_manager.textures.get(&texture_id) {
            let texture_node = texture_node.read().unwrap();
            if let Some(image) = texture_node.image_variants.get(&size_type) {
                let image = image.read().unwrap();
                let texture = if is_bump_map.contains(&texture_id) {
                    get_normal_texture_from_image(device, queue, &image)
                } else {
                    get_color_texture_from_image(device, queue, &image)
                };
                if let Some(texture) = texture {
                    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                        label: Some("Render Texture Sampler"),
                        address_mode_u,
                        address_mode_v,
                        min_filter: wgpu::FilterMode::Linear,
                        mag_filter: wgpu::FilterMode::Linear,
                        ..Default::default()
                    });
                    let render_texture = Arc::new(RenderTexture {
                        id: texture_id,
                        edition: texture_edition.clone(),
                        texture,
                        view,
                        sampler,
                        scale: [uscale, vscale],
                        delta: [udelta, vdelta],
                    });
                    render_resource_manager.add_texture(&render_texture);
                }
            }
        }
    }
}
