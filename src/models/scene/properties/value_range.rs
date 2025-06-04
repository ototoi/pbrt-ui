#[derive(Debug, Clone, Copy)]
pub enum ValueRange {
    FloatRange(f32, f32),
    IntRange(i32, i32),
}
