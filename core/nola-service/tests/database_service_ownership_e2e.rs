// SPDX-License-Identifier: Apache-2.0
#![cfg(unix)]

use nola_service_client::ApplicationServiceClient;
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Service(Child);
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn spawn(database: &Path, socket: &Path) -> Service {
    let diagnostics = std::fs::File::create(socket.with_extension("stderr")).unwrap();
    Service(
        Command::new(env!("CARGO_BIN_EXE_nola-service"))
            .arg("--database")
            .arg(database)
            .arg("--socket")
            .arg(socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(diagnostics))
            .spawn()
            .unwrap(),
    )
}
fn ready(service: &mut Service, client: &ApplicationServiceClient) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.health().is_err() {
        assert!(
            service.0.try_wait().unwrap().is_none(),
            "service exited before readiness"
        );
        assert!(Instant::now() < deadline, "service readiness timed out");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn incomplete_connections_are_bounded_and_disconnect_restores_service() {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("state.db");
    let socket = root.path().join("service.sock");
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let mut service = spawn(&database, &socket);
    ready(&mut service, &client);
    let mut connections = Vec::new();
    for _ in 0..256 {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut stream = loop {
            match UnixStream::connect(&socket) {
                Ok(stream) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
                    assert!(
                        service.0.try_wait().unwrap().is_none(),
                        "service exited under load: {}",
                        std::fs::read_to_string(socket.with_extension("stderr"))
                            .unwrap_or_default()
                    );
                    assert!(
                        Instant::now() < deadline,
                        "connection queue never recovered"
                    );
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!(
                    "connect under load: {error}; service status: {:?}; stderr: {}",
                    service.0.try_wait().unwrap(),
                    std::fs::read_to_string(socket.with_extension("stderr")).unwrap_or_default()
                ),
            }
        };
        // The bounded server may already have closed an excess connection.
        // Darwin rejects SO_SNDTIMEO on that disconnected Unix socket with
        // EINVAL. Nonblocking I/O observes the actual admission/EOF instead of
        // making the test depend on a socket-option race after connect.
        stream.set_nonblocking(true).unwrap();
        let _ = stream.write_all(b"{");
        thread::sleep(Duration::from_millis(1));
        match stream.read(&mut [0; 1]) {
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                connections.push(stream)
            }
            Ok(0) => (),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
                ) =>
            {
                ()
            }
            result => panic!("unexpected admission response: {result:?}"),
        }
    }
    let mut retained = 0;
    for stream in &mut connections {
        match stream.read(&mut [0; 1]) {
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                retained += 1
            }
            Ok(0) => (),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
                ) =>
            {
                ()
            }
            result => panic!("unexpected incomplete-frame response: {result:?}"),
        }
    }
    assert!(
        retained > 0 && retained <= 128,
        "retained {retained} connection workers"
    );
    drop(connections);
    ready(&mut service, &client);
}

#[test]
fn different_sockets_cannot_create_two_service_writers_for_one_database() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("state.db");
    let first_socket = root.path().join("first.sock");
    let second_socket = root.path().join("second.sock");
    let first_client = ApplicationServiceClient::with_socket(&database, &first_socket);
    let second_client = ApplicationServiceClient::with_socket(&database, &second_socket);
    let mut first = spawn(&database, &first_socket);
    ready(&mut first, &first_client);
    let alias = root.path().join("alias.db");
    std::os::unix::fs::symlink(&database, &alias).unwrap();
    for path in [&database, &alias] {
        let mut competing = spawn(path, &second_socket);
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(status) = competing.0.try_wait().unwrap() {
                assert!(!status.success(), "second writer was accepted");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "second writer did not reject ownership"
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !second_socket.exists(),
            "rejected service must not bind its socket"
        );
        assert!(
            first_client.health().is_ok(),
            "original writer was disrupted"
        );
    }
    drop(first);
    let mut replacement = spawn(&database, &second_socket);
    ready(&mut replacement, &second_client);
}
