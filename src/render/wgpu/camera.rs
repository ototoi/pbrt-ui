#[derive(Debug, Clone)]
pub struct RenderCamera {
    pub world_to_camera: glam::Mat4,
    pub camera_to_world: glam::Mat4,
    pub camera_to_clip: glam::Mat4,
    pub position: glam::Vec3,
    pub forward: glam::Vec3,
    pub up: glam::Vec3,
    pub right: glam::Vec3,
}

impl RenderCamera {
    pub fn from_matrices(world_to_camera: glam::Mat4, camera_to_clip: glam::Mat4) -> Self {
        let camera_to_world = world_to_camera.inverse();
        let position = camera_to_world.transform_point3(glam::Vec3::ZERO);
        let forward = camera_to_world
            .transform_vector3(glam::vec3(0.0, 0.0, -1.0))
            .normalize_or_zero();
        let up = camera_to_world
            .transform_vector3(glam::vec3(0.0, 1.0, 0.0))
            .normalize_or_zero();
        let right = camera_to_world
            .transform_vector3(glam::vec3(1.0, 0.0, 0.0))
            .normalize_or_zero();
        Self {
            world_to_camera,
            camera_to_world,
            camera_to_clip,
            position,
            forward,
            up,
            right,
        }
    }
}
