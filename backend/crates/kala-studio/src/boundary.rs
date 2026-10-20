//! Studio request boundary: token lifecycle, loopback authority and origin
//! parsing.
//!
//! The token is minted per launch and stored atomically with owner-only
//! permissions where the platform supports POSIX modes. The Host and Origin
//! parsers mirror the Python source's canonicalization, including bracketed
//! IPv6, default ports, duplicate headers and userinfo rejection.

use std::fs;
use std::io::{self, Read};
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use crate::errors::truncate_chars;
use crate::pytext;
use crate::settings::atomic_write_text;

pub const STUDIO_TOKEN_ENV_VAR: &str = "KALA_STUDIO_TOKEN";
pub const STUDIO_TOKEN_FILE_NAME: &str = "studio-token";
pub const STUDIO_SUBDIRECTORY_NAME: &str = "studio";
pub const STUDIO_API_TOKEN_BYTES: usize = 32;
pub const MIN_API_TOKEN_LENGTH: usize = 16;
pub const MAX_API_TOKEN_LENGTH: usize = 128;
pub const MAX_HEADER_VALUE_LENGTH: usize = 512;
pub const MAX_AUTHORITY_LENGTH: usize = 256;
pub const TOKEN_REQUIRED_MESSAGE: &str = "a valid bearer token is required for this request";
pub const HOST_REFUSED_MESSAGE: &str = "the studio request host is not a loopback address";
pub const NOT_LOOPBACK_MESSAGE: &str = "the studio binds and serves loopback interfaces only; expected one of: 127.0.0.1, ::1, localhost";
pub const STUDIO_BEARER_REALM: &str = "Bearer realm=\"kala-studio\"";
pub const TOKEN_DIR_MODE: u32 = 0o700;
pub const TOKEN_FILE_MODE: u32 = 0o600;

const WILDCARD_AUTHORITIES: &[&str] = &["0.0.0.0", "::", "[::]", "[::0]", "0:0:0:0:0:0:0:0", "*"];
const LOOPBACK_HOSTNAMES: &[&str] = &["localhost", "127.0.0.1", "::1"];

/// Validate a Studio bearer token exactly like the Python boundary.
///
/// # Errors
///
/// Returns a sanitized message for a non-string, wrong-length or
/// unsupported-character token. The submitted value is never included.
pub fn validate_studio_token(value: &str) -> Result<(), String> {
    let length = value.chars().count();
    if !(MIN_API_TOKEN_LENGTH..=MAX_API_TOKEN_LENGTH).contains(&length) {
        return Err(format!(
            "the studio token must be between {MIN_API_TOKEN_LENGTH} and {MAX_API_TOKEN_LENGTH} characters"
        ));
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "._~-".contains(character))
    {
        return Err("the studio token contains unsupported characters".to_owned());
    }
    Ok(())
}

/// Whether a bare hostname or address names the loopback interface.
#[must_use]
pub fn is_loopback_host(host: &str) -> bool {
    let candidate = host.to_lowercase();
    if candidate.is_empty() || candidate.chars().count() > MAX_AUTHORITY_LENGTH {
        return false;
    }
    if WILDCARD_AUTHORITIES.contains(&candidate.as_str()) {
        return false;
    }
    if LOOPBACK_HOSTNAMES.contains(&candidate.as_str()) {
        return true;
    }
    match candidate.parse::<IpAddr>() {
        Ok(address) => address.is_loopback(),
        Err(_) => false,
    }
}

/// Split an HTTP authority into `(host, port)` with the Python source's
/// strictness. A port is returned only when the authority carried one.
#[must_use]
pub fn split_authority(authority: &str) -> Option<(String, Option<String>)> {
    if authority.is_empty() || authority.chars().count() > MAX_AUTHORITY_LENGTH {
        return None;
    }
    if authority.chars().any(pytext::is_space) {
        return None;
    }
    let mut port: Option<String> = None;
    let host: String;
    if let Some(rest) = authority.strip_prefix('[') {
        let closing = rest.find(']')?;
        host = rest[..closing].to_owned();
        let remainder = &rest[closing + 1..];
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

/// Whether a Host header names this process on the loopback interface.
#[must_use]
pub fn is_permitted_authority(authority: &str) -> bool {
    match split_authority(authority) {
        Some((host, _port)) => is_loopback_host(&host),
        None => false,
    }
}

/// Parse a `Host` authority into `(scheme, host, port)`.
#[must_use]
pub fn parse_host_authority(authority: &str) -> Option<(&'static str, String, u16)> {
    let (host, port) = split_authority(authority)?;
    if !is_loopback_host(&host) {
        return None;
    }
    let port = match port {
        Some(text) => text.parse::<u16>().ok()?,
        None => 80,
    };
    Some(("http", host.to_lowercase(), port))
}

/// The result of parsing an `Origin` header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OriginParse {
    /// A complete HTTP(S) origin.
    Valid((String, String, u16)),
    /// Not a same-origin-shaped value; the guard answers 403.
    Rejected,
    /// `urlsplit`/`parsed.port` would raise in Python; the source reaches the
    /// unhandled-exception path and answers a sanitized 500.
    Malformed,
}

/// Parse an `Origin` header.
///
/// Only the exact origin triple is accepted: no path, query, fragment,
/// userinfo, null origin or non-HTTP scheme. NaN/Infinity-style port and
/// IPv6 syntax errors preserve the Python source's 500 branch.
#[must_use]
pub fn parse_origin_header(raw: &str) -> OriginParse {
    let candidate = pytext::trim(raw);
    if candidate.is_empty() || candidate.eq_ignore_ascii_case("null") {
        return OriginParse::Rejected;
    }
    // Python 3.14's urlsplit removes ASCII tab/newline characters anywhere.
    let cleaned: String = candidate
        .chars()
        .filter(|character| !matches!(character, '\t' | '\n' | '\r'))
        .collect();
    let Some((scheme_part, rest)) = cleaned.split_once("://") else {
        return OriginParse::Rejected;
    };
    let scheme = scheme_part.to_lowercase();
    if scheme != "http" && scheme != "https" {
        return OriginParse::Rejected;
    }
    if rest.contains('?') || rest.contains('#') {
        return OriginParse::Rejected;
    }
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, ""),
    };
    if !path.is_empty() && path != "/" {
        return OriginParse::Rejected;
    }
    if authority.is_empty() || authority.contains('@') {
        return OriginParse::Rejected;
    }
    if authority
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\t' | '\n' | '\r'))
    {
        return OriginParse::Malformed;
    }
    let (host, port_text) = if let Some(stripped) = authority.strip_prefix('[') {
        let Some(closing) = stripped.find(']') else {
            return OriginParse::Malformed;
        };
        let host = &stripped[..closing];
        let remainder = &stripped[closing + 1..];
        if remainder.is_empty() {
            (host.to_owned(), None)
        } else {
            let Some(port) = remainder.strip_prefix(':') else {
                return OriginParse::Rejected;
            };
            (host.to_owned(), Some(port.to_owned()))
        }
    } else {
        let mut pieces = authority.split(':');
        let first = pieces.next().unwrap_or("");
        let second = pieces.next();
        if pieces.next().is_some() {
            return OriginParse::Malformed;
        }
        match second {
            Some(port) => (first.to_owned(), Some(port.to_owned())),
            None => (first.to_owned(), None),
        }
    };
    if host.is_empty() || host.chars().count() > MAX_HEADER_VALUE_LENGTH {
        return OriginParse::Rejected;
    }
    let port = match port_text {
        Some(text) if text.is_empty() => {
            if scheme == "https" {
                443
            } else {
                80
            }
        }
        Some(text) => {
            if !text.bytes().all(|byte| byte.is_ascii_digit()) {
                return OriginParse::Malformed;
            }
            let Ok(value) = text.parse::<u64>() else {
                return OriginParse::Malformed;
            };
            if value > 65_535 {
                return OriginParse::Malformed;
            }
            value as u16
        }
        None => {
            if scheme == "https" {
                443
            } else {
                80
            }
        }
    };
    OriginParse::Valid((scheme, host.to_lowercase(), port))
}

/// Whether an Origin triple exactly matches a Host triple.
#[must_use]
pub fn origins_match(
    origin: &(String, String, u16),
    authority: &(&'static str, String, u16),
) -> bool {
    origin.0 == authority.0 && origin.1 == authority.1 && origin.2 == authority.2
}

/// Decode a header value the way the ASGI boundary does: Latin-1, bounded.
#[must_use]
pub fn header_value_text(value: &[u8]) -> String {
    let text: String = value.iter().map(|byte| char::from(*byte)).collect();
    truncate_chars(&text, MAX_HEADER_VALUE_LENGTH)
}

/// Return `data_dir / studio-token`, or `None` when the file is absent,
/// unreadable, over-long or malformed.
#[must_use]
pub fn read_studio_token(data_dir: &Path) -> Option<String> {
    let path = data_dir.join(STUDIO_TOKEN_FILE_NAME);
    let mut file = fs::File::open(path).ok()?;
    let mut raw = Vec::new();
    file.by_ref()
        .take((MAX_API_TOKEN_LENGTH + 2) as u64)
        .read_to_end(&mut raw)
        .ok()?;
    let candidate = String::from_utf8(raw).ok()?;
    let candidate = pytext::trim(&candidate);
    validate_studio_token(candidate).ok()?;
    Some(candidate.to_owned())
}

/// Mint a fresh `secrets.token_urlsafe(32)`-shaped token.
///
/// # Errors
///
/// Returns an error when the operating system entropy source fails.
pub fn mint_studio_token() -> Result<String, String> {
    let mut bytes = [0u8; STUDIO_API_TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| format!("studio token entropy failed: {error}"))?;
    Ok(base64_url_no_pad(&bytes))
}

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

/// Snapshot the process environment without panicking on non-Unicode values.
#[must_use]
pub fn process_environ() -> std::collections::BTreeMap<String, String> {
    std::env::vars_os()
        .filter_map(|(key, value)| {
            let key = key.into_string().ok()?;
            Some((key, value.to_string_lossy().into_owned()))
        })
        .collect()
}

/// Resolve the default data directory from the same environment variables
/// the Python Settings loader uses, falling back to the temporary directory.
///
/// The default path gets the per-studio `studio` child and owner-only mode;
/// an explicit `--data-dir` does not (the CLI passes it directly).
#[must_use]
pub fn default_studio_data_dir(environ: &std::collections::BTreeMap<String, String>) -> PathBuf {
    let fallback = std::env::temp_dir().join("kala-studio");
    if let Some(raw) = environ.get("KALA_AGENT_DATA_DIR") {
        // The Python Settings validator rejects empty, relative and root
        // values, and the CLI then falls back to the temporary directory.
        let Some(parent) = expand_home(raw) else {
            return fallback;
        };
        if !parent.is_absolute() || parent.parent().is_none() {
            return fallback;
        }
        if parent.exists() && !parent.is_dir() {
            return fallback;
        }
        return resolve_studio_data_dir(&parent).unwrap_or(fallback);
    }
    if let Some(home) = home_directory() {
        let parent = home.join(".local").join("share").join("kala-agent");
        if parent.is_absolute()
            && parent.parent().is_some()
            && resolve_studio_data_dir(&parent).is_ok()
        {
            return parent.join(STUDIO_SUBDIRECTORY_NAME);
        }
    }
    fallback
}

/// The per-user home directory on the current platform.
pub(crate) fn home_directory() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("HOME").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(home));
    }
    if let Some(profile) = std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(profile));
    }
    match (std::env::var_os("HOMEDRIVE"), std::env::var_os("HOMEPATH")) {
        (Some(drive), Some(path)) if !drive.is_empty() && !path.is_empty() => {
            let mut home = PathBuf::from(drive);
            home.push(path);
            Some(home)
        }
        _ => None,
    }
}

fn expand_home(value: &str) -> Option<PathBuf> {
    let trimmed = value.trim();
    if trimmed == "~" {
        return home_directory();
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        return home_directory().map(|home| home.join(rest));
    }
    let path = PathBuf::from(trimmed);
    if path.is_absolute() { Some(path) } else { None }
}

/// Write a launch token through the atomic same-directory path.
///
/// # Errors
///
/// Returns the underlying I/O error when the file cannot be replaced.
pub fn store_studio_token_file(data_dir: &Path, token: &str) -> io::Result<()> {
    let payload = format!("{token}\n");
    atomic_write_text(
        &data_dir.join(STUDIO_TOKEN_FILE_NAME),
        &payload,
        TOKEN_FILE_MODE,
    )
}

/// Mint a token and store it, reporting a storage failure without aborting
/// the launch. The in-memory token still guards the process.
pub fn issue_studio_token(data_dir: &Path) -> Result<String, String> {
    let token = mint_studio_token()?;
    if let Err(error) = store_studio_token_file(data_dir, &token) {
        eprintln!(
            "the studio token could not be stored in the local data directory; the token is held in memory only for this launch: {}",
            error.kind()
        );
    }
    Ok(token)
}

/// Resolve the per-launch token from the explicit environment variable or by
/// minting one into the studio data directory.
///
/// # Errors
///
/// Returns a sanitized validation error for a malformed explicit token or an
/// entropy failure.
pub fn resolve_studio_token(data_dir: &Path, explicit: Option<&str>) -> Result<String, String> {
    if let Some(value) = explicit {
        let trimmed = pytext::trim(value);
        if !trimmed.is_empty() {
            validate_studio_token(trimmed)?;
            return Ok(trimmed.to_owned());
        }
    }
    issue_studio_token(data_dir)
}

/// Create the per-studio child directory with owner-only permissions where
/// the platform supports it. Never chmods an ancestor.
///
/// # Errors
///
/// Returns an I/O error when a directory cannot be created.
pub fn resolve_studio_data_dir(parent: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(parent)?;
    let child = parent.join(STUDIO_SUBDIRECTORY_NAME);
    fs::create_dir_all(&child)?;
    tighten_directory_mode(&child);
    Ok(child)
}

/// Set `0o700` on Unix; POSIX mode bits are not meaningful on Windows.
pub fn tighten_directory_mode(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(TOKEN_DIR_MODE));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_authority_rules_match_python() {
        assert!(is_permitted_authority("127.0.0.1"));
        assert!(is_permitted_authority("LOCALHOST:8765"));
        assert!(is_permitted_authority("[::1]:8765"));
        assert!(is_permitted_authority("127.0.0.2"));
        assert!(!is_permitted_authority("0.0.0.0"));
        assert!(!is_permitted_authority("::1"));
        assert!(!is_permitted_authority("localhost."));
        assert!(!is_permitted_authority("evil.example"));
        assert!(!is_permitted_authority("127.0.0.1:70000"));
        assert!(!is_permitted_authority("user@127.0.0.1"));
        assert!(!is_permitted_authority("127.0.0.1/path"));
    }

    #[test]
    fn origin_rules_match_python() {
        let host = parse_host_authority("127.0.0.1").unwrap();
        let origin = |value: &str| match parse_origin_header(value) {
            OriginParse::Valid(triple) => Some(triple),
            _ => None,
        };
        assert!(origins_match(&origin("http://127.0.0.1").unwrap(), &host));
        assert!(origins_match(
            &origin("http://127.0.0.1:80/").unwrap(),
            &host
        ));
        assert!(!origins_match(&origin("https://127.0.0.1").unwrap(), &host));
        assert!(origin("null").is_none());
        assert!(origin("http://user@127.0.0.1").is_none());
        assert!(origin("http://127.0.0.1/path").is_none());
        assert!(origin("http://127.0.0.1?x").is_none());
        assert!(origin("http://127.0.0.1#x").is_none());
        assert_eq!(
            parse_origin_header("http://127.0.0.1:"),
            OriginParse::Valid(("http".to_owned(), "127.0.0.1".to_owned(), 80))
        );
        assert_eq!(
            parse_origin_header("http://127.0.0.1:abc"),
            OriginParse::Malformed
        );
        assert_eq!(parse_origin_header("http://[::1"), OriginParse::Malformed);
    }

    #[test]
    fn generated_tokens_are_urlsafe_and_bounded() {
        let token = mint_studio_token().unwrap();
        assert_eq!(token.len(), 43);
        assert!(validate_studio_token(&token).is_ok());
    }

    #[test]
    fn invalid_data_dir_env_uses_the_temporary_fallback() {
        let mut environ = std::collections::BTreeMap::new();
        for value in ["", "relative", "/"] {
            environ.insert("KALA_AGENT_DATA_DIR".to_owned(), value.to_owned());
            assert_eq!(
                default_studio_data_dir(&environ),
                std::env::temp_dir().join("kala-studio"),
                "value {value:?}"
            );
        }
    }

    #[test]
    fn token_validation_rejects_malformed_values() {
        assert!(validate_studio_token("short").is_err());
        assert!(validate_studio_token(&"a".repeat(129)).is_err());
        assert!(validate_studio_token("has space in it and is long").is_err());
    }
}
