use super::component::Component;
use crate::model::base::*;

#[derive(Debug, Clone)]
pub struct AnimationComponent { 
    pub transforms: [(Matrix4x4, f32); 2],
}

impl AnimationComponent {
    pub fn new(
        start_transform: &Matrix4x4,
        start_time: f32,
        end_transform: &Matrix4x4,
        end_time: f32
    ) -> Self {
        AnimationComponent {
            transforms: [
                (start_transform.clone(), start_time),
                (end_transform.clone(), end_time),
            ],
        }
    }
    pub fn get_keys(&self) -> Vec<(String, String)> {
        return vec![];
    }
}

impl Component for AnimationComponent {}