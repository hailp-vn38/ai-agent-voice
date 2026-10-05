use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    thread::{self, JoinHandle},
    time::Duration,
};
use voice_agent_server::{
    assets::HttpAssetAcquirer,
    providers::assets::{AssetAcquirer, AssetError},
};

fn target() -> PathBuf {
    std::env::temp_dir().join(format!("voice-download-{}.part", uuid::Uuid::new_v4()))
}

fn server(responses: Vec<Vec<u8>>) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/model", listener.local_addr().unwrap());
    let task = thread::spawn(move || {
        for response in responses {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut byte = [0_u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
                assert!(request.len() <= 16 * 1024);
            }
            socket.write_all(&response).unwrap();
        }
    });
    (url, task)
}

fn response(status: &str, body: &[u8], declared_length: usize) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {declared_length}\r\nConnection: close\r\n\r\n"
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}

#[test]
fn interrupted_stream_retries_from_zero_without_appending_partial_bytes() {
    let body = vec![42_u8; 256 * 1024];
    let (url, server) = server(vec![
        response("200 OK", b"partial", body.len()),
        response("200 OK", &body, body.len()),
    ]);
    let target = target();
    HttpAssetAcquirer.acquire(&url, &target).unwrap();
    server.join().unwrap();
    assert_eq!(fs::read(&target).unwrap(), body);
    fs::remove_file(target).unwrap();
}

#[test]
fn transient_server_error_is_retried() {
    let (url, server) = server(vec![
        response("503 Service Unavailable", b"", 0),
        response("200 OK", b"model", 5),
    ]);
    let target = target();
    HttpAssetAcquirer.acquire(&url, &target).unwrap();
    server.join().unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"model");
    fs::remove_file(target).unwrap();
}

#[test]
fn not_found_is_not_retried_and_leaves_no_partial_file() {
    let (url, server) = server(vec![response("404 Not Found", b"", 0)]);
    let target = target();
    fs::write(&target, b"stale partial").unwrap();
    assert!(matches!(
        HttpAssetAcquirer.acquire(&url, &target),
        Err(AssetError::Download(_))
    ));
    server.join().unwrap();
    assert!(!target.exists());
}
