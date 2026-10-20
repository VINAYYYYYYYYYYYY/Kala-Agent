//! The request boundary of the local Kala HTTP adapter.
//!
//! Three controls live here and nothing else: the loopback Host guard that
//! refuses a rebound name, the per-launch bearer token, and the cross-origin
//! guard that refuses every browser-originated request. None of them reaches
//! the network, inspects a payload for a business rule, or writes a row.
//!
//! The launch token is minted once per launch, stored in the local data
//! directory with owner-only permissions, and required on every mutating
//! method. It is never logged, never returned in a response, and never echoed
//! in an error, and a refusal says only that a valid token is required. Reads
//! stay open to the caller that already has the data directory, which is the
//! trust model this process has always claimed: one local operator, no
//! multi-user isolation.

use std::fs;
use std::io::{self, Read, Write};
use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::path::Path;

use crate::middleware::{header_value, header_values};

/// The name of the file the launch token is stored in.
pub const API_TOKEN_FILE_NAME: &str = "api-token";

/// The number of random bytes behind a launch token.
pub const API_TOKEN_BYTES: usize = 32;

/// The owner-only mode of the stored launch token.
pub const API_TOKEN_FILE_MODE: u32 = 0o600;

/// The smallest usable launch token length.
pub const MIN_API_TOKEN_LENGTH: usize = 16;

/// The largest usable launch token length.
pub const MAX_API_TOKEN_LENGTH: usize = 128;

/// The characters a launch token may contain.
pub const API_TOKEN_PATTERN: &str = "^[A-Za-z0-9._~-]+$";

/// The methods that need the launch token.
pub const MUTATING_METHODS: [&str; 4] = ["DELETE", "PATCH", "POST", "PUT"];

/// The authorization scheme the launch token is presented with.
pub const AUTH_SCHEME: &str = "bearer";

/// The challenge a refused mutating request carries.
pub const AUTH_CHALLENGE: (&str, &str) = ("www-authenticate", "Bearer realm=\"kala-agent\"");

/// The largest authority the Host guard will even look at.
pub const MAX_AUTHORITY_LENGTH: usize = 256;

/// The names that name the loopback interface directly.
pub const LOOPBACK_HOSTNAMES: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// The names that name this machine without naming the interface.
pub const WILDCARD_AUTHORITIES: [&str; 6] =
    ["0.0.0.0", "::", "[::]", "[::0]", "0:0:0:0:0:0:0:0", "*"];

/// The message a refused mutating request carries.
pub const TOKEN_REQUIRED_MESSAGE: &str = "a valid bearer token is required for this request";

/// The message a refused Host header carries.
pub const HOST_REFUSED_MESSAGE: &str = "the request host is not a loopback address of this process";

/// The message a refused cross-origin request carries.
pub const ORIGIN_REFUSED_MESSAGE: &str = "this api serves no cross-origin client";

/// One refusal produced by the request boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// The HTTP status the refusal answers with.
    pub status: u16,
    /// The stable machine readable code.
    pub code: &'static str,
    /// The bounded human readable message.
    pub message: String,
    /// The sanitized details.
    pub details: Vec<String>,
    /// The extra headers the refusal carries, such as the challenge.
    pub extra_headers: Vec<(&'static str, String)>,
}

impl Refusal {
    /// A refusal without extra headers.
    #[must_use]
    pub fn new(status: u16, code: &'static str, message: &str) -> Self {
        Self {
            status,
            code,
            message: message.to_owned(),
            details: Vec::new(),
            extra_headers: Vec::new(),
        }
    }

    /// A refusal with one sanitized detail.
    #[must_use]
    pub fn with_detail(mut self, detail: &str) -> Self {
        self.details.push(detail.to_owned());
        self
    }

    /// A refusal with one extra header.
    #[must_use]
    pub fn with_header(mut self, name: &'static str, value: &str) -> Self {
        self.extra_headers.push((name, value.to_owned()));
        self
    }
}

/// An IPv4 or IPv6 address, classified the way CPython's `ipaddress` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Classified {
    /// An IPv4 address.
    V4(Ipv4Addr),
    /// An IPv6 address.
    V6(Ipv6Addr),
}

impl Classified {
    /// Whether the address is a loopback address.
    fn is_loopback(self) -> bool {
        match self {
            Self::V4(address) => address.octets()[0] == 127,
            Self::V6(address) => match ipv4_mapped(&address) {
                Some(mapped) => mapped.octets()[0] == 127,
                None => address == Ipv6Addr::LOCALHOST,
            },
        }
    }

    /// Whether the address is the unspecified address.
    fn is_unspecified(self) -> bool {
        match self {
            Self::V4(address) => address.is_unspecified(),
            Self::V6(address) => match ipv4_mapped(&address) {
                Some(mapped) => mapped.is_unspecified(),
                None => address.is_unspecified(),
            },
        }
    }
}

/// The IPv4 address an IPv4-mapped IPv6 address carries.
fn ipv4_mapped(address: &Ipv6Addr) -> Option<Ipv4Addr> {
    let segments = address.segments();
    if segments[0] == 0
        && segments[1] == 0
        && segments[2] == 0
        && segments[3] == 0
        && segments[4] == 0
        && segments[5] == 0xffff
    {
        Some(Ipv4Addr::new(
            (segments[6] >> 8) as u8,
            (segments[6] & 0xff) as u8,
            (segments[7] >> 8) as u8,
            (segments[7] & 0xff) as u8,
        ))
    } else {
        None
    }
}

/// Parse an address exactly as CPython's `ipaddress.ip_address` does.
///
/// CPython accepts an optional scope identifier on an IPv6 address, rejects
/// an IPv4 address with fewer or more than four octets, and rejects an octet
/// with a leading zero.
fn parse_address(text: &str) -> Option<Classified> {
    if let Ok(address) = text.parse::<Ipv4Addr>() {
        return Some(Classified::V4(address));
    }
    let (address, scope) = match text.split_once('%') {
        Some((address, scope)) => {
            if scope.is_empty() || scope.contains('%') {
                return None;
            }
            (address, Some(scope))
        }
        None => (text, None),
    };
    let _ = scope;
    address.parse::<Ipv6Addr>().ok().map(Classified::V6)
}

/// Return whether a bare hostname or address names the loopback interface.
///
/// A wildcard name is not loopback. `0.0.0.0` and `::` name this machine
/// without naming the interface a request arrived on, so they are refused
/// explicitly rather than by accident, and a name that is not a loopback
/// address at all is refused as well.
#[must_use]
pub fn is_loopback_host(host: &str) -> bool {
    let candidate = crate::pytext::lower(host);
    if candidate.is_empty() || candidate.chars().count() > MAX_AUTHORITY_LENGTH {
        return false;
    }
    if WILDCARD_AUTHORITIES.contains(&candidate.as_str()) {
        return false;
    }
    if LOOPBACK_HOSTNAMES.contains(&candidate.as_str()) {
        return true;
    }
    match parse_address(&candidate) {
        Some(address) => address.is_loopback() && !address.is_unspecified(),
        None => false,
    }
}

/// Split an HTTP authority into its host and its optional port.
///
/// # Errors
///
/// Returns `None` for an authority that is empty, too long, carries
/// whitespace, is not bracketed correctly, carries a userinfo section, or
/// carries a port that is not a number between 1 and 65535.
#[must_use]
pub fn split_authority(authority: &str) -> Option<(String, Option<String>)> {
    if authority.is_empty() || authority.chars().count() > MAX_AUTHORITY_LENGTH {
        return None;
    }
    if authority.chars().any(crate::pytext::is_space) {
        return None;
    }
    let mut port: Option<String> = None;
    let host: String;
    if let Some(rest) = authority.strip_prefix('[') {
        let characters: Vec<char> = rest.chars().collect();
        let closing = characters.iter().position(|character| *character == ']')?;
        host = characters[..closing].iter().collect();
        let remainder: String = characters[closing + 1..].iter().collect();
        if !remainder.is_empty() {
            let port_text = remainder.strip_prefix(':')?;
            port = Some(port_text.to_owned());
        }
    } else if authority.contains(':') {
        let (before, after) = authority.rsplit_once(':')?;
        if before.contains(':') {
            return None;
        }
        host = before.to_owned();
        port = Some(after.to_owned());
    } else {
        host = authority.to_owned();
    }
    if host.is_empty() {
        return None;
    }
    if let Some(port_text) = &port {
        if port_text.is_empty()
            || !port_text.bytes().all(|byte| byte.is_ascii_digit())
            || port_text
                .parse::<u64>()
                .map_or(true, |value| !(1..=65_535).contains(&value))
        {
            return None;
        }
    }
    Some((host, port))
}

/// Return whether a Host header names this process on the loopback interface.
///
/// The optional port is accepted and ignored, because the interface is what
/// the trust model is about. Everything else is refused: a userinfo section, a
/// path, a query, a fragment, a non-numeric or out-of-range port, an empty
/// authority, a trailing dot, and any name that is not loopback.
#[must_use]
pub fn is_permitted_authority(authority: &str) -> bool {
    match split_authority(authority) {
        Some((host, _port)) => is_loopback_host(&host),
        None => false,
    }
}

/// Return a usable bearer token, or refuse one that is malformed.
///
/// # Errors
///
/// Returns a sanitized message for a wrong-length or unsupported-character
/// token. The submitted value is never included.
pub fn validate_api_token(value: &str) -> Result<String, String> {
    let length = value.chars().count();
    if !(MIN_API_TOKEN_LENGTH..=MAX_API_TOKEN_LENGTH).contains(&length) {
        return Err(format!(
            "the api token must be between {MIN_API_TOKEN_LENGTH} and {MAX_API_TOKEN_LENGTH} characters"
        ));
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "._~-".contains(character))
    {
        return Err("the api token contains unsupported characters".to_owned());
    }
    Ok(value.to_owned())
}

/// Whether the method needs the launch token.
#[must_use]
pub fn is_mutating_method(method: &str) -> bool {
    MUTATING_METHODS
        .iter()
        .any(|candidate| method.eq_ignore_ascii_case(candidate))
}

/// The credential presented on the authorization header, if there is one.
///
/// A header that appears more than once, that names another scheme, or that
/// carries an empty or over-long credential presents nothing.
#[must_use]
pub fn presented_credential(headers: &[(Vec<u8>, Vec<u8>)]) -> Option<Vec<u8>> {
    let values = header_values(headers, "authorization");
    if values.len() != 1 {
        return None;
    }
    let (scheme, credential) = values[0].split_once(' ')?;
    if crate::pytext::lower(scheme) != AUTH_SCHEME {
        return None;
    }
    let token = crate::pytext::trim(credential);
    if token.is_empty() || token.chars().count() > MAX_API_TOKEN_LENGTH {
        return None;
    }
    Some(token.as_bytes().to_vec())
}

/// Compare a presented credential with the expected token in constant time.
#[must_use]
pub fn token_matches(presented: &[u8], expected: &[u8]) -> bool {
    if presented.len() != expected.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left, right) in presented.iter().zip(expected.iter()) {
        difference |= left ^ right;
    }
    difference == 0
}

/// Mint a fresh launch token shaped like `secrets.token_urlsafe(32)`.
///
/// # Errors
///
/// Returns an error when the operating system entropy source fails.
pub fn mint_api_token() -> Result<String, String> {
    let mut bytes = [0u8; API_TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| format!("launch token entropy failed: {error}"))?;
    Ok(base64_url_no_pad(&bytes))
}

/// Return a fresh launch token and store it in the local data directory.
///
/// A launch invalidates the previous token by replacing the file. When the
/// directory is unusable the token still guards this process, held in memory
/// only, and the failure is reported by type. The token itself is never
/// written to a log, a response, or an error.
///
/// # Errors
///
/// Returns an error when the operating system entropy source fails. The
/// Python source raises in the same place, so a launch that cannot mint a
/// random token does not start.
pub fn issue_api_token(data_dir: &Path) -> Result<String, String> {
    let token = mint_api_token()?;
    if let Err(error) = store_api_token(data_dir, &token) {
        eprintln!(
            "the launch token was not stored in the local data directory: {}",
            error.kind()
        );
    }
    Ok(token)
}

/// Return the token stored for the running launch, or `None` when there is
/// none.
///
/// This is the retrieval path for the local operator: the CLI reads the file
/// this process wrote. Only a bounded prefix is read, and a missing,
/// unreadable, over-long, or malformed file reads as `None` rather than
/// raising, so a corrupt store is reported as an absence.
#[must_use]
pub fn read_api_token(data_dir: &Path) -> Option<String> {
    let path = data_dir.join(API_TOKEN_FILE_NAME);
    let mut file = fs::File::open(path).ok()?;
    let mut raw = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take((MAX_API_TOKEN_LENGTH + 2) as u64)
        .read_to_end(&mut raw)
        .ok()?;
    let candidate = String::from_utf8(raw).ok()?;
    let candidate = crate::pytext::trim(&candidate);
    validate_api_token(candidate).ok()
}

/// Write the launch token into the local data directory with owner-only
/// permissions.
///
/// The file is opened without following a symlink, so a link planted in the
/// data directory cannot redirect the write.
///
/// # Errors
///
/// Returns the underlying I/O error when the file cannot be replaced.
pub fn store_api_token(data_dir: &Path, token: &str) -> io::Result<()> {
    let path = data_dir.join(API_TOKEN_FILE_NAME);
    let payload = format!("{token}\n");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(API_TOKEN_FILE_MODE);
        // The constant is read from `libc` so the value is right on every
        // supported Unix; using it needs no unsafe code.
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(API_TOKEN_FILE_MODE))?;
    }
    file.write_all(payload.as_bytes())?;
    file.flush()?;
    Ok(())
}

/// The request boundary of one launch.
#[derive(Clone, Debug)]
pub struct Boundary {
    token: Vec<u8>,
    max_body_bytes: usize,
}

impl Boundary {
    /// Build the boundary for one launch.
    ///
    /// # Errors
    ///
    /// Returns a sanitized message when the launch token is malformed or the
    /// body limit is outside the accepted range.
    pub fn new(api_token: &str, max_body_bytes: i64) -> Result<Self, String> {
        let token = validate_api_token(api_token)?.into_bytes();
        let max_body_bytes = crate::middleware::validate_max_body_bytes(max_body_bytes)?;
        Ok(Self {
            token,
            max_body_bytes,
        })
    }

    /// The enforced request body limit in bytes.
    #[must_use]
    pub fn max_body_bytes(&self) -> usize {
        self.max_body_bytes
    }

    /// Refuse a request whose Host header does not name the loopback
    /// interface.
    ///
    /// A request that declares more than one Host is refused too, because the
    /// browser sends exactly one.
    #[must_use]
    pub fn check_host(&self, headers: &[(Vec<u8>, Vec<u8>)]) -> Option<Refusal> {
        let hosts = header_values(headers, "host");
        if hosts.len() != 1 || !is_permitted_authority(&hosts[0]) {
            return Some(Refusal::new(421, "invalid_host", HOST_REFUSED_MESSAGE));
        }
        None
    }

    /// Refuse a request whose declared body length is over the limit.
    #[must_use]
    pub fn check_declared_length(&self, headers: &[(Vec<u8>, Vec<u8>)]) -> Option<Refusal> {
        let declared = crate::middleware::declared_length(headers)?;
        if declared > self.max_body_bytes as u128 {
            return Some(self.payload_too_large());
        }
        None
    }

    /// Refuse a request whose body is over the limit.
    #[must_use]
    pub fn check_body_length(&self, length: usize) -> Option<Refusal> {
        if length > self.max_body_bytes {
            return Some(self.payload_too_large());
        }
        None
    }

    /// Require the launch token on every mutating method.
    #[must_use]
    pub fn check_token(&self, method: &str, headers: &[(Vec<u8>, Vec<u8>)]) -> Option<Refusal> {
        if !is_mutating_method(method) {
            return None;
        }
        let presented = presented_credential(headers);
        match presented {
            Some(presented) if token_matches(&presented, &self.token) => None,
            _ => Some(
                Refusal::new(401, "unauthorized", TOKEN_REQUIRED_MESSAGE)
                    .with_header(AUTH_CHALLENGE.0, AUTH_CHALLENGE.1),
            ),
        }
    }

    /// Refuse every request that carries an Origin header.
    #[must_use]
    pub fn check_origin(&self, headers: &[(Vec<u8>, Vec<u8>)]) -> Option<Refusal> {
        if header_value(headers, "origin").is_some() {
            return Some(Refusal::new(
                403,
                "cross_origin_denied",
                ORIGIN_REFUSED_MESSAGE,
            ));
        }
        None
    }

    /// The refusal an oversized body earns.
    fn payload_too_large(&self) -> Refusal {
        Refusal::new(
            413,
            "payload_too_large",
            crate::middleware::PAYLOAD_TOO_LARGE_MESSAGE,
        )
        .with_detail(&format!(
            "the request body must not exceed {} bytes",
            self.max_body_bytes
        ))
    }
}

/// Encode bytes as unpadded base64url, the shape `secrets.token_urlsafe`
/// produces.
fn base64_url_no_pad(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0] as u32;
        let second = chunk.get(1).copied().unwrap_or(0) as u32;
        let third = chunk.get(2).copied().unwrap_or(0) as u32;
        let combined = (first << 16) | (second << 8) | third;
        output.push(ALPHABET[((combined >> 18) & 0x3F) as usize] as char);
        output.push(ALPHABET[((combined >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            output.push(ALPHABET[((combined >> 6) & 0x3F) as usize] as char);
        }
        if chunk.len() > 2 {
            output.push(ALPHABET[(combined & 0x3F) as usize] as char);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wildcard_address_is_never_loopback() {
        for host in [
            "0.0.0.0",
            "::",
            "[::]",
            "0:0:0:0:0:0:0:0",
            "0.0.0.0:8000",
            "[::]:8000",
            "*",
        ] {
            assert!(!is_loopback_host(host), "{host}");
            assert!(!is_permitted_authority(host), "{host}");
        }
        for host in [
            "localhost",
            "127.0.0.1",
            "127.0.0.2",
            "[::1]",
            "[::1]:8000",
            "localhost:8000",
        ] {
            assert!(is_permitted_authority(host), "{host}");
        }
        for host in ["localhost", "127.0.0.1", "127.0.0.2", "::1"] {
            assert!(is_loopback_host(host), "{host}");
        }
        for host in [
            "evil.example",
            "",
            "127.0.0.1.",
            "http://127.0.0.1",
            "127.0.0.1/path",
        ] {
            assert!(!is_loopback_host(host), "{host}");
            assert!(!is_permitted_authority(host), "{host}");
        }
        assert!(!is_permitted_authority("::1"));
        for authority in [
            "127.0.0.1 ",
            " 127.0.0.1",
            "[::1]x",
            "[::1]x:8000",
            ":8000",
            "1",
            "[::1",
        ] {
            assert!(!is_permitted_authority(authority), "{authority}");
        }
        let overlong = "1".repeat(300);
        assert!(!is_loopback_host(&overlong));
    }

    #[test]
    fn ipv4_mapped_loopback_is_loopback_like_cpython() {
        assert!(is_loopback_host("::ffff:127.0.0.1"));
        assert!(is_loopback_host("::1%eth0"));
        assert!(!is_loopback_host("::ffff:0.0.0.0"));
        assert!(!is_loopback_host("::ffff:0:1"));
        assert!(!is_loopback_host("127.1"));
        assert!(!is_loopback_host("0177.0.0.1"));
    }

    #[test]
    fn token_validation_rejects_malformed_values() {
        assert!(validate_api_token("short").is_err());
        assert!(validate_api_token(&"a".repeat(129)).is_err());
        assert!(validate_api_token("a-b_c.d~e0123456789").is_ok());
        assert!(validate_api_token("a b c d e f g h i").is_err());
        assert!(validate_api_token(&"a".repeat(16)).is_ok());
    }

    #[test]
    fn presented_credentials_follow_the_scheme() {
        let headers = |value: &str| vec![(b"authorization".to_vec(), value.as_bytes().to_vec())];
        assert_eq!(
            presented_credential(&headers("Bearer abcdefghijklmnop")),
            Some(b"abcdefghijklmnop".to_vec())
        );
        assert_eq!(
            presented_credential(&headers("bearer abcdefghijklmnop")),
            Some(b"abcdefghijklmnop".to_vec())
        );
        assert_eq!(
            presented_credential(&headers("Bearer  abcdefghijklmnop")),
            Some(b"abcdefghijklmnop".to_vec())
        );
        assert_eq!(presented_credential(&headers("Bearer")), None);
        assert_eq!(presented_credential(&headers("Bearer ")), None);
        assert_eq!(
            presented_credential(&headers("Basic abcdefghijklmnop")),
            None
        );
        assert_eq!(presented_credential(&headers("abcdefghijklmnop")), None);
        assert_eq!(
            presented_credential(&headers("Bearer a b")),
            Some(b"a b".to_vec())
        );
    }

    #[test]
    fn minted_tokens_are_urlsafe_and_bounded() {
        let token = mint_api_token().unwrap();
        assert_eq!(token.len(), 43);
        assert!(validate_api_token(&token).is_ok());
    }
}
