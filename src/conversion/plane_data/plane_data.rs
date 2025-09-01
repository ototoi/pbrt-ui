#[derive(Debug, Clone)]
pub struct PlaneMesh {
    pub indices: Vec<i32>,
    pub positions: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct PlaneOutline {
    pub positions: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct PlaneRect {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub u_axis: [f32; 3],
    pub v_axis: [f32; 3],
}
