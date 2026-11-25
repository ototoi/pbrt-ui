//! How paths are spelled on the wire.
use std::io;
use std::path::{Path, PathBuf};

/// `std::fs::canonicalize`, without the `\\?\` prefix Windows adds to a
/// drive path, so the paths the host reports read as users type them.
/// Rust's file APIs re-add the prefix for long paths themselves. UNC
/// shares and components Win32 would reinterpret (trailing dot or space,
/// device names) keep the verbatim form. Containment checks that compare
/// canonical paths should use `std::fs::canonicalize` on both sides.
pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    let path = std::fs::canonicalize(path)?;
    Ok(match path.to_str().and_then(simplified) {
        Some(plain) => PathBuf::from(plain),
        None => path,
    })
}

/// `C:\dir` for `\\?\C:\dir` when every component means the same
/// without the prefix; nothing for anything else.
fn simplified(path: &str) -> Option<&str> {
    let rest = path.strip_prefix(r"\\?\")?;
    let bytes = rest.as_bytes();
    let drive =
        bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\';
    (drive && rest[3..].split('\\').all(plain_component)).then_some(rest)
}

/// A component Win32 resolves unchanged: no trailing dot or space and no
/// reserved device name (`NUL`, `com1.txt`).
fn plain_component(component: &str) -> bool {
    if component.ends_with(['.', ' ']) {
        return false;
    }
    let stem = component.split('.').next().unwrap_or_default().trim_end();
    let stem = stem.to_ascii_uppercase();
    let reserved = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || (stem.len() == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.as_bytes()[3].is_ascii_digit());
    !reserved
}

#[cfg(test)]
mod tests {
    use super::simplified;

    #[test]
    fn only_verbatim_drive_paths_with_plain_components_lose_their_prefix() {
        assert_eq!(
            simplified(r"\\?\C:\Users\me\scene"),
            Some(r"C:\Users\me\scene")
        );
        assert_eq!(simplified(r"\\?\C:\"), Some(r"C:\"));
        // Rust's file APIs re-add the prefix for long paths, so length
        // never decides (containment checks see one spelling).
        let long = format!(r"\\?\C:\{}", "a".repeat(300));
        assert_eq!(simplified(&long), Some(&long[4..]));
        assert_eq!(simplified(r"\\?\UNC\server\share"), None);
        assert_eq!(simplified(r"C:\plain"), None);
        assert_eq!(simplified("/home/me"), None);
        // Win32 would drop the trailing dot or space, or open a device.
        assert_eq!(simplified(r"\\?\C:\scenes\take.\scene.pbrt"), None);
        assert_eq!(simplified(r"\\?\C:\scenes\take \scene.pbrt"), None);
        assert_eq!(simplified(r"\\?\C:\scenes\nul.txt"), None);
        assert_eq!(simplified(r"\\?\C:\scenes\COM1"), None);
        assert_eq!(
            simplified(r"\\?\C:\scenes\é\COMé"),
            Some(r"C:\scenes\é\COMé")
        );
        assert_eq!(
            simplified(r"\\?\C:\scenes\console.pbrt"),
            Some(r"C:\scenes\console.pbrt")
        );
    }
}
