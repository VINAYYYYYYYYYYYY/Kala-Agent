//! The shared STEP import ceiling (ADR-0011).
//!
//! The front door and the Python geometry worker must accept exactly the
//! same body: the setting is one variable, `KALA_STEP_IMPORT_MAX_BYTES`,
//! resolved with the worker's own parsing, bounds and default.
//!
//! These tests drive the real front door over a real socket: the import
//! route's own gate is checked at the limit and one byte over it, and the
//! resolution is checked for the default, an override and an unusable
//! value.

mod common;

use std::collections::HashMap;

use common::{RequestSpec, TOKEN, TestServer};
use kala_studio::server::{StudioConfig, StudioService};
use kala_studio::step_limit::{
    DEFAULT_MAX_STEP_BYTES, MAX_STEP_IMPORT_BYTES, MIN_STEP_IMPORT_BYTES,
    STEP_IMPORT_ENV_TYPE_MESSAGE, STEP_IMPORT_LIMIT_RANGE_MESSAGE, STEP_IMPORT_MAX_BYTES_ENV,
};

const IMPORT_LIMIT: usize = 4_096;

fn import_spec(body: Vec<u8>) -> RequestSpec {
    let mut spec = RequestSpec::default();
    spec.headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    spec.headers
        .push(("content-type".to_owned(), "application/step".to_owned()));
    spec.body = Some(body);
    spec
}

fn environ_with_limit(value: &str) -> HashMap<String, String> {
    let mut environ = HashMap::new();
    environ.insert(STEP_IMPORT_MAX_BYTES_ENV.to_owned(), value.to_owned());
    environ
}

#[test]
fn the_default_is_the_64_mib_worker_default() {
    assert_eq!(DEFAULT_MAX_STEP_BYTES, 64 * 1024 * 1024);
    assert_eq!(MIN_STEP_IMPORT_BYTES, 1);
    assert_eq!(MAX_STEP_IMPORT_BYTES, 512 * 1024 * 1024);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_absent_setting_keeps_the_default() {
    let work = tempfile::tempdir().unwrap();
    let service = StudioService::new(StudioConfig {
        data_dir: Some(work.path().join("data")),
        api_token: Some(TOKEN.to_owned()),
        max_body_bytes: 262_144,
        static_root: None,
        environ: Some(HashMap::new()),
        import_max_bytes: None,
        step_exporter: std::sync::Arc::new(kala_studio::geometry::UnavailableStepExporter),
        transport: None,
        geometry_backend: Some(std::sync::Arc::new(common::StubBackend)),
    })
    .expect("the service starts");
    assert_eq!(service.import_max_bytes(), DEFAULT_MAX_STEP_BYTES);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_override_is_used_by_the_front_door() {
    let work = tempfile::tempdir().unwrap();
    let service = StudioService::new(StudioConfig {
        data_dir: Some(work.path().join("data")),
        api_token: Some(TOKEN.to_owned()),
        max_body_bytes: 262_144,
        static_root: None,
        environ: Some(environ_with_limit("1048576")),
        import_max_bytes: None,
        step_exporter: std::sync::Arc::new(kala_studio::geometry::UnavailableStepExporter),
        transport: None,
        geometry_backend: Some(std::sync::Arc::new(common::StubBackend)),
    })
    .expect("the service starts");
    assert_eq!(service.import_max_bytes(), 1_048_576);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unusable_setting_refuses_startup_with_the_workers_message() {
    let work = tempfile::tempdir().unwrap();
    for (value, message) in [
        ("abc", STEP_IMPORT_ENV_TYPE_MESSAGE),
        ("0", STEP_IMPORT_LIMIT_RANGE_MESSAGE),
        ("536870913", STEP_IMPORT_LIMIT_RANGE_MESSAGE),
    ] {
        let error = StudioService::new(StudioConfig {
            data_dir: Some(work.path().join("data")),
            api_token: Some(TOKEN.to_owned()),
            max_body_bytes: 262_144,
            static_root: None,
            environ: Some(environ_with_limit(value)),
            import_max_bytes: None,
            step_exporter: std::sync::Arc::new(kala_studio::geometry::UnavailableStepExporter),
            transport: None,
            geometry_backend: Some(std::sync::Arc::new(common::StubBackend)),
        })
        .err()
        .expect("an unusable STEP import limit must fail startup");
        assert_eq!(error.to_string(), message, "{value:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_body_at_the_limit_reaches_the_worker_and_one_byte_over_is_refused() {
    let work = tempfile::tempdir().unwrap();
    let server = TestServer::start(
        work.path().join("data"),
        TOKEN,
        262_144,
        None,
        environ_with_limit(&IMPORT_LIMIT.to_string()),
    )
    .await;
    let addr = server.addr;

    // Exactly at the limit: the body is forwarded to the worker.
    let response = common::request(
        addr,
        "POST",
        "/api/studio/import",
        import_spec(vec![b'x'; IMPORT_LIMIT]),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    assert_eq!(
        response.body_text(),
        format!("{{\"bytes\":{IMPORT_LIMIT}}}")
    );

    // One byte over the limit: the front door refuses before the worker.
    let response = common::request(
        addr,
        "POST",
        "/api/studio/import",
        import_spec(vec![b'x'; IMPORT_LIMIT + 1]),
    )
    .await;
    assert_eq!(response.status, 413, "{}", response.body_text());
    assert_eq!(response.json()["code"], "payload_too_large");
    assert_eq!(
        response.json()["details"][0],
        format!("the request body must not exceed {IMPORT_LIMIT} bytes")
    );

    // The streamed body is bounded by the same limit.
    let mut chunked = RequestSpec::default();
    chunked
        .headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    chunked
        .headers
        .push(("content-type".to_owned(), "application/step".to_owned()));
    chunked.chunked = Some(vec![vec![b'x'; IMPORT_LIMIT - 16], vec![b'x'; 17]]);
    let response = common::request(addr, "POST", "/api/studio/import", chunked).await;
    assert_eq!(response.status, 413, "{}", response.body_text());
    assert_eq!(response.json()["code"], "payload_too_large");

    // Every other route keeps the shared Studio limit.
    let mut preview = RequestSpec::default();
    preview
        .headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    preview
        .headers
        .push(("content-type".to_owned(), "application/json".to_owned()));
    preview.body = Some(vec![b' '; 262_144]);
    let response = common::request(addr, "POST", "/api/studio/preview", preview).await;
    assert_eq!(response.status, 422, "{}", response.body_text());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_import_route_uses_its_own_ceiling_not_the_shared_one() {
    let work = tempfile::tempdir().unwrap();
    // The import ceiling is above the shared Studio limit.
    let server = TestServer::start(
        work.path().join("data"),
        TOKEN,
        1_024,
        None,
        environ_with_limit("4096"),
    )
    .await;
    let addr = server.addr;

    let response = common::request(
        addr,
        "POST",
        "/api/studio/import",
        import_spec(vec![b'x'; 4_096]),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    assert_eq!(response.body_text(), "{\"bytes\":4096}");

    // The shared limit still applies to every other route.
    let mut preview = RequestSpec::default();
    preview
        .headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    preview
        .headers
        .push(("content-type".to_owned(), "application/json".to_owned()));
    preview.body = Some(vec![b' '; 1_025]);
    let response = common::request(addr, "POST", "/api/studio/preview", preview).await;
    assert_eq!(response.status, 413, "{}", response.body_text());
}
