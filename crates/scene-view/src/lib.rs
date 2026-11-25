//! `scene-view` — wgpu offscreen rendering for the Scene viewport.
//!
//! Create a headless wgpu instance,
//! render a known triangle into an offscreen RGBA8 target, read it back to CPU,
//! and confirm color channel order, vertical orientation, and the 256-byte
//! row-padding rule that `copy_texture_to_buffer` imposes.
//!
//! Written against the wgpu 30 API (async `request_adapter`/`request_device`,
//! `PollType`, `Option` entry points and buffer layouts).

use wgpu::util::DeviceExt;

pub mod capture_server;
pub mod preview;
pub use preview::{
    LitDebug, PreviewFrame, PreviewMode, PreviewRenderer, PreviewRendererStats,
    RenderedLitTextures, SceneUpdateTimings, ViewerCamera, render_preview, render_preview_at_time,
    render_preview_at_time_with_options,
};
pub use scene_parse::tonemap;

/// Result of an offscreen render: unpadded RGBA8 pixels in top-to-bottom order.
pub struct OffscreenFrame {
    pub width: u32,
    pub height: u32,
    /// Unpadded RGBA8, row-major, first row is the top of the image.
    pub rgba: Vec<u8>,
    /// The padded bytes-per-row wgpu required during readback (diagnostic).
    pub padded_bytes_per_row: usize,
    /// Backend that satisfied the adapter request.
    pub backend: &'static str,
}

/// A single fullscreen-space triangle with a per-vertex color.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    pos: [f32; 2],
    color: [f32; 3],
}

const VERTICES: &[Vertex] = &[
    // NDC space: +y is up. We map to image rows so that +y becomes the top row.
    Vertex {
        pos: [0.0, 0.8],
        color: [1.0, 0.0, 0.0],
    }, // top vertex -> red
    Vertex {
        pos: [-0.8, -0.8],
        color: [0.0, 1.0, 0.0],
    }, // bottom-left -> green
    Vertex {
        pos: [0.8, -0.8],
        color: [0.0, 0.0, 1.0],
    }, // bottom-right -> blue
];

const SHADER: &str = r#"
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec2<f32>, @location(1) color: vec3<f32>) -> VsOut {
    var out: VsOut;
    out.clip = vec4<f32>(pos.x, pos.y, 0.0, 1.0);
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
"#;

/// Render the triangle offscreen and return an unpadded RGBA frame.
pub fn render_offscreen_triangle(width: u32, height: u32) -> Result<OffscreenFrame, String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: crate::preferred_backends(),
        flags: Default::default(),
        memory_budget_thresholds: Default::default(),
        backend_options: crate::preferred_backend_options(),
        display: None,
    });

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: crate::preferred_power(),
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .map_err(|e| {
        format!("no GPU adapter available (headless offscreen needs a Vulkan, Metal or DX12 device): {e:?}")
    })?;

    let backend_name = adapter.get_info().backend.to_str();

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("scene-view-offscreen"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults(),
        experimental_features: Default::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: Default::default(),
    }))
    .map_err(|e| format!("request_device failed: {e:?}"))?;

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen-rgba8"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let vertex_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("triangle-wgsl"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("triangle"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &vertex_module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x3],
            })],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &vertex_module,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });

    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("vertices"),
        contents: bytemuck::cast_slice(VERTICES),
        usage: wgpu::BufferUsages::VERTEX,
    });

    let unpadded_bytes_per_row = (width as usize) * 4;
    let padded_bytes_per_row =
        unpadded_bytes_per_row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize);
    let buffer_size = (padded_bytes_per_row * height as usize) as u64;

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: buffer_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("render+copy"),
    });

    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("triangle-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.draw(0..VERTICES.len() as u32, 0..1);
    }

    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            aspect: wgpu::TextureAspect::All,
            origin: wgpu::Origin3d::ZERO,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row as u32),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );

    queue.submit(Some(encoder.finish()));

    let buffer_slice = readback.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    receiver
        .recv()
        .map_err(|e| format!("map_async channel error: {e}"))?
        .map_err(|e| format!("map_async failed: {e:?}"))?;

    let padded = buffer_slice
        .get_mapped_range()
        .map_err(|e| format!("get_mapped_range failed: {e:?}"))?;
    let mut rgba = Vec::with_capacity(unpadded_bytes_per_row * height as usize);
    for row in 0..height as usize {
        let start = row * padded_bytes_per_row;
        let end = start + unpadded_bytes_per_row;
        rgba.extend_from_slice(&padded[start..end]);
    }
    drop(padded);
    readback.unmap();

    Ok(OffscreenFrame {
        width,
        height,
        rgba,
        padded_bytes_per_row,
        backend: backend_name,
    })
}

/// Sample the RGBA pixel at (x, y), y measured from the top (row 0 = top).
pub fn pixel(frame: &OffscreenFrame, x: u32, y: u32) -> [u8; 4] {
    let idx = ((y as usize) * (frame.width as usize) + (x as usize)) * 4;
    [
        frame.rgba[idx],
        frame.rgba[idx + 1],
        frame.rgba[idx + 2],
        frame.rgba[idx + 3],
    ]
}

/// The wgpu backends the previews open: `WGPU_BACKEND` when set, else
/// wgpu's primary set. Windows takes Vulkan only with a discrete adapter
/// (Lit matches Linux there; the Surface's Intel UHD 620 Vulkan driver
/// kills the process in `request_device`) and DX12 otherwise.
pub fn preferred_backends() -> wgpu::Backends {
    if let Some(backends) = wgpu::Backends::from_env() {
        return backends;
    }
    #[cfg(windows)]
    {
        static CHOICE: std::sync::OnceLock<wgpu::Backends> = std::sync::OnceLock::new();
        *CHOICE.get_or_init(|| {
            let probe = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::VULKAN,
                flags: Default::default(),
                memory_budget_thresholds: Default::default(),
                backend_options: Default::default(),
                display: None,
            });
            let discrete = pollster::block_on(probe.enumerate_adapters(wgpu::Backends::VULKAN))
                .iter()
                .any(|adapter| adapter.get_info().device_type == wgpu::DeviceType::DiscreteGpu);
            if discrete {
                wgpu::Backends::VULKAN
            } else {
                // shortcut: on the Intel UHD 620, DX12 misses one channel of
                // one procedural-texture reference pixel; tolerate or
                // investigate if Intel-only machines report differences.
                wgpu::Backends::DX12
            }
        })
    }
    #[cfg(not(windows))]
    wgpu::Backends::PRIMARY
}

/// `WGPU_POWER_PREF` when set; else the discrete GPU on Windows, so the
/// Vulkan choice above never lands on an integrated adapter.
pub fn preferred_power() -> wgpu::PowerPreference {
    wgpu::PowerPreference::from_env().unwrap_or(if cfg!(windows) {
        wgpu::PowerPreference::HighPerformance
    } else {
        wgpu::PowerPreference::default()
    })
}

/// Backend options from the `WGPU_*` environment, with the statically
/// linked DXC as the DX12 shader compiler unless `WGPU_DX12_COMPILER` says
/// otherwise: FXC, wgpu's default, took about 17 minutes for the Lit
/// shaders on the Surface.
pub fn preferred_backend_options() -> wgpu::BackendOptions {
    #[allow(unused_mut)]
    let mut options = wgpu::BackendOptions::from_env_or_default();
    #[cfg(windows)]
    if wgpu::Dx12Compiler::from_env().is_none() {
        options.dx12.shader_compiler = wgpu::Dx12Compiler::StaticDxc;
    }
    options
}
