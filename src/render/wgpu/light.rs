use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum RenderLightType {
    Directional = 0,
    Point = 1,
    Spot = 2,
}

#[derive(Debug, Clone)]
pub struct RenderLight {
    pub id: Uuid,
    pub edition: String,
    pub light_type: RenderLightType,
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub intensity: [f32; 3], // RGB intensity
    pub range: [f32; 2],     // For point and spot lights
}

impl RenderLight {
    pub fn get_id(&self) -> Uuid {
        self.id
    }

    pub fn get_edition(&self) -> String {
        self.edition.clone()
    }
}
