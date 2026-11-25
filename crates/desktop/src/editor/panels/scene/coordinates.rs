//! Image-space picking coordinates and letterboxing.
use gpui_kit::{Bounds, Pixels, Point};
// GPUI coordinates are logical pixels. Normalize inside the contained image,
// excluding letterbox bars, so DPI and panel docking cannot offset picking.
#[cfg(test)]
fn image_coordinates(bounds: Bounds<Pixels>, point: Point<Pixels>) -> Option<[f64; 2]> {
    image_coordinates_for_size(bounds, point, [1280, 720])
}
pub(super) fn image_coordinates_for_size(
    bounds: Bounds<Pixels>,
    point: Point<Pixels>,
    size: [u32; 2],
) -> Option<[f64; 2]> {
    let width = f64::from(f32::from(bounds.size.width));
    let height = f64::from(f32::from(bounds.size.height));
    let image_width = size[0] as f64;
    let image_height = size[1] as f64;
    let scale = (width / image_width).min(height / image_height);
    if scale <= 0. {
        return None;
    }
    let x = (f64::from(f32::from(point.x - bounds.origin.x)) - (width - image_width * scale) / 2.)
        / (image_width * scale);
    let y = (f64::from(f32::from(point.y - bounds.origin.y))
        - (height - image_height * scale) / 2.)
        / (image_height * scale);
    ((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)).then_some([x, y])
}
/// The contained image's placement: (top-left, screen pixels per image
/// pixel). Gizmo drawing and hit tests work in image pixels.
pub(super) fn image_placement(
    bounds: Bounds<Pixels>,
    size: [u32; 2],
) -> Option<(Point<Pixels>, f64)> {
    let width = f64::from(f32::from(bounds.size.width));
    let height = f64::from(f32::from(bounds.size.height));
    let scale = (width / size[0] as f64).min(height / size[1] as f64);
    (scale.is_finite() && scale > 0.).then(|| {
        let x = (width - size[0] as f64 * scale) / 2.;
        let y = (height - size[1] as f64 * scale) / 2.;
        (
            gpui_kit::point(
                bounds.origin.x + gpui_kit::px(x as f32),
                bounds.origin.y + gpui_kit::px(y as f32),
            ),
            scale,
        )
    })
}
#[cfg(test)]
mod tests {
    use super::image_coordinates;
    use gpui_kit::{Bounds, point, px, size};
    #[test]
    fn picking_excludes_letterbox_and_uses_logical_panel_coordinates() {
        let bounds = Bounds::new(point(px(100.), px(50.)), size(px(800.), px(800.)));
        assert_eq!(
            image_coordinates(bounds, point(px(500.), px(450.))),
            Some([0.5, 0.5])
        );
        assert!(image_coordinates(bounds, point(px(500.), px(70.))).is_none());
        assert_eq!(
            image_coordinates(bounds, point(px(100.), px(225.))),
            Some([0., 0.])
        );
    }
}
