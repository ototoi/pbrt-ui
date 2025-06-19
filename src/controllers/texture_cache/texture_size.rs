use std::fmt::Display;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureSize {
    Icon,
    Large,
    Size((usize, usize)),
}

impl Display for TextureSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TextureSize::Icon => write!(f, "icon"),
            TextureSize::Large => write!(f, "large"),
            TextureSize::Size((width, height)) => write!(f, "{}x{}", width, height),
        }
    }
}
