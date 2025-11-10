use super::texture::RenderTexture;

use std::sync::Arc;
use uuid::Uuid;

#[repr(u32)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderCategory {
    #[default]
    Opaque = 1000, //use for opaque surfaces
    Emissive = 1500,    //use for light diffuse no lighting
    Masked = 2500,      //use for masked surfaces
    Transparent = 3000, //use for transparent surfaces
}

#[derive(Debug, Clone)]
pub enum RenderUniformValue {
    Float(f32),
    Vec4([f32; 4]),
    Mat4([f32; 16]),
    Int(i32),
    Bool(bool),
    Texture(Arc<RenderTexture>),
}

#[derive(Debug, Default, Clone)]
pub struct RenderMaterial {
    pub id: Uuid,
    pub edition: String,
    pub material_type: String,
    pub shader_type: String,
    pub render_category: RenderCategory,
    pub uniform_values: Vec<(String, RenderUniformValue)>, //key, value
}

impl RenderMaterial {
    pub fn get_id(&self) -> Uuid {
        self.id
    }

    pub fn get_edition(&self) -> String {
        self.edition.clone()
    }

    pub fn get_material_type(&self) -> String {
        self.material_type.clone()
    }

    pub fn get_shader_type(&self) -> String {
        return self.shader_type.clone();
    }

    pub fn get_uniform_value(&self, key: &str) -> Option<&RenderUniformValue> {
        self.uniform_values
            .iter()
            .find_map(|(k, v)| if k == key { Some(v) } else { None })
    }
}
