use super::*;
use eframe::wgpu;

impl LightingMeshRenderer {
    pub fn create_directional_shadow_bind_group(
        &self,
        device: &wgpu::Device,
        directional_shadow_info_buffer: &wgpu::Buffer,
        directional_shadow_map_view: &wgpu::TextureView,
        directional_shadow_sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Lighting Directional Shadow Bind Group"),
            layout: &self.directional_shadow_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: directional_shadow_info_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(directional_shadow_map_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(directional_shadow_sampler),
                },
            ],
        })
    }

    pub fn create_directional_shadow_map_array(
        device: &wgpu::Device,
        layer_count: u32,
        width: u32,
        height: u32,
    ) -> RenderTexture {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Directional Shadow Map Array"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: layer_count,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[wgpu::TextureFormat::Depth32Float],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("Directional Shadow Map Array View"),
            format: Some(wgpu::TextureFormat::Depth32Float),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            usage: Some(wgpu::TextureUsages::TEXTURE_BINDING),
            aspect: wgpu::TextureAspect::All,
            base_mip_level: 0,
            mip_level_count: Some(1),
            base_array_layer: 0,
            array_layer_count: Some(layer_count),
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        RenderTexture {
            id: Uuid::new_v4(),
            edition: Uuid::new_v4().to_string(),
            texture,
            view,
            sampler,
            scale: [1.0, 1.0],
            delta: [0.0, 0.0],
        }
    }

    pub fn create_directional_shadow_info_buffer(device: &wgpu::Device) -> wgpu::Buffer {
        let directional_shadow_info_buffer_size = (MAX_DIRECTIONAL_SHADOW_NUM
            * std::mem::size_of::<DirectionalShadowInfo>())
            as wgpu::BufferAddress;
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Buffer for Directional Shadow Infos"),
            size: directional_shadow_info_buffer_size,
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
        })
    }
}
