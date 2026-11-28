//! Reusable viewport attachments and bounded, unmapped-after-use readback buffers.
use super::PreviewFrame;

pub(super) const NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgb10a2Unorm;
pub(super) const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

pub(super) struct Targets {
    pub(super) width: u32,
    pub(super) height: u32,
    row: u32,
    color: wgpu::Texture,
    pick: wgpu::Texture,
    depth: wgpu::Texture,
    normal: wgpu::Texture,
    ao: wgpu::Texture,
    _ao_raw: wgpu::Texture,
    _hdr: wgpu::Texture,
    pub(super) color_view: wgpu::TextureView,
    pub(super) pick_view: wgpu::TextureView,
    pub(super) depth_view: wgpu::TextureView,
    /// Lit PreZ: eye-facing normals of the opaque draws, `n * 0.5 + 0.5`.
    pub(super) normal_view: wgpu::TextureView,
    /// Lit AO from GTAO before (`ao_raw_view`) and after the denoise.
    pub(super) ao_raw_view: wgpu::TextureView,
    pub(super) ao_view: wgpu::TextureView,
    pub(super) hdr_view: wgpu::TextureView,
    color_read: wgpu::Buffer,
    pick_read: wgpu::Buffer,
}

impl Targets {
    pub(super) fn new(device: &wgpu::Device, width: u32, height: u32) -> Result<Self, String> {
        let limits = device.limits();
        if width > limits.max_texture_dimension_2d || height > limits.max_texture_dimension_2d {
            return Err("preview dimensions exceed the GPU texture limit".into());
        }
        let row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let size = row as u64 * height as u64;
        if size > limits.max_buffer_size {
            return Err("preview readback exceeds the GPU buffer limit".into());
        }
        let texture = |label, format, usage: wgpu::TextureUsages| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | usage,
                view_formats: &[],
            })
        };
        let copy = wgpu::TextureUsages::COPY_SRC;
        // The AO pass samples depth and normals; tests copy them out.
        let sampled = copy | wgpu::TextureUsages::TEXTURE_BINDING;
        let color = texture("preview-color", wgpu::TextureFormat::Rgba8UnormSrgb, copy);
        let pick = texture("preview-pick", wgpu::TextureFormat::R32Uint, copy);
        let depth = texture("preview-depth", wgpu::TextureFormat::Depth32Float, sampled);
        let normal = texture("preview-normal", NORMAL_FORMAT, sampled);
        let ao = texture("preview-ao", AO_FORMAT, sampled);
        let ao_raw = texture("preview-ao-raw", AO_FORMAT, sampled);
        let hdr = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("preview-hdr"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let read = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        };
        Ok(Self {
            width,
            height,
            row,
            color_view: color.create_view(&Default::default()),
            pick_view: pick.create_view(&Default::default()),
            depth_view: depth.create_view(&Default::default()),
            normal_view: normal.create_view(&Default::default()),
            ao_view: ao.create_view(&Default::default()),
            ao_raw_view: ao_raw.create_view(&Default::default()),
            hdr_view: hdr.create_view(&Default::default()),
            color,
            pick,
            depth,
            normal,
            ao,
            _ao_raw: ao_raw,
            _hdr: hdr,
            color_read: read("color-readback"),
            pick_read: read("pick-readback"),
        })
    }

    pub(super) fn copy(&self, encoder: &mut wgpu::CommandEncoder) {
        for (texture, buffer) in [
            (&self.color, &self.color_read),
            (&self.pick, &self.pick_read),
        ] {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    aspect: wgpu::TextureAspect::All,
                    origin: wgpu::Origin3d::ZERO,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(self.row),
                        rows_per_image: Some(self.height),
                    },
                },
                wgpu::Extent3d {
                    width: self.width,
                    height: self.height,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    pub(super) fn read(
        &self,
        device: &wgpu::Device,
        backend: &str,
    ) -> Result<PreviewFrame, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        for buffer in [&self.color_read, &self.pick_read] {
            let tx = tx.clone();
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = tx.send(result);
                });
        }
        drop(tx);
        let result = (|| {
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: Some(std::time::Duration::from_secs(30)),
                })
                .map_err(|e| format!("preview readback poll: {e}"))?;
            for _ in 0..2 {
                rx.recv_timeout(std::time::Duration::from_secs(1))
                    .map_err(|e| format!("preview readback timeout: {e}"))?
                    .map_err(|e| format!("preview readback map: {e}"))?;
            }
            let color = self
                .color_read
                .get_mapped_range(..)
                .map_err(|e| format!("color readback: {e:?}"))?;
            let pick = self
                .pick_read
                .get_mapped_range(..)
                .map_err(|e| format!("pick readback: {e:?}"))?;
            let mut rgba = Vec::with_capacity(self.width as usize * self.height as usize * 4);
            let mut pick_ids = Vec::with_capacity(self.width as usize * self.height as usize);
            for y in 0..self.height as usize {
                let start = y * self.row as usize;
                rgba.extend_from_slice(&color[start..start + self.width as usize * 4]);
                let row = &pick[start..start + self.width as usize * 4];
                // Mapped rows are normally aligned. Copy them directly on
                // little-endian hosts instead of converting every pixel.
                if cfg!(target_endian = "little")
                    && let Ok(values) = bytemuck::try_cast_slice::<u8, u32>(row)
                {
                    pick_ids.extend_from_slice(values);
                    continue;
                }
                pick_ids.extend(
                    row.chunks_exact(4)
                        .map(|p| u32::from_le_bytes(p.try_into().unwrap())),
                );
            }
            Ok(PreviewFrame {
                width: self.width,
                height: self.height,
                rgba,
                pick_ids,
                backend: backend.to_owned(),
            })
        })();
        // Always cancel pending mappings / release completed mappings, including
        // failures. Reusing a still-mapped readback buffer is a validation error.
        self.color_read.unmap();
        self.pick_read.unmap();
        result
    }

    /// Reads the depth and decoded normals left by the last Lit PreZ, row by row.
    pub(super) fn read_prez(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<(Vec<f32>, Vec<[f32; 3]>), String> {
        let word = |p: &[u8]| u32::from_le_bytes(p.try_into().unwrap());
        let depth = self
            .read_texels(
                device,
                queue,
                &self.depth,
                wgpu::TextureAspect::DepthOnly,
                4,
            )?
            .chunks_exact(4)
            .map(|p| f32::from_bits(word(p)))
            .collect();
        let normal = self
            .read_texels(device, queue, &self.normal, wgpu::TextureAspect::All, 4)?
            .chunks_exact(4)
            .map(|p| {
                let p = word(p);
                std::array::from_fn(|c| ((p >> (10 * c)) & 1023) as f32 / 1023. * 2. - 1.)
            })
            .collect();
        Ok((depth, normal))
    }

    /// Reads the last Lit frame's denoised AO, row by row.
    pub(super) fn read_ao(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<Vec<f32>, String> {
        Ok(self
            .read_texels(device, queue, &self.ao, wgpu::TextureAspect::All, 1)?
            .into_iter()
            .map(|v| v as f32 / 255.)
            .collect())
    }

    /// Copies one texture out and returns its texels row by row, unpadded.
    fn read_texels(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        aspect: wgpu::TextureAspect,
        texel_bytes: u32,
    ) -> Result<Vec<u8>, String> {
        let width = (self.width * texel_bytes) as usize;
        let row = (width as u32).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("texel-readback"),
            size: row as u64 * self.height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                aspect,
                origin: wgpu::Origin3d::ZERO,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .map_err(|e| format!("texel readback poll: {e}"))?;
        let data = buffer
            .get_mapped_range(..)
            .map_err(|e| format!("texel readback: {e:?}"))?;
        Ok((0..self.height as usize)
            .flat_map(|y| {
                let start = y * row as usize;
                data[start..start + width].iter().copied()
            })
            .collect())
    }
}
