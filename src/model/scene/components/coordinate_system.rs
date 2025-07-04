use crate::model::base::*;
use crate::model::scene::Component;

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
