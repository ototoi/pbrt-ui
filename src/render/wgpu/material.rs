use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum RenderUniformValue {
    Float(f32),
    Vec4([f32; 4]),
    Mat4([f32; 16]),
    Int(i32),
    Bool(bool),
}

#[derive(Debug, Clone)]
pub struct RenderMaterial {
    pub id: Uuid,
    pub edition: String,
    pub uniform_values: Vec<(String, RenderUniformValue)>, //key, value
}

impl RenderMaterial {
    pub fn get_id(&self) -> Uuid {
        self.id
    }

    pub fn get_edition(&self) -> String {
        self.edition.clone()
    }

    pub fn get_uniform_value(&self, key: &str) -> Option<&RenderUniformValue> {
        self.uniform_values
            .iter()
            .find_map(|(k, v)| if k == key { Some(v) } else { None })
    }
}
