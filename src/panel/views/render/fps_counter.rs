use std::time::{Duration, Instant};

pub struct FpsCounter {
    last_frame_time: Instant,
    frame_count: u64,
    fps: f64,
}

impl FpsCounter {
    pub fn new() -> Self {
        Self {
            last_frame_time: Instant::now(),
            frame_count: 0,
            fps: 0.0,
        }
    }

    pub fn update(&mut self) {
        self.frame_count += 1;
        let now = Instant::now();
        let duration = now.duration_since(self.last_frame_time);
        if duration >= Duration::from_secs(1) {
            self.fps = self.frame_count as f64 / duration.as_secs_f64();
            self.frame_count = 0;
            self.last_frame_time = now;
        }
    }

    pub fn get_fps(&self) -> f64 {
        self.fps
    }
}
