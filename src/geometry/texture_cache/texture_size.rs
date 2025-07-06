use std::fmt::Display;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureSize {
    Icon,
    Full,
    Size((usize, usize)),
}

impl Display for TextureSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TextureSize::Icon => write!(f, "icon"),
            TextureSize::Full => write!(f, "full"),
            TextureSize::Size((width, height)) => write!(f, "{}x{}", width, height),
        }
    }
}
