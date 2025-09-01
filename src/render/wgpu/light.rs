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
}


#[derive(Debug, Clone, Default)]
pub struct RenderLightRect {
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub u_axis: [f32; 3],    // U axis for rectangle
    pub v_axis: [f32; 3],    // V axis for rectangle
    pub intensity: [f32; 3], // RGB intensity
}

#[derive(Debug, Clone, Default)]
pub struct RectsRenderLight {
    pub id: Uuid,
    pub edition: String,
    pub rects: Vec<RenderLightRect>, // Multiple rectangles
}

#[derive(Debug, Clone)]
pub enum RenderLight {
    Directional(DirectionalRenderLight),
    Sphere(SphereRenderLight),
    Disk(DiskRenderLight),
    Rects(RectsRenderLight),
    // Add other light types as needed
}

impl RenderLight {
    pub fn get_id(&self) -> Uuid {
        match self {
            RenderLight::Directional(light) => light.id,
            RenderLight::Sphere(light) => light.id,
            RenderLight::Disk(light) => light.id,
            RenderLight::Rects(light) => light.id,
            // Handle other light types here
        }
    }

    pub fn get_edition(&self) -> String {
        match self {
            RenderLight::Directional(light) => light.edition.clone(),
            RenderLight::Sphere(light) => light.edition.clone(),
            RenderLight::Disk(light) => light.edition.clone(),
            RenderLight::Rects(light) => light.edition.clone(),
            // Handle other light types here
        }
    }
}
