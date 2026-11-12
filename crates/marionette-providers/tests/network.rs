//! The network transport (cargo feature `net`), checked against a server on this machine
//! only: no outside host is contacted and nothing is paid for.
//! Run with `cargo test -p marionette-providers --features net`.
#![cfg(feature = "net")]

use marionette_providers::http::{HttpRequest, Network, Transport};
use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::time::Duration;

/// Serves one request: returns `status` and echoes method, path, one header and the body.
fn serve_once(status: u16) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut first = String::new();
        reader.read_line(&mut first).unwrap();
        let mut len = 0usize;
        let mut auth = String::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let l = line.trim_end();
            if l.is_empty() {
                break;
            }
            let lower = l.to_ascii_lowercase();
            if let Some(v) = lower.strip_prefix("content-length:") {
                len = v.trim().parse().unwrap();
            }
            if lower.starts_with("authorization:") {
                auth = l["authorization:".len()..].trim().to_string();
            }
        }
        let mut body = vec![0u8; len];
        reader.read_exact(&mut body).unwrap();
        let reply = serde_json::to_vec(&json!({
            "request_line": first.trim_end(),
            "authorization": auth,
            "body": String::from_utf8_lossy(&body),
        }))
        .unwrap();
        let mut stream = stream;
        write!(
            stream,
            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            reply.len()
        )
        .unwrap();
        stream.write_all(&reply).unwrap();
    });
    (format!("http://{addr}"), handle)
}

#[test]
fn network_transport_moves_requests_and_answers_as_data() {
    let net = Network::new(Duration::from_secs(10));
    assert!(net.available());

    let (base, h) = serve_once(200);
    let req = HttpRequest::post_json(format!("{base}/v1/thing"), &json!({"a": 1}))
        .secret_header("authorization", "Bearer secret-for-test");
    let resp = net.send(&req).unwrap();
    h.join().unwrap();
    assert_eq!(resp.status, 200);
    let v: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(v["request_line"], "POST /v1/thing HTTP/1.1");
    assert_eq!(v["authorization"], "Bearer secret-for-test");
    assert_eq!(v["body"], r#"{"a":1}"#);

    // Error statuses come back as data so plug-ins can report the service's message.
    let (base, h) = serve_once(429);
    let resp = net.send(&HttpRequest::get(format!("{base}/slow"))).unwrap();
    h.join().unwrap();
    assert_eq!(resp.status, 429);

    // A refused connection is an error string, not a panic.
    let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    assert!(net.send(&HttpRequest::get(format!("http://{closed}/"))).is_err());
}
