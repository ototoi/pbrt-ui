//! Reusable offscreen renderer. Mesh editions belong to the producer; camera
//! and draw updates never invalidate object-space vertex/index buffers.
use super::lit;
use super::{LitDebug, PreviewFrame, PreviewMode, ViewerCamera, cross};
use protocol::geometry::{DrawRecord, GeometryManifest};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, Instant};
use wgpu::util::DeviceExt;

use super::mesh::{MAX_UPLOAD_BYTES, Mesh, PreparedMesh, Vertex, validate_draws};
use super::targets::Targets;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    world: [[f32; 4]; 3],
    normal: [[f32; 4]; 3],
    info: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct View {
    projection: [[f32; 4]; 4],
    mode: [u32; 4],
    exposure: [f32; 4],
    background: [f32; 4],
    /// xyz: eye - shadow origin (Lit shadow lookups).
    shadow_origin: [f32; 4],
    /// A pixel's view ray: forward + ndc.x * right + ndc.y * up.
    ray_forward: [f32; 4],
    ray_right: [f32; 4],
    ray_up: [f32; 4],
    /// x: Lit AO radius in world units (0 = no AO).
    ao: [f32; 4],
}

/// Layers in the shadow array: six per shadowed light.
const SHADOW_LAYERS: u32 = (lit::shadow::MAX_SHADOWED * 6) as u32;
const SHADOW_PASS_BYTES: u64 = std::mem::size_of::<lit::shadow::ShadowPassGpu>() as u64;
/// Dynamic uniform offsets must be 256-byte aligned.
const SHADOW_PASS_STRIDE: u64 = SHADOW_PASS_BYTES.next_multiple_of(256);
impl Default for View {
    fn default() -> Self {
        Self {
            projection: [[0.; 4]; 4],
            mode: [0; 4],
            exposure: [1., 0., 0., 0.],
            background: [0.; 4],
            shadow_origin: [0.; 4],
            ray_forward: [0.; 4],
            ray_right: [0.; 4],
            ray_up: [0.; 4],
            ao: [0.; 4],
        }
    }
}

const PREVIEW_WGSL: &str = concat!(include_str!("vertex.wgsl"), include_str!("preview.wgsl"));

const LIT_MIP_WGSL: &str = include_str!("lit/image_mip.wgsl");

/// The array path's committed state — the live array with each layer's
/// identity. An unchanged identity sequence reuses the texture and bind
/// group wholesale (zero uploads); a changed one creates the next array and
/// copies surviving layers on the GPU, decoding only new identities.
#[derive(Clone)]
struct LitImageArray {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    /// Identity per layer; `None` layers stayed blank (demoted or missing).
    identities: Vec<Option<lit::ImageIdentity>>,
    /// Every layer's RGBA16Float cost at TEXTURE_SIZE, all mips — the
    /// budget the next update's reuse compares against.
    bytes: u64,
}

/// One binding-array image, kept by identity with its byte size so the
/// GPU budget keeps covering the live set.
struct LitBindingImage {
    texture: wgpu::Texture,
    bytes: u64,
}

/// An identity the binding path rejected — its bytes and why, so a
/// rejected image demotes again without re-decoding. When a later update's
/// set leaves room for it, the loop decodes it normally.
struct RejectedBinding {
    bytes: u64,
    exceeded: &'static str,
}

/// One binding-path image as the upload loop holds it: control
/// maps keep today's RGBA16Float pipeline from their decoded texels, while
/// material textures arrive with level 0 encoded per format — the whole
/// mip chain renders on the GPU.
enum StagedDecoded {
    Control(image::Rgba32FImage),
    Stored(lit::image_store::StoredImage),
}

/// One layer's fate in the array path, computed without decoding.
#[derive(Clone)]
enum ImageLayer {
    /// The committed array already holds this identity at `old`; copy it
    /// over all mips instead of re-uploading.
    Survive { old: u32 },
    /// First sight of this identity: decode at the transfer, demote the
    /// slot here when that fails.
    Fresh {
        key: String,
        identity: lit::ImageIdentity,
        control_map: bool,
    },
    /// No record or bytes: instructions fold to their fallbacks.
    Missing,
    /// This identity failed to decode before: demote without another
    /// attempt.
    Failed,
}

/// The lit texture state an upload stages for `set_scene` to commit — the
/// mesh uploads' rule (PR #296's staged ledger): a refresh that fails keeps
/// the previous caches and the previous scene coherent.
struct LitTextureStage {
    bind_group: wgpu::BindGroup,
    image_views: Vec<wgpu::TextureView>,
    /// The array path's next state; None on the binding-array path.
    array: Option<LitImageArray>,
    /// The binding path's next identity state; empty on the array path.
    bindings: BTreeMap<lit::ImageIdentity, LitBindingImage>,
    /// The binding path's next rejected identities, demoted without another
    /// decode.
    rejects: BTreeMap<lit::ImageIdentity, RejectedBinding>,
    /// The bytes this stage keeps alive on the GPU: per-format texel
    /// bytes on the binding path, the layer cost on the array path.
    bytes: u64,
    /// The binding path's own split of its work — the CPU decode
    /// and level-0 encode, and everything handed to the GPU (the level-0
    /// uploads, each capped source's downscale pass, one pass per mip
    /// level, the batched submit). The array path leaves both zero.
    decode: Duration,
    gpu: Duration,
}

/// The deferred lit texture stage's inputs — what
/// `upload_lit_textures` needs to run outside `set_scene`, on the first
/// render that needs lit. The manifest slice reproduces the stage's
/// lookups exactly: the blob file records (sha256) of the evaluated
/// scene's image slots under the names `lit::resolved_blob` found them —
/// legacy aliases included — and every texture record, so identity,
/// record and bytes decisions answer the same as inside `set_scene`.
struct PendingLitStage {
    manifest: GeometryManifest,
    /// The encoded image bytes the stage would decode, keyed by resolved
    /// blob name. Held for identities neither memo-failed nor committed —
    /// the case the desktop's residency answer keeps reading them; bytes
    /// a partial update did not read come from the previous pending stage
    /// when the content sha matches. Absent bytes stay absent: the
    /// stage's folds reproduce today's missing-blob shape.
    blobs: BTreeMap<String, Vec<u8>>,
    /// The sha256 of every held blob: the residency index (`resident_blob`)
    /// — content the pending stage can decode from its own memory.
    held_shas: BTreeSet<String>,
}

/// The deferred lit texture stage's report when a render ran it:
/// the stage's own durations — `lit_textures` with the binding path's
/// decode/gpu split, the other stages zero — and the scene's notes with
/// the stage's own folded in. `take_rendered_lit_textures` hands it out
/// once, so the desktop can time the stage where it ran and surface the
/// notes that only exist from that point.
pub struct RenderedLitTextures {
    pub timings: SceneUpdateTimings,
    pub notes: Vec<super::LitNote>,
}

/// The environment the last successful refresh used, with the inputs it was
/// made from: equal inputs reuse it without decoding or baking.
struct CommittedEnvironment {
    inputs: Vec<(usize, Option<String>, String)>,
    lights: Vec<usize>,
    /// Irradiance and average; `None` when no image light contributed.
    summary: Option<([[f32; 4]; 9], [f32; 3])>,
    notes: Vec<lit::LitNote>,
    bind_group: wgpu::BindGroup,
}

struct Scene {
    meshes: BTreeMap<u64, Mesh>,
    draws: Vec<DrawRecord>,
    times: Vec<f64>,
    /// Evaluated manifest v2 appearance data for the lit mode.
    lit: lit::LitScene,
    film_exposure: f32,
    bounds: Option<protocol::geometry::Bounds>,
    shadow_dirty: bool,
    /// The shadow passes' uniforms when the maps were last rendered.
    shadow_signature: Vec<Vec<u8>>,
}

/// Cumulative resource diagnostics; useful for measuring cache invalidation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreviewRendererStats {
    pub mesh_uploads: u64,
    pub mesh_upload_bytes: u64,
    pub target_allocations: u64,
    pub instance_allocations: u64,
    pub frames: u64,
    pub shadow_renders: u64,
    /// Lit image decodes since the renderer was created: textures,
    /// light control maps and environment lights — committed only by updates
    /// that succeeded.
    pub image_decodes: u64,
    /// Environment cube bakes: unchanged image-based lights re-bake zero
    /// times.
    pub environment_bakes: u64,
    /// Lit image bytes the last successful update kept alive on the GPU
    /// The binding path's per-format texel bytes including all
    /// mip levels, the array path's layers at their fixed RGBA16Float
    /// 1024² cost.
    pub lit_image_bytes: u64,
}

/// Stage durations of the last successful scene refresh
/// (`upload_scene`/`update_scene`). The caller records these through its own
/// opt-in diagnostics (the desktop's `preview-timing` lines: `gpu_meshes`,
/// `lit_evaluate`, `lit_textures`, `environment`); scene-view never prints.
/// The lit texture stage is deferred until the first render that needs it,
/// so a refresh's `lit_textures` (and its split) stays zero; the stage's
/// real durations arrive in `RenderedLitTextures` when a render runs it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SceneUpdateTimings {
    /// Everything before the lit stages: mesh blob reads, validation and
    /// the buffer uploads.
    pub mesh: Duration,
    /// `lit::evaluate_with_image_capacity` on the manifest and blobs.
    pub lit_evaluate: Duration,
    /// `upload_lit_textures`: image decode, resize, mip generation, upload.
    /// Deferred: zero on refreshes, measured on the render
    /// that runs the stage.
    pub lit_textures: Duration,
    /// The binding path's CPU share of `lit_textures`: the decodes
    /// and level-0 encodes. Zero for the array path and for updates that
    /// decoded nothing.
    pub lit_textures_decode: Duration,
    /// The binding path's GPU share of `lit_textures`: the
    /// staging uploads, each capped source's downscale pass, the mip-chain
    /// render passes and the batched submit. Zero for the array path.
    pub lit_textures_gpu: Duration,
    /// `bake_environment`: image-based lights baked into the cube.
    pub environment: Duration,
}

/// One renderer per display thread. Load geometry once with `upload_scene`,
/// then render camera/mode/time changes or call `update_draws` for placements.
pub struct PreviewRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    backend: String,
    adapter_description: String,
    triangles: wgpu::RenderPipeline,
    pick_only: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    /// Opaque Lit draws, depth `Equal` to what the PreZ pass wrote.
    lit_pipeline: wgpu::RenderPipeline,
    /// PreZ: depth and normals of the opaque draws; `_alpha` evaluates the
    /// shape's alpha cutout.
    prez_pipeline: wgpu::RenderPipeline,
    prez_alpha_pipeline: wgpu::RenderPipeline,
    /// Glass: drawn after the opaque draws, far to near.
    lit_transparent_pipeline: wgpu::RenderPipeline,
    /// Mixes with a transparent leaf: blended, but mostly opaque where a solid
    /// leaf dominates, so they write depth to hide their own far side.
    lit_mix_pipeline: wgpu::RenderPipeline,
    environment_layout: wgpu::BindGroupLayout,
    environment_sampler: wgpu::Sampler,
    environment_bind_group: wgpu::BindGroup,
    environment_background_pipeline: wgpu::RenderPipeline,
    environment_bake_layout: wgpu::BindGroupLayout,
    environment_bake_pipeline: wgpu::RenderPipeline,
    environment_filter_layout: wgpu::BindGroupLayout,
    environment_downsample_pipeline: wgpu::RenderPipeline,
    environment_prefilter_pipeline: wgpu::RenderPipeline,
    output_pipeline: wgpu::RenderPipeline,
    /// Lit AO: GTAO from the PreZ depth and normals, then its denoise.
    ao_layout: wgpu::BindGroupLayout,
    ao_pipeline: wgpu::RenderPipeline,
    ao_denoise_pipeline: wgpu::RenderPipeline,
    /// Per target size: the GTAO and the denoise bindings.
    ao_bind_groups: Option<[wgpu::BindGroup; 2]>,
    lit_bind_group: wgpu::BindGroup,
    light_buffer: wgpu::Buffer,
    appearance_buffer: wgpu::Buffer,
    texture_instruction_buffer: wgpu::Buffer,
    texture_mapping_buffer: wgpu::Buffer,
    shadow_texture_mapping_buffer: wgpu::Buffer,
    emitter_vertex_buffer: wgpu::Buffer,
    shadow_buffer: wgpu::Buffer,
    shadow_pipeline: wgpu::RenderPipeline,
    /// One `ShadowPassGpu` per shadow layer, each at its own dynamic offset.
    shadow_pass_buffer: wgpu::Buffer,
    shadow_pass_bind_group: wgpu::BindGroup,
    shadow_vm_bind_group: wgpu::BindGroup,
    /// One render view per layer of `shadow_texture`.
    shadow_layer_views: Vec<wgpu::TextureView>,
    /// Instances placed relative to the shadow origin, not the eye.
    shadow_instances: Option<wgpu::Buffer>,
    shadow_instance_capacity: u64,
    view: wgpu::Buffer,
    /// Group 0: the view uniform and the AO of the current targets.
    view_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    /// Group 1: the Lit preview's mipmapped image array and sampler.
    texture_layout: wgpu::BindGroupLayout,
    /// The binding path's GPU mip chains — the shared bindings of
    /// one downscale pass (the previous level's view and the filtering
    /// sampler), the fullscreen-triangle module, its pipeline layout, and
    /// one cached render pipeline per stored image format.
    lit_mip_layout: wgpu::BindGroupLayout,
    lit_mip_module: wgpu::ShaderModule,
    lit_mip_pipeline_layout: wgpu::PipelineLayout,
    lit_mip_pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    output_layout: wgpu::BindGroupLayout,
    output_bind_group: Option<wgpu::BindGroup>,
    texture_bind_group: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    repeat_sampler: wgpu::Sampler,
    /// Texture views for `Scene::lit.image_slots`, kept alive for the pass.
    image_views: Vec<wgpu::TextureView>,
    binding_array_capacity: Option<u32>,
    /// Per-identity CPU memos shared by evaluate and the upload paths.
    image_memos: lit::ImageMemos,
    /// The array path's committed array and its layer identities.
    lit_image_array: Option<LitImageArray>,
    /// The binding path's committed identity → texture map.
    lit_binding_images: BTreeMap<lit::ImageIdentity, LitBindingImage>,
    /// Identities the binding path rejected (byte budget, resolution
    /// limit): demoted again without another decode while the set excludes
    /// them.
    lit_binding_rejects: BTreeMap<lit::ImageIdentity, RejectedBinding>,
    environment_committed: Option<CommittedEnvironment>,
    last_view: Option<View>,
    instances: Option<wgpu::Buffer>,
    instance_capacity: u64,
    last_instances: Vec<Instance>,
    targets: Option<Targets>,
    scene: Option<Scene>,
    stats: PreviewRendererStats,
    last_update_timings: SceneUpdateTimings,
    /// The deferred lit texture stage's inputs, replaced by the
    /// next successful refresh's stage and dropped with the scene
    /// (`clear_scene`). Never pending without a scene.
    pending_lit_textures: Option<PendingLitStage>,
    /// The report of the last render that ran the deferred stage,
    /// until the desktop or a harness takes it.
    rendered_lit_textures: Option<RenderedLitTextures>,
}

impl PreviewRenderer {
    pub fn new() -> Result<Self, String> {
        Self::new_with_texture_arrays(true)
    }

    /// Disable optional binding arrays to exercise the portable image path.
    pub fn new_with_texture_arrays(prefer_binding_arrays: bool) -> Result<Self, String> {
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
        .map_err(|e| format!("no GPU adapter: {e:?}"))?;
        let info = adapter.get_info();
        let backend = info.backend.to_str().to_owned();
        let adapter_description = format!("{}; {} {}", info.name, info.driver, info.driver_info);
        let adapter_limits = adapter.limits();
        let array_features = wgpu::Features::TEXTURE_BINDING_ARRAY
            | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING;
        let binding_array_capacity = (prefer_binding_arrays
            && adapter.features().contains(array_features)
            && adapter_limits.max_binding_array_elements_per_shader_stage > 0)
            .then_some(
                adapter_limits
                    .max_binding_array_elements_per_shader_stage
                    .min(lit::MAX_TEXTURES as u32),
            );
        if adapter_limits.max_storage_buffers_per_shader_stage < 7 {
            return Err(format!(
                "Lit texture VM needs 7 fragment storage buffers; adapter exposes {}",
                adapter_limits.max_storage_buffers_per_shader_stage
            ));
        }
        let mut required_limits =
            wgpu::Limits::downlevel_defaults().using_resolution(adapter_limits);
        required_limits.max_storage_buffers_per_shader_stage = 7;
        if let Some(capacity) = binding_array_capacity {
            required_limits.max_binding_array_elements_per_shader_stage = capacity;
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("pbrt-ui-preview"),
            required_features: if binding_array_capacity.is_some() {
                array_features
            } else {
                wgpu::Features::empty()
            },
            required_limits,
            experimental_features: Default::default(),
            memory_hints: Default::default(),
            trace: Default::default(),
        }))
        .map_err(|e| format!("request_device failed: {e:?}"))?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("preview-shader"),
            source: wgpu::ShaderSource::Wgsl(PREVIEW_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("preview-view-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    // The lit mode samples the same uniforms from the fragment
                    // stage, so the view data is visible in both stages.
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<View>() as u64),
                    },
                    count: None,
                },
                // Lit reads the AO of the current targets.
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let texture_entries = [
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: if binding_array_capacity.is_some() {
                        wgpu::TextureViewDimension::D2
                    } else {
                        wgpu::TextureViewDimension::D2Array
                    },
                    multisampled: false,
                },
                count: binding_array_capacity.and_then(std::num::NonZeroU32::new),
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ];
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("preview-texture-layout"),
            entries: &texture_entries,
        });
        // The binding path's GPU mip chains share one downscale
        // pass — the previous level's single-level view and the shared
        // filtering sampler. One render pipeline per stored format is
        // created lazily at first use.
        let lit_mip_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("preview-lit-mip-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let lit_mip_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("preview-lit-image-mip"),
            source: wgpu::ShaderSource::Wgsl(LIT_MIP_WGSL.into()),
        });
        let lit_mip_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("preview-lit-mip-pipeline"),
                bind_group_layouts: &[Some(&lit_mip_layout)],
                immediate_size: 0,
            });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("preview-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let triangles = pipeline(
            &device,
            &shader,
            &pipeline_layout,
            wgpu::PrimitiveTopology::TriangleList,
        );
        let pick_only = pick_only_pipeline(&device, &shader, &pipeline_layout);
        let lines = pipeline(
            &device,
            &shader,
            &pipeline_layout,
            wgpu::PrimitiveTopology::LineList,
        );
        let lit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("preview-lit-shader"),
            source: wgpu::ShaderSource::Wgsl(
                lit::pipeline::shader_source(lit::pipeline::LIT_WGSL, binding_array_capacity)
                    .into(),
            ),
        });
        let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("preview-shadow-shader"),
            source: wgpu::ShaderSource::Wgsl(
                lit::pipeline::shader_source(lit::pipeline::SHADOW_WGSL, binding_array_capacity)
                    .into(),
            ),
        });
        let shadow_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("preview-shadow-array"),
            size: wgpu::Extent3d {
                width: lit::SHADOW_SIZE,
                height: lit::SHADOW_SIZE,
                depth_or_array_layers: (lit::shadow::MAX_SHADOWED * 6) as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_view = shadow_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("preview-shadow-array-view"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            base_array_layer: 0,
            array_layer_count: Some((lit::shadow::MAX_SHADOWED * 6) as u32),
            ..Default::default()
        });
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            compare: Some(wgpu::CompareFunction::LessEqual),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let shadow_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("preview-shadow-entries"),
            size: (lit::MAX_LIGHTS * std::mem::size_of::<lit::shadow::ShadowGpu>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let storage_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("preview-lit-storage-layout"),
            entries: &[
                storage_entry(0),
                storage_entry(1),
                storage_entry(2),
                storage_entry(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                // The GGX LTC table: read with textureLoad (Rgba32Float is
                // not filterable everywhere), interpolated in the shader.
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                storage_entry(7),
                storage_entry(8),
                storage_entry(9),
            ],
        });
        let ltc_view = ltc_table(&device, &queue);
        let light_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("preview-lit-lights"),
            size: (lit::MAX_LIGHTS * std::mem::size_of::<lit::GpuLight>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let appearance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("preview-lit-appearances"),
            size: (lit::MAX_APPEARANCES * std::mem::size_of::<lit::GpuLobe>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let emitter_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("preview-lit-emitter-vertices"),
            // The triangle budget bounds the vertices (three per triangle),
            // and the goniometric/projection light frames append after them.
            size: ((lit::MAX_EMITTER_TRIANGLES * 3 + lit::LIGHT_FRAME_CAPACITY)
                * std::mem::size_of::<[f32; 4]>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mapping_buffer = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: (lit::MAX_TEXTURE_MAPPINGS * 3 * std::mem::size_of::<[f32; 4]>()) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        // Rows relative to the eye for Lit, to the shadow origin for shadows.
        let texture_mapping_buffer = mapping_buffer("preview-lit-texture-mappings");
        let shadow_texture_mapping_buffer = mapping_buffer("preview-shadow-texture-mappings");
        let texture_instruction_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("preview-lit-texture-instructions"),
            size: (lit::MAX_TEXTURE_INSTRUCTIONS
                * std::mem::size_of::<lit::texture_vm::TextureInstruction>())
                as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let noise_perm = lit::texture_vm::NOISE_PERM.repeat(2);
        let noise_perm_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("preview-lit-noise-permutation"),
            contents: bytemuck::cast_slice(&noise_perm),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let lit_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("preview-lit-storage-bindings"),
            layout: &storage_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: light_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: appearance_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: emitter_vertex_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: shadow_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&shadow_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&ltc_view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: texture_instruction_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: noise_perm_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: texture_mapping_buffer.as_entire_binding(),
                },
            ],
        });
        let environment_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("preview-environment-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::Cube,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let environment_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("preview-environment-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let environment_bind_group = environment_bindings(
            &device,
            &environment_layout,
            &environment_sampler,
            1,
            1,
            &[[0.; 4]; 9],
        )
        .1;
        let lit_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("preview-lit-pipeline-layout"),
            bind_group_layouts: &[
                Some(&layout),
                Some(&texture_layout),
                Some(&storage_layout),
                Some(&environment_layout),
            ],
            immediate_size: 0,
        });
        let shadow_pass_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("preview-shadow-pass-layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(SHADOW_PASS_BYTES),
                    },
                    count: None,
                }],
            });
        let shadow_pass_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("preview-shadow-passes"),
            size: SHADOW_PASS_STRIDE * SHADOW_LAYERS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadow_pass_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("preview-shadow-pass-bindings"),
            layout: &shadow_pass_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &shadow_pass_buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(SHADOW_PASS_BYTES),
                }),
            }],
        });
        let shadow_layer_views = (0..SHADOW_LAYERS)
            .map(|layer| {
                shadow_texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("preview-shadow-layer"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        // The VM's buffers, for alpha cutouts in the shadow pass.
        let shadow_vm_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("preview-shadow-vm-layout"),
            entries: &[storage_entry(7), storage_entry(8), storage_entry(9)],
        });
        let shadow_vm_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("preview-shadow-vm-bindings"),
            layout: &shadow_vm_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: texture_instruction_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: noise_perm_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: shadow_texture_mapping_buffer.as_entire_binding(),
                },
            ],
        });
        let shadow_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("preview-shadow-pipeline-layout"),
                bind_group_layouts: &[
                    Some(&shadow_pass_layout),
                    Some(&texture_layout),
                    Some(&shadow_vm_layout),
                ],
                immediate_size: 0,
            });
        let environment_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("preview-environment-background"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("lit/environment_background.wgsl").into(),
            ),
        });
        let environment_background_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("preview-environment-background"),
                layout: Some(&lit_layout),
                vertex: wgpu::VertexState {
                    module: &environment_module,
                    entry_point: Some("vs_background"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &environment_module,
                    entry_point: Some("fs_background"),
                    compilation_options: Default::default(),
                    targets: &[
                        Some(wgpu::ColorTargetState {
                            format: wgpu::TextureFormat::Rgba16Float,
                            blend: Some(ENVIRONMENT_ADD),
                            write_mask: wgpu::ColorWrites::ALL,
                        }),
                        Some(wgpu::ColorTargetState {
                            format: wgpu::TextureFormat::R32Uint,
                            blend: None,
                            write_mask: wgpu::ColorWrites::empty(),
                        }),
                    ],
                }),
                multiview_mask: None,
                cache: None,
            });
        let environment_bake_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("preview-environment-bake-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });
        let environment_bake_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("preview-environment-bake"),
            source: wgpu::ShaderSource::Wgsl(include_str!("lit/environment.wgsl").into()),
        });
        let environment_bake_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("preview-environment-bake"),
                bind_group_layouts: &[Some(&environment_bake_layout)],
                immediate_size: 0,
            });
        let environment_bake_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("preview-environment-bake"),
                layout: Some(&environment_bake_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &environment_bake_module,
                    entry_point: Some("vs_fullscreen"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &environment_bake_module,
                    entry_point: Some("fs_bake"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: Some(ENVIRONMENT_ADD),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        let environment_filter_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("preview-environment-filter-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::Cube,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let environment_filter_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("preview-environment-prefilter"),
            source: wgpu::ShaderSource::Wgsl(include_str!("lit/environment_prefilter.wgsl").into()),
        });
        let environment_filter_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("preview-environment-filter"),
                bind_group_layouts: &[Some(&environment_filter_layout)],
                immediate_size: 0,
            });
        let environment_filter_pipeline = |entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&environment_filter_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &environment_filter_module,
                    entry_point: Some("vs_fullscreen"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &environment_filter_module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let environment_downsample_pipeline = environment_filter_pipeline("fs_downsample");
        let environment_prefilter_pipeline = environment_filter_pipeline("fs_prefilter");
        let lit_transparent_pipeline = lit_pipeline(&device, &lit_shader, &lit_layout, true, false);
        let lit_mix_pipeline = lit_pipeline(&device, &lit_shader, &lit_layout, true, true);
        let lit_pipeline = lit_pipeline(&device, &lit_shader, &lit_layout, false, false);
        let prez_alpha_pipeline = prez_pipeline(&device, &lit_shader, &lit_layout, "fs_prez_alpha");
        let prez_pipeline = prez_pipeline(&device, &lit_shader, &lit_layout, "fs_prez");
        let shadow_pipeline = shadow_pipeline(&device, &shadow_shader, &shadow_pipeline_layout);
        let output_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("preview-output-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<View>() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let output_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("preview-output-shader"),
            source: wgpu::ShaderSource::Wgsl(lit::pipeline::OUTPUT_WGSL.into()),
        });
        let output_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("preview-output-pipeline-layout"),
                bind_group_layouts: &[Some(&output_layout)],
                immediate_size: 0,
            });
        let output_pipeline = output_pipeline(&device, &output_shader, &output_pipeline_layout);
        let texture_entry = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let unfiltered = wgpu::TextureSampleType::Float { filterable: false };
        let ao_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("preview-ao-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<View>() as u64),
                    },
                    count: None,
                },
                texture_entry(1, wgpu::TextureSampleType::Depth),
                texture_entry(2, unfiltered),
                texture_entry(3, unfiltered),
            ],
        });
        let ao_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("preview-ao-shader"),
            source: wgpu::ShaderSource::Wgsl(lit::pipeline::AO_WGSL.into()),
        });
        let ao_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("preview-ao-pipeline-layout"),
            bind_group_layouts: &[Some(&ao_layout)],
            immediate_size: 0,
        });
        let ao_denoise_pipeline =
            ao_pipeline(&device, &ao_shader, &ao_pipeline_layout, "fs_ao_denoise");
        let ao_pipeline = ao_pipeline(&device, &ao_shader, &ao_pipeline_layout, "fs_gtao");
        let pipelines = lit::pipeline::LitPipelines {
            lit: lit_pipeline,
            output: output_pipeline,
        };
        let placeholder_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("preview-texture-placeholder"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            placeholder_texture.as_image_copy(),
            bytemuck::cast_slice(&[half::f16::ONE.to_bits(); 4]),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let placeholder_texture = placeholder_texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(if binding_array_capacity.is_some() {
                wgpu::TextureViewDimension::D2
            } else {
                wgpu::TextureViewDimension::D2Array
            }),
            array_layer_count: Some(1),
            ..Default::default()
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let repeat_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let placeholders = vec![&placeholder_texture; binding_array_capacity.unwrap_or(1) as usize];
        let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("preview-texture-bindings"),
            layout: &texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: if binding_array_capacity.is_some() {
                        wgpu::BindingResource::TextureViewArray(&placeholders)
                    } else {
                        wgpu::BindingResource::TextureView(&placeholder_texture)
                    },
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&repeat_sampler),
                },
            ],
        });
        let view = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("preview-view"),
            size: std::mem::size_of::<View>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Replaced with the targets' AO once they exist.
        let no_ao = device
            .create_texture_with_data(
                &queue,
                &wgpu::TextureDescriptor {
                    label: Some("preview-no-ao"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: super::targets::AO_FORMAT,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                Default::default(),
                &[255],
            )
            .create_view(&Default::default());
        let bind_group = view_bindings(&device, &layout, &view, &no_ao);
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(format!("preview pipeline validation failed: {error}"));
        }
        Ok(Self {
            device,
            queue,
            backend,
            adapter_description,
            triangles,
            pick_only,
            lines,
            lit_pipeline: pipelines.lit,
            prez_pipeline,
            prez_alpha_pipeline,
            lit_transparent_pipeline,
            lit_mix_pipeline,
            environment_layout,
            environment_sampler,
            environment_bind_group,
            environment_background_pipeline,
            environment_bake_layout,
            environment_bake_pipeline,
            environment_filter_layout,
            environment_downsample_pipeline,
            environment_prefilter_pipeline,
            output_pipeline: pipelines.output,
            ao_layout,
            ao_pipeline,
            ao_denoise_pipeline,
            ao_bind_groups: None,
            lit_bind_group,
            light_buffer,
            appearance_buffer,
            texture_instruction_buffer,
            texture_mapping_buffer,
            shadow_texture_mapping_buffer,
            emitter_vertex_buffer,
            shadow_buffer,
            shadow_pipeline,
            shadow_pass_buffer,
            shadow_pass_bind_group,
            shadow_vm_bind_group,
            shadow_layer_views,
            shadow_instances: None,
            shadow_instance_capacity: 0,
            view,
            bind_group,
            view_layout: layout,
            texture_layout,
            lit_mip_layout,
            lit_mip_module,
            lit_mip_pipeline_layout,
            lit_mip_pipelines: HashMap::new(),
            output_layout,
            output_bind_group: None,
            texture_bind_group,
            sampler,
            repeat_sampler,
            image_views: Vec::new(),
            binding_array_capacity,
            image_memos: Default::default(),
            lit_image_array: None,
            lit_binding_images: BTreeMap::new(),
            lit_binding_rejects: BTreeMap::new(),
            environment_committed: None,
            last_view: None,
            instances: None,
            instance_capacity: 0,
            last_instances: Vec::new(),
            targets: None,
            scene: None,
            stats: Default::default(),
            last_update_timings: Default::default(),
            pending_lit_textures: None,
            rendered_lit_textures: None,
        })
    }

    /// The last Lit frame's depth (reversed Z, 0 = background) and the
    /// eye-facing normals its PreZ pass wrote, row by row. For tests. Depth is
    /// read after the frame, so a transmitting mix's own depth shows in it.
    #[doc(hidden)]
    pub fn read_prez(&self) -> Result<(Vec<f32>, Vec<[f32; 3]>), String> {
        self.targets
            .as_ref()
            .ok_or("no frame rendered")?
            .read_prez(&self.device, &self.queue)
    }

    /// The last Lit frame's AO (1 = unoccluded), row by row. For tests.
    #[doc(hidden)]
    pub fn read_ao(&self) -> Result<Vec<f32>, String> {
        self.targets
            .as_ref()
            .ok_or("no frame rendered")?
            .read_ao(&self.device, &self.queue)
    }

    pub fn stats(&self) -> PreviewRendererStats {
        self.stats
    }

    /// Stage durations of the last successful `upload_scene`/`update_scene`,
    /// in the order the stages ran. A failed refresh leaves the previous
    /// timings in place, like the previous scene itself.
    pub fn last_update_timings(&self) -> SceneUpdateTimings {
        self.last_update_timings
    }

    /// Approximation notes for the scene currently uploaded to this renderer.
    pub fn lit_notes(&self) -> &[lit::LitNote] {
        self.scene.as_ref().map_or(&[], |scene| &scene.lit.notes)
    }

    pub fn adapter_description(&self) -> &str {
        &self.adapter_description
    }

    /// Release scene-owned geometry, keeping the device and viewport targets.
    pub fn clear_scene(&mut self) {
        self.scene = None;
        self.last_instances.clear();
        // The lit and environment caches are scene-derived; release
        // them with the scene, mesh buffers with the scene. The bind groups
        // keep holding the GPU references they already did. The
        // deferred texture stage and its report are scene-derived too.
        self.image_memos = Default::default();
        self.lit_image_array = None;
        self.lit_binding_images.clear();
        self.lit_binding_rejects.clear();
        self.environment_committed = None;
        self.pending_lit_textures = None;
        self.rendered_lit_textures = None;
    }

    /// Can an update proceed without `key`'s image
    /// bytes? The question is asked for `texture/` and `light/` blob
    /// names only; a geometry name answers false (its read is the
    /// editions' and bounds' decision, never this one). True when
    /// · the blob feeds an unchanged environment input — the committed
    ///   environment is reused whole, no decode, no bake;
    /// · the identity the blob resolves to (content sha plus decode
    ///   settings) is committed on the GPU — in the array's layers or the
    ///   binding images — so the layer survives without its bytes, or its
    ///   decode failure is memoised and folds again without an attempt;
    /// · the identity's bytes are held by the pending lit texture stage
    ///   The deferred run decodes from its own memory, so
    ///   repeated Solid updates keep skipping the read until either the
    ///   content changes or a Lit render commits the texture.
    /// · a control-map light whose summary memo answers evaluate while its
    ///   image identity is committed (or memo-failed).
    /// Budget-rejected identities follow the pending rule: their bytes
    /// arrive with the pending stage (the next upload re-probes their
    /// headers), so they are no read either.
    pub fn resident_blob(&self, key: &str, manifest: &GeometryManifest) -> bool {
        let environment_inputs = lit::environment::inputs(manifest);
        if environment_inputs
            .iter()
            .any(|(index, _, _)| key == format!("light/{index}"))
        {
            return self
                .environment_committed
                .as_ref()
                .is_some_and(|committed| committed.inputs == environment_inputs);
        }
        let Some(identity) = lit::image_identity(key, manifest, &manifest.textures) else {
            return false;
        };
        if key.starts_with("light/") {
            let record = key
                .strip_prefix("light/")
                .and_then(|index| index.parse::<usize>().ok())
                .and_then(|index| manifest.lights.get(index));
            if let Some(summary) = record.and_then(lit::control_summary_identity) {
                // Control-map lights need the summary memo in evaluate and
                // a committed (or memo-failed) image in the upload; only
                // both let the bytes go unread.
                let summary = lit::ImageIdentity {
                    sha256: identity.sha256.clone(),
                    settings: summary,
                };
                return self.image_memos.has_summary(&summary) && self.identity_resident(&identity);
            }
        }
        self.identity_resident(&identity)
    }

    /// Whether `identity` answers an update without bytes: held in
    /// the committed array or binding images, memoised as failed, or
    /// carried by the pending lit texture stage — the deferred
    /// run decodes from its own copy, so the desktop keeps skipping the
    /// disk read while it waits.
    fn identity_resident(&self, identity: &lit::ImageIdentity) -> bool {
        self.image_memos.failed(identity)
            || self.identity_committed(identity)
            || self
                .pending_lit_textures
                .as_ref()
                .is_some_and(|pending| pending.held_shas.contains(&identity.sha256))
    }

    /// Whether `identity` lives in the committed GPU state that reuse
    /// serves: a binding image, or a layer of the live array.
    fn identity_committed(&self, identity: &lit::ImageIdentity) -> bool {
        self.lit_binding_images.contains_key(identity)
            || self.lit_image_array.as_ref().is_some_and(|array| {
                array
                    .identities
                    .iter()
                    .flatten()
                    .any(|layer| layer == identity)
            })
    }

    /// Replace the scene, including its mesh-id namespace. Failed uploads leave
    /// the last successful scene intact. Source blobs are not retained for the
    /// GPU; the lit texture stage is deferred, and its pending inputs hold
    /// the encoded image bytes it would decode (until the first Lit render
    /// runs them, or the next refresh replaces the stage).
    pub fn upload_scene(
        &mut self,
        manifest: &GeometryManifest,
        blobs: &BTreeMap<String, Vec<u8>>,
    ) -> Result<(), String> {
        self.set_scene(manifest, blobs, &BTreeMap::new(), false)
    }

    /// Refresh an existing scene. Unchanged mesh editions reuse vertex and both
    /// index buffers. Producers advance the edition whenever mesh data changes;
    /// absent editions mean zero. Use upload_scene for an unrelated scene.
    pub fn update_scene(
        &mut self,
        manifest: &GeometryManifest,
        blobs: &BTreeMap<String, Vec<u8>>,
        editions: &BTreeMap<u64, u64>,
    ) -> Result<(), String> {
        self.set_scene(manifest, blobs, editions, true)
    }

    fn set_scene(
        &mut self,
        manifest: &GeometryManifest,
        blobs: &BTreeMap<String, Vec<u8>>,
        editions: &BTreeMap<u64, u64>,
        reuse: bool,
    ) -> Result<(), String> {
        if manifest.supported_times.is_empty()
            || manifest.supported_times.iter().any(|t| !t.is_finite())
        {
            return Err("preview requires finite supported times".into());
        }
        // The stage durations fill a local copy; only a refresh that made it
        // all the way commits them, keeping the last successful scene's
        // timings alongside the scene itself.
        let mut timings = SceneUpdateTimings::default();
        let mesh_started = Instant::now();
        let mut meshes = BTreeMap::new();
        let mut prepared = Vec::new();
        let mut bytes = 0u64;
        let mut ids = BTreeSet::new();
        for record in &manifest.meshes {
            if !ids.insert(record.mesh_id) {
                return Err("duplicate preview mesh id".into());
            }
            let edition = editions.get(&record.mesh_id).copied().unwrap_or(0);
            let cached = self
                .scene
                .as_ref()
                .filter(|_| reuse)
                .and_then(|scene| scene.meshes.get(&record.mesh_id))
                .filter(|mesh| mesh.edition == edition);
            if let Some(mesh) = cached {
                bytes = bytes
                    .checked_add(mesh.bytes)
                    .ok_or("preview mesh size overflow")?;
                meshes.insert(record.mesh_id, mesh.clone());
            } else {
                let mesh = PreparedMesh::read(
                    record,
                    blobs,
                    self.device.limits().max_buffer_size,
                    MAX_UPLOAD_BYTES.saturating_sub(bytes),
                )?;
                bytes = bytes
                    .checked_add(mesh.bytes)
                    .ok_or("preview mesh size overflow")?;
                prepared.push((record.mesh_id, edition, mesh));
            }
            if bytes > MAX_UPLOAD_BYTES {
                return Err("preview geometry exceeds the GPU upload budget".into());
            }
        }
        validate_draws(&manifest.draws, &ids, manifest.supported_times.len())?;
        let uploaded = prepared.len() as u64;
        let uploaded_bytes = prepared.iter().map(|(_, _, mesh)| mesh.bytes).sum::<u64>();
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        for (id, edition, prepared) in prepared {
            meshes.insert(id, prepared.upload(&self.device, edition));
        }
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(format!("preview mesh allocation failed: {error}"));
        }
        timings.mesh = mesh_started.elapsed();
        let started = Instant::now();
        // Unchanged image-light inputs reuse the committed environment: no
        // decode here and no bake below.
        let environment_inputs = lit::environment::inputs(manifest);
        let reused = self
            .environment_committed
            .as_ref()
            .filter(|committed| committed.inputs == environment_inputs);
        let (image_lights, environment, environment_notes) = match reused {
            Some(committed) => (
                committed.lights.clone(),
                committed
                    .summary
                    .map(|(irradiance, average)| lit::environment::Environment {
                        images: Vec::new(),
                        irradiance,
                        average,
                    }),
                committed.notes.clone(),
            ),
            None => {
                for _ in environment_inputs.iter().filter(|input| input.1.is_some()) {
                    self.image_memos.count_decode();
                }
                let mut notes = Vec::new();
                let (lights, environment) =
                    lit::environment::from_manifest(manifest, blobs, &mut notes);
                (lights, environment, notes)
            }
        };
        let reused = reused.map(|committed| committed.bind_group.clone());
        let mut lit = lit::evaluate_with_lit_environment(
            manifest,
            blobs,
            lit::MAX_TEXTURES,
            &mut self.image_memos,
            &image_lights,
            environment,
            environment_notes.clone(),
        );
        timings.lit_evaluate = started.elapsed();
        // The lit texture stage defers to the first render that
        // needs it — a Solid update decodes no images. The pending stage
        // holds what that run needs: the manifest slice its lookups read,
        // and the bytes for slots neither the memos nor the committed GPU
        // state answer. Its own report (durations, notes) arrives when a
        // render runs it.
        let pending = self.pending_lit_stage(&lit, manifest, blobs);
        let started = Instant::now();
        let summary = lit
            .environment
            .as_ref()
            .map(|environment| (environment.irradiance, environment.average));
        match reused {
            Some(bind_group) => self.environment_bind_group = bind_group,
            None => {
                self.bake_environment(lit.environment.as_ref());
                if lit.environment.is_some() {
                    self.stats.environment_bakes += 1;
                }
            }
        }
        // The cube and the harmonics hold what the source images gave.
        if let Some(environment) = &mut lit.environment {
            environment.images = Vec::new();
        }
        timings.environment = started.elapsed();
        self.scene = Some(Scene {
            meshes,
            draws: manifest.draws.clone(),
            times: manifest.supported_times.clone(),
            lit,
            film_exposure: manifest
                .camera
                .as_ref()
                .and_then(|camera| camera.film_exposure)
                .filter(|v| v.is_finite())
                .map(|v| v as f32)
                .filter(|v| v.is_finite())
                .unwrap_or(1.),
            bounds: manifest.bounds.clone(),
            shadow_dirty: true,
            shadow_signature: Vec::new(),
        });
        self.stats.mesh_uploads += uploaded;
        self.stats.mesh_upload_bytes += uploaded_bytes;
        // Everything below commits only now — the refresh succeeded.
        self.environment_committed = Some(CommittedEnvironment {
            inputs: environment_inputs,
            lights: image_lights,
            summary,
            notes: environment_notes,
            bind_group: self.environment_bind_group.clone(),
        });
        self.stats.image_decodes += self.image_memos.take_decodes();
        self.last_update_timings = timings;
        // The pending stage replaces the previous one — a second
        // update before the first lit render never doubles the work. A
        // deferred stage never commits GPU texture state here: the last
        // committed state belongs to the last lit-rendered scene.
        self.pending_lit_textures = Some(pending);
        Ok(())
    }

    /// Build the pending lit texture stage's inputs from the evaluated
    /// scene. The manifest slice keeps every lookup the stage
    /// makes: `resolved_blob`'s file records under the names it found them
    /// (legacy aliases included) and all texture records. Bytes are held
    /// for slots the stage would decode — replicas the memos (`failed`)
    /// and the committed GPU state answer without bytes, matching what
    /// `resident_blob` promised the desktop; bytes this update did not
    /// read (resident content, unchanged since) carry over from the
    /// previous pending stage when the sha matches, so repeated Solid
    /// updates keep the disk read at zero.
    fn pending_lit_stage(
        &self,
        lit: &lit::LitScene,
        manifest: &GeometryManifest,
        blobs: &BTreeMap<String, Vec<u8>>,
    ) -> PendingLitStage {
        let previous = self.pending_lit_textures.as_ref();
        let mut pending = PendingLitStage {
            manifest: GeometryManifest::new(),
            blobs: BTreeMap::new(),
            held_shas: BTreeSet::new(),
        };
        pending.manifest.textures = manifest.textures.clone();
        for (_, key) in &lit.image_slots {
            let Some((resolved, file)) = lit::resolved_blob(key, manifest) else {
                // No record: the fold paths answer without bytes, today's
                // shape.
                continue;
            };
            if pending.manifest.blobs.contains_key(&resolved) {
                continue;
            }
            pending
                .manifest
                .blobs
                .insert(resolved.clone(), file.clone());
            let Some(identity) = lit::image_identity(key, manifest, &manifest.textures) else {
                // A file without its record folds like today's missing
                // record — no bytes held either way.
                continue;
            };
            if self.image_memos.failed(&identity) || self.identity_committed(&identity) {
                // Today's residency answers without bytes: the fold
                // reads the memo, the survivor its GPU texture.
                continue;
            }
            let bytes = blobs.get(&resolved).cloned().or_else(|| {
                // By content: a rename or a second texture on the same image
                // changes the blob name, not the sha `resident_blob` answered.
                previous.and_then(|previous| {
                    previous
                        .blobs
                        .iter()
                        .find(|(name, _)| {
                            previous
                                .manifest
                                .blobs
                                .get(*name)
                                .is_some_and(|held| held.sha256 == file.sha256)
                        })
                        .map(|(_, bytes)| bytes.clone())
                })
            });
            if let Some(bytes) = bytes {
                pending.held_shas.insert(file.sha256.clone());
                pending.blobs.insert(resolved, bytes);
            }
            // Absent bytes stay absent: the folds reproduce today's
            // missing-blob shape, never memoising a content failure.
        }
        pending
    }

    /// Run the pending lit texture stage on the first render that needs it
    /// exactly the work `set_scene` used to do inline: slot
    /// compaction, instruction and frame remaps, decodes, binds. The
    /// instruction buffer write moves with it (its words are the stage's
    /// remapped instructions), and the GPU texture state commits here, so
    /// the last committed state still always belongs to the scene it was
    /// built from. A failed run keeps the stage pending — the next lit
    /// render retries it; its decode counts roll back, and the scene the
    /// update installed stays in place while the render reports the error.
    fn run_pending_lit_textures(&mut self) -> Result<(), String> {
        let Some(pending) = self.pending_lit_textures.take() else {
            return Ok(());
        };
        let Some(mut scene) = self.scene.take() else {
            // A pending stage never outlives its scene (clear_scene drops
            // both); drop the inputs rather than run them against nothing.
            return Ok(());
        };
        let started = Instant::now();
        // The stage remaps these in place; a failed run restores them so the
        // retry starts from the evaluated scene, not a half-remapped one.
        let lit = &scene.lit;
        let before = (
            lit.image_slots.clone(),
            lit.texture_instructions.clone(),
            lit.light_frames.clone(),
            lit.lights.clone(),
            lit.analytic.clone(),
            lit.notes.clone(),
        );
        let staged = self.upload_lit_textures(&mut scene.lit, &pending.manifest, &pending.blobs);
        let textures = match staged {
            Ok(stage) => stage,
            Err(error) => {
                let lit = &mut scene.lit;
                (
                    lit.image_slots,
                    lit.texture_instructions,
                    lit.light_frames,
                    lit.lights,
                    lit.analytic,
                    lit.notes,
                ) = before;
                self.scene = Some(scene);
                // An aborted run's decode counts must not leak into the
                // next successful one's stats — the rule the inline stage
                // kept.
                self.image_memos.take_decodes();
                self.pending_lit_textures = Some(pending);
                return Err(error);
            }
        };
        if !scene.lit.texture_instructions.is_empty() {
            self.queue.write_buffer(
                &self.texture_instruction_buffer,
                0,
                bytemuck::cast_slice(&scene.lit.texture_instructions),
            );
        }
        let notes = scene.lit.notes.clone();
        self.scene = Some(scene);
        // The texture state commits only now — the run succeeded; the
        // meshes' staged rule again.
        self.texture_bind_group = textures.bind_group;
        self.image_views = textures.image_views;
        self.lit_image_array = textures.array;
        self.lit_binding_images = textures.bindings;
        self.lit_binding_rejects = textures.rejects;
        self.stats.lit_image_bytes = textures.bytes;
        self.stats.image_decodes += self.image_memos.take_decodes();
        self.rendered_lit_textures = Some(RenderedLitTextures {
            timings: SceneUpdateTimings {
                lit_textures: started.elapsed(),
                lit_textures_decode: textures.decode,
                lit_textures_gpu: textures.gpu,
                ..Default::default()
            },
            notes,
        });
        Ok(())
    }

    /// The deferred lit texture stage's report from the last render that
    /// ran it: the stage's durations and the scene's notes with
    /// the stage's own folded in — what `last_update_timings` and
    /// `lit_notes` showed when the upload ran inside updates. `None`
    /// until the stage runs and after it has been taken.
    pub fn take_rendered_lit_textures(&mut self) -> Option<RenderedLitTextures> {
        self.rendered_lit_textures.take()
    }

    /// Why light frames or images fall off the array: matching the
    /// texture instruction notes' wording.
    fn array_limit_reason(&self, capacity: usize) -> &'static str {
        if self.device.limits().max_texture_array_layers as usize <= capacity {
            "image array layer limit exceeded"
        } else {
            "image array byte budget exceeded"
        }
    }

    /// Link the lit mode's image slots to their textures. Identities are
    /// the blob's manifest sha256 plus the decode settings, never byte
    /// copies; shapes that differ only in mapping/wrap/scale share the image
    /// body while retaining those per-instruction parameters, and a control
    /// map deduped in front of an identical one keeps its frame pointing at
    /// the shared layer.
    fn upload_lit_textures(
        &mut self,
        lit: &mut lit::LitScene,
        manifest: &GeometryManifest,
        blobs: &BTreeMap<String, Vec<u8>>,
    ) -> Result<LitTextureStage, String> {
        let mut identities = BTreeMap::new();
        let mut remap = BTreeMap::new();
        let mut compact = Vec::new();
        for (slot, key) in &lit.image_slots {
            let identity = lit::image_identity(key, manifest, &manifest.textures);
            let existing = identity
                .as_ref()
                .and_then(|identity| identities.get(identity))
                .copied();
            let mapped = existing.unwrap_or(compact.len() as u32);
            if existing.is_none() {
                if let Some(identity) = identity {
                    identities.insert(identity, mapped);
                }
                compact.push((mapped, key.clone()));
            }
            remap.insert(*slot, mapped);
        }
        for instruction in &mut lit.texture_instructions {
            if instruction.op == lit::texture_vm::OP_IMAGE
                && let Some(slot) = remap.get(&instruction.a)
            {
                instruction.a = *slot;
            }
        }
        // The framed lights' slot words ride the old linker slots; remap
        // them after the compaction, before any path reads them back.
        let vertices = lit.emitter_vertices.len() as u32;
        for index in 0..lit.lights.len() {
            let (light, frames) = (&lit.lights[index], &mut lit.light_frames);
            if !matches!(
                light.kind[0],
                lit::LIGHT_GONIOMETRIC | lit::LIGHT_PROJECTION
            ) || light.kind[3] == 0
            {
                continue;
            }
            let frame = light.kind[2].saturating_sub(vertices) as usize;
            let Some(word) = frames.get_mut(frame) else {
                continue;
            };
            if let Some(mapped) = remap.get(&(word[3] as u32)) {
                word[3] = *mapped as f32;
            }
        }
        lit.image_slots = compact;
        if self.binding_array_capacity.is_some() {
            return self.upload_binding_array(lit, manifest, blobs);
        }
        self.upload_image_array(lit, manifest, blobs)
    }

    /// The portable array path: a layer sequence the committed array
    /// already holds reuses its texture and bind group whole — zero
    /// transfers; a changed one copies surviving layers GPU-to-GPU over all
    /// mips and decodes only new identities, so decode failures show at the
    /// transfer and demote, remembered per identity.
    fn upload_image_array(
        &mut self,
        lit: &mut lit::LitScene,
        manifest: &GeometryManifest,
        blobs: &BTreeMap<String, Vec<u8>>,
    ) -> Result<LitTextureStage, String> {
        let capacity = self.image_capacity();
        // Framed light maps beyond the array's layers fold to their uniform
        // averages, the same treatment the texture instructions below get.
        for slot in framed_light_slots(lit) {
            if slot >= capacity as u32 {
                lit::demote_slot(lit, slot, self.array_limit_reason(capacity));
            }
        }
        let mip_count = lit::TEXTURE_SIZE.ilog2() + 1;
        let layers = lit
            .image_slots
            .iter()
            .filter(|(slot, _)| (*slot as usize) < capacity)
            .map(|(slot, _)| *slot)
            .max()
            .map_or(0, |slot| slot + 1)
            .max(1);
        // What each layer is, sans decodes.
        let committed = self.lit_image_array.clone();
        let mut previous = BTreeMap::new();
        if let Some(committed) = &committed {
            for (layer, identity) in committed.identities.iter().enumerate() {
                if let Some(identity) = identity {
                    previous.insert(identity, layer as u32);
                }
            }
        }
        let mut plans: Vec<Option<ImageLayer>> = vec![None; layers as usize];
        let mut identities: Vec<Option<lit::ImageIdentity>> = vec![None; layers as usize];
        for (slot, key) in lit
            .image_slots
            .iter()
            .filter(|(slot, _)| (*slot as usize) < capacity)
        {
            let control_map = key.starts_with("light/");
            let Some(identity) = lit::image_identity(key, manifest, &manifest.textures) else {
                plans[*slot as usize] = Some(ImageLayer::Missing);
                continue;
            };
            if self.image_memos.failed(&identity) {
                plans[*slot as usize] = Some(ImageLayer::Failed);
                identities[*slot as usize] = Some(identity);
                continue;
            }
            // A committed identity is reused without its bytes — the
            // partial read skips them and the layer survives on the GPU.
            if let Some(old) = previous.get(&identity).copied() {
                identities[*slot as usize] = Some(identity);
                plans[*slot as usize] = Some(ImageLayer::Survive { old });
                continue;
            }
            identities[*slot as usize] = Some(identity.clone());
            if !lit::image_bytes_present(key, blobs) {
                // A sha without its bytes (only hand-built packages and
                // residency misses): nothing can upload; fold like today's
                // missing blob — never memoising a failure of the content.
                identities[*slot as usize] = None;
                plans[*slot as usize] = Some(ImageLayer::Missing);
                continue;
            }
            plans[*slot as usize] = Some(ImageLayer::Fresh {
                key: key.clone(),
                identity,
                control_map,
            });
        }
        // The instruction fold, in today's position and order: budget first,
        // then the slot's fate. Missing and failed identities fold to 0.5 —
        // the colour today's average fallback resolves to.
        for instruction in &mut lit.texture_instructions {
            if instruction.op != lit::texture_vm::OP_IMAGE {
                continue;
            }
            let key = lit
                .image_slots
                .iter()
                .find(|(slot, _)| *slot == instruction.a)
                .map(|(_, key)| key.clone());
            let scale = f32::from_bits(instruction.c);
            let (color, reason) = if instruction.a as usize >= capacity {
                let Some(key) = key else {
                    instruction.op = lit::texture_vm::OP_SPECTRUM;
                    instruction.value = [0.5 * scale; 4];
                    continue;
                };
                let color = self
                    .image_memos
                    .average_of(&key, manifest, &manifest.textures, blobs);
                let reason = if self.device.limits().max_texture_array_layers as usize <= capacity {
                    "image array layer limit exceeded: replaced by its average"
                } else {
                    "image array byte budget exceeded: replaced by its average"
                };
                (color, reason)
            } else {
                let invalid = matches!(
                    plans.get(instruction.a as usize),
                    Some(Some(ImageLayer::Missing)) | Some(Some(ImageLayer::Failed))
                );
                if !invalid {
                    continue;
                }
                ([0.5f32; 3], "missing or invalid imagemap: replaced by 0.5")
            };
            instruction.op = lit::texture_vm::OP_SPECTRUM;
            instruction.value = [color[0] * scale, color[1] * scale, color[2] * scale, 1.0];
            lit.notes.push(lit::LitNote {
                reason: reason.into(),
                material: None,
                draws: 0,
            });
        }
        // Control maps that cannot upload demote their framed lights like
        // the binding path does: the failure is known before the
        // transfer); material instructions above already folded.
        for (slot, plan) in plans.iter().enumerate() {
            if let Some(ImageLayer::Missing) | Some(ImageLayer::Failed) = plan {
                lit::demote_slot(lit, slot as u32, "missing or invalid light control map");
            }
        }
        // An identical layer sequence reuses the committed array and bind
        // group wholesale: zero transfers, zero decodes.
        if committed
            .as_ref()
            .is_some_and(|committed| committed.identities == identities)
        {
            // The identities being equal makes every fold above replay the
            // committed array's own update exactly.
            let committed = committed.expect("checked");
            let bytes = committed.bytes;
            return Ok(LitTextureStage {
                bind_group: committed.bind_group.clone(),
                image_views: vec![committed.view.clone()],
                array: Some(committed),
                bindings: BTreeMap::new(),
                rejects: BTreeMap::new(),
                bytes,
                decode: Duration::ZERO,
                gpu: Duration::ZERO,
            });
        }
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("preview-lit-image-array"),
            size: wgpu::Extent3d {
                width: lit::TEXTURE_SIZE,
                height: lit::TEXTURE_SIZE,
                depth_or_array_layers: layers,
            },
            mip_level_count: mip_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            // COPY_SRC: the next array copies surviving layers out of this
            // one.
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("preview-lit-image-copy"),
            });
        let mut transfer_failures: Vec<(u32, lit::ImageIdentity, bool)> = Vec::new();
        for (slot, plan) in plans.iter().enumerate() {
            let slot = slot as u32;
            match plan {
                Some(ImageLayer::Survive { old }) => {
                    for mip in 0..mip_count {
                        let size = lit::TEXTURE_SIZE >> mip;
                        encoder.copy_texture_to_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: &committed
                                    .as_ref()
                                    .expect("survivors come from it")
                                    .texture,
                                mip_level: mip,
                                origin: wgpu::Origin3d {
                                    x: 0,
                                    y: 0,
                                    z: *old,
                                },
                                aspect: wgpu::TextureAspect::All,
                            },
                            wgpu::TexelCopyTextureInfo {
                                texture: &texture,
                                mip_level: mip,
                                origin: wgpu::Origin3d {
                                    x: 0,
                                    y: 0,
                                    z: slot,
                                },
                                aspect: wgpu::TextureAspect::All,
                            },
                            wgpu::Extent3d {
                                width: size,
                                height: size,
                                depth_or_array_layers: 1,
                            },
                        );
                    }
                }
                Some(ImageLayer::Fresh {
                    key,
                    identity,
                    control_map,
                }) => {
                    if *control_map {
                        // Light control maps upload at their native size in
                        // the layer's top-left: the frame words carry the
                        // resolution and the shader reads exact texels.
                        let Some(bytes) = blobs.get(key) else {
                            continue;
                        };
                        self.image_memos.count_decode();
                        let Ok((width, height, texels)) = lit::environment::decode(bytes) else {
                            transfer_failures.push((slot, identity.clone(), true));
                            continue;
                        };
                        let pixels = texels
                            .iter()
                            .flat_map(|t| t.map(half::f16::from_f32).map(half::f16::to_bits))
                            .collect::<Vec<_>>();
                        self.queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: &texture,
                                mip_level: 0,
                                origin: wgpu::Origin3d {
                                    x: 0,
                                    y: 0,
                                    z: slot,
                                },
                                aspect: wgpu::TextureAspect::All,
                            },
                            bytemuck::cast_slice(&pixels),
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(width * 8),
                                rows_per_image: Some(height),
                            },
                            wgpu::Extent3d {
                                width,
                                height,
                                depth_or_array_layers: 1,
                            },
                        );
                    } else {
                        let Some(record) = lit::texture_from_key(key, &manifest.textures) else {
                            continue;
                        };
                        let Some(bytes) = lit::texture_bytes(key, blobs) else {
                            continue;
                        };
                        self.image_memos.count_decode();
                        let Ok(decoded) = lit::decode_texture(record, bytes) else {
                            transfer_failures.push((slot, identity.clone(), false));
                            continue;
                        };
                        self.image_memos.note_average(identity, &decoded);
                        let mut level = image::imageops::resize(
                            &decoded,
                            lit::TEXTURE_SIZE,
                            lit::TEXTURE_SIZE,
                            image::imageops::FilterType::Triangle,
                        );
                        for mip in 0..mip_count {
                            let width = level.width();
                            let height = level.height();
                            let half_pixels = level
                                .pixels()
                                .flat_map(|pixel| {
                                    pixel.0.map(half::f16::from_f32).map(half::f16::to_bits)
                                })
                                .collect::<Vec<_>>();
                            self.queue.write_texture(
                                wgpu::TexelCopyTextureInfo {
                                    texture: &texture,
                                    mip_level: mip,
                                    origin: wgpu::Origin3d {
                                        x: 0,
                                        y: 0,
                                        z: slot,
                                    },
                                    aspect: wgpu::TextureAspect::All,
                                },
                                bytemuck::cast_slice(&half_pixels),
                                wgpu::TexelCopyBufferLayout {
                                    offset: 0,
                                    bytes_per_row: Some(width * 8),
                                    rows_per_image: Some(height),
                                },
                                wgpu::Extent3d {
                                    width,
                                    height,
                                    depth_or_array_layers: 1,
                                },
                            );
                            if width == 1 && height == 1 {
                                break;
                            }
                            level = image::imageops::resize(
                                &level,
                                (width / 2).max(1),
                                (height / 2).max(1),
                                image::imageops::FilterType::Triangle,
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        // Transfer-time failures demote now: the same fold and note the
        // missing and failed identities got, remembered so the next update
        // does not decode again.
        for (slot, identity, control_map) in transfer_failures {
            self.image_memos
                .note_failure(identity, lit::ControlMapError::Decode(String::new()));
            if control_map {
                lit::demote_slot(lit, slot, "missing or invalid light control map");
                continue;
            }
            for instruction in &mut lit.texture_instructions {
                if instruction.op == lit::texture_vm::OP_IMAGE && instruction.a == slot {
                    let scale = f32::from_bits(instruction.c);
                    instruction.op = lit::texture_vm::OP_SPECTRUM;
                    instruction.value = [0.5 * scale, 0.5 * scale, 0.5 * scale, 1.0];
                }
            }
            lit.notes.push(lit::LitNote {
                reason: "missing or invalid imagemap: replaced by 0.5".into(),
                material: None,
                draws: 0,
            });
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            array_layer_count: Some(layers),
            ..Default::default()
        });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("preview-texture-bindings"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.repeat_sampler),
                },
            ],
        });
        self.queue.submit([encoder.finish()]);
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(format!(
                "preview lit image array allocation failed: {error}"
            ));
        }
        let bytes = layers as u64
            * lit::image_store::storage_bytes(
                lit::TEXTURE_SIZE,
                lit::TEXTURE_SIZE,
                wgpu::TextureFormat::Rgba16Float,
            );
        Ok(LitTextureStage {
            bind_group: bind_group.clone(),
            image_views: vec![view.clone()],
            array: Some(LitImageArray {
                texture,
                view,
                bind_group,
                identities,
                bytes,
            }),
            bindings: BTreeMap::new(),
            rejects: BTreeMap::new(),
            bytes,
            decode: Duration::ZERO,
            gpu: Duration::ZERO,
        })
    }

    /// The binding-array path: an identity → texture map outlives the
    /// slot layout. The same content reuses its texture whatever slot it
    /// lands in, identities that leave the scene drop, undecodable ones fold
    /// once and stay folded, and the byte budget counts the whole live set.
    fn upload_binding_array(
        &mut self,
        lit: &mut lit::LitScene,
        manifest: &GeometryManifest,
        blobs: &BTreeMap<String, Vec<u8>>,
    ) -> Result<LitTextureStage, String> {
        let capacity = self.image_capacity();
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        // One update's fresh images — their level-0 uploads, each
        // capped source's downscale pass and every mip chain — batch into
        // one encoder and one submit. The temporaries a capped source
        // downscale reads stay alive until that submit.
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("preview-lit-image-mips"),
            });
        let mut temporary_sources: Vec<wgpu::Texture> = Vec::new();
        let mut decode_time = Duration::ZERO;
        let mut gpu_time = Duration::ZERO;
        // Reused images record nothing: skip the empty submit on warm updates.
        let mut recorded = false;
        let mut views = Vec::new();
        let mut slots = BTreeMap::new();
        let mut fallbacks = BTreeMap::new();
        // Light slots the loop rejects, folded to their averages after it:
        // the loop holds `lit.image_slots` for its whole run.
        let mut rejected_light_slots: Vec<(u32, String)> = Vec::new();
        let mut staged: BTreeMap<lit::ImageIdentity, LitBindingImage> = BTreeMap::new();
        let mut rejects: BTreeMap<lit::ImageIdentity, RejectedBinding> = BTreeMap::new();
        let mut bytes_used = 0u64;
        for (slot, key) in &lit.image_slots {
            if slots.contains_key(slot) || fallbacks.contains_key(slot) {
                continue;
            }
            let control_map = key.starts_with("light/");
            let Some(identity) = lit::image_identity(key, manifest, &manifest.textures) else {
                // Missing bytes or record: today's folds, no decode.
                demote_binding(
                    &mut lit.notes,
                    &mut fallbacks,
                    &mut rejected_light_slots,
                    (*slot, key),
                    None,
                    || [0.5; 3],
                );
                continue;
            };
            if self.image_memos.failed(&identity) {
                // A decode that failed once folds without another attempt.
                demote_binding(
                    &mut lit.notes,
                    &mut fallbacks,
                    &mut rejected_light_slots,
                    (*slot, key),
                    None,
                    || [0.5; 3],
                );
                continue;
            }
            if let Some(cached) = self.lit_binding_images.get(&identity) {
                // Reuse: the same content is already on the GPU, but
                // fitting the previous set does not mean fitting this one.
                let exceeded = if views.len() >= capacity {
                    Some("binding count")
                } else if bytes_used + cached.bytes > lit::TEXTURE_MEMORY_BUDGET {
                    Some("byte budget")
                } else {
                    None
                };
                if let Some(exceeded) = exceeded {
                    if exceeded == "byte budget" {
                        rejects.insert(
                            identity,
                            RejectedBinding {
                                bytes: cached.bytes,
                                exceeded,
                            },
                        );
                    }
                    demote_binding(
                        &mut lit.notes,
                        &mut fallbacks,
                        &mut rejected_light_slots,
                        (*slot, key),
                        Some(exceeded),
                        || {
                            self.image_memos
                                .average_of(key, manifest, &manifest.textures, blobs)
                        },
                    );
                    continue;
                }
                bytes_used += cached.bytes;
                slots.insert(*slot, views.len() as u32);
                views.push(cached.texture.create_view(&Default::default()));
                staged.insert(
                    identity,
                    LitBindingImage {
                        texture: cached.texture.clone(),
                        bytes: cached.bytes,
                    },
                );
                continue;
            }
            if let Some(rejected) = self.lit_binding_rejects.get(&identity) {
                // Rejected last time: re-evaluate the budgets with the
                // memoised byte size — no decode. Demote again while it still
                // does not fit; fall through to decode when it does.
                let exceeded = if views.len() >= capacity {
                    Some("binding count")
                } else if rejected.exceeded == "resolution limit"
                    || bytes_used + rejected.bytes > lit::TEXTURE_MEMORY_BUDGET
                {
                    Some(rejected.exceeded)
                } else {
                    None
                };
                if let Some(exceeded) = exceeded {
                    rejects.insert(
                        identity,
                        RejectedBinding {
                            bytes: rejected.bytes,
                            exceeded: rejected.exceeded,
                        },
                    );
                    demote_binding(
                        &mut lit.notes,
                        &mut fallbacks,
                        &mut rejected_light_slots,
                        (*slot, key),
                        Some(exceeded),
                        || {
                            // A rejected image never decodes — the
                            // fold reads a stored average when one exists.
                            self.image_memos
                                .memoised_average(key, manifest, &manifest.textures)
                                .unwrap_or([0.5; 3])
                        },
                    );
                    continue;
                }
            }
            // Light control maps stay on today's flow: the environment's
            // decode (sRGB gamma rules, box-shrunk to the layer limit) reads
            // their pixels for the summaries, and their budgets follow the
            // decode. Material textures decide their budgets from the
            // header first: a rejected image is never decoded and
            // its fold falls to a stored average when one exists; what is
            // accepted decodes into the stored format its bits allow.
            let bytes_present = lit::image_bytes_present(key, blobs);
            let decode_started = Instant::now();
            let decoded = if bytes_present {
                if control_map {
                    self.image_memos.count_decode();
                    blobs
                        .get(key)
                        .and_then(|bytes| lit::environment::decode(bytes).ok())
                        .and_then(|(width, height, texels)| {
                            let flat: Vec<f32> =
                                texels.iter().flat_map(|t| t.iter().copied()).collect();
                            image::Rgba32FImage::from_raw(width, height, flat)
                        })
                        .map(StagedDecoded::Control)
                } else {
                    match lit::texture_from_key(key, &manifest.textures)
                        .zip(lit::texture_bytes(key, blobs))
                    {
                        None => None,
                        Some((record, image_bytes)) => {
                            match lit::image_store::plan_texture(record, image_bytes) {
                                Err(_) => {
                                    // A header that cannot even be probed is
                                    // today's failed decode attempt: counted,
                                    // memoised as a failure, folded to 0.5.
                                    self.image_memos.count_decode();
                                    self.image_memos.note_failure(
                                        identity,
                                        lit::ControlMapError::Decode(String::new()),
                                    );
                                    demote_binding(
                                        &mut lit.notes,
                                        &mut fallbacks,
                                        &mut rejected_light_slots,
                                        (*slot, key),
                                        None,
                                        || [0.5; 3],
                                    );
                                    continue;
                                }
                                Ok(plan) => {
                                    let exceeded = if views.len() >= capacity {
                                        Some("binding count")
                                    } else if bytes_used + plan.bytes > lit::TEXTURE_MEMORY_BUDGET {
                                        Some("byte budget")
                                    } else if plan.width.max(plan.height)
                                        > self.device.limits().max_texture_dimension_2d
                                    {
                                        Some("resolution limit")
                                    } else {
                                        None
                                    };
                                    if let Some(exceeded) = exceeded {
                                        if exceeded != "binding count" {
                                            // The binding count is
                                            // order-dependent; the byte budget
                                            // and the resolution are content
                                            // facts.
                                            rejects.insert(
                                                identity,
                                                RejectedBinding {
                                                    bytes: plan.bytes,
                                                    exceeded,
                                                },
                                            );
                                        }
                                        // The header rejected the image before
                                        // any decode: the fold is a
                                        // stored average when one exists, and
                                        // never a fresh decode.
                                        demote_binding(
                                            &mut lit.notes,
                                            &mut fallbacks,
                                            &mut rejected_light_slots,
                                            (*slot, key),
                                            Some(exceeded),
                                            || {
                                                self.image_memos
                                                    .memoised_average(
                                                        key,
                                                        manifest,
                                                        &manifest.textures,
                                                    )
                                                    .unwrap_or([0.5; 3])
                                            },
                                        );
                                        continue;
                                    } else {
                                        self.image_memos.count_decode();
                                        // A capped 8-bit source
                                        // downscales on the GPU from a
                                        // temporary texture of the source's
                                        // own size — kept only while the
                                        // device limit allows one.
                                        // One bilinear tap averages at most 2×
                                        // per axis; larger reductions take the
                                        // CPU Triangle resize instead.
                                        let source_long = plan.source_width.max(plan.source_height);
                                        let source_fits = source_long
                                            <= self.device.limits().max_texture_dimension_2d
                                            && source_long <= 2 * lit::TEXTURE_MAX_EDGE;
                                        match lit::image_store::stored_image(
                                            record,
                                            image_bytes,
                                            &plan,
                                            source_fits,
                                        ) {
                                            Ok(stored) => {
                                                self.image_memos
                                                    .note_average_color(&identity, stored.average);
                                                Some(StagedDecoded::Stored(stored))
                                            }
                                            Err(_) => {
                                                self.image_memos.note_failure(
                                                    identity,
                                                    lit::ControlMapError::Decode(String::new()),
                                                );
                                                demote_binding(
                                                    &mut lit.notes,
                                                    &mut fallbacks,
                                                    &mut rejected_light_slots,
                                                    (*slot, key),
                                                    None,
                                                    || [0.5; 3],
                                                );
                                                continue;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                None
            };
            let Some(decoded) = decoded else {
                // Absent bytes are this read's shape, not the content's
                // failure — the fold stands but nothing is memoised; a real
                // decode failure (bytes present) stays remembered per
                // identity.
                if bytes_present {
                    self.image_memos
                        .note_failure(identity, lit::ControlMapError::Decode(String::new()));
                }
                demote_binding(
                    &mut lit.notes,
                    &mut fallbacks,
                    &mut rejected_light_slots,
                    (*slot, key),
                    None,
                    || [0.5; 3],
                );
                continue;
            };
            decode_time += decode_started.elapsed();
            // Control maps decide their budgets here — their size only
            // arrives with their pixels; material textures already
            // passed theirs against the header.
            let (width, height, bytes) = match &decoded {
                StagedDecoded::Control(image) => {
                    let (width, height) = image.dimensions();
                    let bytes = lit::image_store::storage_bytes(
                        width,
                        height,
                        wgpu::TextureFormat::Rgba16Float,
                    );
                    let exceeded = if views.len() >= capacity {
                        Some("binding count")
                    } else if bytes_used + bytes > lit::TEXTURE_MEMORY_BUDGET {
                        Some("byte budget")
                    } else if width.max(height) > self.device.limits().max_texture_dimension_2d {
                        Some("resolution limit")
                    } else {
                        None
                    };
                    if let Some(exceeded) = exceeded {
                        if exceeded != "binding count" {
                            // The binding count is order-dependent; the byte
                            // budget and the resolution are content facts.
                            rejects.insert(identity, RejectedBinding { bytes, exceeded });
                        }
                        demote_binding(
                            &mut lit.notes,
                            &mut fallbacks,
                            &mut rejected_light_slots,
                            (*slot, key),
                            Some(exceeded),
                            || {
                                self.image_memos.average_of(
                                    key,
                                    manifest,
                                    &manifest.textures,
                                    blobs,
                                )
                            },
                        );
                        continue;
                    }
                    (width, height, bytes)
                }
                StagedDecoded::Stored(stored) => (
                    stored.width,
                    stored.height,
                    lit::image_store::storage_bytes(stored.width, stored.height, stored.format),
                ),
            };
            bytes_used += bytes;
            let (format, mip_count) = match &decoded {
                StagedDecoded::Control(image) => (
                    wgpu::TextureFormat::Rgba16Float,
                    image.width().max(image.height()).ilog2() + 1,
                ),
                // The GPU generates the whole chain, one pass per
                // level: the same per-level sizes `bytes` counted.
                StagedDecoded::Stored(stored) => {
                    (stored.format, stored.width.max(stored.height).ilog2() + 1)
                }
            };
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("preview-lit-native-image"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: mip_count,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                // Every level is written by a render pass on top
                // of the sampling and upload usages.
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let gpu_started = Instant::now();
            recorded = true;
            // Level 0: encoded texels write straight into mip 0,
            // control maps convert once through today's f16 encode, and a
            // capped source uploads its full-size bytes into a temporary
            // texture whose single pass renders level 0 of the stored
            // texture. Every level below samples the one above it.
            match &decoded {
                StagedDecoded::Control(image) => {
                    let pixels: Vec<u16> = image
                        .pixels()
                        .flat_map(|pixel| pixel.0.map(half::f16::from_f32).map(half::f16::to_bits))
                        .collect();
                    self.queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        bytemuck::cast_slice(&pixels),
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(width * 8),
                            rows_per_image: Some(height),
                        },
                        wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                    );
                }
                StagedDecoded::Stored(stored) => match &stored.level_0 {
                    lit::image_store::StoredLevel0::Encoded(level) => {
                        let texel_bytes = lit::image_store::texel_bytes(stored.format) as u32;
                        self.queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: &texture,
                                mip_level: 0,
                                origin: wgpu::Origin3d::ZERO,
                                aspect: wgpu::TextureAspect::All,
                            },
                            level,
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(width * texel_bytes),
                                rows_per_image: Some(height),
                            },
                            wgpu::Extent3d {
                                width,
                                height,
                                depth_or_array_layers: 1,
                            },
                        );
                    }
                    lit::image_store::StoredLevel0::Source {
                        width: source_width,
                        height: source_height,
                        bytes: level,
                    } => {
                        let texel_bytes = lit::image_store::texel_bytes(stored.format) as u32;
                        let temporary = self.device.create_texture(&wgpu::TextureDescriptor {
                            label: Some("preview-lit-image-source"),
                            size: wgpu::Extent3d {
                                width: *source_width,
                                height: *source_height,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING
                                | wgpu::TextureUsages::COPY_DST,
                            view_formats: &[],
                        });
                        self.queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: &temporary,
                                mip_level: 0,
                                origin: wgpu::Origin3d::ZERO,
                                aspect: wgpu::TextureAspect::All,
                            },
                            level,
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(*source_width * texel_bytes),
                                rows_per_image: Some(*source_height),
                            },
                            wgpu::Extent3d {
                                width: *source_width,
                                height: *source_height,
                                depth_or_array_layers: 1,
                            },
                        );
                        let source_view = temporary.create_view(&Default::default());
                        let level_0 = texture.create_view(&wgpu::TextureViewDescriptor {
                            dimension: Some(wgpu::TextureViewDimension::D2),
                            base_mip_level: 0,
                            mip_level_count: Some(1),
                            ..Default::default()
                        });
                        self.record_lit_mip_pass(&mut encoder, format, &source_view, &level_0);
                        // Kept alive until the submit below reads it.
                        temporary_sources.push(temporary);
                    }
                },
            }
            for mip in 1..mip_count {
                let previous = texture.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_mip_level: mip - 1,
                    mip_level_count: Some(1),
                    ..Default::default()
                });
                let next = texture.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_mip_level: mip,
                    mip_level_count: Some(1),
                    ..Default::default()
                });
                self.record_lit_mip_pass(&mut encoder, format, &previous, &next);
            }
            gpu_time += gpu_started.elapsed();
            slots.insert(*slot, views.len() as u32);
            views.push(texture.create_view(&Default::default()));
            staged.insert(identity, LitBindingImage { texture, bytes });
        }
        for instruction in &mut lit.texture_instructions {
            if instruction.op != lit::texture_vm::OP_IMAGE {
                continue;
            }
            if let Some(mapped) = slots.get(&instruction.a) {
                instruction.a = *mapped;
            } else {
                let color = fallbacks.get(&instruction.a).copied().unwrap_or([0.5; 3]);
                let scale = f32::from_bits(instruction.c);
                instruction.op = lit::texture_vm::OP_SPECTRUM;
                instruction.value = [color[0] * scale, color[1] * scale, color[2] * scale, 1.];
            }
        }
        for (slot, reason) in &rejected_light_slots {
            lit::demote_slot(lit, *slot, reason);
        }
        // The light frames index this binding array, so their slot words
        // take the same remap the image instructions got. The words are
        // static, keeping the remap across later rebinds.
        let vertices = lit.emitter_vertices.len() as u32;
        for light in &lit.lights {
            if !matches!(
                light.kind[0],
                lit::LIGHT_GONIOMETRIC | lit::LIGHT_PROJECTION
            ) || light.kind[3] == 0
            {
                continue;
            }
            let frame = light.kind[2].saturating_sub(vertices) as usize;
            let Some(slot) = lit.light_frames.get(frame).map(|word| word[3] as u32) else {
                continue;
            };
            if let Some(view) = slots.get(&slot) {
                lit.light_frames[frame][3] = *view as f32;
            }
        }
        // One submit runs this update's staged uploads, downscale
        // passes and mip chains together — the queue's pending writes flush
        // in front of it — and the temporaries drop with it; wgpu keeps
        // them alive until the commands that read them finish.
        let submit_started = Instant::now();
        if recorded {
            self.queue.submit([encoder.finish()]);
        }
        drop(temporary_sources);
        if views.is_empty() {
            let dummy = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("preview-lit-unused-image"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            views.push(dummy.create_view(&Default::default()));
        }
        // Without PARTIALLY_BOUND_BINDING_ARRAY every declared slot must be populated.
        views.resize(capacity, views[0].clone());
        let references = views.iter().collect::<Vec<_>>();
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("preview-native-image-bindings"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureViewArray(&references),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.repeat_sampler),
                },
            ],
        });
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(format!(
                "preview lit image bindings allocation failed: {error}"
            ));
        }
        gpu_time += submit_started.elapsed();
        Ok(LitTextureStage {
            bind_group,
            image_views: views,
            array: None,
            bindings: staged,
            rejects,
            bytes: bytes_used,
            decode: decode_time,
            gpu: gpu_time,
        })
    }

    /// The downscale pass's pipeline for one stored lit image format
    /// created once per format at first use and cached.
    fn lit_mip_pipeline(&mut self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        if let Some(pipeline) = self.lit_mip_pipelines.get(&format) {
            return pipeline.clone();
        }
        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("preview-lit-image-mip"),
                layout: Some(&self.lit_mip_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.lit_mip_module,
                    entry_point: Some("vs_fullscreen"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &self.lit_mip_module,
                    entry_point: Some("fs_mip"),
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
        self.lit_mip_pipelines.insert(format, pipeline.clone());
        pipeline
    }

    /// One downscale pass: the fullscreen triangle samples the
    /// `source` view's single level bilinearly once and overwrites
    /// `target` — the next mip level of a chain, or level 0 of a capped
    /// texture straight from its full-size temporary source.
    fn record_lit_mip_pass(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        format: wgpu::TextureFormat,
        source: &wgpu::TextureView,
        target: &wgpu::TextureView,
    ) {
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("preview-lit-mip-bindings"),
            layout: &self.lit_mip_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let pipeline = self.lit_mip_pipeline(format);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("preview-lit-image-mip"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn image_capacity(&self) -> usize {
        if let Some(capacity) = self.binding_array_capacity {
            return capacity as usize;
        }
        let mip_factor = 4u64;
        let bytes_per_layer = (lit::TEXTURE_SIZE as u64)
            .saturating_mul(lit::TEXTURE_SIZE as u64)
            .saturating_mul(8)
            .saturating_mul(mip_factor)
            / 3;
        let memory_layers = (lit::TEXTURE_MEMORY_BUDGET / bytes_per_layer.max(1)) as usize;
        self.device
            .limits()
            .max_texture_array_layers
            .min(lit::MAX_TEXTURES as u32)
            .min(memory_layers as u32) as usize
    }

    /// Replace placements without reading blobs or rebuilding geometry buffers.
    pub fn update_draws(&mut self, draws: &[DrawRecord]) -> Result<(), String> {
        let scene = self.scene.as_mut().ok_or("no preview scene uploaded")?;
        validate_draws(
            draws,
            &scene.meshes.keys().copied().collect(),
            scene.times.len(),
        )?;
        lit::rebind(draws, &mut scene.lit);
        scene.draws = draws.to_vec();
        // Moved geometry can leave the uploaded bounds; shadow maps size
        // their far distance and distant-light extent from them.
        if let Some(bounds) = draws_bounds(draws) {
            scene.bounds = Some(bounds);
        }
        scene.shadow_dirty = true;
        Ok(())
    }

    pub fn render(
        &mut self,
        camera: ViewerCamera,
        width: u32,
        height: u32,
        mode: PreviewMode,
        time: f64,
    ) -> Result<PreviewFrame, String> {
        self.render_with_options(camera, width, height, mode, time, LitDebug::Off)
    }

    pub fn render_with_options(
        &mut self,
        camera: ViewerCamera,
        width: u32,
        height: u32,
        mode: PreviewMode,
        time: f64,
        debug: LitDebug,
    ) -> Result<PreviewFrame, String> {
        let debug_albedo = debug == LitDebug::Albedo;
        let lit_mode = mode == PreviewMode::Lit || debug != LitDebug::Off;
        // The deferred lit texture stage runs on the first render
        // that needs it — before any pass reads the lit scene, so the
        // instruction buffer and the texture bindings belong to this
        // scene by the time the light passes sample them.
        if lit_mode {
            self.run_pending_lit_textures()?;
        }
        let mut view = view(camera, width, height, mode)?;
        // Shadow layout for Lit; maps are re-rendered only when the scene,
        // its placements or the shadowed lights change, never for the camera.
        let shadows = match (&self.scene, mode) {
            (Some(scene), PreviewMode::Lit) => {
                let setup = lit::shadow::shadow_setup(
                    &scene.lit.lights,
                    &scene.lit.positions,
                    scene.bounds.as_ref(),
                    &scene.lit.draw_of_light,
                );
                // The shadowed lights' passes and the time sample (animated
                // placements) decide whether the maps are still valid.
                let mut signature = setup
                    .passes
                    .iter()
                    .map(|pass| bytemuck::bytes_of(&pass.uniform).to_vec())
                    .collect::<Vec<_>>();
                signature.push(time.to_bits().to_le_bytes().to_vec());
                let scene = self.scene.as_mut().unwrap();
                if signature != scene.shadow_signature {
                    scene.shadow_signature = signature;
                    scene.shadow_dirty = true;
                }
                view.shadow_origin = [
                    (camera.eye[0] - setup.origin[0]) as f32,
                    (camera.eye[1] - setup.origin[1]) as f32,
                    (camera.eye[2] - setup.origin[2]) as f32,
                    0.,
                ];
                Some(setup)
            }
            _ => None,
        };
        let scene = self.scene.as_ref().ok_or("no preview scene uploaded")?;
        let time_index = scene
            .times
            .iter()
            .position(|sample| *sample == time)
            .ok_or("requested time is not an exported preview sample")?;
        let instance_bytes = (scene.draws.len() as u64)
            .checked_mul(std::mem::size_of::<Instance>() as u64)
            .ok_or("instance buffer size overflow")?;
        if instance_bytes > self.device.limits().max_buffer_size {
            return Err("preview instance buffer exceeds device limit".into());
        }
        if lit_mode {
            view.mode[1] = scene.lit.lights.len().min(lit::MAX_LIGHTS) as u32;
            view.mode[2] = scene.lit.appearances.len().min(lit::MAX_APPEARANCES) as u32;
            view.exposure[0] = if debug_albedo {
                1.0
            } else {
                scene.film_exposure
            };
            view.background = if debug_albedo {
                [0., 0., 0., 1.]
            } else {
                [
                    scene.lit.background[0],
                    scene.lit.background[1],
                    scene.lit.background[2],
                    1.,
                ]
            };
            view.mode[3] = match debug {
                LitDebug::Off => 0,
                LitDebug::Albedo => 1,
                LitDebug::Ao => 2,
            };
            // AO reaches a fifth of the scene's diagonal: the best fit to r4
            // in `tests/lit_ao_reference` (larger radii add halos).
            view.ao[0] = scene.bounds.as_ref().map_or(0., |b| {
                (0..3)
                    .map(|i| (b.max[i] - b.min[i]).powi(2))
                    .sum::<f64>()
                    .sqrt()
                    * 0.2
            }) as f32;
            let gpu_lights = scene
                .lit
                .lights
                .iter()
                .take(lit::MAX_LIGHTS)
                .zip(&scene.lit.positions)
                .map(|(light, position)| {
                    let mut light = *light;
                    if light.kind[0] == lit::LIGHT_POINT
                        || light.kind[0] == lit::LIGHT_SPOT
                        || light.kind[0] == lit::LIGHT_POLYGON
                        || light.kind[0] == lit::LIGHT_SPHERE
                        || light.kind[0] == lit::LIGHT_GONIOMETRIC
                        || light.kind[0] == lit::LIGHT_PROJECTION
                    {
                        // Subtract in f64 so distant lights keep precision.
                        for i in 0..3 {
                            light.position[i] = (position[i] - camera.eye[i]) as f32;
                        }
                    }
                    light
                })
                .collect::<Vec<_>>();
            if !gpu_lights.is_empty() {
                self.queue
                    .write_buffer(&self.light_buffer, 0, bytemuck::cast_slice(&gpu_lights));
            }
            if !scene.lit.appearances.is_empty() {
                self.queue.write_buffer(
                    &self.appearance_buffer,
                    0,
                    bytemuck::cast_slice(
                        &scene.lit.appearances
                            [..scene.lit.appearances.len().min(lit::MAX_APPEARANCES)],
                    ),
                );
            }
            let mut word_count = 0usize;
            let mut words = Vec::new();
            if !scene.lit.emitter_vertices.is_empty() {
                // Eye-relative like the light positions above.
                let vertices = scene.lit.emitter_vertices[..scene
                    .lit
                    .emitter_vertices
                    .len()
                    .min(lit::MAX_EMITTER_TRIANGLES * 3)]
                    .iter()
                    .map(|v| {
                        [
                            (v[0] - camera.eye[0]) as f32,
                            (v[1] - camera.eye[1]) as f32,
                            (v[2] - camera.eye[2]) as f32,
                            0.,
                        ]
                    })
                    .collect::<Vec<_>>();
                word_count = vertices.len();
                words.extend(vertices);
            }
            // The goniometric/projection frames follow the vertices: static
            // data, `kind.z` shifted by the vertex count in rebind. The
            // kind.z offsets promise all frames fit — the vertex budget
            // (192) plus MAX_LIGHTS frames equals the buffer's capacity.
            let frames = &scene.lit.light_frames
                [..scene.lit.light_frames.len().min(lit::LIGHT_FRAME_CAPACITY)];
            debug_assert!(
                word_count + frames.len()
                    <= lit::MAX_EMITTER_TRIANGLES * 3 + lit::LIGHT_FRAME_CAPACITY,
                "vertex words + light frames exceed the frame buffer"
            );
            words.extend_from_slice(frames);
            if !words.is_empty() {
                self.queue.write_buffer(
                    &self.emitter_vertex_buffer,
                    0,
                    bytemuck::cast_slice(&words),
                );
            }
        }
        let mut instances = Vec::with_capacity(scene.draws.len());
        for ((draw, appearance), alpha) in scene
            .draws
            .iter()
            .zip(&scene.lit.appearance_of)
            .zip(&scene.lit.alpha_of)
        {
            let matrix = draw
                .matrix_at(time_index)
                .ok_or("missing preview transform sample")?;
            let mesh = &scene.meshes[&draw.mesh_id];
            let mut instance = instance(matrix, camera.eye, draw, mesh.extent, &view)?;
            instance.info[1] = *appearance;
            instance.info[2] = *alpha;
            instances.push(instance);
        }
        let scene = self.scene.as_ref().unwrap();
        if !scene.lit.texture_mappings.is_empty() {
            self.queue.write_buffer(
                &self.texture_mapping_buffer,
                0,
                bytemuck::cast_slice(&scene.lit.mapping_rows(camera.eye)),
            );
        }
        if let Some(setup) = &shadows {
            if !setup.entries.is_empty() {
                self.queue.write_buffer(
                    &self.shadow_buffer,
                    0,
                    bytemuck::cast_slice(
                        &setup.entries[..setup.entries.len().min(lit::MAX_LIGHTS)],
                    ),
                );
            }
        }
        // Without draws nothing casts shadows (and no instance buffer exists).
        let render_shadows = scene.shadow_dirty
            && !scene.draws.is_empty()
            && shadows.as_ref().is_some_and(|s| !s.passes.is_empty());
        if let (true, Some(setup)) = (render_shadows, &shadows) {
            let mut placed = Vec::with_capacity(scene.draws.len());
            for (draw, alpha) in scene.draws.iter().zip(&scene.lit.alpha_of) {
                let matrix = draw
                    .matrix_at(time_index)
                    .ok_or("missing preview transform sample")?;
                let mesh = &scene.meshes[&draw.mesh_id];
                let mut placement = instance(matrix, setup.origin, draw, mesh.extent, &view)?;
                placement.info[2] = *alpha;
                placed.push(placement);
            }
            let bytes = (placed.len() * std::mem::size_of::<Instance>()) as u64;
            if bytes > self.shadow_instance_capacity {
                self.shadow_instances = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("preview-shadow-instances"),
                    size: bytes,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
                self.shadow_instance_capacity = bytes;
            }
            if !placed.is_empty() {
                self.queue.write_buffer(
                    self.shadow_instances.as_ref().unwrap(),
                    0,
                    bytemuck::cast_slice(&placed),
                );
            }
            if !scene.lit.texture_mappings.is_empty() {
                self.queue.write_buffer(
                    &self.shadow_texture_mapping_buffer,
                    0,
                    bytemuck::cast_slice(&scene.lit.mapping_rows(setup.origin)),
                );
            }
            for (index, pass) in setup.passes.iter().enumerate() {
                self.queue.write_buffer(
                    &self.shadow_pass_buffer,
                    index as u64 * SHADOW_PASS_STRIDE,
                    bytemuck::bytes_of(&pass.uniform),
                );
            }
        }
        if self
            .targets
            .as_ref()
            .is_none_or(|t| t.width != width || t.height != height)
        {
            let scope = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
            let targets = Targets::new(&self.device, width, height);
            let error = pollster::block_on(scope.pop());
            if let Some(error) = error {
                return Err(format!("preview target allocation failed: {error}"));
            }
            self.targets = Some(targets?);
            let target = self.targets.as_ref().unwrap();
            self.output_bind_group =
                Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("preview-output-bindings"),
                    layout: &self.output_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: self.view.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&target.hdr_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&target.ao_view),
                        },
                    ],
                }));
            let ao_bindings = |input: &wgpu::TextureView| {
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("preview-ao-bindings"),
                    layout: &self.ao_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: self.view.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&target.depth_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&target.normal_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(input),
                        },
                    ],
                })
            };
            // GTAO's input slot is unused; the denoise reads the raw AO.
            self.bind_group =
                view_bindings(&self.device, &self.view_layout, &self.view, &target.ao_view);
            self.ao_bind_groups = Some([
                ao_bindings(&target.ao_view),
                ao_bindings(&target.ao_raw_view),
            ]);
            self.stats.target_allocations += 1;
        }
        if instance_bytes > self.instance_capacity {
            let scope = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("preview-instances"),
                size: instance_bytes,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            if let Some(error) = pollster::block_on(scope.pop()) {
                return Err(format!("preview instance allocation failed: {error}"));
            }
            self.instances = Some(buffer);
            self.instance_capacity = instance_bytes;
            self.last_instances.clear();
            self.stats.instance_allocations += 1;
        }
        if self.last_view != Some(view) {
            self.queue
                .write_buffer(&self.view, 0, bytemuck::bytes_of(&view));
            self.last_view = Some(view);
        }
        if instances != self.last_instances {
            if !instances.is_empty() {
                self.queue.write_buffer(
                    self.instances.as_ref().unwrap(),
                    0,
                    bytemuck::cast_slice(&instances),
                );
            }
            self.last_instances = instances;
        }
        let targets = self.targets.as_ref().unwrap();
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("preview-encoder"),
            });
        if let (true, Some(setup)) = (render_shadows, &shadows) {
            for (index, shadow_pass) in setup.passes.iter().enumerate() {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("preview-shadow-pass"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.shadow_layer_views[shadow_pass.layer as usize],
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.shadow_pipeline);
                pass.set_bind_group(
                    0,
                    &self.shadow_pass_bind_group,
                    &[(index as u64 * SHADOW_PASS_STRIDE) as u32],
                );
                pass.set_bind_group(1, &self.texture_bind_group, &[]);
                pass.set_bind_group(2, &self.shadow_vm_bind_group, &[]);
                pass.set_vertex_buffer(1, self.shadow_instances.as_ref().unwrap().slice(..));
                for (draw_index, draw) in scene.draws.iter().enumerate() {
                    // Glass lets light through rather than casting a dark
                    // shadow. A mix casts whole, as in r4: its shadow rays stop
                    // at the surface whichever material it would have selected.
                    if draw.interface
                        || shadow_pass.exclude_draw == Some(draw_index)
                        || (is_glass(scene, draw_index) && !is_mix(scene, draw_index))
                    {
                        continue;
                    }
                    if scene
                        .lit
                        .appearance_of
                        .get(draw_index)
                        .is_some_and(|index| {
                            scene
                                .lit
                                .appearances
                                .get(*index as usize)
                                .is_some_and(|lobe| lobe.kind == lit::KIND_INVISIBLE)
                        })
                    {
                        continue;
                    }
                    let mesh = &scene.meshes[&draw.mesh_id];
                    if mesh.triangle_count == 0 {
                        continue;
                    }
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.triangles.slice(..), wgpu::IndexFormat::Uint32);
                    let instance = draw_index as u32;
                    pass.draw_indexed(0..mesh.triangle_count, 0, instance..instance + 1);
                }
            }
            self.stats.shadow_renders += 1;
        }
        let opaque_lit = |i: usize| {
            !(scene.draws[i].interface
                || lobe_of(scene, i).is_some_and(|lobe| lobe.kind == lit::KIND_INVISIBLE)
                || is_glass(scene, i))
        };
        if lit_mode {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("preview-prez-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.normal_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_bind_group(1, &self.texture_bind_group, &[]);
            pass.set_bind_group(2, &self.lit_bind_group, &[]);
            pass.set_bind_group(3, &self.environment_bind_group, &[]);
            if let Some(buffer) = &self.instances {
                pass.set_vertex_buffer(1, buffer.slice(..));
            }
            for (i, draw) in scene.draws.iter().enumerate() {
                let mesh = &scene.meshes[&draw.mesh_id];
                if !opaque_lit(i) || mesh.triangle_count == 0 {
                    continue;
                }
                pass.set_pipeline(if scene.lit.alpha_of.get(i).is_some_and(|a| *a != 0) {
                    &self.prez_alpha_pipeline
                } else {
                    &self.prez_pipeline
                });
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.triangles.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.triangle_count, 0, i as u32..i as u32 + 1);
            }
        }
        if lit_mode && !debug_albedo {
            let bind_groups = self.ao_bind_groups.as_ref().ok_or("missing AO bindings")?;
            for (pipeline, bind_group, target, label) in [
                (
                    &self.ao_pipeline,
                    &bind_groups[0],
                    &targets.ao_raw_view,
                    "preview-ao-pass",
                ),
                (
                    &self.ao_denoise_pipeline,
                    &bind_groups[1],
                    &targets.ao_view,
                    "preview-ao-denoise-pass",
                ),
            ] {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(label),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        {
            let first_color_view = if lit_mode {
                &targets.hdr_view
            } else {
                &targets.color_view
            };
            let clear = if lit_mode {
                wgpu::Color {
                    r: view.background[0] as f64,
                    g: view.background[1] as f64,
                    b: view.background[2] as f64,
                    a: 1.,
                }
            } else {
                wgpu::Color {
                    r: 0.055,
                    g: 0.065,
                    b: 0.08,
                    a: 1.,
                }
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(if lit_mode {
                    "preview-lit-pass"
                } else {
                    "preview-pass"
                }),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: first_color_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(clear),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &targets.pick_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: if lit_mode {
                            wgpu::LoadOp::Load
                        } else {
                            wgpu::LoadOp::Clear(0.)
                        },
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let wire = mode == PreviewMode::Wireframe;
            pass.set_pipeline(if lit_mode {
                &self.lit_pipeline
            } else if wire {
                &self.lines
            } else {
                &self.triangles
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            if lit_mode {
                pass.set_bind_group(1, &self.texture_bind_group, &[]);
                pass.set_bind_group(2, &self.lit_bind_group, &[]);
                pass.set_bind_group(3, &self.environment_bind_group, &[]);
                if scene.lit.environment.is_some() && !debug_albedo {
                    pass.set_pipeline(&self.environment_background_pipeline);
                    pass.draw(0..3, 0..1);
                    pass.set_pipeline(&self.lit_pipeline);
                }
            }
            if let Some(buffer) = &self.instances {
                pass.set_vertex_buffer(1, buffer.slice(..));
            }
            // Lit draws in reverse: `Equal` lets the last of coincident surfaces
            // shade, and PreZ's `Greater` kept the first.
            let order: Vec<usize> = if lit_mode {
                (0..scene.draws.len()).rev().collect()
            } else {
                (0..scene.draws.len()).collect()
            };
            for i in order {
                let draw = &scene.draws[i];
                if draw.interface || (lit_mode && !opaque_lit(i)) {
                    continue;
                }
                let mesh = &scene.meshes[&draw.mesh_id];
                let count = if wire {
                    mesh.line_count
                } else {
                    mesh.triangle_count
                };
                if count == 0 {
                    continue;
                }
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(
                    if wire {
                        mesh.lines.slice(..)
                    } else {
                        mesh.triangles.slice(..)
                    },
                    wgpu::IndexFormat::Uint32,
                );
                pass.draw_indexed(0..count, 0, i as u32..i as u32 + 1);
            }
            if lit_mode {
                // Glass, far to near: object-level ordering (no OIT).
                for i in glass_far_to_near(scene, camera.eye, time_index) {
                    let mesh = &scene.meshes[&scene.draws[i].mesh_id];
                    if mesh.triangle_count == 0 {
                        continue;
                    }
                    pass.set_pipeline(if is_mix(scene, i) {
                        &self.lit_mix_pipeline
                    } else {
                        &self.lit_transparent_pipeline
                    });
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.triangles.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..mesh.triangle_count, 0, i as u32..i as u32 + 1);
                }
            }
        }
        // Interface has no visible surface, but its geometry still supplies
        // pick ids (and therefore the desktop's selection outline). This pass
        // tests against visible depth without changing color.
        if scene.draws.iter().any(|draw| draw.interface) {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("preview-interface-pick-pass"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &targets.color_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &targets.pick_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pick_only);
            pass.set_bind_group(0, &self.bind_group, &[]);
            if let Some(buffer) = &self.instances {
                pass.set_vertex_buffer(1, buffer.slice(..));
            }
            for (i, draw) in scene
                .draws
                .iter()
                .enumerate()
                .filter(|(_, draw)| draw.interface)
            {
                let mesh = &scene.meshes[&draw.mesh_id];
                if mesh.triangle_count == 0 {
                    continue;
                }
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.triangles.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.triangle_count, 0, i as u32..i as u32 + 1);
            }
        }
        if lit_mode {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("preview-output-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.output_pipeline);
            pass.set_bind_group(
                0,
                self.output_bind_group
                    .as_ref()
                    .ok_or("missing output bindings")?,
                &[],
            );
            pass.draw(0..3, 0..1);
        }
        targets.copy(&mut encoder);
        self.queue.submit(Some(encoder.finish()));
        if lit_mode && self.scene.as_ref().is_some_and(|scene| scene.shadow_dirty) {
            self.scene.as_mut().unwrap().shadow_dirty = false;
        }
        let frame = targets.read(&self.device, &self.backend)?;
        self.stats.frames += 1;
        Ok(frame)
    }
}

/// The most pixels one preview frame may have.
pub const MAX_PREVIEW_PIXELS: u64 = 12_000_000;

fn view(camera: ViewerCamera, width: u32, height: u32, mode: PreviewMode) -> Result<View, String> {
    if width == 0 || height == 0 || width as u64 * height as u64 > MAX_PREVIEW_PIXELS {
        return Err("preview dimensions must be positive and at most 12 million pixels".into());
    }
    if !protocol::view::camera_is_valid(&camera.to_protocol()) {
        return Err("invalid viewer camera".into());
    }
    // Shared with the gizmo's projection (pbrt's left-handed LookAt).
    let super::gizmo::Basis {
        forward,
        right,
        up,
        ty,
    } = super::gizmo::basis(&camera).ok_or("invalid viewer camera")?;
    // The view's fov is vertical; Render View writes the matching pbrt
    // `screenwindow`, so both frame alike at any aspect.
    let tx = width as f64 / height as f64 * ty;
    // Reversed Z: near maps to depth 1 and far to 0. With a float depth
    // buffer this keeps precision at distance even when near is tiny (a
    // camera inside the scene clamps near to 0.001), so back faces do not
    // speckle through front ones. Depth clears to 0 and tests `Greater`.
    let a = -camera.near / (camera.far - camera.near);
    let b = camera.near * camera.far / (camera.far - camera.near);
    let rows = [
        [right[0] / tx, right[1] / tx, right[2] / tx, 0.],
        [up[0] / ty, up[1] / ty, up[2] / ty, 0.],
        [a * forward[0], a * forward[1], a * forward[2], b],
        [forward[0], forward[1], forward[2], 0.],
    ];
    let projection: [[f32; 4]; 4] =
        std::array::from_fn(|column| std::array::from_fn(|row| rows[row][column] as f32));
    if projection.iter().flatten().any(|v| !v.is_finite()) {
        return Err("camera exceeds finite preview range".into());
    }
    let ray = |v: [f64; 3], scale: f64| {
        [
            (v[0] * scale) as f32,
            (v[1] * scale) as f32,
            (v[2] * scale) as f32,
            0.,
        ]
    };
    Ok(View {
        projection,
        ray_forward: ray(forward, 1.),
        ray_right: ray(right, tx),
        ray_up: ray(up, ty),
        mode: [
            match mode {
                PreviewMode::Solid => 0,
                PreviewMode::Normal => 1,
                PreviewMode::Wireframe => 2,
                PreviewMode::Lit => 3,
            },
            0,
            0,
            0,
        ],
        ..Default::default()
    })
}

fn lobe_of(scene: &Scene, draw: usize) -> Option<&lit::GpuLobe> {
    let index = *scene.lit.appearance_of.get(draw)?;
    scene.lit.appearances.get(index as usize)
}

/// Draws whose lobe transmits light: drawn transparent, no shadow. Only
/// lobes the GPU holds count; beyond MAX_APPEARANCES a draw keeps the
/// opaque default shading.
fn is_glass(scene: &Scene, draw: usize) -> bool {
    scene
        .lit
        .appearance_of
        .get(draw)
        .is_some_and(|index| (*index as usize) < lit::MAX_APPEARANCES)
        && lobe_of(scene, draw)
            .is_some_and(|lobe| lobe.transmission > 0.0 && lobe.kind != lit::KIND_INVISIBLE)
}

fn is_mix(scene: &Scene, draw: usize) -> bool {
    lobe_of(scene, draw).is_some_and(|lobe| lobe.kind == lit::KIND_MIX)
}

/// Glass draws ordered far to near by the eye's distance to each mesh's
/// center placed at the rendered time sample (animated glass reorders as it
/// moves); ties keep draw order.
fn glass_far_to_near(scene: &Scene, eye: [f64; 3], time_index: usize) -> Vec<usize> {
    let mut glass = (0..scene.draws.len())
        .filter(|i| is_glass(scene, *i))
        .filter_map(|i| {
            let draw = &scene.draws[i];
            let m = draw.matrix_at(time_index)?;
            let c = scene.meshes[&draw.mesh_id].center;
            let center: [f64; 3] = std::array::from_fn(|r| {
                m[r * 4] * c[0] + m[r * 4 + 1] * c[1] + m[r * 4 + 2] * c[2] + m[r * 4 + 3]
            });
            let distance = (0..3).map(|a| (center[a] - eye[a]).powi(2)).sum::<f64>();
            Some((i, distance))
        })
        .collect::<Vec<_>>();
    glass.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    glass.into_iter().map(|(i, _)| i).collect()
}

/// Upload the GGX LTC table as a two-layer Rgba32Float array.
fn ltc_table(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    let size = lit::ltc::LTC_SIZE;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("preview-lit-ltc"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 2,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (layer, data) in lit::ltc::ltc_layers().iter().enumerate() {
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
                bytes_per_row: Some(16 * size),
                rows_per_image: Some(size),
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );
    }
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    })
}

/// Union of the draws' bounds, when every draw has bounds.
fn draws_bounds(draws: &[DrawRecord]) -> Option<protocol::geometry::Bounds> {
    let mut bounds = draws.iter().map(|draw| draw.bounds.as_ref());
    let first = bounds.next()??.clone();
    bounds.try_fold(first, |union, next| Some(union.union(next?)))
}

fn instance(
    m: [f64; 16],
    eye: [f64; 3],
    draw: &DrawRecord,
    extent: [f64; 3],
    view: &View,
) -> Result<Instance, String> {
    // Subtract in f64 before casting: the mesh origin is already part of m.
    let relative: [[f64; 4]; 3] = std::array::from_fn(|r| {
        std::array::from_fn(|c| {
            if c == 3 {
                m[r * 4 + c] - eye[r]
            } else {
                m[r * 4 + c]
            }
        })
    });
    let bounds: [f64; 3] = std::array::from_fn(|r| {
        relative[r][3].abs()
            + (0..3)
                .map(|c| relative[r][c].abs() * extent[c])
                .sum::<f64>()
    });
    if bounds
        .iter()
        .any(|b| *b > f32::MAX as f64 || !b.is_finite())
        || (0..4).any(|r| {
            (0..3)
                .map(|c| (view.projection[c][r] as f64).abs() * bounds[c])
                .sum::<f64>()
                + (view.projection[3][r] as f64).abs()
                > f32::MAX as f64
        })
    {
        return Err("vertex exceeds the finite preview clip range".into());
    }
    // Cofactors are proportional to inverse-transpose, including the
    // determinant sign required for geometric normals under negative scale.
    let scale = (0..3)
        .flat_map(|r| (0..3).map(move |c| m[r * 4 + c].abs()))
        .fold(0f64, f64::max)
        .max(f64::MIN_POSITIVE);
    let cols: [[f64; 3]; 3] =
        std::array::from_fn(|c| std::array::from_fn(|r| m[r * 4 + c] / scale));
    let cofactors = [
        cross(cols[1], cols[2]),
        cross(cols[2], cols[0]),
        cross(cols[0], cols[1]),
    ];
    let factor = cofactors
        .iter()
        .flatten()
        .map(|n| n.abs())
        .fold(0f64, f64::max)
        .max(f64::MIN_POSITIVE);
    let sign = if draw.reverse_orientation { -1. } else { 1. };
    Ok(Instance {
        world: relative.map(|r| r.map(|v| v as f32)),
        normal: std::array::from_fn(|r| {
            std::array::from_fn(|c| {
                if c == 3 {
                    0.
                } else {
                    (sign * cofactors[c][r] / factor) as f32
                }
            })
        }),
        info: [draw.pick_id, 0, 0, 0],
    })
}

fn pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    topology: wgpu::PrimitiveTopology,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("preview-pipeline"), layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader, entry_point: Some("vs_main"), compilation_options: Default::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x2] }),
                Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Instance>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &wgpu::vertex_attr_array![3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Float32x4,7=>Float32x4,8=>Float32x4,9=>Uint32x4] }),
            ],
        },
        primitive: wgpu::PrimitiveState { topology, front_face: wgpu::FrontFace::Ccw, cull_mode: None, ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Greater), stencil: Default::default(), bias: Default::default() }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader, entry_point: Some("fs_main"), compilation_options: Default::default(),
            targets: &[
                Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8UnormSrgb, blend: None, write_mask: wgpu::ColorWrites::ALL }),
                Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::R32Uint, blend: None, write_mask: wgpu::ColorWrites::ALL }),
            ],
        }), multiview_mask: None, cache: None,
    })
}

fn pick_only_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("preview-interface-pick-only-pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x2] }),
                Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Instance>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &wgpu::vertex_attr_array![3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Float32x4,7=>Float32x4,8=>Float32x4,9=>Uint32x4] }),
            ],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            // Nothing reads depth after this pass, so writing it lets the
            // nearest of overlapping interface surfaces keep the pick id.
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Greater),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[
                Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    blend: None,
                    write_mask: wgpu::ColorWrites::empty(),
                }),
                Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::R32Uint,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                }),
            ],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn storage_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// The Lit pipeline. Opaque draws write depth; transparent ones (glass)
/// test against it without writing, and blend premultiplied: their
/// reflection adds and `alpha = 1 − transmission` lets the rest through.
/// Pick ids are written either way, so the front glass is pickable.
fn lit_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    transparent: bool,
    depth_write: bool,
) -> wgpu::RenderPipeline {
    // Opaque draws only shade the surface the PreZ pass left in front.
    let compare = if transparent {
        wgpu::CompareFunction::Greater
    } else {
        wgpu::CompareFunction::Equal
    };
    let blend = transparent.then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(if transparent { "preview-lit-transparent-pipeline" } else { "preview-lit-pipeline" }),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x2] }),
                Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Instance>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &wgpu::vertex_attr_array![3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Float32x4,7=>Float32x4,8=>Float32x4,9=>Uint32x4] }),
            ],
        },
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, front_face: wgpu::FrontFace::Ccw, cull_mode: None, ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(depth_write),
            depth_compare: Some(compare),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(if transparent { "fs_lit_transparent" } else { "fs_lit" }),
            compilation_options: Default::default(),
            targets: &[
                Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba16Float, blend, write_mask: wgpu::ColorWrites::ALL }),
                Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::R32Uint, blend: None, write_mask: wgpu::ColorWrites::ALL }),
            ],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Lit PreZ: the same vertex stage as `lit_pipeline` (its position is
/// `@invariant`), writing depth and the normal target only.
fn prez_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    entry: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x2] }),
                Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Instance>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &wgpu::vertex_attr_array![3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Float32x4,7=>Float32x4,8=>Float32x4,9=>Uint32x4] }),
            ],
        },
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, front_face: wgpu::FrontFace::Ccw, cull_mode: None, ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Greater),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format: super::targets::NORMAL_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn shadow_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("preview-shadow-pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_shadow"),
            compilation_options: Default::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x2] }),
                Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Instance>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &wgpu::vertex_attr_array![3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Float32x4,7=>Float32x4,8=>Float32x4,9=>Uint32x4] }),
            ],
        },
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState { module: shader, entry_point: Some("fs_shadow"), compilation_options: Default::default(), targets: &[] }),
        multiview_mask: None,
        cache: None,
    })
}

fn view_bindings(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::Buffer,
    ao: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("preview-view-bindings"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: view.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(ao),
            },
        ],
    })
}

/// Lit AO: a full-screen triangle writing one `AO_FORMAT` channel.
fn ao_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    entry: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_ao"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: super::targets::AO_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn output_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("preview-output-pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_output"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_output"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod camera_basis_tests {
    use super::*;

    /// Shadow maps size themselves from the draws' current bounds, so moved
    /// geometry stays inside them (review #264).
    #[test]
    fn draws_bounds_unions_every_draw_or_gives_up() {
        let mut a = test_draw();
        a.bounds = Some(protocol::geometry::Bounds {
            min: [0.; 3],
            max: [1.; 3],
        });
        let mut b = test_draw();
        b.bounds = Some(protocol::geometry::Bounds {
            min: [5.; 3],
            max: [6.; 3],
        });
        let union = draws_bounds(&[a.clone(), b]).unwrap();
        assert_eq!((union.min, union.max), ([0.; 3], [6.; 3]));
        let mut unknown = test_draw();
        unknown.bounds = None;
        assert!(draws_bounds(&[a, unknown]).is_none());
        assert!(draws_bounds(&[]).is_none());
    }

    fn test_draw() -> DrawRecord {
        DrawRecord {
            mesh_id: 0,
            shape_index: 0,
            source: None,
            shape_type: "trianglemesh".into(),
            pick_id: 1,
            entity_id: None,
            instance_use_id: None,
            instance_source: None,
            bounds: None,
            world_from_object: vec![[
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
            ]],
            reverse_orientation: false,
            material_id: None,
            interface: false,
            area_light: None,
            emitter_shape: None,
            alpha: None,
        }
    }

    #[test]
    fn projection_preserves_camera_up_and_right_without_a_gpu() {
        let camera = ViewerCamera {
            eye: [0., 0., 5.],
            target: [0.; 3],
            up: [0., 1., 0.],
            fov_y_degrees: 45.,
            near: 0.1,
            far: 100.,
            mirror: false,
        };
        let view = view(camera, 64, 64, PreviewMode::Solid).unwrap();
        assert!(
            view.projection[1][1] > 0.,
            "world +Y projects toward clip-space top"
        );
        // pbrt's LookAt is left-handed (right = up × forward): looking down
        // -Z from +Z, world +X lands on the image's left, as in its renders.
        assert!(
            view.projection[0][0] < 0.,
            "world +X projects toward clip-space left, as pbrt renders it"
        );
        // A right-handed scene's mirror flips only the horizontal axis.
        let mirrored = super::view(
            ViewerCamera {
                mirror: true,
                ..camera
            },
            64,
            64,
            PreviewMode::Solid,
        )
        .unwrap();
        assert_eq!(mirrored.projection[0][0], -view.projection[0][0]);
        assert_eq!(mirrored.projection[1][1], view.projection[1][1]);
        let mut panned = camera;
        panned.pan(0., 0.1).unwrap();
        assert!(panned.eye[1] > camera.eye[1]);
        assert!(panned.target[1] > camera.target[1]);
    }
}

/// Radiance adds: several lights into one cube, and the cube over the uniform
/// background the color target cleared to.
const ENVIRONMENT_ADD: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent::REPLACE,
};
/// Cube face size. Source maps above this density lose detail in the background;
/// the lighting integrates them regardless.
const ENVIRONMENT_FACE: u32 = 512;

/// Distinct image slots the framed goniometric/projection lights carry in
/// their frame words (the slot rides the first row's w component).
fn framed_light_slots(lit: &lit::LitScene) -> Vec<u32> {
    let vertices = lit.emitter_vertices.len() as u32;
    let mut slots = std::collections::BTreeSet::new();
    for light in &lit.lights {
        if matches!(
            light.kind[0],
            lit::LIGHT_GONIOMETRIC | lit::LIGHT_PROJECTION
        ) && light.kind[3] > 0
        {
            let frame = light.kind[2].saturating_sub(vertices) as usize;
            if let Some(word) = lit.light_frames.get(frame) {
                slots.insert(word[3] as u32);
            }
        }
    }
    slots.into_iter().collect()
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct EnvironmentFace {
    forward: [f32; 4],
    s: [f32; 4],
    t: [f32; 4],
    light_from_world: [[f32; 4]; 3],
    scale: [f32; 4],
}

fn environment_cube(device: &wgpu::Device, label: &str, face: u32, levels: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: face,
            height: face,
            depth_or_array_layers: 6,
        },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

/// A cube map of the given face size and the bindings that read it.
fn environment_bindings(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    face: u32,
    levels: u32,
    irradiance: &[[f32; 4]; 9],
) -> (wgpu::Texture, wgpu::BindGroup) {
    let cube = environment_cube(device, "preview-environment-cube", face, levels);
    let view = cube.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::Cube),
        ..Default::default()
    });
    let harmonics = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("preview-environment-irradiance"),
        contents: bytemuck::cast_slice(irradiance),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("preview-environment-bindings"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: harmonics.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });
    (cube, bind_group)
}

impl PreviewRenderer {
    /// Bake the scene's image-based lights into a world-space cube. A scene
    /// without any keeps a one-texel cube and zero harmonics.
    fn bake_environment(&mut self, environment: Option<&lit::environment::Environment>) {
        let Some(environment) = environment else {
            self.environment_bind_group = environment_bindings(
                &self.device,
                &self.environment_layout,
                &self.environment_sampler,
                1,
                1,
                &[[0.; 4]; 9],
            )
            .1;
            return;
        };
        let (cube, bind_group) = environment_bindings(
            &self.device,
            &self.environment_layout,
            &self.environment_sampler,
            ENVIRONMENT_FACE,
            lit::ENVIRONMENT_LEVELS,
            &environment.irradiance,
        );
        self.environment_bind_group = bind_group;
        // The radiance, with its own full mip chain for the prefilter to read.
        let source_levels = ENVIRONMENT_FACE.ilog2() + 1;
        let radiance = environment_cube(
            &self.device,
            "preview-environment-radiance",
            ENVIRONMENT_FACE,
            source_levels,
        );
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("preview-environment-bake"),
            });
        // Vulkan's cube layout: t grows down each face's image.
        const FACES: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
            ([1., 0., 0.], [0., 0., -1.], [0., -1., 0.]),
            ([-1., 0., 0.], [0., 0., 1.], [0., -1., 0.]),
            ([0., 1., 0.], [1., 0., 0.], [0., 0., 1.]),
            ([0., -1., 0.], [1., 0., 0.], [0., 0., -1.]),
            ([0., 0., 1.], [1., 0., 0.], [0., -1., 0.]),
            ([0., 0., -1.], [-1., 0., 0.], [0., -1., 0.]),
        ];
        // One upload per map: only the face uniform changes between faces.
        let sources = environment
            .images
            .iter()
            .map(|image| self.environment_source_view(image))
            .collect::<Vec<_>>();
        for (layer, (forward, s, t)) in FACES.into_iter().enumerate() {
            let target = radiance.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_mip_level: 0,
                mip_level_count: Some(1),
                base_array_layer: layer as u32,
                array_layer_count: Some(1),
                ..Default::default()
            });
            let groups = environment
                .images
                .iter()
                .zip(&sources)
                .map(|(image, view)| self.environment_face_bindings(image, view, forward, s, t))
                .collect::<Vec<_>>();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("preview-environment-bake"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.environment_bake_pipeline);
            for group in &groups {
                pass.set_bind_group(0, group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        for (layer, (forward, s, t)) in FACES.into_iter().enumerate() {
            let layer = layer as u32;
            for target_level in 1..source_levels {
                let source = radiance.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::Cube),
                    base_mip_level: target_level - 1,
                    mip_level_count: Some(1),
                    ..Default::default()
                });
                self.environment_filter_pass(
                    &mut encoder,
                    &self.environment_downsample_pipeline,
                    &source,
                    (&radiance, target_level, layer),
                    (forward, s, t),
                    [0., (ENVIRONMENT_FACE >> target_level) as f32, 0., 0.],
                );
            }
        }
        let source = radiance.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        for (layer, (forward, s, t)) in FACES.into_iter().enumerate() {
            for level in 0..lit::ENVIRONMENT_LEVELS {
                let roughness = level as f32 / (lit::ENVIRONMENT_LEVELS - 1) as f32;
                self.environment_filter_pass(
                    &mut encoder,
                    &self.environment_prefilter_pipeline,
                    &source,
                    (&cube, level, layer as u32),
                    (forward, s, t),
                    [
                        roughness * roughness,
                        (ENVIRONMENT_FACE >> level) as f32,
                        ENVIRONMENT_FACE as f32,
                        0.,
                    ],
                );
            }
        }
        self.queue.submit([encoder.finish()]);
    }

    /// Draw one level of one cube face from `source`.
    fn environment_filter_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        source: &wgpu::TextureView,
        (texture, level, layer): (&wgpu::Texture, u32, u32),
        (forward, s, t): ([f32; 3], [f32; 3], [f32; 3]),
        params: [f32; 4],
    ) {
        let v4 = |v: [f32; 3]| [v[0], v[1], v[2], 0.];
        let uniform = [v4(forward), v4(s), v4(t), params];
        let buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("preview-environment-level"),
                contents: bytemuck::cast_slice(&uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("preview-environment-level"),
            layout: &self.environment_filter_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.environment_sampler),
                },
            ],
        });
        let target = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2),
            base_mip_level: level,
            mip_level_count: Some(1),
            base_array_layer: layer,
            array_layer_count: Some(1),
            ..Default::default()
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("preview-environment-level"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn environment_source_view(
        &self,
        image: &lit::environment::EnvironmentImage,
    ) -> wgpu::TextureView {
        self.device
            .create_texture_with_data(
                &self.queue,
                &wgpu::TextureDescriptor {
                    label: Some("preview-environment-source"),
                    size: wgpu::Extent3d {
                        width: image.width,
                        height: image.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba32Float,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                bytemuck::cast_slice(&image.texels),
            )
            .create_view(&Default::default())
    }

    fn environment_face_bindings(
        &self,
        image: &lit::environment::EnvironmentImage,
        source: &wgpu::TextureView,
        forward: [f32; 3],
        s: [f32; 3],
        t: [f32; 3],
    ) -> wgpu::BindGroup {
        let m = &image.light_from_world;
        let row = |r: usize| {
            [
                m[r * 4] as f32,
                m[r * 4 + 1] as f32,
                m[r * 4 + 2] as f32,
                0.,
            ]
        };
        let v3 = |v: [f32; 3]| [v[0], v[1], v[2], 0.];
        let face = EnvironmentFace {
            forward: v3(forward),
            s: v3(s),
            t: v3(t),
            light_from_world: [row(0), row(1), row(2)],
            scale: [image.scale, ENVIRONMENT_FACE as f32, 0., 0.],
        };
        let buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("preview-environment-face"),
                contents: bytemuck::bytes_of(&face),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("preview-environment-face"),
            layout: &self.environment_bake_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(source),
                },
            ],
        })
    }
}

/// Fold one binding-path slot the loop cannot bind: a control map's light
/// demotes after the loop; a texture takes `fallback` (0.5 when invalid, its
/// average when a limit was `exceeded`) with a note.
fn demote_binding(
    notes: &mut Vec<lit::LitNote>,
    fallbacks: &mut BTreeMap<u32, [f32; 3]>,
    rejected_light_slots: &mut Vec<(u32, String)>,
    (slot, key): (u32, &str),
    exceeded: Option<&str>,
    fallback: impl FnOnce() -> [f32; 3],
) {
    if key.starts_with("light/") {
        rejected_light_slots.push((
            slot,
            match exceeded {
                Some(exceeded) => format!("image {exceeded} exceeded"),
                None => "missing or invalid light control map".to_owned(),
            },
        ));
        return;
    }
    fallbacks.insert(slot, fallback());
    notes.push(lit::LitNote {
        reason: match exceeded {
            Some(exceeded) => format!("image {exceeded} exceeded: {key} replaced by its average"),
            None => format!("missing or invalid imagemap {key}: replaced by 0.5"),
        },
        material: None,
        draws: 0,
    });
}
