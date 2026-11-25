//! Bounded receiver for the external renderer's Tev display messages.
//! RGB values are replacement tiles; snapshots cross the adapter seam as
//! files. Pixels no tile has reached publish as a mid-grey checker so a
//! partial render reads as unfinished, not dark.
use std::io::{self, Read};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct ProgressImage {
    name: String,
    size: [usize; 2],
    rgb: Vec<f32>,
    /// Which pixels an UpdateImage has written (any of R/G/B); the count
    /// keeps the received fraction O(1) to read.
    received: Vec<bool>,
    received_count: usize,
}
struct Message<'a>(&'a [u8]);
impl<'a> Message<'a> {
    fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        if count > self.0.len() {
            return Err(invalid());
        }
        let (value, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(value)
    }
    fn number(&mut self) -> io::Result<usize> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()) as usize)
    }
    fn string(&mut self) -> io::Result<&'a str> {
        let length = self
            .0
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(invalid)?;
        let value = std::str::from_utf8(self.take(length)?).map_err(|_| invalid())?;
        self.take(1)?;
        Ok(value)
    }
}
/// Checker cell size in pixels for image area no tile reached yet.
const CHECKER_CELL: usize = 16;
/// Unreceived pixels show these linear levels instead of the zero-filled
/// RGB. Through the display tone map (value / (1 + value), then sRGB) they
/// read as roughly 40% and 55% grey, which reads "obviously unfinished"
/// on a dark render without hiding finished tiles.
const CHECKER_DARK: f32 = 0.15;
const CHECKER_LIGHT: f32 = 0.36;
/// The checker value at one pixel; cells alternate between the two greys.
fn checker(x: usize, y: usize) -> f32 {
    if (x / CHECKER_CELL + y / CHECKER_CELL).is_multiple_of(2) {
        CHECKER_DARK
    } else {
        CHECKER_LIGHT
    }
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid renderer display message",
    )
}
impl ProgressImage {
    /// Accepts one message body (without its four-byte total-length prefix).
    pub fn apply(&mut self, body: &[u8]) -> io::Result<bool> {
        let mut message = Message(body);
        let kind = message.take(1)?[0];
        message.take(1)?; // grab-focus flag
        let name = message.string()?;
        if kind == 4 {
            let width = message.number()?;
            let height = message.number()?;
            let channels = message.number()?;
            if width == 0
                || height == 0
                || width > 8192
                || height > 8192
                || width * height > 16_777_216
                || channels == 0
                || channels > 64
            {
                return Err(invalid());
            }
            for _ in 0..channels {
                message.string()?;
            }
            if !message.0.is_empty() {
                return Err(invalid());
            }
            self.name = name.into();
            self.size = [width, height];
            self.rgb = vec![0.; width * height * 3];
            self.received = vec![false; width * height];
            self.received_count = 0;
            return Ok(false);
        }
        if kind != 6 || name != self.name || self.rgb.is_empty() {
            return Err(invalid());
        }
        let count = message.number()?;
        if count == 0 || count > 64 {
            return Err(invalid());
        }
        let channels = (0..count)
            .map(|_| message.string())
            .collect::<io::Result<Vec<_>>>()?;
        let x = message.number()?;
        let y = message.number()?;
        let width = message.number()?;
        let height = message.number()?;
        if width == 0
            || height == 0
            || width > self.size[0]
            || height > self.size[1]
            || x > self.size[0] - width
            || y > self.size[1] - height
        {
            return Err(invalid());
        }
        let mut offsets = Vec::new();
        let mut strides = Vec::new();
        for _ in 0..count {
            offsets.push(u64::from_le_bytes(message.take(8)?.try_into().unwrap()));
        }
        for _ in 0..count {
            strides.push(u64::from_le_bytes(message.take(8)?.try_into().unwrap()));
        }
        let pixels = width * height;
        let payload = message.0;
        for channel in 0..count {
            let last = strides[channel]
                .checked_mul((pixels - 1) as u64)
                .and_then(|value| value.checked_add(offsets[channel]))
                .and_then(|value| value.checked_add(1))
                .and_then(|value| value.checked_mul(4))
                .ok_or_else(invalid)?;
            if last > payload.len() as u64 {
                return Err(invalid());
            }
        }
        // Validate the whole message before changing any pixel.
        for (channel, name) in channels.iter().enumerate() {
            let component = match *name {
                "R" => 0,
                "G" => 1,
                "B" => 2,
                _ => continue,
            };
            for pixel in 0..pixels {
                let offset = (offsets[channel] + pixel as u64 * strides[channel]) as usize * 4;
                let value = f32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap());
                let target = (y + pixel / width) * self.size[0] + x + pixel % width;
                self.rgb[target * 3 + component] = value;
                if !self.received[target] {
                    self.received[target] = true;
                    self.received_count += 1;
                }
            }
        }
        Ok(true)
    }
    /// The fraction (0..=1) of pixels an UpdateImage has written; 0 for an
    /// untouched (or not yet created) image.
    pub fn received_fraction(&self) -> f64 {
        let total = self.size[0] * self.size[1];
        if total == 0 {
            0.
        } else {
            self.received_count as f64 / total as f64
        }
    }
    pub fn write_snapshot(&self, path: &Path) -> io::Result<()> {
        use std::io::{BufWriter, Write};
        let mut file = BufWriter::new(std::fs::File::create(path)?);
        file.write_all(&(self.size[0] as u32).to_le_bytes())?;
        file.write_all(&(self.size[1] as u32).to_le_bytes())?;
        let [width, _] = self.size;
        for pixel in 0..self.rgb.len() / 3 {
            let received = self.received.get(pixel) == Some(&true);
            for component in 0..3 {
                // Unreceived pixels show the mid-grey checker instead of
                // the zero-filled RGB; received ones pass through as-is.
                let value = if received {
                    self.rgb[pixel * 3 + component]
                } else {
                    checker(pixel % width, pixel / width)
                };
                file.write_all(&value.to_le_bytes())?;
            }
        }
        file.flush()
    }
}

/// Time between two published snapshots of a final render, `elapsed` after
/// the render started: 2 fps for the first 16 s, then stretching so PNG
/// conversion does not keep competing with a long render, up to 10 s.
pub fn publish_interval(elapsed: Duration) -> Duration {
    (elapsed / 32).clamp(Duration::from_millis(500), Duration::from_secs(10))
}

/// Drains the renderer continuously; snapshot publication is sampled at
/// `interval(time since start)` or slower — except a CreateImage, which
/// publishes immediately so the panel has a frame (all checker, pixel
/// fraction 0) before the first tile arrives. The listener is already
/// bound before the renderer starts connecting.
pub fn receive(
    listener: TcpListener,
    stopped: Arc<AtomicBool>,
    cancelled: impl Fn() -> bool,
    interval: impl Fn(Duration) -> Duration,
    mut snapshot: impl FnMut(&ProgressImage),
) -> io::Result<()> {
    listener.set_nonblocking(true)?;
    let mut stream = None;
    let mut bytes = Vec::new();
    let mut image = ProgressImage::default();
    let mut changed = false;
    let start = Instant::now();
    let mut previous = start - Duration::from_secs(10);
    while !stopped.load(Ordering::Acquire) && !cancelled() {
        if stream.is_none() {
            match listener.accept() {
                Ok((connection, _)) => {
                    connection.set_nonblocking(true)?;
                    stream = Some(connection);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error),
            }
        }
        if let Some(connection) = stream.as_mut() {
            let mut buffer = [0; 65536];
            // Keep work per iteration bounded so cancellation cannot starve.
            for _ in 0..64 {
                match connection.read(&mut buffer) {
                    Ok(0) => {
                        if changed {
                            snapshot(&image);
                        }
                        return Ok(());
                    }
                    Ok(length) => bytes.extend_from_slice(&buffer[..length]),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) => return Err(error),
                }
                let mut consumed = 0;
                while bytes.len() - consumed >= 4 {
                    let length =
                        u32::from_le_bytes(bytes[consumed..consumed + 4].try_into().unwrap())
                            as usize;
                    if !(6..=4_194_304).contains(&length) {
                        return Err(invalid());
                    }
                    if bytes.len() - consumed < length {
                        break;
                    }
                    if image.apply(&bytes[consumed + 4..consumed + length])? {
                        changed = true;
                    } else {
                        // CreateImage sized the image: publish the all-
                        // checker frame immediately instead of waiting
                        // for the first finished tile or the interval.
                        snapshot(&image);
                        previous = Instant::now();
                    }
                    consumed += length;
                }
                bytes.drain(..consumed);
            }
        }
        if changed && previous.elapsed() >= interval(start.elapsed()) {
            snapshot(&image);
            previous = Instant::now();
            changed = false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publish_interval_starts_at_two_fps_and_stretches_to_ten_seconds() {
        let interval = |seconds| publish_interval(Duration::from_secs(seconds)).as_secs_f64();
        assert_eq!(interval(0), 0.5);
        assert_eq!(interval(16), 0.5);
        assert_eq!(interval(64), 2.);
        assert_eq!(interval(600), 10.);
    }
}
