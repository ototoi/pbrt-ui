#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TextureSize {
    Icon,
    Large,
    Size((usize, usize)),
}
