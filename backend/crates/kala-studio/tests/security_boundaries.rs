//! Real-socket boundary tests: Host, Origin, bearer, body caps, static paths
//! and security headers with first-error precedence.

mod common;

use std::collections::HashMap;
use std::fs;

use common::{RequestSpec, TOKEN, TestServer};

const SECURITY: &[(&str, &str)] = kala_studio::server::SECURITY_HEADERS;

fn spec(method_body: Option<&[u8]>) -> RequestSpec {
    let mut spec = RequestSpec::default();
    spec.headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    if let Some(body) = method_body {
        spec.headers
            .push(("content-type".to_owned(), "application/json".to_owned()));
        spec.body = Some(body.to_vec());
    }
    spec
}

fn assert_secure(response: &common::TestResponse) {
    for (name, value) in SECURITY {
        let matching: Vec<&String> = response
            .headers
            .iter()
            .filter(|(key, _)| key == name)
            .map(|(_, value)| value)
            .collect();
        assert_eq!(matching.len(), 1, "security header {name} count");
        assert_eq!(matching[0], value, "security header {name} value");
    }
    for absent in ["server", "x-powered-by", "set-cookie"] {
        assert!(
            response.header(absent).is_none(),
            "header {absent} must be absent"
        );
    }
}

struct Fixture {
    server: TestServer,
    _work: tempfile::TempDir,
}

async fn fixture(limit: usize) -> Fixture {
    let work = tempfile::tempdir().unwrap();
    let static_dir = work.path().join("static");
    fs::create_dir(&static_dir).unwrap();
    fs::write(static_dir.join("index.html"), "<html>spa</html>").unwrap();
    fs::write(static_dir.join("asset.txt"), "asset\n").unwrap();
    let server = TestServer::start(
        work.path().join("data"),
        TOKEN,
        limit,
        Some(static_dir),
        HashMap::new(),
    )
    .await;
    Fixture {
        server,
        _work: work,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_guard_and_origin_matching() {
    let fixture = fixture(16_384).await;
    let addr = fixture.server.addr;
    let body = br#"{"kind":"box"}"#;

    let mut bad_host = spec(Some(body));
    bad_host
        .headers
        .push(("Host".to_owned(), "evil.example".to_owned()));
    let response = common::request(addr, "GET", "/api/studio/health", bad_host).await;
    assert_eq!(response.status, 421);
    assert_eq!(
        response.body_text(),
        "{\"code\":\"invalid_host\",\"details\":[],\"message\":\"the studio request host is not a loopback address\"}"
    );
    assert_secure(&response);

    let mut duplicate_host = spec(None);
    duplicate_host
        .headers
        .push(("Host".to_owned(), "127.0.0.1".to_owned()));
    duplicate_host
        .headers
        .push(("Host".to_owned(), "127.0.0.1".to_owned()));
    let response = common::request(addr, "GET", "/api/studio/health", duplicate_host).await;
    assert_eq!(response.status, 421);

    let mut wildcard_host = spec(None);
    wildcard_host
        .headers
        .push(("Host".to_owned(), "0.0.0.0".to_owned()));
    assert_eq!(
        common::request(addr, "GET", "/api/studio/health", wildcard_host)
            .await
            .status,
        421
    );

    let mut bad_port = spec(None);
    bad_port
        .headers
        .push(("Host".to_owned(), "127.0.0.1:70000".to_owned()));
    assert_eq!(
        common::request(addr, "GET", "/api/studio/health", bad_port)
            .await
            .status,
        421
    );

    // Same-origin with default port.
    let mut same = spec(None);
    same.headers
        .push(("Origin".to_owned(), "http://127.0.0.1".to_owned()));
    let response = common::request(addr, "GET", "/api/studio/health", same).await;
    assert_eq!(response.status, 200);
    assert_secure(&response);

    // Same-origin with an explicit matching authority port.
    let mut same_port = spec(None);
    same_port
        .headers
        .push(("Host".to_owned(), "127.0.0.1:8765".to_owned()));
    same_port
        .headers
        .push(("Origin".to_owned(), "http://127.0.0.1:8765".to_owned()));
    assert_eq!(
        common::request(addr, "GET", "/api/studio/health", same_port)
            .await
            .status,
        200
    );

    for origin in [
        "http://evil.example",
        "http://127.0.0.1:1234",
        "https://127.0.0.1",
        "null",
        "http://user@127.0.0.1",
        "http://127.0.0.1/path",
    ] {
        let mut cross = spec(None);
        cross.headers.push(("Origin".to_owned(), origin.to_owned()));
        let response = common::request(addr, "GET", "/api/studio/health", cross).await;
        assert_eq!(response.status, 403, "origin {origin}");
        assert_secure(&response);
    }

    let mut duplicate_origin = spec(None);
    duplicate_origin
        .headers
        .push(("Origin".to_owned(), "http://127.0.0.1".to_owned()));
    duplicate_origin
        .headers
        .push(("Origin".to_owned(), "http://127.0.0.1".to_owned()));
    assert_eq!(
        common::request(addr, "GET", "/api/studio/health", duplicate_origin)
            .await
            .status,
        403
    );

    let mut ipv6 = spec(None);
    ipv6.headers
        .push(("Host".to_owned(), "[::1]:8765".to_owned()));
    ipv6.headers
        .push(("Origin".to_owned(), "http://[::1]:8765".to_owned()));
    assert_eq!(
        common::request(addr, "GET", "/api/studio/health", ipv6)
            .await
            .status,
        200
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bearer_and_precedence() {
    let fixture = fixture(16_384).await;
    let addr = fixture.server.addr;
    let body = br#"{"kind":"box"}"#;

    let mut anonymous = RequestSpec::default();
    anonymous
        .headers
        .push(("content-type".to_owned(), "application/json".to_owned()));
    anonymous.body = Some(body.to_vec());
    let response = common::request(addr, "POST", "/api/studio/preview", anonymous).await;
    assert_eq!(response.status, 401);
    assert_eq!(
        response.header("www-authenticate"),
        Some("Bearer realm=\"kala-studio\"")
    );
    assert_eq!(
        response.body_text(),
        "{\"code\":\"unauthorized\",\"details\":[],\"message\":\"a valid bearer token is required for this request\"}"
    );
    assert_secure(&response);

    // Wrong token.
    let mut wrong = spec(Some(body));
    wrong.headers[0] = (
        "Authorization".to_owned(),
        "Bearer wrong-token-0123456789abcdef".to_owned(),
    );
    assert_eq!(
        common::request(addr, "POST", "/api/studio/preview", wrong)
            .await
            .status,
        401
    );

    // Duplicate authorization headers.
    let mut duplicate = spec(Some(body));
    duplicate
        .headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    assert_eq!(
        common::request(addr, "POST", "/api/studio/preview", duplicate)
            .await
            .status,
        401
    );

    // Lower-case scheme and surrounding spaces in the credential.
    let mut lower = RequestSpec::default();
    lower
        .headers
        .push(("Authorization".to_owned(), format!("bearer   {TOKEN}   ")));
    lower
        .headers
        .push(("content-type".to_owned(), "application/json".to_owned()));
    lower.body = Some(body.to_vec());
    assert_eq!(
        common::request(addr, "POST", "/api/studio/preview", lower)
            .await
            .status,
        200
    );

    // GET is not a mutating method and needs no token.
    assert_eq!(
        common::request(addr, "GET", "/api/studio/health", RequestSpec::default())
            .await
            .status,
        200
    );

    // Cross-origin wins over a missing token.
    // A client that disconnects mid-body does not take the server down.
    {
        use tokio::io::AsyncWriteExt;
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(
                b"POST /api/studio/preview HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 1000\r\nContent-Type: application/json\r\n\r\n{\"kind\":",
            )
            .await
            .unwrap();
        drop(stream);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(
        common::request(addr, "GET", "/api/studio/health", RequestSpec::default())
            .await
            .status,
        200
    );

    let mut cross_no_token = RequestSpec::default();
    cross_no_token
        .headers
        .push(("Origin".to_owned(), "http://evil.example".to_owned()));
    assert_eq!(
        common::request(addr, "POST", "/api/studio/preview", cross_no_token)
            .await
            .status,
        403
    );

    // Oversized body wins over origin and token.
    let mut oversized_no_token = RequestSpec::default();
    oversized_no_token
        .headers
        .push(("Origin".to_owned(), "http://evil.example".to_owned()));
    oversized_no_token
        .headers
        .push(("content-type".to_owned(), "application/json".to_owned()));
    oversized_no_token.body = Some(vec![b'x'; 20_000]);
    let response = common::request(addr, "POST", "/api/studio/preview", oversized_no_token).await;
    assert_eq!(response.status, 413);
    assert_secure(&response);

    // Forwarded headers never change the trusted Host/Origin decision.
    let mut forwarded = spec(None);
    forwarded
        .headers
        .push(("X-Forwarded-Host".to_owned(), "evil.example".to_owned()));
    forwarded
        .headers
        .push(("Forwarded".to_owned(), "host=evil.example".to_owned()));
    assert_eq!(
        common::request(addr, "GET", "/api/studio/health", forwarded)
            .await
            .status,
        200
    );

    // Bad host wins over an oversized body.
    let mut host_first = RequestSpec::default();
    host_first
        .headers
        .push(("Host".to_owned(), "evil.example".to_owned()));
    host_first.body = Some(vec![b'x'; 20_000]);
    assert_eq!(
        common::request(addr, "POST", "/api/studio/preview", host_first)
            .await
            .status,
        421
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn body_caps_are_declared_and_streamed() {
    let fixture = fixture(1_024).await;
    let addr = fixture.server.addr;

    let mut exact = spec(None);
    exact.body = Some(vec![b' '; 1_024]);
    let response = common::request(addr, "POST", "/api/studio/preview", exact).await;
    assert_eq!(response.status, 422);

    let mut over = spec(None);
    over.body = Some(vec![b' '; 1_025]);
    let response = common::request(addr, "POST", "/api/studio/preview", over).await;
    assert_eq!(response.status, 413);
    assert_eq!(
        response.body_text(),
        "{\"code\":\"payload_too_large\",\"details\":[\"the request body must not exceed 1024 bytes\"],\"message\":\"the request body exceeds the configured size limit\"}"
    );
    assert_secure(&response);

    let mut chunked_exact = spec(None);
    chunked_exact.chunked = Some(vec![vec![b' '; 700], vec![b' '; 324]]);
    assert_eq!(
        common::request(addr, "POST", "/api/studio/preview", chunked_exact)
            .await
            .status,
        422
    );

    let mut chunked_over = spec(None);
    chunked_over.chunked = Some(vec![vec![b' '; 700], vec![b' '; 325]]);
    let response = common::request(addr, "POST", "/api/studio/preview", chunked_over).await;
    assert_eq!(response.status, 413);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_read_only_routes_carry_their_own_bearer_gate() {
    let fixture = fixture(16_384).await;
    let addr = fixture.server.addr;

    // The source's `_require_bearer` gate: the journal history, the design
    // views and the CAD provider probe are not readable by a same-origin
    // page that never proved it holds the launch token.
    for path in [
        "/api/studio/journal",
        "/api/studio/journal/verify",
        "/api/studio/providers/cad",
    ] {
        let response = common::request(addr, "GET", path, RequestSpec::default()).await;
        assert_eq!(response.status, 401, "{path}");
        assert_eq!(
            response.header("www-authenticate"),
            Some("Bearer"),
            "{path}"
        );
        assert_eq!(response.json()["code"], "unauthorized", "{path}");
        assert_secure(&response);

        // The right token passes the gate; the route then answers (the
        // offline stub has no worker route for these, so it reports 501).
        let mut authorised = RequestSpec::default();
        authorised
            .headers
            .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
        let response = common::request(addr, "GET", path, authorised).await;
        assert_ne!(response.status, 401, "{path}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn routing_static_and_methods() {
    let fixture = fixture(16_384).await;
    let addr = fixture.server.addr;

    let response = common::request(addr, "GET", "/api/studio/not-a-route", spec(None)).await;
    assert_eq!(response.status, 404);
    assert_eq!(
        response.body_text(),
        "{\"code\":\"not_found\",\"message\":\"the requested resource does not exist\",\"details\":[]}"
    );

    let response = common::request(addr, "DELETE", "/api/studio/preview", spec(None)).await;
    assert_eq!(response.status, 405);
    assert_eq!(
        response.body_text(),
        "{\"code\":\"method_not_allowed\",\"message\":\"the requested method is not allowed for this resource\",\"details\":[]}"
    );

    // HEAD is not implicitly added to GET routes.
    let response = common::request(addr, "HEAD", "/api/studio/health", spec(None)).await;
    assert_eq!(response.status, 405);
    assert_eq!(response.header("content-length"), Some("108"));
    assert!(response.body.is_empty());

    // API reservation: the SPA is never served under /api.
    let response = common::request(addr, "GET", "/api/not-a-route", spec(None)).await;
    assert_eq!(response.status, 404);
    assert_eq!(response.header("content-type"), Some("application/json"));

    // Static index, asset, SPA fallback, and query strings.
    for (path, expected) in [
        ("/", "<html>spa</html>"),
        ("/missing", "<html>spa</html>"),
        ("/asset.txt", "asset\n"),
        ("/asset.txt?cache=1", "asset\n"),
    ] {
        let response = common::request(addr, "GET", path, spec(None)).await;
        assert_eq!(response.status, 200, "path {path}");
        assert_eq!(response.body_text(), expected, "path {path}");
        assert_secure(&response);
    }

    let response = common::request(addr, "HEAD", "/asset.txt", spec(None)).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.header("content-length"), Some("6"));
    assert!(response.body.is_empty());
    assert_secure(&response);

    // Non-GET static paths fall through to the API boundary.
    assert_eq!(
        common::request(addr, "POST", "/asset.txt", spec(None))
            .await
            .status,
        404
    );
    let anonymous = RequestSpec {
        body: Some(b"{}".to_vec()),
        ..RequestSpec::default()
    };
    assert_eq!(
        common::request(addr, "POST", "/asset.txt", anonymous)
            .await
            .status,
        401
    );

    // Starlette redirect_slashes: known routes redirect 307 before dispatch.
    let response = common::request(addr, "GET", "/api/studio/health/", spec(None)).await;
    assert_eq!(response.status, 307);
    assert_eq!(
        response.header("location"),
        Some("http://127.0.0.1/api/studio/health")
    );
    assert_eq!(response.header("content-length"), Some("0"));
    assert_eq!(response.header("content-type"), None);
    assert!(response.body.is_empty());
    assert_secure(&response);
    let response = common::request(addr, "POST", "/api/studio/preview/", spec(None)).await;
    assert_eq!(response.status, 307);
    assert_eq!(
        response.header("location"),
        Some("http://127.0.0.1/api/studio/preview")
    );
    let response = common::request(addr, "GET", "/api/studio/health/?x=1", spec(None)).await;
    assert_eq!(
        response.header("location"),
        Some("http://127.0.0.1/api/studio/health?x=1")
    );
    assert_eq!(
        common::request(addr, "GET", "/api/studio/not-a-route/", spec(None))
            .await
            .status,
        404
    );

    // Percent-encoded API path is decoded for routing but reserved in static.
    assert_eq!(
        common::request(addr, "GET", "/%61pi/studio/health", spec(None))
            .await
            .status,
        200
    );

    // Traversal never escapes the root; missing after normalization falls
    // back to the SPA index.
    let response = common::request(addr, "GET", "/../../etc/passwd", spec(None)).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.body_text(), "<html>spa</html>");

    let response = common::request(addr, "GET", "/%2e%2e/%2e%2e/etc/passwd", spec(None)).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.body_text(), "<html>spa</html>");

    #[cfg(unix)]
    {
        let outside = fixture._work.path().join("outside.txt");
        fs::write(&outside, "outside").unwrap();
        std::os::unix::fs::symlink(&outside, fixture._work.path().join("static/escape")).unwrap();
        let response = common::request(addr, "GET", "/escape", spec(None)).await;
        assert_eq!(response.status, 404);
        assert_eq!(
            response.body_text(),
            "{\"code\":\"not_found\",\"details\":[],\"message\":\"the requested resource does not exist\"}"
        );
    }
}
