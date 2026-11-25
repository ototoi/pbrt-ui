use core::render_progress::ProgressImage;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;
fn create(width: u32, height: u32) -> Vec<u8> {
    let mut message = vec![4, 0];
    message.extend_from_slice(b"render\0");
    for value in [width, height, 3] {
        message.extend_from_slice(&value.to_le_bytes());
    }
    message.extend_from_slice(b"R\0G\0B\0");
    message
}
fn tile(x: u32, values: &[f32]) -> Vec<u8> {
    let mut message = vec![6, 0];
    message.extend_from_slice(b"render\0");
    message.extend_from_slice(&3_u32.to_le_bytes());
    // Deliberately reorder channels to exercise channel-name mapping.
    message.extend_from_slice(b"B\0R\0G\0");
    for value in [x, 0, 1, 1] {
        message.extend_from_slice(&value.to_le_bytes());
    }
    for value in [0_u64, 1, 2, 3, 3, 3] {
        message.extend_from_slice(&value.to_le_bytes());
    }
    for value in values {
        message.extend_from_slice(&value.to_le_bytes());
    }
    message
}
#[test]
fn replacement_tiles_preserve_other_pixels_and_channel_order() {
    let mut image = ProgressImage::default();
    assert!(!image.apply(&create(2, 1)).unwrap());
    assert!(image.apply(&tile(1, &[0.3, 0.1, 0.2])).unwrap());
    assert!(image.apply(&tile(1, &[0.9, 0.7, 0.8])).unwrap());
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("image.rgb32f");
    image.write_snapshot(&path).unwrap();
    let bytes = std::fs::read(path).unwrap();
    let values: Vec<_> = bytes[8..]
        .chunks_exact(4)
        .map(|value| f32::from_le_bytes(value.try_into().unwrap()))
        .collect();
    // The untouched pixel keeps the checker (not the tile's components);
    // pixel 1 holds R, G, B in order despite the reordered channels.
    assert_eq!(values, vec![0.15, 0.15, 0.15, 0.7, 0.8, 0.9]);
}
#[test]
fn invalid_dimensions_tiles_and_truncated_payloads_are_rejected() {
    let mut image = ProgressImage::default();
    assert!(image.apply(&create(8193, 1)).is_err());
    assert!(image.apply(&create(8192, 8192)).is_err());
    image.apply(&create(2, 1)).unwrap();
    assert!(image.apply(&tile(2, &[0.; 3])).is_err());
    let valid = tile(0, &[1.; 3]);
    for length in 0..valid.len() {
        assert!(image.apply(&valid[..length]).is_err());
    }
    assert!(image.apply(&valid).unwrap());
}

/// The greys `write_snapshot` substitutes for pixels no tile reached:
/// linear 0.15 / 0.36 read as roughly 40% / 55% grey through the
/// reinhard + sRGB display pass (`scene_parse::tonemap::reinhard_channel`).
const UNRECEIVED: [f32; 2] = [0.15, 0.36];

/// Frames one display-server message with its four-byte length prefix,
/// the way `receive` reads it off the wire.
fn framed(body: &[u8]) -> Vec<u8> {
    let mut message = ((body.len() + 4) as u32).to_le_bytes().to_vec();
    message.extend_from_slice(body);
    message
}

fn wait_for(published: &Mutex<Vec<f64>>, count: usize) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while published.lock().unwrap().len() < count {
        assert!(
            std::time::Instant::now() < deadline,
            "receive published too few snapshots"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn received_fraction_starts_at_zero_counts_each_pixel_once_and_resets_on_create_image() {
    let mut image = ProgressImage::default();
    image.apply(&create(2, 2)).unwrap();
    assert_eq!(image.received_fraction(), 0.);
    image.apply(&tile(0, &[0.9, 0.7, 0.8])).unwrap();
    assert_eq!(image.received_fraction(), 0.25);
    // An overlapping tile rewrites pixel (0, 0) and must not count twice.
    image.apply(&tile(0, &[0.2, 0.4, 0.6])).unwrap();
    assert_eq!(image.received_fraction(), 0.25);
    image.apply(&tile(1, &[0.9, 0.7, 0.8])).unwrap();
    assert_eq!(image.received_fraction(), 0.5);
    // A new image starts over: the mask resets with the pixels.
    image.apply(&create(1, 1)).unwrap();
    assert_eq!(image.received_fraction(), 0.);
    image.apply(&tile(0, &[1.; 3])).unwrap();
    assert_eq!(image.received_fraction(), 1.);
}

#[test]
fn snapshots_draw_the_checker_only_over_unreceived_pixels() {
    let mut image = ProgressImage::default();
    // 16px checker cells: unreceived column 0 falls in the dark cell,
    // column 17 in the light one; the tiled columns keep their values.
    image.apply(&create(34, 2)).unwrap();
    image.apply(&tile(16, &[0.9, 0.7, 0.8])).unwrap();
    image.apply(&tile(33, &[0.3, 0.1, 0.2])).unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("image.rgb32f");
    image.write_snapshot(&path).unwrap();
    let bytes = std::fs::read(path).unwrap();
    let size = (
        u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
        u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
    );
    assert_eq!(size, (34, 2));
    let values: Vec<_> = bytes[8..]
        .chunks_exact(4)
        .map(|value| f32::from_le_bytes(value.try_into().unwrap()))
        .collect();
    let pixel = |x: usize, y: usize| {
        [
            values[(y * 34 + x) * 3],
            values[(y * 34 + x) * 3 + 1],
            values[(y * 34 + x) * 3 + 2],
        ]
    };
    // Unreceived pixels show the mid-grey checker, not the zero-filled RGB.
    assert_eq!(pixel(0, 0), [UNRECEIVED[0]; 3]);
    assert_eq!(pixel(17, 0), [UNRECEIVED[1]; 3]);
    assert_eq!(pixel(0, 1), [UNRECEIVED[0]; 3]);
    assert_eq!(pixel(17, 1), [UNRECEIVED[1]; 3]);
    // Received pixels keep the tiles' exact values, even on a checker cell.
    assert_eq!(pixel(16, 0), [0.7, 0.8, 0.9]);
    assert_eq!(pixel(33, 0), [0.1, 0.2, 0.3]);
    assert_eq!(image.received_fraction(), 2. / 68.);
}

#[test]
fn receive_publishes_an_empty_snapshot_on_create_image_before_any_tile() {
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let published = Arc::new(Mutex::new(Vec::new()));
    let recorded = published.clone();
    let worker = std::thread::spawn(move || {
        let never = Duration::from_secs(3600);
        core::render_progress::receive(
            listener,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            || false,
            |_| never,
            |image| recorded.lock().unwrap().push(image.received_fraction()),
        )
        .unwrap();
    });
    let mut stream = std::net::TcpStream::connect(address).unwrap();
    let mut send = |body: &[u8]| {
        stream.write_all(&framed(body)).unwrap();
    };
    send(&create(2, 2));
    wait_for(&published, 1);
    assert_eq!(*published.lock().unwrap(), vec![0.]);
    // The interval never elapses, so the tile alone must not publish.
    send(&tile(0, &[0.9, 0.7, 0.8]));
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(published.lock().unwrap().len(), 1);
    // End of stream publishes one final snapshot of the pending tile.
    drop(stream);
    worker.join().unwrap();
    assert_eq!(*published.lock().unwrap(), vec![0., 0.25]);
}
