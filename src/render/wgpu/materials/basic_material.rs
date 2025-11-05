use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod, Zeroable)]
pub struct BasicMaterialUniforms {
    pub kd: [f32; 4],    // Diffuse color
    pub ks: [f32; 4],    // Specular color
    pub _pad1: [f32; 4], // Padding to ensure alignment
    pub _pad2: [f32; 4], // Padding to ensure alignment
}