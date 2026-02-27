#[derive(Debug, Clone)]
pub struct RenderCamera {
    pub world_to_camera: glam::Mat4,
    pub camera_to_world: glam::Mat4,
    pub camera_to_clip: glam::Mat4,
    pub near: f32,
    pub far: f32,
    pub hfov: f32,
    pub vfov: f32,
    pub position: glam::Vec3,
    pub forward: glam::Vec3,
    pub up: glam::Vec3,
    pub right: glam::Vec3,
}

impl RenderCamera {
    pub fn from_matrices(world_to_camera: glam::Mat4, camera_to_clip: glam::Mat4) -> Self {
        let (near, far) = Self::infer_near_far(&camera_to_clip).unwrap_or((0.1, 1000.0));
        let (tan_half_hfov, tan_half_vfov) =
            Self::infer_tan_half_fov(&camera_to_clip).unwrap_or((1.0, 1.0));
        let hfov = 2.0 * tan_half_hfov.atan();
        let vfov = 2.0 * tan_half_vfov.atan();
        Self::new_common(world_to_camera, camera_to_clip, near, far, hfov, vfov)
    }

    pub fn from_perspective(
        world_to_camera: glam::Mat4,
        camera_to_clip: glam::Mat4,
        near: f32,
        far: f32,
        vfov: f32,
        aspect: f32,
    ) -> Self {
        let half_v = 0.5 * vfov;
        let half_h = (half_v.tan() * aspect).atan();
        let hfov = 2.0 * half_h;
        Self::new_common(world_to_camera, camera_to_clip, near, far, hfov, vfov)
    }

    fn new_common(
        world_to_camera: glam::Mat4,
        camera_to_clip: glam::Mat4,
        near: f32,
        far: f32,
        hfov: f32,
        vfov: f32,
    ) -> Self {
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
            near,
            far,
            hfov,
            vfov,
            position,
            forward,
            up,
            right,
        }
    }

    fn infer_near_far(camera_to_clip: &glam::Mat4) -> Option<(f32, f32)> {
        let clip_to_camera = camera_to_clip.inverse();
        let p_near = clip_to_camera * glam::vec4(0.0, 0.0, 0.0, 1.0);
        let p_far = clip_to_camera * glam::vec4(0.0, 0.0, 1.0, 1.0);
        if p_near.w.abs() < 1e-8 || p_far.w.abs() < 1e-8 {
            return None;
        }
        let p_near = p_near / p_near.w;
        let p_far = p_far / p_far.w;
        let near = -p_near.z;
        let far = -p_far.z;
        if near <= 1e-5 || far <= near {
            return None;
        }
        Some((near, far))
    }

    fn infer_tan_half_fov(camera_to_clip: &glam::Mat4) -> Option<(f32, f32)> {
        let m00 = camera_to_clip.x_axis.x;
        let m11 = camera_to_clip.y_axis.y;
        if m00.abs() < 1e-8 || m11.abs() < 1e-8 {
            return None;
        }
        Some((1.0 / m00.abs(), 1.0 / m11.abs()))
    }
}
