use crate::models::base::*;
use crate::models::scene::Component;

#[derive(Debug, Clone)]
pub struct CoordinateSystemComponent {
    pub up: Vector3,
}

impl CoordinateSystemComponent {
    pub fn new(up: &Vector3) -> Self {
        let up = up.clone();
        CoordinateSystemComponent { up }
    }
}

impl Component for CoordinateSystemComponent {}
