//! Private AF_UNIX socket service (uds_windows on Windows). All connections submit to the same core queue.
use crate::CoreHandle;
use protocol::{ErrorCode, ErrorPayload, Method, Request, Response};
use serde_json::json;
use std::fs;
use std::io::{self, BufReader};
use std::net::Shutdown;
#[cfg(unix)]
use std::os::unix::{
    fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    net::{UnixListener, UnixStream},
};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
#[cfg(windows)]
use uds_windows::{UnixListener, UnixStream};

pub struct SocketHost {
    endpoint: PathBuf,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    _temporary: Option<tempfile::TempDir>,
}
impl SocketHost {
    pub fn bind(core: CoreHandle, endpoint: Option<PathBuf>) -> io::Result<Self> {
        let state = core.request(Request {
            protocol: protocol::PROTOCOL_VERSION.into(),
            request_id: uuid::Uuid::new_v4().to_string(),
            method: Method::AppGetState,
            params: json!({}),
        });
        let session = state
            .result
            .ok_or_else(|| io::Error::other("core state unavailable"))?["session_id"]
            .as_str()
            .ok_or_else(|| io::Error::other("missing session"))?
            .to_owned();
        let mut temporary = None;
        let endpoint = match endpoint {
            Some(path) => path,
            None => {
                let directory = match std::env::var_os("XDG_RUNTIME_DIR") {
                    Some(root) => PathBuf::from(root).join("pbrt-ui").join(session),
                    None => {
                        let mut builder = tempfile::Builder::new();
                        #[cfg(unix)]
                        builder.permissions(fs::Permissions::from_mode(0o700));
                        let dir = builder.prefix("pui-").tempdir()?;
                        let path = dir.path().to_owned();
                        temporary = Some(dir);
                        path
                    }
                };
                directory.join("control.sock")
            }
        };
        if !endpoint.is_absolute() || endpoint.to_str().is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "endpoint must be an absolute UTF-8 path",
            ));
        }
        // sun_path holds 104 bytes on macOS and 108 on Linux and Windows,
        // NUL included.
        #[cfg(unix)]
        let limit = unsafe { std::mem::zeroed::<libc::sockaddr_un>() }
            .sun_path
            .len()
            - 1;
        #[cfg(windows)]
        let limit = 107;
        if endpoint.as_os_str().len() > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("endpoint path must be at most {limit} bytes"),
            ));
        }
        let parent = endpoint
            .parent()
            .ok_or_else(|| io::Error::other("missing endpoint directory"))?;
        private_directory(parent)?;
        // bind refuses an occupied path; never unlink another live host. The
        // lock keeps stale recovery and cleanup of the same endpoint from
        // interleaving across processes.
        let listener = lock_endpoint(&endpoint).and_then(|_lock| bind_private(&endpoint))?;
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = stopping.clone();
        let thread = thread::spawn(move || serve(listener, core, stop));
        Ok(Self {
            endpoint,
            stopping,
            thread: Some(thread),
            _temporary: temporary,
        })
    }
    pub fn endpoint(&self) -> &Path {
        &self.endpoint
    }
}
impl Drop for SocketHost {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        {
            let _lock = lock_endpoint(&self.endpoint);
            let _ = fs::remove_file(&self.endpoint);
        }
    }
}
/// Create the endpoint's directory, private to this user: mode 0700 and
/// owned by us on Unix. On Windows the default lives in the user's own
/// `%TEMP%`, whose ACL already admits only this user and administrators.
fn private_directory(parent: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
        let metadata = fs::symlink_metadata(parent)?;
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "endpoint directory must be owned by this user with mode 0700",
            ));
        }
    }
    #[cfg(windows)]
    {
        fs::create_dir_all(parent)?;
        if !fs::symlink_metadata(parent)?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "endpoint directory must be a real directory",
            ));
        }
    }
    Ok(())
}
/// Bind `endpoint` (replacing a stale socket) as a private, non-blocking
/// listener. The caller holds the endpoint lock for the whole call.
fn bind_private(endpoint: &Path) -> io::Result<UnixListener> {
    let bound = match UnixListener::bind(endpoint) {
        Err(error) if error.kind() == io::ErrorKind::AddrInUse && is_stale(endpoint) => {
            fs::remove_file(endpoint).and_then(|_| UnixListener::bind(endpoint))
        }
        result => result,
    };
    bound.and_then(|listener| {
        #[cfg(unix)]
        let private = fs::set_permissions(endpoint, fs::Permissions::from_mode(0o600));
        #[cfg(windows)]
        let private = Ok(());
        match private.and_then(|_| listener.set_nonblocking(true)) {
            Ok(()) => Ok(listener),
            Err(error) => {
                let _ = fs::remove_file(endpoint);
                Err(error)
            }
        }
    })
}
/// Exclusive lock on `<endpoint>.lock`, released when the file is dropped.
/// The lock file stays: unlinking it would let two processes lock
/// different inodes.
fn lock_endpoint(endpoint: &Path) -> io::Result<fs::File> {
    let mut path = endpoint.as_os_str().to_owned();
    path.push(".lock");
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options.open(path)?;
    file.lock()?;
    Ok(file)
}
/// A socket file left by a host that exited without cleanup: nobody accepts
/// on it. Only this user's sockets qualify (the directory is already 0700).
fn is_stale(endpoint: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(endpoint) else {
        return false;
    };
    #[cfg(unix)]
    let owned_socket =
        metadata.file_type().is_socket() && metadata.uid() == unsafe { libc::geteuid() };
    // Windows: the socket is a reparse point std cannot classify; the
    // per-user directory already keeps other users out.
    #[cfg(windows)]
    let owned_socket = !metadata.is_dir();
    owned_socket
        && UnixStream::connect(endpoint)
            .is_err_and(|error| error.kind() == io::ErrorKind::ConnectionRefused)
}
struct Connection {
    stream: UnixStream,
    thread: JoinHandle<()>,
}
fn serve(listener: UnixListener, core: CoreHandle, stop: Arc<AtomicBool>) {
    let mut connections: Vec<Connection> = Vec::new();
    while !stop.load(Ordering::Acquire) {
        let mut index = 0;
        while index < connections.len() {
            if connections[index].thread.is_finished() {
                let connection = connections.swap_remove(index);
                let _ = connection.thread.join();
            } else {
                index += 1;
            }
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if connections.len() >= 64 {
                    let _ = stream.shutdown(Shutdown::Both);
                    continue;
                }
                // BSD/macOS accept() inherits the listener's O_NONBLOCK; Linux does not.
                if stream.set_nonblocking(false).is_err() {
                    continue;
                }
                let Ok(owned) = stream.try_clone() else {
                    continue;
                };
                let handle = core.clone();
                let stopping = stop.clone();
                let thread = thread::spawn(move || connection(owned, handle, stopping));
                connections.push(Connection { stream, thread });
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(_) => break,
        }
    }
    // Let connections finish writing an in-flight response (the `app.quit`
    // answer) before idle ones are cut off.
    let grace = Instant::now() + Duration::from_millis(100);
    while Instant::now() < grace && connections.iter().any(|c| !c.thread.is_finished()) {
        thread::sleep(Duration::from_millis(5));
    }
    for connection in &connections {
        let _ = connection.stream.shutdown(Shutdown::Both);
    }
    for connection in connections {
        let _ = connection.thread.join();
    }
}
fn connection(mut stream: UnixStream, core: CoreHandle, stop: Arc<AtomicBool>) {
    let Ok(read_stream) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(read_stream);
    let mut greeted = false;
    while !stop.load(Ordering::Acquire) {
        let request = match protocol::ndjson::read_message::<Request>(&mut reader) {
            Ok(Some(request)) => request,
            Ok(None) => break,
            Err(error) => {
                let response = Response::err(
                    "",
                    ErrorPayload::new(ErrorCode::InvalidRequest, error.to_string()),
                );
                let _ = protocol::ndjson::write_message(&mut stream, &response);
                break;
            }
        };
        let id = request.request_id.clone();
        let hello = request.method == Method::AppHello;
        let response = if !greeted && !hello {
            Response::err(
                id,
                ErrorPayload::new(
                    ErrorCode::InvalidRequest,
                    "app.hello must be the first request",
                ),
            )
        } else {
            match core.submit(request) {
                Err(error) => Response::err(id, error),
                // A response that is already in lands before a stop is
                // honoured, so `app.quit` answers even though it stops the
                // host.
                Ok(receiver) => loop {
                    match receiver.recv_timeout(Duration::from_millis(20)) {
                        Ok(response) => break response,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                            if stop.load(Ordering::Acquire) {
                                return;
                            }
                        }
                        Err(_) => return,
                    }
                },
            }
        };
        if hello && response.error.is_none() {
            greeted = true;
        }
        let written = match protocol::ndjson::write_message(&mut stream, &response) {
            Err(protocol::ndjson::WireError::LineTooLong) => protocol::ndjson::write_message(
                &mut stream,
                &Response::err(
                    response.request_id,
                    ErrorPayload::new(
                        ErrorCode::ResourceExhausted,
                        "response exceeds wire limit; request a smaller page",
                    ),
                ),
            ),
            result => result,
        };
        if written.is_err() {
            break;
        }
    }
}
