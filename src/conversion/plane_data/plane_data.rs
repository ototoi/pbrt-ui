pub struct PlaneMesh {
    pub indices: Vec<i32>,
    pub positions: Vec<f32>,
}

pub struct PlaneOutline {
    pub positions: Vec<f32>,
}

pub struct PlaneRect {
    pub center: [f32; 3],
    pub axis_u: [f32; 3],
    pub axis_v: [f32; 3],
    pub normal: [f32; 3],
}
