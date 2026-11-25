//! Unix-only: these tests check socket file modes and ownership.
#![cfg(unix)]
use core::{CoreHost, socket::SocketHost};
use protocol::commands::Limits;
use protocol::{ErrorCode, Method, Request, Response};
use serde_json::json;
use std::io::BufReader;
use std::os::unix::{fs::PermissionsExt, net::UnixStream};
use std::time::{Duration, Instant};
fn request(method: Method, params: serde_json::Value) -> Request {
    Request {
        protocol: protocol::PROTOCOL_VERSION.into(),
        request_id: uuid::Uuid::new_v4().to_string(),
        method,
        params,
    }
}
#[test]
fn test_runtime_long_poll_does_not_block_other_requests() {
    let host = CoreHost::start(Limits::default());
    let handle = host.handle();
    let pending = handle
        .submit(request(
            Method::EventsRead,
            json!({"after":"0","wait_ms":30000}),
        ))
        .unwrap();
    let started = Instant::now();
    assert!(
        handle
            .request(request(Method::AppGetState, json!({})))
            .error
            .is_none()
    );
    let changed = handle.request(request(
        Method::RenderSetSettings,
        // samples_per_pixel moved into the scene's Sampler component; a
        // threads-only change still notifies the long-poll.
        json!({"settings":{"threads":2,"max_depth":null},"expected_settings_revision":"0"}),
    ));
    assert!(changed.error.is_none(), "{changed:?}");
    assert!(
        !pending
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .result
            .unwrap()["events"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    drop(host);
    assert!(started.elapsed() < Duration::from_secs(2));
}
#[test]
fn test_socket_requires_hello_rejects_duplicate_keys_and_cleans_up() {
    let directory = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let endpoint = directory.path().join("control.sock");
    let host = CoreHost::start(Limits::default());
    let socket = SocketHost::bind(host.handle(), Some(endpoint.clone())).unwrap();
    assert_eq!(
        std::fs::metadata(&endpoint).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(SocketHost::bind(host.handle(), Some(endpoint.clone())).is_err());
    let mut stream = UnixStream::connect(&endpoint).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    protocol::ndjson::write_message(&mut stream, &request(Method::AppGetState, json!({}))).unwrap();
    assert_eq!(
        protocol::ndjson::read_message::<Response>(&mut reader)
            .unwrap()
            .unwrap()
            .error
            .unwrap()
            .code,
        ErrorCode::InvalidRequest
    );
    protocol::ndjson::write_message(
        &mut stream,
        &request(
            Method::AppHello,
            json!({"client_name":"test","supported_protocols":[protocol::PROTOCOL_VERSION]}),
        ),
    )
    .unwrap();
    assert!(
        protocol::ndjson::read_message::<Response>(&mut reader)
            .unwrap()
            .unwrap()
            .error
            .is_none()
    );
    use std::io::Write;
    stream.write_all(b"{\"protocol\":\"1\",\"protocol\":\"1\",\"request_id\":\"r\",\"method\":\"app.get_state\"}\n").unwrap();
    assert_eq!(
        protocol::ndjson::read_message::<Response>(&mut reader)
            .unwrap()
            .unwrap()
            .error
            .unwrap()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(
        protocol::ndjson::read_message::<Response>(&mut reader)
            .unwrap()
            .is_none()
    );
    drop(socket);
    assert!(!endpoint.exists());
}
#[test]
fn test_socket_shutdown_interrupts_idle_and_long_poll_connections() {
    let directory = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let endpoint = directory.path().join("control.sock");
    let host = CoreHost::start(Limits::default());
    let socket = SocketHost::bind(host.handle(), Some(endpoint.clone())).unwrap();
    let _idle = UnixStream::connect(&endpoint).unwrap();
    let mut stream = UnixStream::connect(&endpoint).unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    protocol::ndjson::write_message(
        &mut stream,
        &request(
            Method::AppHello,
            json!({"client_name":"test","supported_protocols":[protocol::PROTOCOL_VERSION]}),
        ),
    )
    .unwrap();
    let _: Response = protocol::ndjson::read_message(&mut reader)
        .unwrap()
        .unwrap();
    protocol::ndjson::write_message(
        &mut stream,
        &request(Method::EventsRead, json!({"after":"0","wait_ms":30000})),
    )
    .unwrap();
    let started = Instant::now();
    drop(socket);
    drop(host);
    assert!(started.elapsed() < Duration::from_secs(2));
}
#[test]
fn test_socket_replaces_a_stale_endpoint_but_not_a_live_one() {
    let directory = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let endpoint = directory.path().join("control.sock");
    // A listener dropped without unlinking leaves the file, like a crashed host.
    drop(std::os::unix::net::UnixListener::bind(&endpoint).unwrap());
    assert!(endpoint.exists());
    let host = CoreHost::start(Limits::default());
    let socket = SocketHost::bind(host.handle(), Some(endpoint.clone())).unwrap();
    assert!(UnixStream::connect(&endpoint).is_ok());
    assert!(SocketHost::bind(host.handle(), Some(endpoint.clone())).is_err());
    drop(socket);
    assert!(!endpoint.exists());
}
#[test]
fn test_socket_rejects_an_endpoint_longer_than_sun_path() {
    let directory = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let endpoint = directory.path().join("x".repeat(120));
    let host = CoreHost::start(Limits::default());
    let error = SocketHost::bind(host.handle(), Some(endpoint))
        .err()
        .unwrap();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("at most"), "{error}");
}
#[test]
fn test_concurrent_hosts_recovering_one_stale_endpoint_leave_one_live_host() {
    let directory = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let endpoint = directory.path().join("control.sock");
    let host = CoreHost::start(Limits::default());
    // The race is intermittent; enough rounds to hit it without the lock.
    for _ in 0..100 {
        drop(std::os::unix::net::UnixListener::bind(&endpoint).unwrap());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let bound: Vec<_> = (0..2)
            .map(|_| {
                let (handle, endpoint, barrier) =
                    (host.handle(), endpoint.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    SocketHost::bind(handle, Some(endpoint)).ok()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|thread| thread.join().unwrap())
            .collect();
        assert_eq!(bound.len(), 1);
        assert!(UnixStream::connect(&endpoint).is_ok());
        drop(bound);
        assert!(!endpoint.exists());
    }
}
/// `app.quit`: the host drops the socket as soon as the state says
/// `quit_requested`, which the core sets while answering the request. The
/// client must still get `{"quitting": true}`, not a closed connection.
#[test]
fn test_socket_delivers_the_quit_response_before_shutting_down() {
    for _ in 0..20 {
        let directory = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let endpoint = directory.path().join("control.sock");
        let host = CoreHost::start(Limits::default());
        let socket = SocketHost::bind(host.handle(), Some(endpoint.clone())).unwrap();
        let mut stream = UnixStream::connect(&endpoint).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        protocol::ndjson::write_message(
            &mut stream,
            &request(
                Method::AppHello,
                json!({"client_name":"test","supported_protocols":[protocol::PROTOCOL_VERSION]}),
            ),
        )
        .unwrap();
        let _: Response = protocol::ndjson::read_message(&mut reader)
            .unwrap()
            .unwrap();
        protocol::ndjson::write_message(&mut stream, &request(Method::AppQuit, json!({}))).unwrap();
        let handle = host.handle();
        let started = Instant::now();
        while handle
            .request(request(Method::AppGetState, json!({})))
            .result
            .unwrap()["quit_requested"]
            != true
        {
            assert!(started.elapsed() < Duration::from_secs(5));
        }
        drop(socket);
        let response: Option<Response> = protocol::ndjson::read_message(&mut reader).unwrap();
        assert_eq!(
            response.and_then(|r| r.result),
            Some(json!({"quitting":true}))
        );
        drop(host);
    }
}
