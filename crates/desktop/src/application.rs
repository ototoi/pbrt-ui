//! Executable options, core/socket lifetime and desktop startup.
use crate::backend::Backend;
use std::cell::RefCell;
use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;
pub(crate) fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut renderer = None;
    let mut endpoint = None;
    let mut startup = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--renderer-path") => {
                renderer = Some(PathBuf::from(args.next().ok_or("missing renderer path")?))
            }
            Some("--endpoint") => {
                endpoint = Some(PathBuf::from(args.next().ok_or("missing endpoint")?))
            }
            Some("--scene") => {
                startup = Some(crate::backend::Startup::Scene(PathBuf::from(
                    args.next().ok_or("missing scene path")?,
                )))
            }
            Some("--project") => {
                startup = Some(crate::backend::Startup::Project(PathBuf::from(
                    args.next().ok_or("missing project path")?,
                )))
            }
            Some("--help") => {
                println!(
                    "pbrt-ui [--scene FILE.pbrt | --project DIR] [--renderer-path RENDERER] [--endpoint /private/control.sock]"
                );
                return Ok(());
            }
            _ => return Err("unknown desktop argument".into()),
        }
    }
    let mut services = app_core::services::ServiceConfig::discover(renderer)
        .map_err(|e| std::io::Error::other(e.message))?;
    services.gui = true;
    let core = app_core::CoreHost::start_with_services(
        protocol::commands::Limits::default(),
        Some(services),
    );
    let socket = app_core::socket::SocketHost::bind(core.handle(), endpoint)?;
    println!(
        "{}",
        serde_json::json!({"endpoint":socket.endpoint(),"protocol":protocol::PROTOCOL_VERSION})
    );
    let backend = Backend::start(core.handle(), startup);
    let endpoint = socket.endpoint().display().to_string();
    let config = config_directory(|name| std::env::var_os(name)).map(|p| p.join("layout.json"));
    // macOS exits inside NSApplication terminate:, so `run` never returns there.
    // Shut down from the quit hook, or after `run` where it does return; the
    // socket goes first so its file is removed before core stops.
    let shutdown = Rc::new(RefCell::new(Some((socket, core))));
    let on_quit = shutdown.clone();
    crate::editor::run(backend, config, endpoint, move || {
        drop(on_quit.borrow_mut().take())
    });
    drop(shutdown.borrow_mut().take());
    Ok(())
}

/// Per-user settings directory.
/// `XDG_CONFIG_HOME` wins on every OS so tests and development can redirect it;
/// otherwise the platform default: `~/.config`, `~/Library/Application Support`
/// or `%APPDATA%`.
fn config_directory(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let root = match var("XDG_CONFIG_HOME") {
        Some(root) => PathBuf::from(root),
        None if cfg!(target_os = "macos") => {
            PathBuf::from(var("HOME")?).join("Library/Application Support")
        }
        None if cfg!(windows) => PathBuf::from(var("APPDATA")?),
        None => PathBuf::from(var("HOME")?).join(".config"),
    };
    Some(root.join("pbrt-ui"))
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
    fn test_config_directory_prefers_xdg_config_home() {
        let vars = [
            ("XDG_CONFIG_HOME", "/xdg"),
            ("HOME", "/home/u"),
            ("APPDATA", "/appdata"),
        ];
        assert_eq!(
            config_directory(lookup(&vars)),
            Some(PathBuf::from("/xdg/pbrt-ui"))
        );
    }

    #[test]
    fn test_config_directory_falls_back_to_the_platform_default() {
        let vars = [("HOME", "/home/u"), ("APPDATA", "/appdata")];
        let expected = if cfg!(target_os = "macos") {
            "/home/u/Library/Application Support/pbrt-ui"
        } else if cfg!(windows) {
            "/appdata/pbrt-ui"
        } else {
            "/home/u/.config/pbrt-ui"
        };
        assert_eq!(
            config_directory(lookup(&vars)),
            Some(PathBuf::from(expected))
        );
        assert_eq!(config_directory(lookup(&[])), None);
    }
}
