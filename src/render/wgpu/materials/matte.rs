use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod, Zeroable)]
pub struct MatteMaterialUniforms {
    pub kd: [f32; 4],    // Diffuse color
    pub _pad1: [f32; 4], // Padding to ensure alignment
    pub _pad2: [f32; 4], // Padding to ensure alignment
    pub _pad3: [f32; 4], // Padding to ensure alignment
}
