//! The real HTTP client against a mock server on the loopback interface.
//!
//! No provider is contacted. These tests prove the properties the destination policy relies on: redirects are
//! not followed, oversized responses are cut off, bodies and headers arrive intact, and a refused connection is
//! reported as "not sent". The mock server is a few lines of `std::net`; it is a mock, not provider evidence.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use imagegen_providers::transport::{send, HttpRequest, HttpTransport, Method, TransportError, UreqTransport};
use imagegen_providers::{ErrorKind, HostPolicy};

struct Mock {
    port: u16,
    seen: Arc<Mutex<Vec<String>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Mock {
    /// Serve the given raw responses to successive connections, then stop.
    fn start(responses: Vec<Vec<u8>>) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        let handle = thread::spawn(move || {
            listener.set_nonblocking(false).unwrap();
            for response in responses {
                let (mut stream, _) = match listener.accept() {
                    Ok(s) => s,
                    Err(_) => return,
                };
                stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
                let request = read_request(&mut stream);
                seen2.lock().unwrap().push(request);
                let _ = stream.write_all(&response);
                let _ = stream.flush();
            }
        });
        Mock { port, seen, handle: Some(handle) }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn finish(mut self) -> Vec<String> {
        // unblock a server still waiting for a connection it will never get
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        self.seen.lock().unwrap().iter().filter(|s| !s.is_empty()).cloned().collect()
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut header_end = None;
    while header_end.is_none() {
        let n = stream.read(&mut chunk).unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        header_end = buf.windows(4).position(|w| w == b"\r\n\r\n");
    }
    let Some(end) = header_end else { return String::from_utf8_lossy(&buf).into_owned() };
    let head = String::from_utf8_lossy(&buf[..end]).to_string();
    let length = head
        .lines()
        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
        .unwrap_or(0);
    while buf.len() < end + 4 + length {
        let n = stream.read(&mut chunk).unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

fn http(status: &str, headers: &[&str], body: &[u8]) -> Vec<u8> {
    let mut out = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
    for h in headers {
        out.push_str(h);
        out.push_str("\r\n");
    }
    out.push_str("\r\n");
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn loopback_policy() -> HostPolicy {
    let mut p = HostPolicy::api(&["127.0.0.1"]);
    p.allow_loopback_http = true;
    p.timeout = Duration::from_secs(10);
    p
}

#[test]
fn post_body_and_headers_arrive_and_the_response_is_read() {
    let mock = Mock::start(vec![http("200 OK", &["Content-Type: application/json", "x-request-id: req_local"], br#"{"ok":true}"#)]);
    let req = HttpRequest::post_json(mock.url("/v1/images/generations"), &serde_json::json!({"prompt": "wall", "n": 1}))
        .header("Authorization", "Bearer sk-local-test");
    let resp = send(&UreqTransport::new(), &loopback_policy(), &req).unwrap();
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body, br#"{"ok":true}"#);
    assert_eq!(resp.header("x-request-id"), Some("req_local"));
    let seen = mock.finish();
    assert_eq!(seen.len(), 1);
    let text = seen[0].to_ascii_lowercase();
    assert!(text.starts_with("post /v1/images/generations http/1.1"), "{text}");
    assert!(text.contains("authorization: bearer sk-local-test"));
    assert!(text.contains("content-type: application/json"));
    assert!(text.ends_with(r#"{"n":1,"prompt":"wall"}"#) || text.ends_with(r#"{"prompt":"wall","n":1}"#), "{text}");
}

#[test]
fn redirects_are_returned_not_followed() {
    let mock = Mock::start(vec![
        http("302 Found", &["Location: http://127.0.0.1:1/never"], b""),
        http("200 OK", &[], b"should not be fetched"),
    ]);
    let resp = send(&UreqTransport::new(), &loopback_policy(), &HttpRequest::get(mock.url("/start"))).unwrap();
    assert_eq!(resp.status, 302, "a redirect must reach the caller as a 3xx, which the adapters treat as an error");
    let seen = mock.finish();
    assert_eq!(seen.len(), 1, "only one request may be made: {seen:?}");
}

#[test]
fn error_statuses_are_responses_not_transport_errors() {
    let mock = Mock::start(vec![http("429 Too Many Requests", &[], br#"{"error":{"message":"slow down"}}"#)]);
    let resp = send(&UreqTransport::new(), &loopback_policy(), &HttpRequest::get(mock.url("/x"))).unwrap();
    assert_eq!(resp.status, 429);
    mock.finish();
}

#[test]
fn oversized_responses_are_cut_off() {
    let big = vec![b'x'; 200_000];
    let mock = Mock::start(vec![http("200 OK", &[], &big)]);
    let mut policy = loopback_policy();
    policy.max_response_bytes = 10_000;
    let err = send(&UreqTransport::new(), &policy, &HttpRequest::get(mock.url("/big"))).unwrap_err();
    assert_eq!(err.kind, ErrorKind::BadImage, "{err}");
    mock.finish();
}

#[test]
fn a_refused_connection_is_reported_as_not_sent() {
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let t = UreqTransport::new();
    let err = t
        .execute(&HttpRequest::get(format!("http://127.0.0.1:{port}/")), 1000, Duration::from_secs(5))
        .unwrap_err();
    assert!(matches!(err, TransportError::NotSent(_)), "{err:?}");
    // through `send`, a POST that was never sent is safe to repeat
    let mut req = HttpRequest::post_json(format!("http://127.0.0.1:{port}/"), &serde_json::json!({}));
    req.method = Method::Post;
    let err = send(&t, &loopback_policy(), &req).unwrap_err();
    assert_eq!(err.retry, imagegen_core::error::RetryClass::SafeSameInputs);
}

#[test]
fn the_policy_blocks_before_a_socket_is_opened() {
    let mock = Mock::start(vec![http("200 OK", &[], b"x")]);
    // the mock is on loopback but the policy does not list it
    let err = send(&UreqTransport::new(), &HostPolicy::api(&["api.openai.com"]), &HttpRequest::get(mock.url("/"))).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Blocked);
    let seen = mock.finish();
    assert!(seen.is_empty(), "nothing may be sent to a blocked destination: {seen:?}");
}
