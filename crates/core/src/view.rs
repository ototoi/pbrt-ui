//! Validation and framing math for the GPU-independent shared view state.
//! The math itself lives in `protocol::view` so the desktop viewport and the
//! headless capture adapter share one camera definition.
use protocol::{ErrorCode, ErrorPayload, geometry::GeometryManifest, view::*};
pub fn validate(camera: &Camera) -> Result<(), ErrorPayload> {
    if !protocol::view::camera_is_valid(camera) {
        return Err(ErrorPayload::new(
            ErrorCode::InvalidCamera,
            "camera basis, field of view or clipping planes are invalid",
        ));
    }
    Ok(())
}
pub fn frame(bounds: Option<&protocol::Bounds>, size: [u32; 2]) -> Camera {
    protocol::view::framing_camera(bounds, size[0] as f64 / size[1].max(1) as f64)
}
pub fn initial(geometry: Option<&GeometryManifest>) -> ViewState {
    protocol::view::initial_view(geometry)
}
