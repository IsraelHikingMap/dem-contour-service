use std::net::TcpListener;
use std::process::{Child, Command, ExitStatus};
use std::thread::sleep;
use std::time::{Duration, Instant};

const BINARY: &str = env!("CARGO_BIN_EXE_dem-contour-service");

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn healthcheck(port: u16) -> ExitStatus {
    Command::new(BINARY)
        .arg("healthcheck")
        .env("PORT", port.to_string())
        .status()
        .unwrap()
}

fn start_service(port: u16) -> Child {
    let child = Command::new(BINARY)
        .env("PORT", port.to_string())
        .env("DEM_URL_PATTERN", "http://127.0.0.1:1/{z}/{x}/{y}.png")
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "service did not start");
        sleep(Duration::from_millis(50));
    }
    child
}

#[test]
fn healthcheck_succeeds_while_the_service_runs() {
    let port = free_port();
    let mut service = start_service(port);
    let status = healthcheck(port);
    service.kill().unwrap();
    service.wait().unwrap();
    assert!(status.success());
}

#[test]
fn healthcheck_fails_when_the_service_is_down() {
    let port = free_port();
    let status = healthcheck(port);
    assert_eq!(status.code(), Some(1));
}

#[test]
fn healthcheck_fails_when_another_server_answers() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        use std::io::{Read, Write};
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 1024];
        let _ = stream.read(&mut buffer).unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\nconnection: close\r\n\r\nhello")
            .unwrap();
    });
    let status = healthcheck(port);
    server.join().unwrap();
    assert_eq!(status.code(), Some(1));
}
