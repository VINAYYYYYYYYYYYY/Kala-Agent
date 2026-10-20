//! Provider endpoint parsing and egress normalization.
//!
//! The Python module parses a provider endpoint with `urllib.parse.urlsplit`
//! and normalizes the path with `posixpath.normpath`. Both are reproduced
//! here so a destination that Python accepts is accepted here, and a
//! destination Python refuses is refused here with the same message.

use std::net::IpAddr;
use std::str::FromStr;

use super::error::ModelError;

/// The maximum length of a provider endpoint.
pub const MAX_ENDPOINT_LENGTH: usize = 2_048;

/// The host names that always denote the loopback interface.
pub const LOOPBACK_HOSTS: &[&str] = &["localhost", "ip6-localhost", "ip6-loopback"];

/// The parsed parts of a URL, mirroring `urllib.parse.SplitResult`.
#[derive(Clone, Debug, Eq, PartialEq)]
struct SplitUrl {
    scheme: String,
    netloc: String,
    path: String,
    query: Option<String>,
    fragment: Option<String>,
}

const SCHEME_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789+-.";

/// Split a URL the way CPython's `urllib.parse.urlsplit` does.
fn urlsplit(value: &str) -> Result<SplitUrl, ()> {
    // CPython strips the leading WHATWG C0 control or space characters.
    let trimmed = value.trim_start_matches(|character: char| (character as u32) <= 0x20);
    let (scheme, rest) = split_scheme(trimmed);
    let (netloc, remainder) = if let Some(authority) = rest.strip_prefix("//") {
        split_netloc(authority)
    } else {
        (String::new(), rest)
    };
    if (netloc.contains('[') && !netloc.contains(']'))
        || (netloc.contains(']') && !netloc.contains('['))
    {
        return Err(());
    }
    if netloc.contains('[') && netloc.contains(']') {
        let bracketed = netloc
            .split_once('[')
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        let host = bracketed
            .split_once(']')
            .map(|(host, _)| host)
            .unwrap_or_default();
        check_bracketed_host(host)?;
    }
    let (rest, fragment) = match remainder.split_once('#') {
        Some((rest, fragment)) => (rest, Some(fragment.to_owned())),
        None => (remainder.as_str(), None),
    };
    let (rest, query) = match rest.split_once('?') {
        Some((rest, query)) => (rest, Some(query.to_owned())),
        None => (rest, None),
    };
    Ok(SplitUrl {
        scheme,
        netloc,
        path: rest.to_owned(),
        query,
        fragment,
    })
}

fn split_scheme(value: &str) -> (String, String) {
    if value.starts_with("//") {
        return (String::new(), value.to_owned());
    }
    let Some(index) = value.find(':') else {
        return (String::new(), value.to_owned());
    };
    if index == 0 {
        return (String::new(), value.to_owned());
    }
    let head = &value[..index];
    let first = head.chars().next().unwrap_or_default();
    if !first.is_ascii_alphabetic() {
        return (String::new(), value.to_owned());
    }
    if !head.bytes().all(|byte| SCHEME_CHARS.contains(&byte)) {
        return (String::new(), value.to_owned());
    }
    (head.to_ascii_lowercase(), value[index + 1..].to_owned())
}

fn split_netloc(value: &str) -> (String, String) {
    let mut delimiter = value.len();
    for character in ['/', '?', '#'] {
        if let Some(index) = value.find(character) {
            delimiter = delimiter.min(index);
        }
    }
    (value[..delimiter].to_owned(), value[delimiter..].to_owned())
}

fn check_bracketed_host(host: &str) -> Result<(), ()> {
    if host.starts_with('v') || host.starts_with('V') {
        // An IPvFuture address must be `v<hex>+.<host>`.
        let rest = &host[1..];
        let Some((version, address)) = rest.split_once('.') else {
            return Err(());
        };
        if version.is_empty()
            || !version.bytes().all(|byte| byte.is_ascii_hexdigit())
            || address.is_empty()
        {
            return Err(());
        }
        return Ok(());
    }
    IpAddr::from_str(host).map(|_| ()).map_err(|_| ())
}

/// Split a string around the first occurrence of a separator.
fn partition_at(value: &str, separator: char) -> (&str, &str, &str) {
    match value.find(separator) {
        Some(index) => {
            let width = separator.len_utf8();
            (
                &value[..index],
                &value[index..index + width],
                &value[index + width..],
            )
        }
        None => (value, "", ""),
    }
}

/// The host and port parts of a netloc, mirroring `SplitResult._hostinfo`.
fn host_info(netloc: &str) -> (String, Option<String>) {
    let hostinfo = match netloc.rsplit_once('@') {
        Some((_, hostinfo)) => hostinfo,
        None => netloc,
    };
    let (hostname, port) = if let Some((_, after_bracket)) = hostinfo.split_once('[') {
        let (host, rest) = after_bracket.split_once(']').unwrap_or((after_bracket, ""));
        let (_, _, port) = partition_at(rest, ':');
        (host.to_owned(), port.to_owned())
    } else {
        let (host, _, port) = partition_at(hostinfo, ':');
        (host.to_owned(), port.to_owned())
    };
    let port = if port.is_empty() { None } else { Some(port) };
    (hostname, port)
}

/// The lower-cased host of a netloc, mirroring `SplitResult.hostname`.
///
/// CPython lower-cases the host but keeps the IPv6 zone identifier as it is.
fn host_name(netloc: &str) -> String {
    let (hostname, _) = host_info(netloc);
    let (host, percent, zone) = partition_at(&hostname, '%');
    format!("{}{}{}", host.to_ascii_lowercase(), percent, zone)
}

/// The user information of a netloc, mirroring `SplitResult._userinfo`.
fn user_info(netloc: &str) -> (Option<String>, Option<String>) {
    let Some((userinfo, _)) = netloc.rsplit_once('@') else {
        return (None, None);
    };
    let (username, have_password, password) = partition_at(userinfo, ':');
    let password = if have_password.is_empty() {
        None
    } else {
        Some(password.to_owned())
    };
    (Some(username.to_owned()), password)
}

/// A normalized provider destination.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Destination {
    scheme: String,
    host: String,
    port: u16,
    path: String,
}

impl Destination {
    /// The canonical `scheme://host:port/path` form.
    #[must_use]
    pub fn normalized(&self) -> String {
        format!("{}://{}:{}{}", self.scheme, self.host, self.port, self.path)
    }

    /// Whether the destination names the loopback interface.
    #[must_use]
    pub fn is_loopback(&self) -> bool {
        is_loopback_host(&self.host)
    }

    /// Whether the destination uses TLS.
    #[must_use]
    pub fn is_tls(&self) -> bool {
        self.scheme == "https"
    }

    /// The destination scheme.
    #[must_use]
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// The destination host.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The destination port.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The destination path.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
}

/// Parse a provider endpoint.
///
/// The Python implementation accepts any object and refuses a non-string with
/// `EgressDeniedError`; the Rust port takes a `&str`.
pub fn parse_destination(value: &str) -> Result<Destination, ModelError> {
    let candidate = super::text::python_strip(value);
    if candidate.is_empty() || super::text::code_point_len(candidate) > MAX_ENDPOINT_LENGTH {
        return Err(ModelError::egress_denied(
            "the provider endpoint must be nonempty and bounded",
        ));
    }
    let parts = urlsplit(candidate)
        .map_err(|()| ModelError::egress_denied("the provider endpoint is not a valid URL"))?;
    // CPython reads the port while it splits the URL, so an unusable port is
    // reported before the scheme, the credentials and the query checks.
    let (_raw_host, raw_port) = host_info(&parts.netloc);
    let resolved_port = match raw_port {
        Some(port) => {
            if !port.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(ModelError::egress_denied(
                    "the provider endpoint is not a valid URL",
                ));
            }
            let parsed: u32 = port.parse().map_err(|_| {
                ModelError::egress_denied("the provider endpoint is not a valid URL")
            })?;
            if parsed > 65_535 {
                return Err(ModelError::egress_denied(
                    "the provider endpoint is not a valid URL",
                ));
            }
            parsed
        }
        None => {
            if parts.scheme.eq_ignore_ascii_case("https") {
                443
            } else {
                80
            }
        }
    };
    let scheme = parts.scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(ModelError::egress_denied(
            "the provider endpoint must use http or https",
        ));
    }
    let (username, password) = user_info(&parts.netloc);
    if username.is_some() || password.is_some() {
        return Err(ModelError::egress_denied(
            "the provider endpoint cannot contain credentials",
        ));
    }
    if parts.query.is_some() || parts.fragment.is_some() {
        return Err(ModelError::egress_denied(
            "the provider endpoint cannot contain a query or fragment",
        ));
    }
    let host = host_name(&parts.netloc);
    if host.is_empty() {
        return Err(ModelError::egress_denied(
            "the provider endpoint must contain a host",
        ));
    }
    if !(1..=65_535).contains(&resolved_port) {
        return Err(ModelError::egress_denied(
            "the provider endpoint port is out of range",
        ));
    }
    let path = normalize_path(if parts.path.is_empty() {
        "/"
    } else {
        &parts.path
    });
    Ok(Destination {
        scheme,
        host,
        port: resolved_port as u16,
        path,
    })
}

/// Normalize a URL path the way `posixpath.normpath` does.
#[must_use]
pub fn normalize_path(path: &str) -> String {
    if path.is_empty() {
        return ".".to_owned();
    }
    let initial_slashes = if path.starts_with('/') {
        if path.starts_with("//") && !path.starts_with("///") {
            2
        } else {
            1
        }
    } else {
        0
    };
    let mut components: Vec<&str> = Vec::new();
    for component in path.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if component != ".."
            || (initial_slashes == 0 && components.is_empty())
            || components.last() == Some(&"..")
        {
            components.push(component);
        } else if !components.is_empty() {
            components.pop();
        }
    }
    let mut normalized = "/".repeat(initial_slashes);
    normalized.push_str(&components.join("/"));
    if normalized.is_empty() {
        ".".to_owned()
    } else {
        normalized
    }
}

/// Normalize a provider endpoint to its canonical form.
pub fn normalized_destination(value: &str) -> Result<String, ModelError> {
    Ok(parse_destination(value)?.normalized())
}

/// Whether a host names the loopback interface.
#[must_use]
pub fn is_loopback_host(host: &str) -> bool {
    let normalized = host.to_ascii_lowercase();
    if LOOPBACK_HOSTS.contains(&normalized.as_str()) {
        return true;
    }
    IpAddr::from_str(&normalized)
        .map(|address| address.is_loopback())
        .unwrap_or(false)
}

/// Normalize a provider endpoint and apply the egress rules.
///
/// A loopback endpoint is always allowed. A plaintext endpoint is refused
/// unless it is a loopback host. A hosted endpoint needs explicit consent and
/// must be a member of the allow-list.
pub fn normalize_endpoint(
    value: &str,
    allowed_endpoints: &[String],
    consent: bool,
) -> Result<String, ModelError> {
    let destination = parse_destination(value)?;
    if destination.is_loopback() {
        return Ok(destination.normalized());
    }
    if !destination.is_tls() {
        return Err(ModelError::egress_denied(
            "plaintext provider endpoints are limited to loopback hosts",
        ));
    }
    if !consent {
        return Err(ModelError::egress_denied(
            "hosted provider endpoints require explicit egress consent",
        ));
    }
    let mut allowlist: Vec<String> = Vec::with_capacity(allowed_endpoints.len());
    for item in allowed_endpoints {
        allowlist.push(normalized_destination(item)?);
    }
    if !allowlist.contains(&destination.normalized()) {
        return Err(ModelError::egress_denied(
            "the hosted provider endpoint is not in the egress allowlist",
        ));
    }
    Ok(destination.normalized())
}
