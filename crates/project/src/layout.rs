//! Shared placement rules for `Assets/` items sanitize labels, assign `_N`
//! suffixes, and select the default folder for each kind.
//! Import and resource creation use these so a created item and an
//! imported one land by the same rules; the desktop's ` (N)` display
//! naming is a screen-only rule and deliberately lives on that side.
use crate::ResourceKind;
use std::collections::BTreeSet;

/// Make a label safe as one path component. Characters that cannot appear
/// in an item name fold to underscores, trailing dots go away and empty or
/// dot-only names fall back to `fallback`.
pub fn sanitize(label: &str, fallback: &str) -> String {
    let mut safe = String::new();
    for c in label.chars() {
        if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() {
            safe.push('_');
        } else {
            safe.push(c);
        }
    }
    let safe = safe.trim().trim_end_matches('.').trim().to_owned();
    if safe.is_empty() || safe == "." || safe == ".." {
        fallback.into()
    } else {
        safe
    }
}

/// The stem used when a label sanitizes away to nothing, per extension.
pub fn fallback_stem(extension: &str) -> &'static str {
    match extension {
        "material" => "material",
        "texture" => "texture",
        "medium" => "medium",
        "object" => "object",
        "scene" => "scene",
        "array" => "array",
        "spectrum" => "spectrum",
        _ => "file",
    }
}

/// Hands out unique `folder/<stem>.<extension>` locations, `_N` after the
/// first collision, case-insensitively across everything it handed
/// out before.
#[derive(Default)]
pub struct LocationNamer {
    used: BTreeSet<String>,
}

impl LocationNamer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark an existing location as taken, so `take` numbers around it.
    pub fn reserve(&mut self, location: &str) {
        self.used.insert(location.to_lowercase());
    }

    pub fn take(&mut self, folder: &str, label: &str, extension: &str) -> String {
        let stem = sanitize(label, fallback_stem(extension));
        let prefix = if folder.is_empty() {
            String::new()
        } else {
            format!("{folder}/")
        };
        let mut candidate = format!("{prefix}{stem}.{extension}");
        let mut number = 1;
        while !self.used.insert(candidate.to_lowercase()) {
            candidate = format!("{prefix}{stem}_{number}.{extension}");
            number += 1;
        }
        candidate
    }
}

/// The folder items of a kind live in by default.
pub fn kind_folder(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Material => "Materials",
        ResourceKind::Texture => "Textures",
        ResourceKind::Medium => "Media",
        ResourceKind::ObjectDefinition => "Objects",
        ResourceKind::SpectrumData => "Spectra",
    }
}

/// The file extension a kind's location must carry; the store checks the
/// same on save.
pub fn kind_extension(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Material => "material",
        ResourceKind::Texture => "texture",
        ResourceKind::Medium => "medium",
        ResourceKind::ObjectDefinition => "object",
        ResourceKind::SpectrumData => "spectrum",
    }
}
