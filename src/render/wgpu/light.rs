use uuid::Uuid;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum RenderLightType {
    #[default]
    Directional = 0,
    Point = 1,
    Spot = 2,
}

#[derive(Debug, Clone, Default)]
pub struct RenderLight {
    pub id: Uuid,
    pub edition: String,
    pub light_type: RenderLightType,
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub intensity: [f32; 3], // RGB intensity
    pub range: [f32; 2],     // For point and spot lights
    pub angle: [f32; 2],     // For spot lights inner and outer angles
    pub center: [f32; 3],   // For spot lights center position
}

impl RenderLight {
    pub fn get_id(&self) -> Uuid {
        self.id
    }

    pub fn get_edition(&self) -> String {
        self.edition.clone()
    }
}
