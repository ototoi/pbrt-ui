use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod, Zeroable)]
pub struct PlasticMaterialUniforms {
    pub kd: [f32; 4],        // Diffuse color
    pub ks: [f32; 4],        // Specular color
    pub roughness: [f32; 4], // Roughness
    pub _pad1: [f32; 4],     // Padding to ensure alignment
}
