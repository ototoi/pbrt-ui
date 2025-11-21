use crate::model::base::*;
use crate::model::scene::Component;

#[derive(Debug, Clone)]
pub struct CoordinateSystemComponent {
    up: Vector3,
}

impl CoordinateSystemComponent {
    pub fn new(up: &Vector3) -> Self {
        let up = up.clone();
        CoordinateSystemComponent { up }
    }
    pub fn get_up_vector(&self) -> Vector3 {
        return self.up;
    }
}

impl Component for CoordinateSystemComponent {}
