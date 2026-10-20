//! A minimal loopback HTTP/1.1 transport.
//!
//! The bridge is a local service on `127.0.0.1`, so this transport is
//! deliberately small and deliberately unfriendly:
//!
//! * it never follows a redirect, so a misconfigured or hostile service cannot
//!   move the client off loopback;
//! * it sends exactly one `Authorization` header and no `Origin` header,
//!   which is what the bridge requires;
//! * it bounds both the response size and the overall time, and it never
//!   trusts `Content-Length` as proof of size;
//! * it never logs the bearer token.
//!
//! The transport is not a general HTTP client. It speaks to one local bridge.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

/// The chunk size used while reading a response body.
pub(crate) const CHUNK_BYTES: usize = 64 * 1024;

/// The largest response header block accepted.
const MAX_HEADER_BYTES: usize = 64 * 1024;

/// A transport failure.
#[derive(Debug)]
pub(crate) enum TransportError {
    /// The socket timed out, or the overall deadline passed.
    Timeout,
    /// The socket failed.
    Io(String),
    /// The response was not usable HTTP.
    Protocol(String),
    /// The response body was larger than the caller's limit.
    TooLarge,
    /// The caller refused the body it was reading.
    Refused(String),
}

/// One request to the bridge.
pub(crate) struct HttpRequest<'a> {
    /// The HTTP method.
    pub method: &'a str,
    /// The request target, including the path.
    pub target: &'a str,
    /// The `Host` authority the bridge expects.
    pub authority: &'a str,
    /// Where to connect.
    pub address: SocketAddr,
    /// The bearer token.
    pub token: &'a str,
    /// The request body, if any.
    pub body: Option<&'a [u8]>,
    /// The request body content type.
    pub content_type: Option<&'a str>,
    /// The `Accept` header, if any.
    pub accept: Option<&'a str>,
    /// The overall deadline for this request.
    pub timeout: Duration,
}

/// The head of one response, with the body still unread.
pub(crate) struct ResponseHead {
    status: u16,
    declared: Option<u64>,
    leftover: Vec<u8>,
    stream: TcpStream,
    deadline: Instant,
}

impl ResponseHead {
    /// The status code.
    pub(crate) const fn status(&self) -> u16 {
        self.status
    }

    /// The declared body length, if the bridge sent one.
    ///
    /// The length is a hint, never a proof: the body is still capped and
    /// still checked against the bytes that actually arrive.
    pub(crate) const fn declared_length(&self) -> Option<u64> {
        self.declared
    }

    /// Read the body, handing the bytes to `sink`.
    ///
    /// At most `max_body + 1` bytes are read, so an oversize body is
    /// detected without buffering it.
    pub(crate) fn read_body(
        &mut self,
        max_body: usize,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), TransportError>,
    ) -> Result<(), TransportError> {
        let mut total: usize = 0;
        let mut chunk = [0_u8; CHUNK_BYTES];
        let mut pending = std::mem::take(&mut self.leftover);
        loop {
            if let Some(declared) = self.declared {
                if total as u64 >= declared {
                    return Ok(());
                }
            }
            if total > max_body {
                return Ok(());
            }
            if Instant::now() >= self.deadline {
                return Err(TransportError::Timeout);
            }
            let wanted = CHUNK_BYTES.min(max_body + 1 - total);
            let count = if pending.is_empty() {
                read_with_deadline(&mut self.stream, &mut chunk[..wanted], self.deadline)?
            } else {
                let count = pending.len().min(wanted);
                chunk[..count].copy_from_slice(&pending[..count]);
                pending.drain(..count);
                count
            };
            if count == 0 {
                return Ok(());
            }
            total += count;
            sink(&chunk[..count])?;
        }
    }
}

/// Send one request and return the head of the response.
pub(crate) fn send_head(request: &HttpRequest<'_>) -> Result<ResponseHead, TransportError> {
    let deadline = Instant::now() + request.timeout;
    let stream = connect(request, deadline)?;
    let mut stream = stream;
    write_request(&mut stream, request)?;
    read_head(stream, deadline)
}

fn connect(request: &HttpRequest<'_>, deadline: Instant) -> Result<TcpStream, TransportError> {
    let remaining = remaining(deadline)?;
    let stream = TcpStream::connect_timeout(&request.address, remaining).map_err(io_error)?;
    stream.set_nodelay(true).map_err(io_error)?;
    Ok(stream)
}

fn write_request(stream: &mut TcpStream, request: &HttpRequest<'_>) -> Result<(), TransportError> {
    let mut head = String::with_capacity(256);
    head.push_str(request.method);
    head.push(' ');
    head.push_str(request.target);
    head.push_str(" HTTP/1.1\r\n");
    head.push_str("Host: ");
    head.push_str(request.authority);
    head.push_str("\r\n");
    head.push_str("Authorization: Bearer ");
    head.push_str(request.token);
    head.push_str("\r\n");
    if let Some(accept) = request.accept {
        head.push_str("Accept: ");
        head.push_str(accept);
        head.push_str("\r\n");
    }
    if let Some(content_type) = request.content_type {
        head.push_str("Content-Type: ");
        head.push_str(content_type);
        head.push_str("\r\n");
    }
    if let Some(body) = request.body {
        head.push_str("Content-Length: ");
        head.push_str(&body.len().to_string());
        head.push_str("\r\n");
    }
    head.push_str("Connection: close\r\n\r\n");

    let mut payload = head.into_bytes();
    if let Some(body) = request.body {
        payload.extend_from_slice(body);
    }
    stream.write_all(&payload).map_err(io_error)?;
    stream.flush().map_err(io_error)?;
    Ok(())
}

fn read_head(mut stream: TcpStream, deadline: Instant) -> Result<ResponseHead, TransportError> {
    let mut buffer: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 1024];
    let end = loop {
        if let Some(index) = find_header_end(&buffer) {
            break index;
        }
        if buffer.len() > MAX_HEADER_BYTES {
            return Err(TransportError::Protocol(
                "the bridge response headers are too large".to_owned(),
            ));
        }
        let count = read_with_deadline(&mut stream, &mut chunk, deadline)?;
        if count == 0 {
            return Err(TransportError::Protocol(
                "the bridge closed the connection before answering".to_owned(),
            ));
        }
        buffer.extend_from_slice(&chunk[..count]);
    };

    let head = String::from_utf8_lossy(&buffer[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| TransportError::Protocol("the bridge sent no status line".to_owned()))?;
    let mut parts = status_line.split_whitespace();
    let _version = parts.next();
    let code = parts
        .next()
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| TransportError::Protocol("the bridge sent no status code".to_owned()))?;

    let mut headers: Vec<(String, String)> = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
    }
    let declared = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse::<u64>().ok());
    // The body may already have arrived with the headers.
    let leftover = buffer[end..].to_vec();
    Ok(ResponseHead {
        status: code,
        declared,
        leftover,
        stream,
        deadline,
    })
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn read_with_deadline(
    stream: &mut TcpStream,
    buffer: &mut [u8],
    deadline: Instant,
) -> Result<usize, TransportError> {
    let remaining = remaining(deadline)?;
    stream.set_read_timeout(Some(remaining)).map_err(io_error)?;
    match stream.read(buffer) {
        Ok(count) => Ok(count),
        Err(error) if is_timeout(&error) => Err(TransportError::Timeout),
        Err(error) => Err(io_error(error)),
    }
}

fn remaining(deadline: Instant) -> Result<Duration, TransportError> {
    let now = Instant::now();
    if now >= deadline {
        return Err(TransportError::Timeout);
    }
    Ok(deadline - now)
}

fn is_timeout(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

fn io_error(error: std::io::Error) -> TransportError {
    if is_timeout(&error) {
        return TransportError::Timeout;
    }
    TransportError::Io(error.to_string())
}
