//! Solid Edge provider tests against an in-process mock bridge.
//!
//! The mock implements the bridge's authenticated health contract only. It is
//! not Solid Edge, and no test here is evidence about a Windows host.

mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;

use kala_cad_providers::{
    CadProvider, CadProviderUnavailableError, NOT_REACHABLE_MESSAGE, SolidEdgeProvider,
    SolidEdgeProviderOptions,
};
use support::{CannedResponse, MockBridge, health_body, pad_script};

const TOKEN: &str = "token-0123456789abcdef";

fn token_file(directory: &std::path::Path, contents: &str) -> PathBuf {
    let path = directory.join("bridge.token");
    std::fs::write(&path, contents).expect("write the token file");
    path
}

fn bridge_provider(url: &str, token: Option<PathBuf>) -> SolidEdgeProvider {
    SolidEdgeProvider::new(SolidEdgeProviderOptions {
        platform: Some("win32".to_owned()),
        environ: Some(BTreeMap::new()),
        url: Some(url.to_owned()),
        token_file: token,
        timeout_seconds: Some(2.0),
        probe_cache_seconds: Some(30.0),
    })
    .expect("a valid provider")
}

#[test]
fn a_reachable_bridge_is_probed_authenticated_and_still_refuses_a_build() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(
        200,
        &health_body(true, true, "2024", ""),
    ));
    let mut provider = bridge_provider(&bridge.url(), Some(token_file(directory.path(), TOKEN)));
    let availability = provider.probe();
    assert!(availability.is_available());
    assert_eq!(availability.version(), Some("2024"));
    assert_eq!(availability.reason(), None);

    let requests = bridge.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, "/v1/health");
    assert_eq!(
        requests[0].header("authorization"),
        Some(format!("Bearer {TOKEN}").as_str())
    );
    assert!(requests[0].header("origin").is_none());

    let error = provider
        .build(&pad_script())
        .expect_err("this milestone has no native executor");
    assert_eq!(
        error.message(),
        "the Solid Edge bridge is reachable but this Studio worker has no verified \
         native build executor; no Solid Edge build was attempted"
    );
    assert_eq!(error.op(), Some("bridge.build"));
    // The probe is cached, so the refusal did not open a second connection.
    assert_eq!(bridge.requests().len(), 1);
}

#[test]
fn a_rejected_token_is_reported_without_leaking_it() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(
        401,
        r#"{"code":"invalid_request","message":"authentication failed"}"#,
    ));
    let mut provider = bridge_provider(&bridge.url(), Some(token_file(directory.path(), TOKEN)));
    let availability = provider.probe();
    let reason = availability.reason().expect("a reason");
    assert!(!reason.contains(TOKEN), "the token leaked: {reason}");
    assert_eq!(
        reason,
        format!(
            "the Solid Edge bridge at {} did not answer a valid health probe",
            bridge.url()
        )
    );
}

#[test]
fn a_redirect_is_refused_not_followed() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(302, "").with_headers(vec![(
        "Location".to_owned(),
        "http://example.com/".to_owned(),
    )]));
    let mut provider = bridge_provider(&bridge.url(), Some(token_file(directory.path(), TOKEN)));
    let availability = provider.probe();
    assert!(!availability.is_available());
    assert_eq!(
        availability.reason(),
        Some(
            format!(
                "the Solid Edge bridge at {} did not answer a valid health probe",
                bridge.url()
            )
            .as_str()
        )
    );
    assert_eq!(bridge.requests().len(), 1);
}

#[test]
fn a_non_protocol_health_document_is_refused() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    for body in [
        "not json".to_owned(),
        r#"{"protocol_version":2,"bridge_version":"0.1.0","platform":"win32","supported":true,"available":true,"busy":false}"#.to_owned(),
        r#"{"protocol_version":1,"bridge_version":"0.1.0","platform":"win32","supported":true,"available":true,"busy":false,"unexpected":true}"#.to_owned(),
        r#"{"protocol_version":1,"platform":"win32","supported":true,"available":true,"busy":false}"#.to_owned(),
    ] {
        let bridge = MockBridge::start();
        bridge.queue(CannedResponse::json(200, &body));
        let mut provider = bridge_provider(&bridge.url(), Some(token_file(directory.path(), TOKEN)));
        let availability = provider.probe();
        assert!(!availability.is_available(), "{body}");
        assert_eq!(
            availability.reason(),
            Some(format!(
                "the Solid Edge bridge at {} did not answer a valid health probe",
                bridge.url()
            ).as_str()),
            "{body}"
        );
    }
}

#[test]
fn the_health_detail_is_the_reason_only_when_it_is_not_blank() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let cases = [
        (
            health_body(false, true, "2024", "Solid Edge is not running"),
            Some("Solid Edge is not running"),
        ),
        (
            health_body(true, false, "2024", ""),
            Some("the bridge reports no native build capability"),
        ),
        (
            health_body(false, true, "2024", "   "),
            Some("the bridge reports no native build capability"),
        ),
    ];
    for (body, expected) in cases {
        let bridge = MockBridge::start();
        bridge.queue(CannedResponse::json(200, &body));
        let mut provider =
            bridge_provider(&bridge.url(), Some(token_file(directory.path(), TOKEN)));
        let availability = provider.probe();
        assert!(!availability.is_available());
        assert_eq!(availability.reason(), expected);
        assert_eq!(availability.version(), Some("2024"));
    }
}

#[test]
fn the_probe_is_cached_and_a_zero_cache_probes_again() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(
        200,
        &health_body(true, true, "2024", ""),
    ));
    let mut cached = bridge_provider(&bridge.url(), Some(token_file(directory.path(), TOKEN)));
    assert!(cached.probe().is_available());
    assert!(cached.probe().is_available());
    assert_eq!(bridge.requests().len(), 1);

    bridge.queue(CannedResponse::json(
        200,
        &health_body(true, true, "2024", ""),
    ));
    bridge.queue(CannedResponse::json(
        200,
        &health_body(true, true, "2024", ""),
    ));
    let mut fresh = SolidEdgeProvider::new(SolidEdgeProviderOptions {
        platform: Some("win32".to_owned()),
        environ: Some(BTreeMap::new()),
        url: Some(bridge.url()),
        token_file: Some(directory.path().join("bridge.token")),
        timeout_seconds: Some(2.0),
        probe_cache_seconds: Some(0.0),
    })
    .expect("a valid provider");
    assert!(fresh.probe().is_available());
    assert!(fresh.probe().is_available());
    assert_eq!(bridge.requests().len(), 3);
}

#[test]
fn the_token_file_sources_are_resolved_like_python() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let bridge = MockBridge::start();
    // Explicit token file wins.
    bridge.queue(CannedResponse::json(
        200,
        &health_body(true, true, "2024", ""),
    ));
    let explicit_directory = directory.path().join("explicit");
    std::fs::create_dir_all(&explicit_directory).expect("the directory");
    let explicit = token_file(&explicit_directory, TOKEN);
    let mut provider = SolidEdgeProvider::new(SolidEdgeProviderOptions {
        platform: Some("win32".to_owned()),
        environ: Some(BTreeMap::new()),
        url: Some(bridge.url()),
        token_file: Some(explicit),
        timeout_seconds: Some(2.0),
        probe_cache_seconds: Some(0.0),
    })
    .expect("a valid provider");
    assert!(provider.probe().is_available());

    // Environment variable next.
    bridge.queue(CannedResponse::json(
        200,
        &health_body(true, true, "2024", ""),
    ));
    let mut environment = BTreeMap::new();
    environment.insert(
        "KALA_SOLIDEDGE_TOKEN_FILE".to_owned(),
        token_file(directory.path(), TOKEN)
            .to_string_lossy()
            .into_owned(),
    );
    let mut provider = SolidEdgeProvider::new(SolidEdgeProviderOptions {
        platform: Some("win32".to_owned()),
        environ: Some(environment),
        url: Some(bridge.url()),
        token_file: None,
        timeout_seconds: Some(2.0),
        probe_cache_seconds: Some(0.0),
    })
    .expect("a valid provider");
    assert!(provider.probe().is_available());

    // `%LOCALAPPDATA%\Kala\SolidEdgeBridge\bridge.token` last.
    let local_app_data = directory.path().join("local");
    let default_path = local_app_data
        .join("Kala")
        .join("SolidEdgeBridge")
        .join("bridge.token");
    std::fs::create_dir_all(default_path.parent().expect("a parent")).expect("the directory");
    std::fs::write(&default_path, TOKEN).expect("the default token");
    bridge.queue(CannedResponse::json(
        200,
        &health_body(true, true, "2024", ""),
    ));
    let mut environment = BTreeMap::new();
    environment.insert(
        "LOCALAPPDATA".to_owned(),
        local_app_data.to_string_lossy().into_owned(),
    );
    let mut provider = SolidEdgeProvider::new(SolidEdgeProviderOptions {
        platform: Some("win32".to_owned()),
        environ: Some(environment),
        url: Some(bridge.url()),
        token_file: None,
        timeout_seconds: Some(2.0),
        probe_cache_seconds: Some(0.0),
    })
    .expect("a valid provider");
    assert!(provider.probe().is_available());

    // A missing token file is reported, not guessed.
    let mut provider = bridge_provider(&bridge.url(), Some(directory.path().join("missing")));
    let availability = provider.probe();
    assert_eq!(
        availability.reason(),
        Some(
            "the Solid Edge bridge token file was not found; run the bridge token \
             command on this Windows host"
        )
    );
}

#[test]
fn the_url_must_be_a_loopback_http_url_with_a_port() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let token = token_file(directory.path(), TOKEN);
    for url in [
        "https://127.0.0.1:8797",
        "http://example.com:8797",
        "http://127.0.0.1",
        "http://127.0.0.1:0",
        "http://127.0.0.1:65536",
        "http://127.0.0.1:8797/v1",
        "http://127.0.0.1:8797/?a=1",
        "http://127.0.0.1:8797/#x",
        "http://user:pass@127.0.0.1:8797",
        "http://127.0.0.1:http",
    ] {
        let mut provider = bridge_provider(url, Some(token.clone()));
        let availability = provider.probe();
        assert!(!availability.is_available(), "{url}");
        assert_eq!(
            availability.reason(),
            Some("the Solid Edge bridge URL is not a loopback HTTP URL"),
            "{url}"
        );
    }
}

#[test]
fn a_linux_build_refuses_without_opening_a_socket() {
    let bridge = MockBridge::start();
    let mut provider = SolidEdgeProvider::new(SolidEdgeProviderOptions {
        platform: Some("linux".to_owned()),
        environ: Some(BTreeMap::new()),
        url: Some(bridge.url()),
        token_file: None,
        timeout_seconds: Some(2.0),
        probe_cache_seconds: Some(0.0),
    })
    .expect("a valid provider");
    let error = provider
        .build(&pad_script())
        .expect_err("Linux cannot run the bridge");
    let unavailable: &CadProviderUnavailableError = error.unavailable().expect("unavailable");
    assert_eq!(unavailable.message(), NOT_REACHABLE_MESSAGE);
    assert_eq!(
        unavailable.reason(),
        "Solid Edge build requires the Windows bridge (not reachable); this host is \
         linux, so the ADR-0008 bridge cannot run here"
    );
    assert_eq!(unavailable.provider(), "solidedge");
    assert!(bridge.requests().is_empty());
}

#[test]
fn constructor_validation_and_platform_defaults_match_python() {
    let error = SolidEdgeProvider::new(SolidEdgeProviderOptions {
        timeout_seconds: Some(0.0),
        ..SolidEdgeProviderOptions::default()
    })
    .expect_err("zero timeout");
    assert_eq!(error.message(), "the bridge probe timeout must be positive");
    let error = SolidEdgeProvider::new(SolidEdgeProviderOptions {
        timeout_seconds: Some(-1.0),
        ..SolidEdgeProviderOptions::default()
    })
    .expect_err("negative timeout");
    assert_eq!(error.message(), "the bridge probe timeout must be positive");
    let error = SolidEdgeProvider::new(SolidEdgeProviderOptions {
        probe_cache_seconds: Some(-1.0),
        ..SolidEdgeProviderOptions::default()
    })
    .expect_err("negative cache");
    assert_eq!(
        error.message(),
        "the probe cache duration must be non-negative"
    );
    let provider = SolidEdgeProvider::with_defaults().expect("the defaults are valid");
    #[cfg(target_os = "linux")]
    assert_eq!(provider.platform(), "linux");
    #[cfg(target_os = "macos")]
    assert_eq!(provider.platform(), "darwin");
    #[cfg(target_os = "windows")]
    assert_eq!(provider.platform(), "win32");
    assert_eq!(provider.timeout_seconds(), 5.0);
}

#[test]
fn the_status_row_publishes_the_bridge_identity() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(
        200,
        &health_body(true, true, "2024", ""),
    ));
    let mut provider = bridge_provider(&bridge.url(), Some(token_file(directory.path(), TOKEN)));
    let status = provider.status();
    assert_eq!(status.id(), "solidedge");
    assert_eq!(status.name(), "Solid Edge");
    assert!(status.available());
    assert_eq!(status.version(), Some("2024"));
    assert_eq!(status.measured_by(), "Solid Edge");
    assert_eq!(status.mode(), "bridge");
}

#[test]
fn an_over_long_health_detail_is_refused_by_the_protocol_client() {
    // Intentional difference: the Python provider reads any JSON object and
    // truncates the reason to 512 characters. The accepted client enforces the
    // bridge's own protocol bound on `detail`, so this document is refused.
    let directory = tempfile::tempdir().expect("a temporary directory");
    let bridge = MockBridge::start();
    let detail = "d".repeat(600);
    bridge.queue(CannedResponse::json(
        200,
        &health_body(false, true, "2024", &detail),
    ));
    let mut provider = bridge_provider(&bridge.url(), Some(token_file(directory.path(), TOKEN)));
    let availability = provider.probe();
    assert!(!availability.is_available());
    assert_eq!(
        availability.reason(),
        Some(
            format!(
                "the Solid Edge bridge at {} did not answer a valid health probe",
                bridge.url()
            )
            .as_str()
        )
    );
}

#[test]
fn an_out_of_range_port_is_an_unavailable_url_not_a_python_value_error() {
    // Intentional difference: Python's `urlparse().port` raises ValueError for
    // a port above 65535 outside its try block, so its probe propagates the
    // exception. The native provider classifies it as an invalid URL.
    let directory = tempfile::tempdir().expect("a temporary directory");
    let mut provider = bridge_provider(
        "http://127.0.0.1:65536",
        Some(token_file(directory.path(), TOKEN)),
    );
    let availability = provider.probe();
    assert!(!availability.is_available());
    assert_eq!(
        availability.reason(),
        Some("the Solid Edge bridge URL is not a loopback HTTP URL")
    );
}
