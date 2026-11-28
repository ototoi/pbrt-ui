//! P0 offscreen verification binary.
//!
//! Renders a known triangle headlessly, reads it back, and asserts:
//!   * a Vulkan (or software) adapter was found,
//!   * readback row padding obeys COPY_BYTES_PER_ROW_ALIGNMENT,
//!   * color channel order is RGBA (red near the top vertex, green/blue at the
//!     bottom corners),
//!   * vertical orientation is top-to-bottom (red at top, not bottom),
//!   * alpha is opaque inside the triangle.
//!
//! Writes a PNG for visual inspection and exits non-zero on any failure.

use image::{ImageBuffer, Rgba};
use scene_view::{pixel, render_offscreen_triangle};

fn main() {
    // 100x100 forces 400-byte unpadded rows -> 512-byte padded rows, exercising
    // the readback unpadding path.
    let width = 100u32;
    let height = 100u32;

    let frame = match render_offscreen_triangle(width, height) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("P0 FAIL: offscreen render error: {e}");
            std::process::exit(1);
        }
    };

    println!("backend = {}", frame.backend);
    println!(
        "padded_bytes_per_row = {} (must be multiple of 256)",
        frame.padded_bytes_per_row
    );

    let mut failures: Vec<String> = Vec::new();

    if frame.padded_bytes_per_row % 256 != 0 {
        failures.push(format!(
            "padded_bytes_per_row {} is not a multiple of 256",
            frame.padded_bytes_per_row
        ));
    }

    // Orientation + channel-order probes.
    let top = pixel(&frame, width / 2, 15);
    let bottom_left = pixel(&frame, 15, 85);
    let bottom_right = pixel(&frame, 85, 85);
    let above_vertex = pixel(&frame, width / 2, 0);

    println!("top-center(50,15)   = {top:?}  (expect red-dominant)");
    println!("bottom-left(15,85)   = {bottom_left:?}  (expect green-dominant)");
    println!("bottom-right(85,85)  = {bottom_right:?}  (expect blue-dominant)");
    println!("top-row(50,0)        = {above_vertex:?}  (expect background/black)");

    let dominant = |p: [u8; 4], want: usize| -> bool {
        let (r, g, b, a) = (p[0] as i32, p[1] as i32, p[2] as i32, p[3] as i32);
        if a < 200 {
            return false;
        }
        let channels = [r, g, b];
        let max = *channels.iter().max().unwrap();
        if max < 60 {
            return false;
        }
        channels[want] == max && channels[want] > channels[(want + 1) % 3]
    };

    if !dominant(top, 0) {
        failures.push(format!("top-center not red-dominant: {top:?}"));
    }
    if !dominant(bottom_left, 1) {
        failures.push(format!("bottom-left not green-dominant: {bottom_left:?}"));
    }
    if !dominant(bottom_right, 2) {
        failures.push(format!("bottom-right not blue-dominant: {bottom_right:?}"));
    }
    // Orientation: the red vertex must be near the top, so the very top row at
    // the horizontal center must be background (near-black), not red.
    if (top[0] as i32) > 60 && (above_vertex[0] as i32) > 60 {
        failures.push(format!(
            "orientation suspect: red appears at both row 15 and row 0 ({top:?}, {above_vertex:?})"
        ));
    }

    // Write PNG for visual inspection.
    let out_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "p0_offscreen.png".to_string());
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_raw(width, height, frame.rgba.clone()).expect("buffer size matches");
    if let Err(e) = img.save(&out_path) {
        failures.push(format!("failed to write PNG {out_path}: {e}"));
    } else {
        println!("wrote {out_path}");
    }

    if failures.is_empty() {
        println!("P0 OFFSCREEN OK");
    } else {
        for f in &failures {
            eprintln!("P0 FAIL: {f}");
        }
        std::process::exit(1);
    }
}
