//! Python `urllib` proxy environment semantics for the remote provider.
//!
//! `urllib.request.build_opener` builds a `ProxyHandler` per request, which
//! calls `getproxies()` → `getproxies_environment()` and consults
//! `proxy_bypass(host)` before a proxy is used. The behaviours that differ
//! from a naive implementation, and that this module reproduces exactly:
//!
//! * any `<scheme>_proxy` name is matched case-insensitively after the
//!   scheme part, but a **lowercase** `*_proxy` variable always wins over an
//!   uppercase spelling, in a second pass that also lets an empty lowercase
//!   value remove an uppercase one;
//! * `ALL_PROXY`/`all_proxy` is *ignored* by `urllib` for HTTPS requests
//!   (there is no `all` fallback), so this module never falls back to it;
//! * uppercase `HTTP_PROXY` is dropped when `REQUEST_METHOD` is present (the
//!   CGI hijack guard); a lowercase `http_proxy` is still honoured;
//! * `NO_PROXY` is not a scheme: it lives in the same map under the key
//!   `no` and is applied by `proxy_bypass_environment` with its exact
//!   suffix rules (`*`, comma list, leading dots stripped, port stripped).
//!
//! Only the documented `https` destination of the fixed provider endpoints
//! is ever looked up, so an operator cannot direct a request to another
//! scheme through this code. No proxy URL is echoed in errors.

use std::collections::BTreeMap;

use crate::pytext;

/// Extract `<scheme>` from an environment variable name ending in `_proxy`
/// under CPython's two-pass rules. `lowercase_pass` selects the second pass,
/// which only accepts a literal lowercase `_proxy` suffix.
fn proxy_name_part(name: &str, lowercase_pass: bool) -> Option<String> {
    let bytes = name.as_bytes();
    if bytes.len() <= 5 {
        return None;
    }
    if bytes[bytes.len() - 6] != b'_' {
        return None;
    }
    let suffix = &name[bytes.len() - 5..];
    if lowercase_pass {
        if suffix != "proxy" {
            return None;
        }
    } else if !suffix.eq_ignore_ascii_case("proxy") {
        return None;
    }
    Some(name[..name.len() - 6].to_lowercase())
}

/// CPython `urllib.request.getproxies_environment()`.
#[must_use]
pub fn environment_proxies(environ: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut proxies = BTreeMap::new();
    for (name, value) in environ {
        let Some(proxy_name) = proxy_name_part(name, false) else {
            continue;
        };
        if !value.is_empty() {
            proxies.insert(proxy_name, value.clone());
        }
    }
    // CVE-2016-1000110: a CGI `HTTP_PROXY` may come from the client, so the
    // uppercase spelling is forgotten before the lowercase pass.
    if environ.contains_key("REQUEST_METHOD") {
        proxies.remove("http");
    }
    for (name, value) in environ {
        let Some(proxy_name) = proxy_name_part(name, true) else {
            continue;
        };
        if value.is_empty() {
            proxies.remove(&proxy_name);
        } else {
            proxies.insert(proxy_name, value.clone());
        }
    }
    proxies
}

/// CPython `urllib.parse._splitport`.
fn split_port(host: &str) -> (&str, Option<&str>) {
    // `_portprog = re.compile('(.*):([0-9]*)', re.DOTALL)` with `fullmatch`:
    // the last colon followed by zero or more digits splits, and an empty
    // port yields `(host, None)` with the colon removed.
    let Some(colon) = host.rfind(':') else {
        return (host, None);
    };
    let head = &host[..colon];
    let port = &host[colon + 1..];
    if !port.bytes().all(|byte| byte.is_ascii_digit()) {
        return (host, None);
    }
    if port.is_empty() {
        (head, None)
    } else {
        (head, Some(port))
    }
}

/// CPython `urllib.request.proxy_bypass_environment`.
#[must_use]
pub fn proxy_bypass(host: &str, proxies: &BTreeMap<String, String>) -> bool {
    // The `no_proxy` variable is stored under the scheme name `no`.
    let Some(no_proxy) = proxies.get("no") else {
        return false;
    };
    if no_proxy == "*" {
        return true;
    }
    let host = host.to_lowercase();
    let (hostonly, _port) = split_port(&host);
    for name in no_proxy.split(',') {
        let name = pytext::trim(name);
        if name.is_empty() {
            continue;
        }
        let name = name.trim_start_matches('.').to_lowercase();
        if hostonly == name || host == name {
            return true;
        }
        let name = format!(".{name}");
        if hostonly.ends_with(&name) || host.ends_with(&name) {
            return true;
        }
    }
    false
}

/// The proxy URL `urllib` would use for one scheme and host, or `None` for a
/// direct request.
///
/// Only the requested scheme is consulted, exactly like `ProxyHandler`: there
/// is no `ALL_PROXY` fallback. `REQUEST_METHOD`-driven HTTP suppression and
/// the lowercase-over-uppercase rule come from [`environment_proxies`].
#[must_use]
pub fn proxy_for(environ: &BTreeMap<String, String>, scheme: &str, host: &str) -> Option<String> {
    let proxies = environment_proxies(environ);
    if proxy_bypass(host, &proxies) {
        return None;
    }
    proxies.get(scheme).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environ(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn lowercase_wins_over_uppercase_in_both_directions() {
        let proxies = environment_proxies(&environ(&[
            ("HTTP_PROXY", "http://upper:1"),
            ("http_proxy", "http://lower:2"),
            ("HTTPS_PROXY", "http://upper-secure:3"),
        ]));
        assert_eq!(
            proxies.get("http").map(String::as_str),
            Some("http://lower:2")
        );
        assert_eq!(
            proxies.get("https").map(String::as_str),
            Some("http://upper-secure:3")
        );

        // An empty lowercase value removes an uppercase spelling.
        let proxies = environment_proxies(&environ(&[
            ("HTTPS_PROXY", "http://upper:3"),
            ("https_proxy", ""),
        ]));
        assert_eq!(proxies.get("https"), None);
    }

    #[test]
    fn all_proxy_is_recorded_but_never_selected() {
        let proxies = environment_proxies(&environ(&[
            ("ALL_PROXY", "http://all:9"),
            ("all_proxy", "http://all-lower:8"),
        ]));
        assert_eq!(
            proxies.get("all").map(String::as_str),
            Some("http://all-lower:8")
        );
        assert_eq!(
            proxy_for(
                &environ(&[("ALL_PROXY", "http://all:9")]),
                "https",
                "example.invalid"
            ),
            None
        );
    }

    #[test]
    fn cgi_suppresses_only_uppercase_http_proxy() {
        let uppercase = environ(&[("HTTP_PROXY", "http://upper:1"), ("REQUEST_METHOD", "GET")]);
        assert_eq!(environment_proxies(&uppercase).get("http"), None);
        let lowercase = environ(&[
            ("HTTP_PROXY", "http://upper:1"),
            ("http_proxy", "http://lower:2"),
            ("REQUEST_METHOD", "GET"),
        ]);
        assert_eq!(
            environment_proxies(&lowercase)
                .get("http")
                .map(String::as_str),
            Some("http://lower:2")
        );
    }

    #[test]
    fn no_proxy_matching_follows_cpython() {
        let proxies = environment_proxies(&environ(&[(
            "NO_PROXY",
            "LocalHost, .example.com,1.2.3.4:9",
        )]));
        assert!(proxy_bypass("LOCALHOST", &proxies));
        assert!(proxy_bypass("api.example.com", &proxies));
        assert!(proxy_bypass("example.com", &proxies));
        assert!(!proxy_bypass("notexample.com", &proxies));
        assert!(proxy_bypass("1.2.3.4:9", &proxies));
        assert!(!proxy_bypass("1.2.3.4:10", &proxies));

        let all = environment_proxies(&environ(&[("no_proxy", "*")]));
        assert!(proxy_bypass("anything.invalid", &all));

        let empty = environment_proxies(&environ(&[("no_proxy", "")]));
        assert!(!proxy_bypass("anything.invalid", &empty));
    }

    #[test]
    fn proxy_selection_uses_the_matching_scheme_and_bypass() {
        let base = environ(&[
            ("HTTPS_PROXY", "http://proxy:3128"),
            ("NO_PROXY", "internal.example"),
        ]);
        assert_eq!(
            proxy_for(&base, "https", "api.openai.com").as_deref(),
            Some("http://proxy:3128")
        );
        assert_eq!(proxy_for(&base, "https", "internal.example"), None);
        assert_eq!(proxy_for(&base, "http", "api.openai.com"), None);
    }

    #[test]
    fn split_port_matches_cpython_examples() {
        assert_eq!(split_port("host"), ("host", None));
        assert_eq!(split_port("host:80"), ("host", Some("80")));
        assert_eq!(split_port("host:"), ("host", None));
        assert_eq!(split_port("[::1]:443"), ("[::1]", Some("443")));
        assert_eq!(split_port("[::1]"), ("[::1]", None));
        assert_eq!(split_port("a:1:2"), ("a:1", Some("2")));
        assert_eq!(split_port("host:x"), ("host:x", None));
    }
}
