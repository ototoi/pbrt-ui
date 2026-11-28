use gpui_kit::MouseButton;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::editor) enum CameraDrag {
    Orbit,
    Pan,
}

/// Resolve navigation shortcuts when the press begins so modifier changes
/// during a drag cannot switch between orbiting, panning, and selection.
pub(in crate::editor) fn camera_drag(
    button: MouseButton,
    shift: bool,
    alt: bool,
    control: bool,
    platform: bool,
) -> Option<CameraDrag> {
    match button {
        MouseButton::Left if alt && (control || platform) => Some(CameraDrag::Pan),
        MouseButton::Left if alt && !control && !platform => Some(CameraDrag::Orbit),
        MouseButton::Right if shift => Some(CameraDrag::Pan),
        MouseButton::Right => Some(CameraDrag::Orbit),
        MouseButton::Middle => Some(CameraDrag::Pan),
        _ => None,
    }
}

pub(in crate::editor) fn pinch_dolly(delta: f32) -> f64 {
    f64::from(delta) * 1.2
}

#[cfg(test)]
mod tests {
    use super::{CameraDrag, camera_drag, pinch_dolly};
    use gpui_kit::MouseButton;

    #[test]
    fn navigation_shortcuts_take_precedence_over_selection_and_gizmo_drags() {
        assert_eq!(
            camera_drag(MouseButton::Left, false, false, false, false),
            None
        );
        assert_eq!(
            camera_drag(MouseButton::Left, false, true, false, false),
            Some(CameraDrag::Orbit)
        );
        assert_eq!(
            camera_drag(MouseButton::Left, false, true, true, false),
            Some(CameraDrag::Pan)
        );
        assert_eq!(
            camera_drag(MouseButton::Left, false, true, false, true),
            Some(CameraDrag::Pan)
        );
        assert_eq!(
            camera_drag(MouseButton::Left, false, false, true, false),
            None
        );
        assert_eq!(
            camera_drag(MouseButton::Left, false, false, false, true),
            None
        );
        assert_eq!(
            camera_drag(MouseButton::Right, true, false, false, false),
            Some(CameraDrag::Pan)
        );
        assert_eq!(
            camera_drag(MouseButton::Right, false, false, false, false),
            Some(CameraDrag::Orbit)
        );
        assert_eq!(
            camera_drag(MouseButton::Middle, false, false, false, false),
            Some(CameraDrag::Pan)
        );
    }

    #[test]
    fn spreading_maps_to_dolly_in_and_pinching_maps_to_dolly_out() {
        assert!(pinch_dolly(0.1) > 0.0);
        assert!(pinch_dolly(-0.1) < 0.0);
        assert_eq!(pinch_dolly(0.0), 0.0);
    }
}
