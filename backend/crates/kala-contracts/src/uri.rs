//! Artifact URI normalization and security checks.
//!
//! This mirrors `contracts.models._normalize_artifact_uri`, including Python
//! `urllib.parse.urlsplit` parsing, `ipaddress` normalization, IDNA
//! processing, strict percent-decoding, POSIX path normalization and the
//! scheme-specific host/port rules.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use idna::uts46::{AsciiDenyList, DnsLength, Hyphens, Uts46};

use crate::error::ContractError;
use crate::text::{Text, is_python_whitespace};

const ALLOWED_SCHEMES: [&str; 3] = ["artifact", "file", "https"];

/// Normalize and validate an artifact URI.
pub fn normalize_artifact_uri(value: &Text) -> Result<Text, ContractError> {
    if value.is_empty() || value.len() > 4096 {
        return Err(error(
            "artifact URI must be nonempty and no longer than 4096 characters",
        ));
    }
    for point in value.code_points() {
        if is_python_whitespace(*point) || *point < 32 || *point == 127 {
            return Err(error(
                "artifact URI cannot contain whitespace or control characters",
            ));
        }
    }
    let source = value
        .to_utf8()
        .map_err(|_| error("artifact URI is invalid"))?;

    let (scheme, rest) = split_scheme(&source)?;
    let scheme = scheme.to_ascii_lowercase();
    if !ALLOWED_SCHEMES.contains(&scheme.as_str()) {
        return Err(error("artifact URI scheme is not allowed"));
    }

    let (authority, path_and_more) = if let Some(after) = rest.strip_prefix("//") {
        match after.find(['/', '?', '#']) {
            Some(index) => (Some(&after[..index]), &after[index..]),
            None => (Some(after), ""),
        }
    } else {
        (None, rest)
    };

    let (path, query, fragment) = split_path_query_fragment(path_and_more);
    if !query.is_empty() || !fragment.is_empty() {
        return Err(error(
            "artifact URI cannot contain query parameters or fragments",
        ));
    }

    let (raw_host, port) = match authority {
        Some(authority) => {
            if authority.contains('@') {
                return Err(error("artifact URI cannot contain credentials"));
            }
            parse_authority(authority)?
        }
        None => (None, None),
    };

    if !path.starts_with('/') {
        return Err(error("artifact URI must contain an absolute path"));
    }

    let normalized_host = match raw_host.as_deref() {
        Some(host) => normalize_hostname(host)?,
        None => String::new(),
    };
    let mut normalized_port = port;
    if scheme != "file" && normalized_host.is_empty() {
        return Err(error("artifact URI must contain a host"));
    }
    if scheme == "file" && !normalized_host.is_empty() && normalized_host != "localhost" {
        return Err(error("file artifact URI must be local"));
    }
    if scheme == "https" && matches!(normalized_port, Some(80) | Some(443)) {
        normalized_port = None;
    }

    validate_percent_escapes(path)?;
    let decoded = percent_decode(path)?;
    for character in decoded.chars() {
        if (character as u32) < 32 || character == '\u{7f}' {
            return Err(error("artifact URI path cannot contain control characters"));
        }
    }
    if decoded.contains('\\') {
        return Err(error("artifact URI path cannot contain backslashes"));
    }
    if decoded
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err(error("artifact URI path cannot contain traversal segments"));
    }

    let mut normalized_path = posix_normpath(path);
    if normalized_path == "." {
        normalized_path = "/".to_owned();
    }
    while normalized_path.starts_with("//") {
        normalized_path.remove(0);
    }

    let netloc = if scheme == "file" {
        String::new()
    } else {
        let mut netloc = normalized_host;
        if let Some(port) = normalized_port {
            netloc.push(':');
            netloc.push_str(&port.to_string());
        }
        netloc
    };
    Text::from_code_points(
        format!("{scheme}://{netloc}{normalized_path}")
            .chars()
            .map(u32::from)
            .collect(),
    )
    .map_err(|_| error("artifact URI is invalid"))
}

fn error(message: &'static str) -> ContractError {
    ContractError::new("ArtifactUri", message)
}

fn split_scheme(value: &str) -> Result<(&str, &str), ContractError> {
    let Some(index) = value.find(':') else {
        return Ok(("", value));
    };
    let scheme = &value[..index];
    let bytes = scheme.as_bytes();
    if bytes.is_empty()
        || !bytes[0].is_ascii_alphabetic()
        || !bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
    {
        // Python `urlsplit` treats a non-scheme prefix as part of the path.
        return Ok(("", value));
    }
    if value[..index].contains('/') {
        return Ok(("", value));
    }
    Ok((scheme, &value[index + 1..]))
}

fn split_path_query_fragment(value: &str) -> (&str, &str, &str) {
    let (before_fragment, fragment) = match value.split_once('#') {
        Some((left, right)) => (left, right),
        None => (value, ""),
    };
    let (path, query) = match before_fragment.split_once('?') {
        Some((left, right)) => (left, right),
        None => (before_fragment, ""),
    };
    (path, query, fragment)
}

fn parse_authority(authority: &str) -> Result<(Option<String>, Option<u16>), ContractError> {
    if authority.is_empty() {
        return Ok((None, None));
    }
    if let Some(rest) = authority.strip_prefix('[') {
        let Some(end) = rest.find(']') else {
            return Err(error("artifact URI is invalid"));
        };
        if rest[end + 1..].contains(['[', ']']) {
            return Err(error("artifact URI is invalid"));
        }
        let host = &rest[..end];
        let after = &rest[end + 1..];
        let port = parse_port_suffix(after)?;
        return Ok(((!host.is_empty()).then(|| host.to_owned()), port));
    }
    if authority.contains(['[', ']']) {
        return Err(error("artifact URI is invalid"));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => {
            if host.contains(':') {
                return Err(error("artifact URI is invalid"));
            }
            let port = if port.is_empty() {
                None
            } else {
                Some(parse_port(port)?)
            };
            Ok(((!host.is_empty()).then(|| host.to_owned()), port))
        }
        None => Ok(((!authority.is_empty()).then(|| authority.to_owned()), None)),
    }
}

fn parse_port_suffix(value: &str) -> Result<Option<u16>, ContractError> {
    if value.is_empty() {
        return Ok(None);
    }
    let Some(port) = value.strip_prefix(':') else {
        return Err(error("artifact URI is invalid"));
    };
    if port.is_empty() {
        return Ok(None);
    }
    Ok(Some(parse_port(port)?))
}

fn parse_port(value: &str) -> Result<u16, ContractError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(error("artifact URI is invalid"));
    }
    value
        .parse::<u16>()
        .map_err(|_| error("artifact URI is invalid"))
}

fn normalize_hostname(host: &str) -> Result<String, ContractError> {
    if let Ok(address) = Ipv4Addr::from_str(host) {
        return Ok(address.to_string());
    }
    if host.contains(':') {
        let (address, scope) = match host.split_once('%') {
            Some((address, scope)) => (address, Some(scope)),
            None => (host, None),
        };
        if let Ok(address) = Ipv6Addr::from_str(address) {
            let mut formatted = format_ipv6_python(address);
            if let Some(scope) = scope {
                formatted.push('%');
                formatted.push_str(scope);
            }
            return Ok(format!("[{formatted}]"));
        }
    }
    idna_to_ascii(host)
}

fn idna_to_ascii(host: &str) -> Result<String, ContractError> {
    let hostname_error = || error("artifact URI host is invalid");
    let trailing_dot = host.ends_with('.');
    let core = if trailing_dot {
        &host[..host.len() - 1]
    } else {
        host
    };
    if core.is_empty() {
        return Err(hostname_error());
    }
    let mut out = String::new();
    for (index, label) in core.split('.').enumerate() {
        if index > 0 {
            out.push('.');
        }
        if label.is_empty() {
            return Err(hostname_error());
        }
        let ascii = if label.is_ascii() {
            label.to_ascii_lowercase()
        } else {
            idna_label(label)?
        };
        if ascii.is_empty() || ascii.len() > 63 {
            return Err(hostname_error());
        }
        out.push_str(&ascii);
    }
    if trailing_dot {
        out.push('.');
    }
    Ok(out)
}

fn idna_label(label: &str) -> Result<String, ContractError> {
    // Python's IDNA codec is IDNA2003: transitional processing maps ß to ss,
    // final sigma to sigma and removes the join controls. UTS #46 with the
    // same transitional mapping is the closest standard implementation.
    let mut mapped = String::with_capacity(label.len());
    for character in label.chars() {
        match character {
            'ß' | 'ẞ' => mapped.push_str("ss"),
            'ς' => mapped.push('σ'),
            '\u{200C}' | '\u{200D}' => {}
            _ => mapped.push(character),
        }
    }
    Uts46::new()
        .to_ascii(
            mapped.as_bytes(),
            AsciiDenyList::EMPTY,
            Hyphens::Allow,
            DnsLength::Ignore,
        )
        .map(|value| value.into_owned())
        .map_err(|_| error("artifact URI host is invalid"))
}

fn validate_percent_escapes(path: &str) -> Result<(), ContractError> {
    let bytes = path.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return Err(error("artifact URI contains an invalid percent escape"));
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    Ok(())
}

fn percent_decode(path: &str) -> Result<String, ContractError> {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex_value(bytes[index + 1]);
            let low = hex_value(bytes[index + 2]);
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| error("artifact URI path is not valid UTF-8"))
}

fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => 0,
    }
}

/// Python `posixpath.normpath`.
fn posix_normpath(path: &str) -> String {
    if path.is_empty() {
        return ".".to_owned();
    }
    let initial_slashes = if path.starts_with('/') { 1 } else { 0 };
    let initial_slashes =
        if initial_slashes == 1 && path.starts_with("//") && !path.starts_with("///") {
            2
        } else {
            initial_slashes
        };
    let mut components: Vec<&str> = Vec::new();
    for component in path.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if component != ".."
            || (initial_slashes == 0 && components.is_empty())
            || components.last().is_some_and(|last| *last == "..")
        {
            components.push(component);
        } else if !components.is_empty() {
            components.pop();
        }
    }
    let mut result = components.join("/");
    if initial_slashes > 0 {
        result = format!("{}{result}", "/".repeat(initial_slashes));
    }
    if result.is_empty() {
        ".".to_owned()
    } else {
        result
    }
}

/// Python `ipaddress.IPv6Address.__str__` (RFC 5952 plus the IPv4-mapped
/// dotted-quad spelling and an optional scope id).
fn format_ipv6_python(address: Ipv6Addr) -> String {
    let segments = address.segments();
    if segments[..5].iter().all(|segment| *segment == 0) && segments[5] == 0xFFFF {
        let octets = address.octets();
        return format!(
            "::ffff:{}.{}.{}.{}",
            octets[12], octets[13], octets[14], octets[15]
        );
    }
    let hextets: Vec<String> = segments
        .iter()
        .map(|segment| format!("{segment:x}"))
        .collect();
    let mut best_start = 0;
    let mut best_length = 0;
    let mut run_start = 0;
    let mut run_length = 0;
    for (index, hextet) in hextets.iter().enumerate() {
        if hextet == "0" {
            if run_length == 0 {
                run_start = index;
            }
            run_length += 1;
        } else {
            if run_length > best_length {
                best_start = run_start;
                best_length = run_length;
            }
            run_length = 0;
        }
    }
    if run_length > best_length {
        best_start = run_start;
        best_length = run_length;
    }
    if best_length < 2 {
        return hextets.join(":");
    }
    let prefix = hextets[..best_start].join(":");
    let suffix = hextets[best_start + best_length..].join(":");
    if prefix.is_empty() && suffix.is_empty() {
        "::".to_owned()
    } else if prefix.is_empty() {
        format!("::{suffix}")
    } else if suffix.is_empty() {
        format!("{prefix}::")
    } else {
        format!("{prefix}::{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normalized(value: &str) -> String {
        normalize_artifact_uri(&Text::new(value))
            .unwrap_or_else(|error| panic!("{value}: {}", error.message()))
            .to_utf8()
            .expect("ASCII test URI")
    }

    fn refused(value: &str) -> String {
        normalize_artifact_uri(&Text::new(value))
            .expect_err(value)
            .message()
            .to_owned()
    }

    #[test]
    fn matches_the_python_normalization_examples() {
        assert_eq!(
            normalized("HTTPS://Example.COM:443/objects//item%20one"),
            "https://example.com/objects/item%20one"
        );
        assert_eq!(normalized("file:///tmp/a//b/"), "file:///tmp/a/b");
        assert_eq!(normalized("file://localhost:123/a"), "file:///a");
        assert_eq!(
            normalized("artifact://Objects/foo"),
            "artifact://objects/foo"
        );
        assert_eq!(
            normalized("https://example.com:80/a"),
            "https://example.com/a"
        );
        assert_eq!(
            normalized("https://example.com:8443/a"),
            "https://example.com:8443/a"
        );
        assert_eq!(
            normalized("https://example.com/%2f%41"),
            "https://example.com/%2f%41"
        );
        assert_eq!(
            normalized("https://example.com//a///b"),
            "https://example.com/a/b"
        );
        assert_eq!(
            normalized("https://bücher.example/a"),
            "https://xn--bcher-kva.example/a"
        );
        assert_eq!(normalized("https://faß.de/a"), "https://fass.de/a");
        assert_eq!(
            normalized("https://[2001:DB8::1]:443/foo"),
            "https://[2001:db8::1]/foo"
        );
        assert_eq!(
            normalized("https://[::ffff:c000:201]/a"),
            "https://[::ffff:192.0.2.1]/a"
        );
        assert_eq!(
            normalized("https://[::192.0.2.1]/a"),
            "https://[::c000:201]/a"
        );
        assert_eq!(
            normalized("https://[2001:db8::1:0:0:0]/a"),
            "https://[2001:db8:0:0:1::]/a"
        );
    }

    #[test]
    fn refuses_the_security_cases() {
        for uri in [
            "relative/path",
            "javascript:alert(1)",
            "https://user:password@example.com/object",
            "https://example.com/object?token=secret",
            "https://example.com/object#fragment",
            "file://remote-host/object",
            "https://example.com/a/%2e%2e/b",
            "https://example.com/a/%00",
            "https://example.com/a\\b",
            "https://example.com/%ff",
            "artifact:/a",
            "artifact:a",
            "https:///a",
        ] {
            let message = refused(uri);
            assert!(!message.is_empty(), "{uri}");
        }
    }

    #[test]
    fn idna_ascii_labels_keep_python_verbatim_behaviour() {
        assert_eq!(normalized("https://-bad.com/a"), "https://-bad.com/a");
        assert_eq!(
            normalized("https://exa_mple.com/a"),
            "https://exa_mple.com/a"
        );
        assert_eq!(
            normalized("https://example.com./a"),
            "https://example.com./a"
        );
        assert_eq!(normalized("https://a\u{200d}b.com/a"), "https://ab.com/a");
        assert_eq!(normalized("https://①.com/a"), "https://1.com/a");
    }
}
