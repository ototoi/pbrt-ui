use std::fmt::Display;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureCacheSize {
    Icon,
    Full,
    Size((usize, usize)),
}

impl Display for TextureCacheSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TextureCacheSize::Icon => write!(f, "icon"),
            TextureCacheSize::Full => write!(f, "full"),
            TextureCacheSize::Size((width, height)) => write!(f, "{}x{}", width, height),
        }
    }
}
