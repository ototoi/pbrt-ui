use core::{CoreHost, socket::SocketHost};
use protocol::commands::Limits;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
static STOP: AtomicBool = AtomicBool::new(false);
#[cfg(unix)]
extern "C" fn stop(_: libc::c_int) {
    STOP.store(true, Ordering::Release);
}
/// Ctrl+C, Ctrl+Break, console close, logoff and shutdown all stop the host.
#[cfg(windows)]
unsafe extern "system" fn stop(_: u32) -> windows_sys::core::BOOL {
    STOP.store(true, Ordering::Release);
    1
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut endpoint = None;
    let mut renderer = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--renderer-path") => {
                renderer = Some(PathBuf::from(args.next().ok_or("missing renderer path")?))
            }
            Some("--endpoint") => {
                endpoint = Some(PathBuf::from(args.next().ok_or("missing endpoint")?))
            }
            Some("--help") => {
                println!(
                    "pbrt-ui-host [--endpoint /private/control.sock] [--renderer-path /path/pbrt-r4]"
                );
                return Ok(());
            }
            _ => return Err("unknown host argument".into()),
        }
    }
    let core = CoreHost::start_with_services(
        Limits::default(),
        Some(
            core::services::ServiceConfig::discover(renderer)
                .map_err(|e| std::io::Error::other(e.message))?,
        ),
    );
    let socket = SocketHost::bind(core.handle(), endpoint)?;
    #[cfg(unix)]
    unsafe {
        if libc::signal(libc::SIGINT, stop as *const () as libc::sighandler_t) == libc::SIG_ERR
            || libc::signal(libc::SIGTERM, stop as *const () as libc::sighandler_t) == libc::SIG_ERR
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    #[cfg(windows)]
    // SAFETY: registers a handler that only stores an atomic.
    if unsafe { windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(stop), 1) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    println!(
        "{}",
        serde_json::json!({"endpoint":socket.endpoint().to_str().ok_or("endpoint is not UTF-8")?,"protocol":protocol::PROTOCOL_VERSION})
    );
    let quit_requested = || {
        let state = core.handle().request(protocol::Request {
            protocol: protocol::PROTOCOL_VERSION.into(),
            request_id: uuid::Uuid::new_v4().to_string(),
            method: protocol::Method::AppGetState,
            params: serde_json::json!({}),
        });
        state
            .result
            .is_some_and(|state| state["quit_requested"] == true)
    };
    while !STOP.load(Ordering::Acquire) && !quit_requested() {
        std::thread::sleep(Duration::from_millis(200));
    }
    drop(socket);
    drop(core);
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("host: {error}");
        std::process::exit(1);
    }
}
