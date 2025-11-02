use super::texture::RenderTexture;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct DirectionalRenderLight {
    pub id: Uuid,
    pub edition: String,
    pub direction: [f32; 3],
    pub intensity: [f32; 3], // RGB intensity
}

#[derive(Debug, Clone, Default)]
pub struct SphereRenderLight {
    pub id: Uuid,
    pub edition: String,
    pub position: [f32; 3],
    pub intensity: [f32; 3], // RGB intensity
    pub radius: f32,         // Sphere radius
    //pub points: [[f32; 3]; 4], // Precomputed points on the sphere surface
}

#[derive(Debug, Clone, Default)]
pub struct DiskRenderLight {
    pub id: Uuid,
    pub edition: String,
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub intensity: [f32; 3], // RGB intensity
    pub radius: f32,         // Sphere radius
    pub inner_angle: f32,    // Inner radius for disk
    pub outer_angle: f32,    // Outer radius for disk
    pub twosided: bool,      // Whether the disk emits light on both sides
    //pub points: [[f32; 3]; 4], // Precomputed corner points of the disk
}

#[derive(Debug, Clone, Default)]
pub struct RectRenderLight {
    pub id: Uuid,
    pub edition: String,
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub u_axis: [f32; 3],    // U axis for rectangle
    pub v_axis: [f32; 3],    // V axis for rectangle
    pub intensity: [f32; 3], // RGB intensity
    pub twosided: bool,      // Whether the rectangle emits light on both sides
}

#[derive(Debug, Clone, Default)]
pub struct RectsRenderLight {
    pub id: Uuid,
    pub edition: String,
    pub rects: Vec<Arc<RenderLight>>, // Multiple rectangles
}

#[derive(Debug, Clone, Default)]
pub struct InfiniteRenderLight {
    pub id: Uuid,
    pub edition: String,
    pub intensity: [f32; 3],                 // RGB intensity
    pub texture: Option<Arc<RenderTexture>>, // Environment map texture
}

#[derive(Debug, Clone)]
pub enum RenderLight {
    Directional(DirectionalRenderLight),
    Sphere(SphereRenderLight),
    Disk(DiskRenderLight),
    Rect(RectRenderLight),
    Infinite(InfiniteRenderLight),
    // Add other light types as needed
    _Rects(RectsRenderLight),
}

impl RenderLight {
    pub fn get_id(&self) -> Uuid {
        match self {
            RenderLight::Directional(light) => light.id,
            RenderLight::Sphere(light) => light.id,
            RenderLight::Disk(light) => light.id,
            RenderLight::Rect(light) => light.id,
            RenderLight::Infinite(light) => light.id,
            // Handle other light types here
            RenderLight::_Rects(light) => light.id,
        }
    }

    pub fn get_edition(&self) -> String {
        match self {
            RenderLight::Directional(light) => light.edition.clone(),
            RenderLight::Sphere(light) => light.edition.clone(),
            RenderLight::Disk(light) => light.edition.clone(),
            RenderLight::Rect(light) => light.edition.clone(),
            RenderLight::Infinite(light) => light.edition.clone(),
            // Handle other light types here
            RenderLight::_Rects(light) => light.edition.clone(),
        }
    }
}
