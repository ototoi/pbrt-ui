use std::{ffi::OsString, io, path::PathBuf};

#[derive(Clone, Copy)]
#[allow(dead_code)] // All variants are exercised by the environment resolution tests.
enum Platform {
    Linux,
    MacOs,
    Windows,
}

fn resolve_cache_directory(
    platform: Platform,
    var: impl Fn(&str) -> Option<OsString>,
) -> Option<PathBuf> {
    let root = match platform {
        Platform::Linux => var("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| Some(PathBuf::from(var("HOME")?).join(".cache")))?,
        Platform::MacOs => PathBuf::from(var("HOME")?).join("Library/Caches"),
        Platform::Windows => PathBuf::from(var("LOCALAPPDATA")?),
    };
    Some(root.join("pbrt-ui"))
}

fn resolved_cache_directory(var: impl Fn(&str) -> Option<OsString>) -> PathBuf {
    #[cfg(target_os = "macos")]
    let platform = Platform::MacOs;
    #[cfg(windows)]
    let platform = Platform::Windows;
    #[cfg(not(any(target_os = "macos", windows)))]
    let platform = Platform::Linux;

    // This is only a shortcut for machines without a usable per-user cache path.
    resolve_cache_directory(platform, var).unwrap_or_else(|| std::env::temp_dir().join("pbrt-ui"))
}

/// Return the private per-user application cache directory, creating it as needed.
pub fn cache_directory() -> io::Result<PathBuf> {
    let path = resolved_cache_directory(|name| std::env::var_os(name));
    crate::temp_stage::create_private_dir_all(&path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup<'a>(vars: &'a [(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn cache_directory_uses_platform_environment_rules() {
        assert_eq!(
            resolve_cache_directory(
                Platform::Linux,
                lookup(&[("XDG_CACHE_HOME", "/xdg"), ("HOME", "/home/u")])
            ),
            Some(PathBuf::from("/xdg/pbrt-ui"))
        );
        assert_eq!(
            resolve_cache_directory(Platform::Linux, lookup(&[("HOME", "/home/u")])),
            Some(PathBuf::from("/home/u/.cache/pbrt-ui"))
        );
        assert_eq!(
            resolve_cache_directory(Platform::MacOs, lookup(&[("HOME", "/home/u")])),
            Some(PathBuf::from("/home/u/Library/Caches/pbrt-ui"))
        );
        assert_eq!(
            resolve_cache_directory(
                Platform::Windows,
                lookup(&[("LOCALAPPDATA", "C:/Users/u/Local")])
            ),
            Some(PathBuf::from("C:/Users/u/Local").join("pbrt-ui"))
        );
        assert_eq!(
            resolve_cache_directory(Platform::Windows, lookup(&[])),
            None
        );
    }
}
