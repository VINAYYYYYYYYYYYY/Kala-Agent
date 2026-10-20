//! Real local TLS and proxy evidence for the production provider transport.
//!
//! The source provider uses `urllib` with Python/system default trust: an
//! `SSLContext` created by `ssl.create_default_context()` (system store on
//! Windows/macOS, OpenSSL defaults plus `SSL_CERT_FILE`/`SSL_CERT_DIR` on
//! Unix). Certificate and hostname verification are always enabled. This
//! test starts an isolated local HTTPS server with synthetic test
//! certificates (never user credentials) and proves the production
//! `ReqwestTransport::new()` builder:
//!
//! * trusts a CA supplied through `SSL_CERT_FILE` (the same environment
//!   contract Python's OpenSSL defaults honour),
//! * rejects a server signed by an untrusted CA,
//! * rejects a trusted certificate whose SAN does not match the hostname,
//! * routes through `HTTPS_PROXY` and honours `NO_PROXY`.
//!
//! No live model, external provider, real credential or public network call
//! is made.

use std::path::PathBuf;
use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use kala_studio::providers::remote::{
    HttpTransport, RemoteRequest, RemoteResponse, ReqwestTransport, TransportError,
};

fn tls_data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/tls")
        .join(name)
}

fn load_certs(name: &str) -> Vec<CertificateDer<'static>> {
    let mut reader = std::io::BufReader::new(std::fs::File::open(tls_data(name)).unwrap());
    rustls_pemfile::certs(&mut reader)
        .unwrap()
        .into_iter()
        .map(CertificateDer::from)
        .collect()
}

fn load_key(name: &str) -> PrivateKeyDer<'static> {
    let mut reader = std::io::BufReader::new(std::fs::File::open(tls_data(name)).unwrap());
    let keys = rustls_pemfile::pkcs8_private_keys(&mut reader).unwrap();
    PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(keys.into_iter().next().unwrap()))
}

/// Serve one minimal HTTP/1.1 response over TLS for every accepted socket.
async fn spawn_tls_server(cert: &str, key: &str) -> u16 {
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(load_certs(cert), load_key(key))
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((stream, _peer)) = listener.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut stream) = acceptor.accept(stream).await else {
                    return;
                };
                let mut request = Vec::new();
                let mut chunk = [0u8; 4096];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    match stream.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(count) => request.extend_from_slice(&chunk[..count]),
                    }
                }
                let response = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
                let _ = stream.write_all(response).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    port
}

/// A tiny proxy that records the CONNECT target and answers 502.
async fn spawn_proxy() -> (u16, tokio::sync::oneshot::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let Ok((mut stream, _peer)) = listener.accept().await else {
            return;
        };
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    buffer.extend_from_slice(&chunk[..count]);
                    if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
            }
        }
        let text = String::from_utf8_lossy(&buffer).into_owned();
        let _ = sender.send(text);
        let _ = stream
            .write_all(
                b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await;
        let _ = stream.shutdown().await;
    });
    (port, receiver)
}

fn set_env(name: &str, value: &str) {
    // The test process owns this environment for the duration of the test.
    unsafe { std::env::set_var(name, value) };
}

fn request(url: &str) -> RemoteRequest {
    RemoteRequest {
        url: Box::leak(url.to_owned().into_boxed_str()),
        headers: vec![("Accept".to_owned(), "application/json".to_owned())],
        body: b"{}".to_vec(),
        timeout: std::time::Duration::from_secs(10),
    }
}

async fn post(transport: &ReqwestTransport, url: &str) -> Result<RemoteResponse, TransportError> {
    transport.post(request(url)).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn default_trust_and_https_proxy_environment_match_the_source() {
    for name in [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
        "NO_PROXY",
        "no_proxy",
        "SSL_CERT_DIR",
    ] {
        unsafe { std::env::remove_var(name) };
    }

    // The synthetic CA is trusted only through the OpenSSL-compatible
    // environment contract Python's default context honours.
    set_env("SSL_CERT_FILE", tls_data("ca.pem").to_str().unwrap());
    let localhost_port = spawn_tls_server("localhost.pem", "localhost.key").await;
    let wronghost_port = spawn_tls_server("wronghost.pem", "wronghost.key").await;

    let trusted = ReqwestTransport::new().unwrap();
    let url = format!("https://localhost:{localhost_port}/");
    let response = post(&trusted, &url).await.expect("trusted local CA");
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"{}");

    // The Windows branch's assembly (built-in roots disabled, explicit DER
    // roots) runs the same way on Linux against the synthetic CAs: the
    // supplied CA is trusted, a CA outside the supplied set is refused, and
    // certificate verification is never disabled.
    let mut trusted_env = std::collections::BTreeMap::new();
    trusted_env.insert(
        "SSL_CERT_FILE".to_owned(),
        tls_data("ca.pem").to_string_lossy().into_owned(),
    );
    let client = kala_studio::tls::add_der_roots(
        reqwest::Client::builder().no_proxy(),
        kala_studio::tls::environment_certificates(&trusted_env),
    )
    .build()
    .unwrap();
    let response = client
        .get(&url)
        .send()
        .await
        .expect("explicit DER root is trusted");
    assert_eq!(response.status(), 200);

    let mut untrusted_env = std::collections::BTreeMap::new();
    untrusted_env.insert(
        "SSL_CERT_FILE".to_owned(),
        tls_data("untrusted-ca.pem").to_string_lossy().into_owned(),
    );
    let client = kala_studio::tls::add_der_roots(
        reqwest::Client::builder().no_proxy(),
        kala_studio::tls::environment_certificates(&untrusted_env),
    )
    .build()
    .unwrap();
    assert!(
        client.get(&url).send().await.is_err(),
        "a CA outside the assembled root set must be refused"
    );

    // Hostname verification must remain enabled: the chain is trusted but
    // the certificate is valid only for wrong.example.invalid.
    let mismatch = post(&trusted, &format!("https://localhost:{wronghost_port}/")).await;
    assert!(matches!(mismatch, Err(TransportError::Network(_))));

    // A server signed by a CA outside the configured trust fails the
    // handshake; certificate verification is never disabled.
    set_env(
        "SSL_CERT_FILE",
        tls_data("untrusted-ca.pem").to_str().unwrap(),
    );
    let untrusted = ReqwestTransport::new().unwrap();
    let failure = post(&untrusted, &url).await;
    assert!(matches!(failure, Err(TransportError::Network(_))));

    // Python's urllib consults HTTPS_PROXY/https_proxy for HTTPS requests.
    // The native system-proxy matcher must do the same; capture the CONNECT.
    set_env("SSL_CERT_FILE", tls_data("ca.pem").to_str().unwrap());
    let (proxy_port, captured) = spawn_proxy().await;
    set_env("HTTPS_PROXY", &format!("http://127.0.0.1:{proxy_port}"));
    let proxied = ReqwestTransport::new().unwrap();
    let _ = post(&proxied, "https://example.invalid/").await;
    let captured = tokio::time::timeout(std::time::Duration::from_secs(5), captured)
        .await
        .expect("proxy captured a request")
        .expect("proxy channel");
    assert!(
        captured.starts_with("CONNECT example.invalid:443 "),
        "unexpected proxy request: {captured:?}"
    );

    // `NO_PROXY` bypasses the proxy and reaches the trusted local server.
    set_env("NO_PROXY", "localhost");
    let direct = ReqwestTransport::new().unwrap();
    let response = post(&direct, &url).await.expect("NO_PROXY direct request");
    assert_eq!(response.status, 200);

    // Python's lowercase `https_proxy` wins over an uppercase `HTTPS_PROXY`,
    // regardless of environment order, and an empty lowercase value removes
    // the uppercase one.
    unsafe { std::env::remove_var("NO_PROXY") };
    let upper_port = proxy_port;
    let (lower_port, lower_captured) = spawn_proxy().await;
    set_env("HTTPS_PROXY", &format!("http://127.0.0.1:{upper_port}"));
    set_env("https_proxy", &format!("http://127.0.0.1:{lower_port}"));
    let lowercase_wins = ReqwestTransport::new().unwrap();
    let _ = post(&lowercase_wins, "https://example.invalid/").await;
    let captured = tokio::time::timeout(std::time::Duration::from_secs(5), lower_captured)
        .await
        .expect("the lowercase proxy captured the request")
        .expect("proxy channel");
    assert!(captured.starts_with("CONNECT example.invalid:443 "));

    let (empty_port, empty_captured) = spawn_proxy().await;
    set_env("HTTPS_PROXY", &format!("http://127.0.0.1:{empty_port}"));
    set_env("https_proxy", "");
    let removed = ReqwestTransport::new().unwrap();
    let response = post(&removed, &url)
        .await
        .expect("empty lowercase removes uppercase");
    assert_eq!(response.status, 200);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(200), empty_captured)
            .await
            .is_err(),
        "the removed uppercase proxy must not be contacted"
    );

    // Python's urllib has no `ALL_PROXY` fallback for HTTPS requests: the
    // request goes to the fixed endpoint directly.
    unsafe { std::env::remove_var("HTTPS_PROXY") };
    unsafe { std::env::remove_var("https_proxy") };
    let (all_port, all_captured) = spawn_proxy().await;
    set_env("ALL_PROXY", &format!("http://127.0.0.1:{all_port}"));
    let all_proxy = ReqwestTransport::new().unwrap();
    let response = post(&all_proxy, &url).await.expect("ALL_PROXY is ignored");
    assert_eq!(response.status, 200);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(200), all_captured)
            .await
            .is_err(),
        "ALL_PROXY must not be used for an HTTPS request"
    );

    // Avoid leaking the synthetic trust setting into other test binaries.
    unsafe { std::env::remove_var("SSL_CERT_FILE") };
    unsafe { std::env::remove_var("HTTPS_PROXY") };
    unsafe { std::env::remove_var("https_proxy") };
    unsafe { std::env::remove_var("ALL_PROXY") };
    unsafe { std::env::remove_var("NO_PROXY") };
}
