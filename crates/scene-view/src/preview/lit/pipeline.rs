//! Shader sources and the two Lit-specific render pipelines.
pub(in crate::preview) const LIT_WGSL: &str = concat!(
    include_str!("../vertex.wgsl"),
    include_str!("common.wgsl"),
    include_str!("texture_vm.wgsl"),
    include_str!("shadow.wgsl"),
    include_str!("lights.wgsl"),
    include_str!("ltc.wgsl"),
    include_str!("main.wgsl")
);
pub(in crate::preview) const SHADOW_WGSL: &str = concat!(
    include_str!("shadow_pass.wgsl"),
    include_str!("texture_vm.wgsl"),
);
pub(in crate::preview) const OUTPUT_WGSL: &str = include_str!("output.wgsl");
pub(in crate::preview) const AO_WGSL: &str = include_str!("ao.wgsl");

pub(in crate::preview) struct LitPipelines {
    pub(in crate::preview) lit: wgpu::RenderPipeline,
    pub(in crate::preview) output: wgpu::RenderPipeline,
}

/// Both image resource paths execute the same VM and shading code.
pub(in crate::preview) fn shader_source(
    source: &str,
    binding_array_capacity: Option<u32>,
) -> String {
    let Some(capacity) = binding_array_capacity else {
        return source.into();
    };
    ("enable wgpu_binding_array;\n".to_owned() + source)
        .replace(
            "var images: texture_2d_array<f32>;",
            &format!("var images: binding_array<texture_2d<f32>, {capacity}>;"),
        )
        .replace(
            "textureSampleGrad(images, image_sampler, sample_uv, i32(instruction.a),",
            "textureSampleGrad(images[instruction.a], image_sampler, sample_uv,",
        )
        .replace(
            "textureSampleGrad(images, repeat_sampler, sample_uv, i32(instruction.a),",
            "textureSampleGrad(images[instruction.a], repeat_sampler, sample_uv,",
        )
        .replace(
            "textureDimensions(images, level)",
            "textureDimensions(images[slot], level)",
        )
        .replace(
            "textureDimensions(images, 0)",
            "textureDimensions(images[slot], 0)",
        )
        .replace("textureNumLevels(images)", "textureNumLevels(images[slot])")
        .replace(
            "textureLoad(images, coord, i32(slot), i32(level))",
            "textureLoad(images[slot], coord, i32(level))",
        )
}
