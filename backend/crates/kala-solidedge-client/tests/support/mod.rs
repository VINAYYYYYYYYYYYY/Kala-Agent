//! A tiny in-process mock bridge.
//!
//! The mock is a real loopback HTTP server, so the client's transport, the
//! bearer header, the body caps and the timeouts are exercised for real. It
//! implements the *contract* of `kala_solidedge_bridge.protocol`, not Solid
//! Edge: it never claims a native build, and it is not evidence about
//! anything but this crate.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// One request the mock received.
#[derive(Clone, Debug)]
pub struct RecordedRequest {
    /// The HTTP method.
    pub method: String,
    /// The request target.
    pub path: String,
    /// The headers, with lowercased names.
    pub headers: Vec<(String, String)>,
    /// The request body.
    pub body: Vec<u8>,
}

impl RecordedRequest {
    /// The first value of a header.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// One canned answer.
#[derive(Clone, Debug)]
pub enum CannedResponse {
    /// A normal response.
    Body {
        /// The status code.
        status: u16,
        /// The body.
        body: Vec<u8>,
        /// Extra headers.
        headers: Vec<(String, String)>,
    },
    /// A response that arrives only after a delay.
    Delayed {
        /// The status code.
        status: u16,
        /// The body.
        body: Vec<u8>,
        /// How long the answer is held back.
        delay: Duration,
    },
}

impl CannedResponse {
    /// A JSON response.
    pub fn json(status: u16, body: &str) -> Self {
        Self::Body {
            status,
            body: body.as_bytes().to_vec(),
            headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
        }
    }

    /// A response with extra headers.
    pub fn with_headers(mut self, extra: Vec<(String, String)>) -> Self {
        if let Self::Body { headers, .. } = &mut self {
            headers.extend(extra);
        }
        self
    }
}

/// A loopback HTTP server that answers with canned responses.
pub struct MockBridge {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    responses: Arc<Mutex<VecDeque<CannedResponse>>>,
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockBridge {
    /// Start the server on an ephemeral loopback port.
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let address = listener.local_addr().expect("the local address");
        let requests: Arc<Mutex<Vec<RecordedRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let responses: Arc<Mutex<VecDeque<CannedResponse>>> = Arc::new(Mutex::new(VecDeque::new()));
        let running = Arc::new(AtomicBool::new(true));
        let handle = {
            let requests = Arc::clone(&requests);
            let responses = Arc::clone(&responses);
            let running = Arc::clone(&running);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if !running.load(Ordering::Relaxed) {
                        break;
                    }
                    let Ok(mut stream) = stream else {
                        continue;
                    };
                    // One thread per connection: a slow client must never
                    // block the next one.
                    let requests = Arc::clone(&requests);
                    let responses = Arc::clone(&responses);
                    std::thread::spawn(move || {
                        let recorded = read_request(&mut stream);
                        requests.lock().expect("the request log").push(recorded);
                        let response = responses
                            .lock()
                            .expect("the response queue")
                            .pop_front()
                            .unwrap_or_else(|| {
                                CannedResponse::json(
                                    500,
                                    r#"{"code":"internal_error","message":"no response"}"#,
                                )
                            });
                        write_response(&mut stream, response);
                    });
                }
            })
        };
        Self {
            address,
            requests,
            responses,
            running,
            handle: Some(handle),
        }
    }

    /// The base URL of the mock.
    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// Queue one response.
    pub fn queue(&self, response: CannedResponse) {
        self.responses
            .lock()
            .expect("the response queue")
            .push_back(response);
    }

    /// The requests the mock received.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().expect("the request log").clone()
    }
}

impl Drop for MockBridge {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        // Unblock the accept loop with one throwaway connection.
        let _ = TcpStream::connect(self.address);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> RecordedRequest {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0_u8; 1024];
    let end = loop {
        if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
        match stream.read(&mut chunk) {
            Ok(0) => break buffer.len(),
            Ok(count) => buffer.extend_from_slice(&chunk[..count]),
            Err(_) => break buffer.len(),
        }
    };
    let mut leftover = buffer[end..].to_vec();
    let head = String::from_utf8_lossy(&buffer[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    let length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = std::mem::take(&mut leftover);
    if body.len() > length {
        body.truncate(length);
    }
    while body.len() < length {
        let mut chunk = vec![0_u8; length - body.len()];
        match stream.read_exact(&mut chunk) {
            Ok(()) => body.extend_from_slice(&chunk),
            Err(_) => {
                body.clear();
                break;
            }
        }
    }
    RecordedRequest {
        method,
        path,
        headers,
        body,
    }
}

fn write_response(stream: &mut TcpStream, response: CannedResponse) {
    let (status, body, headers, delay) = match response {
        CannedResponse::Body {
            status,
            body,
            headers,
        } => (status, body, headers, Duration::ZERO),
        CannedResponse::Delayed {
            status,
            body,
            delay,
        } => (status, body, Vec::new(), delay),
    };
    if !delay.is_zero() {
        std::thread::sleep(delay);
    }
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        302 => "Found",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    };
    let declared = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| body.len().to_string());
    let mut head = format!("HTTP/1.1 {status} {reason}\r\n");
    head.push_str("Content-Type: application/json\r\n");
    head.push_str(&format!("Content-Length: {declared}\r\n"));
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body);
    let _ = stream.flush();
}
