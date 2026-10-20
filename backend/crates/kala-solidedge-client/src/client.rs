//! Authenticated loopback HTTP client for the local Solid Edge bridge.
//!
//! Only `http://127.0.0.1[:port]` is accepted. A URL with a userinfo, a
//! query, a fragment, a base path, another host, or another scheme is refused
//! before a socket exists. Redirects are refused rather than followed, so a
//! compromised or misconfigured service cannot move the client off loopback.
//!
//! Response bodies are bounded by reading at most one byte past the cap.
//! Artifacts are streamed to a temporary file in the caller's output
//! directory, hashed while they are written, and only renamed into place
//! after the declared size and SHA-256 both match. `Content-Length` is never
//! trusted as proof of size.
//!
//! The bearer token is read from a private file and never appears in a
//! message, an exception, a URL, or a log line.

use std::io::Write as _;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use sha2::Digest as _;

use crate::auth;
use crate::errors::SolidEdgeClientError;
use crate::http::{HttpRequest, TransportError, send_head};
use crate::protocol::{
    Artifact, BuildAccepted, BuildRequest, BuildStatus, ErrorCode, ErrorResponse, Health,
    PROTOCOL_VERSION,
};

/// The largest JSON document the client reads or writes.
pub const MAX_JSON_BYTES: usize = 1024 * 1024;
/// The largest artifact the client downloads.
pub const MAX_ARTIFACT_BYTES: usize = 256 * 1024 * 1024;
/// The default timeout of one request, in seconds.
pub const DEFAULT_TIMEOUT_SECONDS: f64 = 30.0;
/// The default timeout of one artifact download, in seconds.
pub const DEFAULT_ARTIFACT_TIMEOUT_SECONDS: f64 = 300.0;

const LOOPBACK_HOSTS: [&str; 2] = ["127.0.0.1", "::1"];
const DEFAULT_HTTP_PORT: u16 = 80;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A normalised loopback bridge base URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeUrl {
    /// The normalised base URL, without a trailing slash.
    pub base: String,
    /// The loopback host.
    pub host: String,
    /// The port.
    pub port: u16,
}

impl BridgeUrl {
    /// The `Host` authority the bridge expects.
    ///
    /// The bridge compares the header with its own `host:port`, so the port
    /// is always present, even when the URL did not name one.
    pub fn authority(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    /// The socket address to connect to.
    pub fn address(&self) -> SocketAddr {
        let ip = if self.host == "::1" {
            IpAddr::V6(Ipv6Addr::LOCALHOST)
        } else {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        };
        SocketAddr::new(ip, self.port)
    }
}

/// Return the normalised loopback base URL or refuse it.
///
/// The rules are the ones the Python client enforces: `http` only, no user
/// information, no query or fragment, no base path, and loopback only. A port
/// that is not a number, or one outside `1..=65535`, is refused with a typed
/// error instead of an untyped failure.
pub fn validate_bridge_url(url: &str) -> Result<String, SolidEdgeClientError> {
    Ok(parse_bridge_url(url)?.base)
}

fn parse_bridge_url(url: &str) -> Result<BridgeUrl, SolidEdgeClientError> {
    let scheme_end = url.find(':');
    let scheme = match scheme_end {
        Some(index) if index > 0 && url.starts_with(|c: char| c.is_ascii_alphabetic()) => {
            let candidate = &url[..index];
            if candidate
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
            {
                candidate.to_ascii_lowercase()
            } else {
                String::new()
            }
        }
        _ => String::new(),
    };
    if scheme != "http" {
        return Err(SolidEdgeClientError::client(
            "the bridge URL must use http on loopback; no TLS or other scheme",
        ));
    }
    let rest = match scheme_end {
        Some(index) if !scheme.is_empty() => &url[index + 1..],
        _ => url,
    };
    let (netloc, remainder) = match rest.strip_prefix("//") {
        Some(after) => {
            let end = after.find(['/', '?', '#']).unwrap_or(after.len());
            (&after[..end], &after[end..])
        }
        None => ("", rest),
    };
    if netloc.contains('@') {
        return Err(SolidEdgeClientError::client(
            "the bridge URL must not contain user information",
        ));
    }
    let (before_query, query) = match remainder.split_once('?') {
        Some((left, right)) => (left, Some(right)),
        None => (remainder, None),
    };
    let (path, fragment) = match before_query.split_once('#') {
        Some((left, right)) => (left, Some(right)),
        None => (before_query, None),
    };
    if query.is_some_and(|value| !value.is_empty())
        || fragment.is_some_and(|value| !value.is_empty())
    {
        return Err(SolidEdgeClientError::client(
            "the bridge URL must not contain a query or fragment",
        ));
    }
    if !path.is_empty() && path != "/" {
        return Err(SolidEdgeClientError::client(
            "the bridge URL must not contain a base path",
        ));
    }
    let (host, port_text) = split_authority(netloc)?;
    if !LOOPBACK_HOSTS.contains(&host.as_str()) {
        return Err(SolidEdgeClientError::client(
            "the bridge URL must address loopback (127.0.0.1) only",
        ));
    }
    let port = match port_text {
        None => DEFAULT_HTTP_PORT,
        Some(text) => {
            let parsed = text.parse::<u32>().map_err(|_| {
                SolidEdgeClientError::client("the bridge URL port must be a number")
            })?;
            if !(1..=65_535).contains(&parsed) {
                return Err(SolidEdgeClientError::client(
                    "the bridge URL port is out of range",
                ));
            }
            parsed as u16
        }
    };
    let authority = if host.contains(':') {
        if port_text.is_some() {
            format!("[{host}]:{port}")
        } else {
            format!("[{host}]")
        }
    } else {
        match port_text {
            Some(_) => format!("{host}:{port}"),
            None => host.clone(),
        }
    };
    Ok(BridgeUrl {
        base: format!("http://{authority}"),
        host,
        port,
    })
}

fn split_authority(netloc: &str) -> Result<(String, Option<&str>), SolidEdgeClientError> {
    if let Some(rest) = netloc.strip_prefix('[') {
        let Some(end) = rest.find(']') else {
            return Err(SolidEdgeClientError::client(
                "the bridge URL must address loopback (127.0.0.1) only",
            ));
        };
        let host = rest[..end].to_ascii_lowercase();
        let remainder = &rest[end + 1..];
        let port = match remainder.strip_prefix(':') {
            Some(port) => Some(port),
            None if remainder.is_empty() => None,
            None => {
                return Err(SolidEdgeClientError::client(
                    "the bridge URL must address loopback (127.0.0.1) only",
                ));
            }
        };
        return Ok((host, port));
    }
    match netloc.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => Ok((host.to_ascii_lowercase(), Some(port))),
        _ => Ok((netloc.to_ascii_lowercase(), None)),
    }
}

/// A verified local artifact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Download {
    /// The artifact the bridge declared.
    pub artifact: Artifact,
    /// Where the verified bytes were published.
    pub path: PathBuf,
}

/// A small typed client. The token is read once per client instance.
#[derive(Clone, Debug)]
pub struct BridgeClient {
    base_url: String,
    url: BridgeUrl,
    token: String,
    timeout: Duration,
    artifact_timeout: Duration,
}

impl BridgeClient {
    /// Build a client for one bridge.
    ///
    /// The token is trimmed and never empty. The timeouts must be positive.
    pub fn new(
        base_url: &str,
        token: &str,
        timeout: f64,
        artifact_timeout: f64,
    ) -> Result<Self, SolidEdgeClientError> {
        if token.trim().is_empty() {
            return Err(SolidEdgeClientError::authentication(
                "the bridge token is empty",
            ));
        }
        if !timeout.is_finite()
            || !artifact_timeout.is_finite()
            || timeout <= 0.0
            || artifact_timeout <= 0.0
        {
            return Err(SolidEdgeClientError::client(
                "the bridge timeouts must be positive",
            ));
        }
        let url = parse_bridge_url(base_url)?;
        Ok(Self {
            base_url: url.base.clone(),
            url,
            token: token.trim().to_owned(),
            timeout: seconds(timeout),
            artifact_timeout: seconds(artifact_timeout),
        })
    }

    /// Build a client from the token file the bridge created.
    pub fn from_token_file(
        token_file: &Path,
        base_url: &str,
        timeout: f64,
        artifact_timeout: f64,
    ) -> Result<Self, SolidEdgeClientError> {
        let token = auth::read_token_file(token_file)?;
        Self::new(base_url, &token, timeout, artifact_timeout)
    }

    /// The normalised base URL of the bridge.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The request timeout.
    pub const fn timeout(&self) -> Duration {
        self.timeout
    }

    /// The artifact download timeout.
    pub const fn artifact_timeout(&self) -> Duration {
        self.artifact_timeout
    }

    /// Return the bridge's honest health document.
    ///
    /// The document must speak protocol v1. A different version is refused
    /// before the caller can act on a capability it does not understand.
    pub fn health(&self) -> Result<Health, SolidEdgeClientError> {
        let body = self.request("GET", "/v1/health", None, None, false)?;
        let health = Health::from_json_bytes(&body)
            .map_err(|violation| protocol_violation("health", &violation))?;
        if health.protocol_version != PROTOCOL_VERSION {
            return Err(SolidEdgeClientError::protocol(format!(
                "the bridge health is not protocol v1: protocol_version: Input should be \
                 '{PROTOCOL_VERSION}'"
            )));
        }
        Ok(health)
    }

    /// Submit one build.
    ///
    /// A network failure here is ambiguous: the bridge may have admitted the
    /// job. The caller must keep `request.request_id` and retry the same
    /// semantic request rather than mint a new one.
    pub fn submit(&self, request: &BuildRequest) -> Result<BuildAccepted, SolidEdgeClientError> {
        let payload = request
            .to_wire_json()
            .map_err(|violation| SolidEdgeClientError::client(violation.to_request_message()))?;
        if payload.len() > MAX_JSON_BYTES {
            return Err(SolidEdgeClientError::client(
                "the build request exceeds the 1 MiB JSON limit",
            ));
        }
        let body = self.request(
            "POST",
            "/v1/builds",
            Some(payload.as_bytes()),
            Some("application/json"),
            true,
        )?;
        BuildAccepted::from_json_bytes(&body)
            .map_err(|violation| protocol_violation("build acceptance", &violation))
    }

    /// Return the status of one build.
    pub fn status(&self, build_id: &str) -> Result<BuildStatus, SolidEdgeClientError> {
        let path = format!("/v1/builds/{}", path_id(build_id)?);
        let body = self.request("GET", &path, None, None, false)?;
        BuildStatus::from_json_bytes(&body)
            .map_err(|violation| protocol_violation("build status", &violation))
    }

    /// Ask the bridge to cancel cooperatively between operations.
    ///
    /// This is a request, not a promise: a COM call already in progress is not
    /// proven to have stopped.
    pub fn cancel(&self, build_id: &str) -> Result<BuildStatus, SolidEdgeClientError> {
        let path = format!("/v1/builds/{}/cancel", path_id(build_id)?);
        let body = self.request("POST", &path, Some(&[]), None, false)?;
        BuildStatus::from_json_bytes(&body)
            .map_err(|violation| protocol_violation("cancel status", &violation))
    }

    /// Poll a build with one overall deadline. No unbounded wait.
    pub fn wait(
        &self,
        build_id: &str,
        timeout: f64,
        interval: f64,
    ) -> Result<BuildStatus, SolidEdgeClientError> {
        let start = std::time::Instant::now();
        let mut clock = || start.elapsed().as_secs_f64();
        let mut sleeper = |seconds: f64| {
            if seconds > 0.0 {
                std::thread::sleep(Duration::from_secs_f64(seconds));
            }
        };
        self.wait_with(build_id, timeout, interval, &mut clock, &mut sleeper)
    }

    /// Poll a build with an injected clock and sleeper.
    ///
    /// The deadline is checked after every status call, so the first status
    /// is always requested.
    pub fn wait_with(
        &self,
        build_id: &str,
        timeout: f64,
        interval: f64,
        clock: &mut dyn FnMut() -> f64,
        sleep: &mut dyn FnMut(f64),
    ) -> Result<BuildStatus, SolidEdgeClientError> {
        let deadline = clock() + timeout;
        loop {
            let status = self.status(build_id)?;
            if status.state.is_terminal() {
                return Ok(status);
            }
            if clock() >= deadline {
                return Err(SolidEdgeClientError::network(
                    "the build did not finish before the polling deadline; the job may still \
                     be running, so keep the build id instead of resubmitting",
                ));
            }
            sleep(interval);
        }
    }

    /// Stream one artifact into `output_dir` and verify size and SHA-256.
    ///
    /// The bytes are written to a temporary file in the same directory and are
    /// only published under their final name once both the declared size and
    /// the declared digest match. A mismatch discards the file.
    pub fn download(
        &self,
        build_id: &str,
        artifact: &Artifact,
        output_dir: &Path,
    ) -> Result<Download, SolidEdgeClientError> {
        let name = safe_artifact_name(&artifact.name)?;
        std::fs::create_dir_all(output_dir).map_err(|error| {
            SolidEdgeClientError::artifact(format!(
                "the artifact directory could not be created: {error}"
            ))
        })?;
        let target = output_dir.join(&name);
        let temporary = output_dir.join(format!(
            ".{name}.{}.{}.tmp",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));

        let mut file = std::fs::File::create(&temporary).map_err(|error| {
            SolidEdgeClientError::artifact(format!(
                "the artifact file could not be created: {error}"
            ))
        })?;
        let mut hasher = sha2::Sha256::new();
        let mut written: u64 = 0;
        let mut outcome: Result<(), SolidEdgeClientError> = Ok(());

        let path = format!(
            "/v1/builds/{}/artifacts/{}",
            path_id(build_id)?,
            percent_encode(&name)
        );
        let request = HttpRequest {
            method: "GET",
            target: &path,
            authority: &self.url.authority(),
            address: self.url.address(),
            token: &self.token,
            body: None,
            content_type: None,
            accept: None,
            timeout: self.artifact_timeout,
        };
        let mut sink = |chunk: &[u8]| -> Result<(), TransportError> {
            written += chunk.len() as u64;
            if written as usize > MAX_ARTIFACT_BYTES {
                return Err(TransportError::Refused(format!(
                    "artifact {name} exceeds the 256 MiB client limit; it was discarded"
                )));
            }
            hasher.update(chunk);
            if let Err(error) = file.write_all(chunk) {
                return Err(TransportError::Refused(format!(
                    "the artifact could not be written: {error}"
                )));
            }
            Ok(())
        };
        match send_head(&request) {
            Ok(mut head) => {
                if head
                    .declared_length()
                    .is_some_and(|declared| declared > MAX_ARTIFACT_BYTES as u64)
                {
                    outcome = Err(SolidEdgeClientError::artifact(
                        "the bridge declared an artifact larger than the 256 MiB client limit",
                    ));
                } else if head.status() >= 400 {
                    outcome = Err(self.typed_http_error(head.status(), &[]));
                } else {
                    match head.read_body(MAX_ARTIFACT_BYTES, &mut sink) {
                        Ok(()) => {}
                        Err(error) => outcome = Err(stream_error(&error)),
                    }
                }
            }
            Err(error) => outcome = Err(stream_error(&error)),
        }
        drop(file);
        if let Err(error) = outcome {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
        if written != artifact.size_bytes {
            let _ = std::fs::remove_file(&temporary);
            return Err(SolidEdgeClientError::artifact(format!(
                "artifact {name} is {written} bytes but the bridge declared {}; it was discarded",
                artifact.size_bytes
            )));
        }
        let actual = hex(&hasher.finalize());
        if actual != artifact.sha256 {
            let _ = std::fs::remove_file(&temporary);
            return Err(SolidEdgeClientError::artifact(format!(
                "artifact {name} has digest {actual} but the bridge declared {}; it was discarded",
                artifact.sha256
            )));
        }
        if let Err(error) = std::fs::rename(&temporary, &target) {
            let _ = std::fs::remove_file(&temporary);
            return Err(SolidEdgeClientError::artifact(format!(
                "the artifact could not be published: {error}"
            )));
        }
        Ok(Download {
            artifact: artifact.clone(),
            path: target,
        })
    }

    // ------------------------------------------------------------------ transport

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        content_type: Option<&str>,
        ambiguous: bool,
    ) -> Result<Vec<u8>, SolidEdgeClientError> {
        let request = HttpRequest {
            method,
            target: path,
            authority: &self.url.authority(),
            address: self.url.address(),
            token: &self.token,
            body,
            content_type,
            accept: Some("application/json"),
            timeout: self.timeout,
        };
        let mut collected: Vec<u8> = Vec::new();
        let mut sink = |chunk: &[u8]| -> Result<(), TransportError> {
            collected.extend_from_slice(chunk);
            if collected.len() > MAX_JSON_BYTES {
                return Err(TransportError::TooLarge);
            }
            Ok(())
        };
        let mut head = match send_head(&request) {
            Ok(head) => head,
            Err(error) => return Err(transport_error(&error, ambiguous)),
        };
        let body = head.read_body(MAX_JSON_BYTES, &mut sink);
        if (300..400).contains(&head.status()) {
            return Err(SolidEdgeClientError::protocol(
                "the bridge answered with a redirect, which is not accepted",
            ));
        }
        match body {
            Ok(()) => {
                if head.status() >= 400 {
                    return Err(self.typed_http_error(head.status(), &collected));
                }
                Ok(collected)
            }
            Err(TransportError::TooLarge) => {
                if head.status() >= 400 {
                    // The error document is discarded, exactly like the Python
                    // client does when the error body cannot be read.
                    return Err(self.typed_http_error(head.status(), &[]));
                }
                Err(SolidEdgeClientError::protocol(
                    "the bridge response exceeds the 1 MiB JSON limit",
                ))
            }
            Err(error) => Err(transport_error(&error, ambiguous)),
        }
    }

    fn typed_http_error(&self, status: u16, body: &[u8]) -> SolidEdgeClientError {
        let (code, message) = wire_error(body);
        if status == 401 {
            return SolidEdgeClientError::authentication(
                "the bridge rejected the token; the token file is stale or wrong",
            );
        }
        if status == 403 {
            return SolidEdgeClientError::authentication("the bridge refused this client");
        }
        if status == 409 {
            return SolidEdgeClientError::client(format!(
                "the bridge refused the request as busy or conflicting: {message}"
            ));
        }
        if status == 503 {
            return SolidEdgeClientError::client(format!("the bridge is not ready: {message}"));
        }
        if status == 413 {
            return SolidEdgeClientError::client("the bridge refused the request as too large");
        }
        if status == 404 {
            return SolidEdgeClientError::client(format!(
                "the bridge has no such build or artifact: {message}"
            ));
        }
        if code == Some(ErrorCode::CalibrationRequired) {
            return SolidEdgeClientError::client(format!("the host is not calibrated: {message}"));
        }
        SolidEdgeClientError::client(format!(
            "the bridge refused the request ({status}): {message}"
        ))
    }
}

fn protocol_violation(
    label: &str,
    violation: &crate::protocol::ProtocolViolation,
) -> SolidEdgeClientError {
    SolidEdgeClientError::protocol(format!(
        "the bridge {label} is not protocol v1: {}: {}",
        violation.location, violation.message
    ))
}

fn transport_error(error: &TransportError, ambiguous: bool) -> SolidEdgeClientError {
    match error {
        TransportError::Timeout => {
            if ambiguous {
                SolidEdgeClientError::network(
                    "the submission may or may not have reached the bridge; keep the request \
                     id and retry the same request (timed out)",
                )
            } else {
                SolidEdgeClientError::network("the bridge could not be reached (timed out)")
            }
        }
        TransportError::Io(reason) => {
            if ambiguous {
                SolidEdgeClientError::network(format!(
                    "the submission may or may not have reached the bridge; keep the request \
                     id and retry the same request ({reason})"
                ))
            } else {
                SolidEdgeClientError::network(format!("the bridge could not be reached ({reason})"))
            }
        }
        TransportError::Protocol(reason) => SolidEdgeClientError::protocol(reason.clone()),
        TransportError::TooLarge => {
            SolidEdgeClientError::protocol("the bridge response exceeds the 1 MiB JSON limit")
        }
        TransportError::Refused(reason) => SolidEdgeClientError::artifact(reason.clone()),
    }
}

fn stream_error(error: &TransportError) -> SolidEdgeClientError {
    match error {
        TransportError::Timeout => SolidEdgeClientError::network(
            "the artifact download exceeded the overall timeout; the partial file was discarded",
        ),
        TransportError::Io(reason) => SolidEdgeClientError::network(format!(
            "the artifact download failed ({reason}); the partial file was discarded"
        )),
        TransportError::Protocol(reason) => SolidEdgeClientError::protocol(reason.clone()),
        TransportError::TooLarge => SolidEdgeClientError::artifact(
            "the bridge declared an artifact larger than the 256 MiB client limit",
        ),
        TransportError::Refused(reason) => SolidEdgeClientError::artifact(reason.clone()),
    }
}

fn wire_error(body: &[u8]) -> (Option<ErrorCode>, String) {
    if body.is_empty() {
        return (None, "no error document".to_owned());
    }
    match ErrorResponse::from_json_bytes(body) {
        Ok(error) => (Some(error.code), error.message),
        Err(_) => (None, "the bridge returned an untyped error".to_owned()),
    }
}

fn seconds(value: f64) -> Duration {
    Duration::from_secs_f64(value.clamp(0.000_001, 86_400.0))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Return a build id usable as one path segment.
fn path_id(value: &str) -> Result<String, SolidEdgeClientError> {
    if value.is_empty()
        || value.contains('/')
        || value.contains('\\')
        || value.contains('%')
        || !is_printable(value)
    {
        return Err(SolidEdgeClientError::client(
            "the build id is not a safe path segment",
        ));
    }
    Ok(percent_encode(value))
}

/// Percent-encode a path segment, keeping the characters Python's `quote`
/// keeps with `safe="._:-"`.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        let character = *byte as char;
        if character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-' | '~' | ':') {
            out.push(character);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

/// Reject traversal and separators before a local file is created.
fn safe_artifact_name(name: &str) -> Result<String, SolidEdgeClientError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('%')
        || name.contains(':')
        || !is_printable(name)
    {
        return Err(SolidEdgeClientError::artifact(
            "the bridge supplied an artifact name that is not safe locally",
        ));
    }
    Ok(name.to_owned())
}

/// Whether a string has no control characters.
fn is_printable(value: &str) -> bool {
    !value.chars().any(|character| {
        let point = character as u32;
        point < 0x20
            || (0x7f..=0x9f).contains(&point)
            || matches!(character, '\u{2028}' | '\u{2029}')
    })
}

#[cfg(test)]
mod tests {
    use super::{
        BridgeClient, DEFAULT_ARTIFACT_TIMEOUT_SECONDS, DEFAULT_TIMEOUT_SECONDS,
        validate_bridge_url,
    };

    #[test]
    fn only_a_plain_loopback_base_url_is_accepted() {
        for url in [
            "https://127.0.0.1:8797",
            "http://localhost:8797",
            "http://192.168.1.5:8797",
            "http://user:pass@127.0.0.1:8797",
            "http://127.0.0.1:8797/base",
            "http://127.0.0.1:8797/?a=1",
            "http://127.0.0.1:8797/#frag",
            "ftp://127.0.0.1:8797",
            "127.0.0.1:8797",
        ] {
            assert!(
                validate_bridge_url(url).is_err(),
                "accepted {url} unexpectedly"
            );
        }
    }

    #[test]
    fn a_loopback_url_is_normalised() {
        assert_eq!(
            validate_bridge_url("http://127.0.0.1:8797/").expect("url"),
            "http://127.0.0.1:8797"
        );
        assert_eq!(
            validate_bridge_url("http://127.0.0.1").expect("url"),
            "http://127.0.0.1"
        );
        assert_eq!(
            validate_bridge_url("http://[::1]:8797").expect("url"),
            "http://[::1]:8797"
        );
    }

    #[test]
    fn an_empty_token_is_refused() {
        let error = BridgeClient::new(
            "http://127.0.0.1:8797",
            "  ",
            DEFAULT_TIMEOUT_SECONDS,
            DEFAULT_ARTIFACT_TIMEOUT_SECONDS,
        )
        .expect_err("refused");
        assert!(error.message().contains("empty"), "{}", error.message());
    }

    #[test]
    fn a_non_positive_timeout_is_refused() {
        let error =
            BridgeClient::new("http://127.0.0.1:8797", "token", 0.0, 1.0).expect_err("refused");
        assert!(
            error.message().contains("timeouts must be positive"),
            "{}",
            error.message()
        );
    }

    #[test]
    fn a_missing_token_file_is_an_authentication_error() {
        let error = BridgeClient::from_token_file(
            std::path::Path::new("/nonexistent/bridge.token"),
            "http://127.0.0.1:8797",
            DEFAULT_TIMEOUT_SECONDS,
            DEFAULT_ARTIFACT_TIMEOUT_SECONDS,
        )
        .expect_err("refused");
        assert!(
            error.message().contains("does not exist"),
            "{}",
            error.message()
        );
    }
}
