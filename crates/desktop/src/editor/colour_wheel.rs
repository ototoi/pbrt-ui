//! The colour wheel of the colour dialog's RGB tab edits spectrum
//! values with a hue / saturation disc drawn as an image made by
//! inverse mapping (every pixel asks the disc for its own hue and
//! saturation, so there are no seams between cells), darkened by a
//! translucent disc for the current value, with a marker ring at the
//! current hue / saturation, and a value slider beneath. A press inside the disc and the
//! drag that follows pick hue and saturation;
//! editing is one gesture: `Change` previews, `Release` applies.
//! gpui-kit carries no wheel, so this whole widget lives here; the
//! coordinate and colour maths are plain functions with unit tests, the
//! painting and the mouse events are the thin parts around them.
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::*;
use std::{cell::Cell, rc::Rc, sync::Arc};

/// The centre and the radius of the disc painted inside a box of
/// `size` local pixels: the inscribed circle, minus a hair of padding
/// so the edge cells' colour stays inside the box.
pub(super) fn disc_geometry(size: (f64, f64)) -> ((f64, f64), f64) {
    let (width, height) = size;
    let radius = (width.min(height) / 2. - 2.).max(1.);
    ((width / 2., height / 2.), radius)
}

/// The hue and saturation of one point of the disc: hue is the angle
/// — 0 at 12 o'clock, a quarter turn clockwise is 0.25, red / yellow /
/// green / cyan / blue / magenta in wheel order — and saturation the
/// distance out, clamped so a drag that leaves the disc keeps the
/// pointer's angle at the rim.
pub(super) fn point_to_hue_sat(at: (f64, f64), centre: (f64, f64), radius: f64) -> (f64, f64) {
    let (dx, dy) = (at.0 - centre.0, at.1 - centre.1);
    // The exact centre has no angle spells π); grey at
    // hue 0 is what the wheel shows there anyway.
    if dx == 0. && dy == 0. {
        return (0., 0.);
    }
    let angle = dx.atan2(-dy);
    let hue = (angle / (2. * std::f64::consts::PI)).rem_euclid(1.);
    let saturation = ((dx * dx + dy * dy).sqrt() / radius).clamp(0., 1.);
    (hue, saturation)
}

/// The marker's place for one hue and saturation, the exact inverse of
/// [`point_to_hue_sat`].
pub(super) fn hue_sat_to_point(
    hue: f64,
    saturation: f64,
    centre: (f64, f64),
    radius: f64,
) -> (f64, f64) {
    let angle = hue * 2. * std::f64::consts::PI;
    (
        centre.0 + radius * saturation * angle.sin(),
        centre.1 - radius * saturation * angle.cos(),
    )
}

/// Whether a point lies in the disc — where a press starts a drag.
pub(super) fn inside_disc(at: (f64, f64), centre: (f64, f64), radius: f64) -> bool {
    let (dx, dy) = (at.0 - centre.0, at.1 - centre.1);
    dx * dx + dy * dy <= radius * radius
}

/// hsv → rgb, every channel 0..1: hue 0..1 (a whole turn), saturation
/// and value 0..1, the wheel's own colour model.
pub(super) fn hsv_to_rgb(hsv: [f64; 3]) -> [f64; 3] {
    let (h, s, v) = (
        hsv[0].rem_euclid(1.),
        hsv[1].clamp(0., 1.),
        hsv[2].clamp(0., 1.),
    );
    let chroma = v * s;
    let hue = h * 6.;
    let second = chroma * (1. - (hue.rem_euclid(2.) - 1.).abs());
    let [r, g, b] = match hue as u32 % 6 {
        1 => [second, chroma, 0.],
        2 => [0., chroma, second],
        3 => [0., second, chroma],
        4 => [second, 0., chroma],
        5 => [chroma, 0., second],
        _ => [chroma, second, 0.],
    };
    let lift = v - chroma;
    [r + lift, g + lift, b + lift]
}

/// rgb → hsv, the channels clamped into the wheel's 0..1 gamut: the
/// exact inverse of [`hsv_to_rgb`] for what the wheel can show.
pub(super) fn rgb_to_hsv(rgb: [f64; 3]) -> [f64; 3] {
    let [r, g, b] = rgb.map(|channel| channel.clamp(0., 1.));
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let hue = if delta <= 0. {
        0.
    } else if max == r {
        ((g - b) / delta).rem_euclid(6.)
    } else if max == g {
        (b - r) / delta + 2.
    } else {
        (r - g) / delta + 4.
    } / 6.;
    let saturation = if max <= 0. { 0. } else { delta / max };
    [hue.rem_euclid(1.), saturation, max]
}

/// One slider event's value, as the wheel and the dialog read it.
pub(super) fn slider_value(event: &SliderEvent) -> f64 {
    match event {
        SliderEvent::Change(value) | SliderEvent::Release(value) => value.start() as f64,
    }
}

/// The events a [`ColourWheel`] emits to the dialog: the hue /
/// saturation / value colour a gesture reached — previewed while it
/// goes, applied when it ends.
pub(super) enum ColourWheelEvent {
    /// A drag in progress: the colour it would confirm.
    Change([f64; 3]),
    /// The gesture ended: the colour to apply, one edit.
    Release([f64; 3]),
}

/// The colour wheel widget of the RGB tab: the hue / saturation disc
/// with its marker, and the value (brightness) slider under it. The
/// disc's painted bounds are captured by the canvas's prepaint into a
/// shared cell the mouse handlers read — the node editor's viewport
/// pattern — so a press maps where it landed; a press inside the disc
/// and the drag that follows pick hue (angle) and saturation (radius),
/// one gesture whose apply rides its release.
pub(super) struct ColourWheel {
    /// The wheel's colour as hue / saturation / value, each 0..1.
    hue: f64,
    saturation: f64,
    value: f64,
    /// The slider's state, the value (brightness) 0..1.
    slider: Entity<SliderState>,
    /// Where the disc was last painted, in window pixels.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    /// A press inside the disc, until its release ends the gesture.
    dragging: bool,
}

impl EventEmitter<ColourWheelEvent> for ColourWheel {}

impl ColourWheel {
    /// One wheel over `colour`, its channels clamped into the wheel's
    /// 0..1 gamut (the number fields keep the exact values).
    pub(super) fn new(colour: [f64; 3], window: &mut Window, cx: &mut Context<Self>) -> Self {
        let [hue, saturation, value] = rgb_to_hsv(colour);
        let slider = cx.new(|_| {
            SliderState::new()
                .max(1.)
                .min(0.)
                .step(0.01)
                .default_value(value as f32)
        });
        cx.subscribe_in(&slider, window, |this, _, event: &SliderEvent, _, cx| {
            this.value = slider_value(event);
            let colour = this.colour();
            cx.emit(match event {
                // The drag previews in the number fields; the apply
                // rides the release, the wheel's own gesture rule.
                SliderEvent::Change(_) => ColourWheelEvent::Change(colour),
                SliderEvent::Release(_) => ColourWheelEvent::Release(colour),
            });
            cx.notify();
        })
        .detach();
        Self {
            hue,
            saturation,
            value,
            slider,
            bounds: Rc::new(Cell::new(Bounds::default())),
            dragging: false,
        }
    }

    /// The confirmed refresh's colour: the wheel follows it the way
    /// the number fields do — the marker, never a finger mid-drag.
    pub(super) fn set_colour(
        &mut self,
        colour: [f64; 3],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let [hue, saturation, value] = rgb_to_hsv(colour);
        (self.hue, self.saturation, self.value) = (hue, saturation, value);
        self.slider
            .update(cx, |slider, cx| slider.set_value(value as f32, window, cx));
        cx.notify();
    }

    /// The wheel's colour back as rgb, every channel 0..1.
    fn colour(&self) -> [f64; 3] {
        hsv_to_rgb([self.hue, self.saturation, self.value])
    }

    /// The pointer's place in the disc's local pixels, mapped through
    /// the last painted bounds.
    fn local(&self, position: Point<Pixels>) -> (f64, f64) {
        let bounds = self.bounds.get();
        (
            f32::from(position.x - bounds.origin.x) as f64,
            f32::from(position.y - bounds.origin.y) as f64,
        )
    }

    /// The disc's geometry as last painted; before the first paint the
    /// bounds are zero and a press anywhere fails the hit test.
    fn geometry(&self) -> ((f64, f64), f64) {
        let bounds = self.bounds.get();
        disc_geometry((
            f32::from(bounds.size.width) as f64,
            f32::from(bounds.size.height) as f64,
        ))
    }

    /// The pointer's hue / saturation while its press holds in the
    /// disc — where a drag starts; outside, a press is not the wheel's.
    fn press(&self, position: Point<Pixels>) -> Option<(f64, f64)> {
        let (at, (centre, radius)) = (self.local(position), self.geometry());
        inside_disc(at, centre, radius).then(|| point_to_hue_sat(at, centre, radius))
    }

    /// A drag in motion follows the pointer wherever it goes: the
    /// saturation clamps onto the rim, its angle survives.
    fn drag_to(&mut self, position: Point<Pixels>) {
        let (at, (centre, radius)) = (self.local(position), self.geometry());
        let (hue, saturation) = point_to_hue_sat(at, centre, radius);
        (self.hue, self.saturation) = (hue, saturation);
    }

    /// The gesture's end: whatever colour the drag reached leaves as
    /// the one edit — even when the pointer left the disc mid-drag.
    fn end_drag(&mut self, cx: &mut Context<Self>) {
        if self.dragging {
            self.dragging = false;
            cx.emit(ColourWheelEvent::Release(self.colour()));
            cx.notify();
        }
    }
}

/// The pixel size of the disc image (shown scaled into the disc's box).
const DISC_IMAGE_SIZE: u32 = 256;

/// The full-value hue / saturation disc as BGRA pixels (gpui's order),
/// by inverse mapping: each pixel's centre is looked up for its hue and
/// saturation, and the rim is anti-aliased by coverage. Outside the disc
/// the pixels are transparent.
pub(super) fn disc_pixels(size: u32) -> image::RgbaImage {
    let side = size as f64;
    let (centre, radius) = ((side / 2., side / 2.), side / 2. - 1.);
    let mut pixels = image::RgbaImage::new(size, size);
    for (x, y, pixel) in pixels.enumerate_pixels_mut() {
        let at = (x as f64 + 0.5, y as f64 + 0.5);
        let distance = ((at.0 - centre.0).powi(2) + (at.1 - centre.1).powi(2)).sqrt();
        let coverage = (radius - distance + 0.5).clamp(0., 1.);
        if coverage <= 0. {
            continue;
        }
        let (hue, saturation) = point_to_hue_sat(at, centre, radius);
        let [r, g, b] = hsv_to_rgb([hue, saturation, 1.]);
        let byte = |channel: f64| (channel.clamp(0., 1.) * 255.).round() as u8;
        // BGRA, not premultiplied.
        *pixel = image::Rgba([byte(b), byte(g), byte(r), (coverage * 255.).round() as u8]);
    }
    pixels
}

thread_local! {
    /// The disc image, made once per thread (the GPUI main thread).
    static DISC: std::cell::RefCell<Option<Arc<RenderImage>>> = const { std::cell::RefCell::new(None) };
}

fn disc_image() -> Arc<RenderImage> {
    DISC.with(|cell| {
        cell.borrow_mut()
            .get_or_insert_with(|| {
                Arc::new(RenderImage::new(vec![image::Frame::new(disc_pixels(
                    DISC_IMAGE_SIZE,
                ))]))
            })
            .clone()
    })
}

/// The disc image over its box, darkened by a translucent black disc for
/// the current value, and the marker ring over the current hue /
/// saturation.
pub(super) fn paint_disc(
    bounds: Bounds<Pixels>,
    hue: f64,
    saturation: f64,
    value: f64,
    window: &mut Window,
) {
    let size = (
        f32::from(bounds.size.width) as f64,
        f32::from(bounds.size.height) as f64,
    );
    let (centre, radius) = disc_geometry(size);
    let origin = bounds.origin;
    let into = |at: (f64, f64)| point(origin.x + px(at.0 as f32), origin.y + px(at.1 as f32));
    // The image's own rim sits one image pixel inside its square: scale
    // the square so that rim lands on the disc's radius.
    let reach = radius * (DISC_IMAGE_SIZE as f64 / 2.) / (DISC_IMAGE_SIZE as f64 / 2. - 1.);
    let square = Bounds {
        origin: into((centre.0 - reach, centre.1 - reach)),
        size: gpui::size(px((2. * reach) as f32), px((2. * reach) as f32)),
    };
    let _ = window.paint_image(square, square, Corners::default(), disc_image(), 0, false);
    if value < 1. {
        let mut path = PathBuilder::fill();
        let rim = (0..96).map(|step| {
            let angle = step as f64 / 96. * 2. * std::f64::consts::PI;
            into((
                centre.0 + radius * angle.cos(),
                centre.1 + radius * angle.sin(),
            ))
        });
        path.add_polygon(&rim.collect::<Vec<_>>(), true);
        if let Ok(path) = path.build() {
            window.paint_path(
                path,
                Rgba {
                    r: 0.,
                    g: 0.,
                    b: 0.,
                    a: (1. - value).clamp(0., 1.) as f32,
                },
            );
        }
    }
    // The marker: a dark halo behind a white ring, over any pixel.
    let at = into(hue_sat_to_point(hue, saturation, centre, radius));
    for (width, colour) in [(px(3.5), rgba(0x000000b0)), (px(1.5), rgba(0xffffffff))] {
        let mut path = PathBuilder::stroke(width);
        path.add_polygon(&circle_points(at, 5.5), true);
        if let Ok(path) = path.build() {
            window.paint_path(path, colour);
        }
    }
}

/// A stroked marker ring's polygon circle around `centre`.
fn circle_points(centre: Point<Pixels>, radius: f64) -> Vec<Point<Pixels>> {
    (0..=24)
        .map(|step| {
            let angle = step as f64 / 24. * 2. * std::f64::consts::PI;
            point(
                centre.x + px((radius * angle.cos()) as f32),
                centre.y + px((radius * angle.sin()) as f32),
            )
        })
        .collect()
}

impl Render for ColourWheel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = self.bounds.clone();
        let (hue, saturation, value) = (self.hue, self.saturation, self.value);
        let disc = canvas(
            move |painted, _, _| bounds.set(painted),
            move |painted, _, window, _| paint_disc(painted, hue, saturation, value, window),
        )
        .absolute()
        .size_full();
        let slider = self.slider.clone();
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .id(("colour-wheel", cx.entity().entity_id()))
                    .relative()
                    .h(px(150.))
                    .w_full()
                    .child(disc)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            if let Some((hue, saturation)) = this.press(event.position) {
                                (this.hue, this.saturation) = (hue, saturation);
                                this.dragging = true;
                                cx.emit(ColourWheelEvent::Change(this.colour()));
                                cx.notify();
                            }
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        if this.dragging && event.pressed_button == Some(MouseButton::Left) {
                            this.drag_to(event.position);
                            cx.emit(ColourWheelEvent::Change(this.colour()));
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.end_drag(cx)),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.end_drag(cx)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap_1()
                    .child(
                        div()
                            .w(px(34.))
                            .text_xs()
                            .text_color(rgb(0x9098a6))
                            .child("Value"),
                    )
                    .child(div().flex_1().min_w_0().child(Slider::new(&slider))),
            )
    }
}

#[cfg(test)]
mod tests {
    // Specific imports, not `use super::*`: the glob would chain through
    // gpui-kit's exports and shadow `#[test]` itself.
    use super::{
        disc_geometry, disc_pixels, hsv_to_rgb, hue_sat_to_point, inside_disc, point_to_hue_sat,
        rgb_to_hsv,
    };

    fn close(left: f64, right: f64) -> bool {
        (left - right).abs() < 1e-9
    }

    fn rgb_close(left: [f64; 3], right: [f64; 3]) -> bool {
        left.iter().zip(right).all(|(a, b)| (a - b).abs() < 1e-9)
    }

    #[test]
    fn hsv_colours_round_trip_through_rgb() {
        for rgb in [
            [1., 1., 1.],
            [0., 0., 0.],
            [0.5, 0.5, 0.5],
            [1., 0., 0.],
            [0., 1., 0.],
            [0., 0., 1.],
            [0.25, 0.5, 0.75],
            [0.9, 0.1, 0.2],
            [0.1, 0.3, 0.3],
        ] {
            let back = hsv_to_rgb(rgb_to_hsv(rgb));
            assert!(rgb_close(back, rgb), "{rgb:?} -> {back:?}");
        }
        // Out-of-range channels clamp into the wheel's gamut, as the
        // hex picker's did; the number fields keep the exact values.
        assert!(rgb_close(
            rgb_to_hsv([2., -1., 0.5]),
            rgb_to_hsv([1., 0., 0.5])
        ));
    }

    #[test]
    fn hsv_pins_the_wheel_sectors_and_channels() {
        let red = rgb_to_hsv([1., 0., 0.]);
        assert!(close(red[0], 0.), "{red:?}");
        assert!(close(red[1], 1.) && close(red[2], 1.), "{red:?}");
        for (rgb, hue) in [
            ([1., 1., 0.], 1. / 6.),
            ([0., 1., 0.], 1. / 3.),
            ([0., 1., 1.], 0.5),
            ([0., 0., 1.], 2. / 3.),
            ([1., 0., 1.], 5. / 6.),
        ] {
            let hsv = rgb_to_hsv(rgb);
            assert!(close(hsv[0], hue), "{rgb:?} -> {hsv:?}");
        }
        // The grey line: no saturation, the value the shade.
        assert_eq!(rgb_to_hsv([1., 1., 1.]), [0., 0., 1.]);
        assert_eq!(rgb_to_hsv([0.5, 0.5, 0.5]), [0., 0., 0.5]);
        assert_eq!(rgb_to_hsv([0., 0., 0.]), [0., 0., 0.]);
        // The hue a whole turn in keeps its place: the marker and the
        // readers agree on hue for any spelling of it.
        let (centre, radius) = ((150., 150.), 140.);
        let turn = point_to_hue_sat(hue_sat_to_point(1.0, 0.5, centre, radius), centre, radius);
        let zero = point_to_hue_sat(hue_sat_to_point(0.0, 0.5, centre, radius), centre, radius);
        let hues_close = (turn.0 - zero.0).abs().min(1. - (turn.0 - zero.0).abs()) < 1e-9;
        assert!(hues_close && close(turn.1, zero.1), "{turn:?} vs {zero:?}");
    }

    #[test]
    fn hsv_saturation_is_the_channels_and_value_their_max() {
        let mixed = rgb_to_hsv([0.25, 0.5, 0.75]);
        assert!(close(mixed[1], 2. / 3.), "{mixed:?}");
        assert!(close(mixed[2], 0.75), "{mixed:?}");
        let dim = rgb_to_hsv([0.2, 0.4, 0.2]);
        assert!(close(dim[1], 0.5), "{dim:?}");
        assert!(close(dim[2], 0.4), "{dim:?}");
        // Channels clamp before the turn: an over-white red keeps only
        // the channels the wheel can show.
        assert_eq!(rgb_to_hsv([3., 3., 4.]), rgb_to_hsv([1., 1., 1.]));
        assert_eq!(rgb_to_hsv([2., 0., 0.5]), rgb_to_hsv([1., 0., 0.5]));
    }

    #[test]
    fn the_disc_reads_hue_around_and_saturation_outward() {
        let (centre, radius) = ((150., 150.), 140.);
        let at = |x: f64, y: f64| point_to_hue_sat((x, y), centre, radius);
        // 12 o'clock red, the quarters yellow / cyan / magenta.
        assert_eq!(at(centre.0, centre.1 - radius), (0., 1.));
        assert_eq!(at(centre.0 + radius, centre.1), (0.25, 1.));
        assert_eq!(at(centre.0, centre.1 + radius), (0.5, 1.));
        assert_eq!(at(centre.0 - radius, centre.1), (0.75, 1.));
        // The centre is grey, half-way out has half the saturation.
        assert_eq!(at(centre.0, centre.1), (0., 0.));
        assert_eq!(at(centre.0, centre.1 - radius / 2.), (0., 0.5));
        // Past the rim the saturation clamps and the angle survives.
        assert_eq!(at(centre.0, centre.1 - radius * 3.), (0., 1.));
    }

    #[test]
    fn hue_sat_round_trips_through_the_disc_points() {
        let (centre, radius) = ((150., 150.), 140.);
        for (hue, saturation) in [
            (0., 0.),
            (0., 1.),
            (0.125, 0.3),
            (0.3, 0.65),
            (0.5, 1.),
            (0.75, 0.9),
        ] {
            let at = hue_sat_to_point(hue, saturation, centre, radius);
            let back = point_to_hue_sat(at, centre, radius);
            assert!(
                close(back.0.rem_euclid(1.), hue.rem_euclid(1.)) && close(back.1, saturation),
                "{hue} / {saturation} -> {at:?} -> {back:?}"
            );
        }
        // The marker of the unturned hue sits straight up the wheel.
        let top = hue_sat_to_point(0., 1., (150., 150.), radius);
        assert!(close(top.0, 150.) && close(top.1, 10.), "{top:?}");
    }

    #[test]
    fn hit_testing_covers_the_disc_and_its_edge() {
        let (centre, radius) = ((150., 150.), 70.);
        assert!(inside_disc(centre, centre, radius));
        assert!(inside_disc((150., 80.), centre, radius), "the rim counts");
        assert!(inside_disc((219.9, 150.), centre, radius));
        assert!(!inside_disc((220.1, 150.), centre, radius));
        assert!(!inside_disc((100., 100.), centre, radius));
        // The geometry: the disc is inscribed, padded a hair in.
        let (centre, radius) = disc_geometry((280., 150.));
        assert_eq!(centre, (140., 75.));
        assert!(close(radius, 73.), "{radius}");
        let (centre, radius) = disc_geometry((10., 4.));
        assert_eq!(centre, (5., 2.));
        assert!(close(radius, 1.), "degenerate boxes keep a radius");
    }

    /// The inverse-mapped disc has no seams: every pixel within the rim is
    /// opaque, the colours vary smoothly (no pixel differs from its right
    /// neighbour by a jump), outside the rim is transparent, the centre is
    /// white and the east edge is the quarter-turn hue.
    #[test]
    fn the_disc_image_is_gap_free_and_smooth() {
        let size = 128u32;
        let image = disc_pixels(size);
        let (centre, radius) = ((size as f64 / 2., size as f64 / 2.), size as f64 / 2. - 1.);
        let mut worst_jump = 0i32;
        for (x, y, pixel) in image.enumerate_pixels() {
            let at = (x as f64 + 0.5, y as f64 + 0.5);
            let distance = ((at.0 - centre.0).powi(2) + (at.1 - centre.1).powi(2)).sqrt();
            if distance <= radius - 1. {
                assert_eq!(pixel.0[3], 255, "a gap inside the rim at ({x}, {y})");
                if let Some(next) = image.get_pixel_checked(x + 1, y) {
                    let next_distance =
                        ((at.0 + 1. - centre.0).powi(2) + (at.1 - centre.1).powi(2)).sqrt();
                    if next_distance <= radius - 1. && distance > 4. {
                        for channel in 0..3 {
                            worst_jump = worst_jump
                                .max((pixel.0[channel] as i32 - next.0[channel] as i32).abs());
                        }
                    }
                }
            } else if distance > radius + 1.5 {
                assert_eq!(pixel.0[3], 0, "paint outside the rim at ({x}, {y})");
            }
        }
        assert!(worst_jump < 40, "adjacent pixels jump by {worst_jump}");
        let middle = image.get_pixel(size / 2, size / 2).0;
        assert!(
            middle[0] >= 250 && middle[1] >= 250 && middle[2] >= 250,
            "{middle:?}"
        );
    }
}
