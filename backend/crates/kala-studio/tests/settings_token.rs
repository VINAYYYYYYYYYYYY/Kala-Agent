//! Token lifecycle, settings precedence/format and owner-only atomic writes.

mod common;

use std::collections::HashMap;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use kala_studio::boundary::{default_studio_data_dir, read_studio_token, validate_studio_token};
use kala_studio::server::{StudioConfig, StudioService};
use kala_studio::settings::{
    STUDIO_SETTINGS_FILE_NAME, read_stored_keys, read_stored_keys_checked, write_stored_key,
};

use common::{RequestSpec, TOKEN, TestServer, empty_environ};

fn post_settings(addr: std::net::SocketAddr, token: &str, body: &str) -> RequestSpec {
    let mut spec = RequestSpec::default();
    spec.headers
        .push(("Authorization".to_owned(), format!("Bearer {token}")));
    spec.headers
        .push(("content-type".to_owned(), "application/json".to_owned()));
    spec.body = Some(body.as_bytes().to_vec());
    let _ = addr;
    spec
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn token_file_is_minted_atomically_and_guards_mutations() {
    let work = tempfile::tempdir().unwrap();
    let data_dir = work.path().join("data");
    let server = TestServer::start_config(StudioConfig {
        data_dir: Some(data_dir.clone()),
        api_token: None,
        max_body_bytes: 16_384,
        static_root: None,
        environ: Some(empty_environ()),
        import_max_bytes: None,
        step_exporter: std::sync::Arc::new(kala_studio::geometry::UnavailableStepExporter),
        transport: None,
        geometry_backend: Some(std::sync::Arc::new(common::StubBackend)),
    })
    .await;
    let token = read_studio_token(&data_dir).expect("minted token");
    assert_eq!(token.len(), 43);
    assert!(validate_studio_token(&token).is_ok());
    let raw = fs::read_to_string(data_dir.join("studio-token")).unwrap();
    assert_eq!(raw, format!("{token}\n"));
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(data_dir.join("studio-token"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        common::request(
            server.addr,
            "GET",
            "/api/studio/health",
            RequestSpec::default()
        )
        .await
        .status,
        200
    );
    assert_eq!(
        common::request(
            server.addr,
            "POST",
            "/api/studio/preview",
            post_settings(server.addr, &token, r#"{"kind":"box"}"#)
        )
        .await
        .status,
        200
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_env_token_does_not_overwrite_the_file() {
    let work = tempfile::tempdir().unwrap();
    let data_dir = work.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(
        data_dir.join("studio-token"),
        "old-token-0123456789abcdef\n",
    )
    .unwrap();
    let mut environ = HashMap::new();
    environ.insert("KALA_STUDIO_TOKEN".to_owned(), TOKEN.to_owned());
    let server = TestServer::start_config(StudioConfig {
        data_dir: Some(data_dir.clone()),
        api_token: None,
        max_body_bytes: 16_384,
        static_root: None,
        environ: Some(environ),
        import_max_bytes: None,
        step_exporter: std::sync::Arc::new(kala_studio::geometry::UnavailableStepExporter),
        transport: None,
        geometry_backend: Some(std::sync::Arc::new(common::StubBackend)),
    })
    .await;
    assert_eq!(
        fs::read_to_string(data_dir.join("studio-token")).unwrap(),
        "old-token-0123456789abcdef\n"
    );
    assert_eq!(
        common::request(
            server.addr,
            "POST",
            "/api/studio/preview",
            post_settings(server.addr, TOKEN, r#"{"kind":"box"}"#)
        )
        .await
        .status,
        200
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_tokens_fail_startup_without_echoing_the_value() {
    for token in [
        "short",
        "bad token with spaces and enough length",
        &"a".repeat(129),
    ] {
        let work = tempfile::tempdir().unwrap();
        let error = StudioService::new(StudioConfig {
            data_dir: Some(work.path().join("data")),
            api_token: Some(token.to_owned()),
            max_body_bytes: 16_384,
            static_root: None,
            environ: Some(empty_environ()),
            import_max_bytes: None,
            step_exporter: std::sync::Arc::new(kala_studio::geometry::UnavailableStepExporter),
            transport: None,
            geometry_backend: Some(std::sync::Arc::new(common::StubBackend)),
        })
        .err()
        .expect("invalid token must fail");
        assert!(!error.to_string().contains(token));
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn minted_token_replaces_a_symlink_without_touching_the_target() {
    let work = tempfile::tempdir().unwrap();
    let data_dir = work.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let outside = work.path().join("outside-token");
    fs::write(&outside, "outside\n").unwrap();
    std::os::unix::fs::symlink(&outside, data_dir.join("studio-token")).unwrap();
    let server = TestServer::start_config(StudioConfig {
        data_dir: Some(data_dir.clone()),
        api_token: None,
        max_body_bytes: 16_384,
        static_root: None,
        environ: Some(empty_environ()),
        import_max_bytes: None,
        step_exporter: std::sync::Arc::new(kala_studio::geometry::UnavailableStepExporter),
        transport: None,
        geometry_backend: Some(std::sync::Arc::new(common::StubBackend)),
    })
    .await;
    assert_eq!(fs::read_to_string(&outside).unwrap(), "outside\n");
    let metadata = fs::symlink_metadata(data_dir.join("studio-token")).unwrap();
    assert!(metadata.file_type().is_file());
    assert!(read_studio_token(&data_dir).is_some());
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stored_keys_override_environment_and_are_never_echoed() {
    let work = tempfile::tempdir().unwrap();
    let data_dir = work.path().join("data");
    let mut environ = HashMap::new();
    environ.insert("OPENAI_API_KEY".to_owned(), "sk-env-openai".to_owned());
    environ.insert("OPENROUTER_API_KEY".to_owned(), "   ".to_owned());
    let server = TestServer::start(data_dir.clone(), TOKEN, 16_384, None, environ).await;

    let response = common::request(server.addr, "GET", "/api/studio/settings", {
        let mut spec = RequestSpec::default();
        spec.headers
            .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
        spec
    })
    .await;
    let body = response.json();
    assert_eq!(body["credentials"][0]["provider"], "openai");
    assert_eq!(body["credentials"][0]["source"], "environment");
    assert_eq!(body["credentials"][0]["configured"], true);
    assert_eq!(body["credentials"][1]["source"], "none");

    let stored_key = "sk-or-stored-not-a-live-credential-0001";
    let response = common::request(
        server.addr,
        "POST",
        "/api/studio/settings",
        post_settings(
            server.addr,
            TOKEN,
            &format!(r#"{{"provider":"openrouter","api_key":"{stored_key}"}}"#),
        ),
    )
    .await;
    assert_eq!(response.status, 200);
    assert!(!response.body_text().contains(stored_key));
    let body = response.json();
    assert_eq!(body["credentials"][1]["source"], "stored");
    assert_eq!(body["credentials"][1]["configured"], true);
    assert_eq!(body["offline"], false);

    let file = data_dir.join(STUDIO_SETTINGS_FILE_NAME);
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.contains(stored_key));
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        text,
        format!("{{\n  \"api_keys\": {{\n    \"openrouter\": \"{stored_key}\"\n  }}\n}}\n")
    );

    // Stored openai overrides the environment entry.
    let response = common::request(
        server.addr,
        "POST",
        "/api/studio/settings",
        post_settings(
            server.addr,
            TOKEN,
            r#"{"provider":"openai","api_key":"sk-stored-openai-0001"}"#,
        ),
    )
    .await;
    assert_eq!(response.json()["credentials"][0]["source"], "stored");

    // Clearing the stored key falls back to the environment key.
    let response = common::request(
        server.addr,
        "POST",
        "/api/studio/settings",
        post_settings(
            server.addr,
            TOKEN,
            r#"{"provider":"openai","api_key":null}"#,
        ),
    )
    .await;
    assert_eq!(response.json()["credentials"][0]["source"], "environment");
    assert_eq!(read_stored_keys(Some(&data_dir)).get("openai"), None);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn valid_large_settings_survive_the_former_one_mib_boundary() {
    // Python's `read_text`/`json.loads` has no file-size cap; the source
    // keeps a stored key in a file with legal JSON whitespace beyond 1 MiB.
    let work = tempfile::tempdir().unwrap();
    let data_dir = work.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let key = "sk-large-synthetic-not-a-live-credential-0003";
    let mut payload = format!(r#"{{"api_keys":{{"openai":"{key}"}}}}"#);
    payload.push_str(&" ".repeat(2 * 1024 * 1024));
    fs::write(data_dir.join(STUDIO_SETTINGS_FILE_NAME), &payload).unwrap();

    let server = TestServer::start(data_dir.clone(), TOKEN, 16_384, None, HashMap::new()).await;
    let response = common::request(
        server.addr,
        "GET",
        "/api/studio/settings",
        RequestSpec::default(),
    )
    .await;
    assert_eq!(response.status, 200);
    let body = response.json();
    assert_eq!(body["credentials"][0]["provider"], "openai");
    assert_eq!(body["credentials"][0]["configured"], true);
    assert_eq!(body["credentials"][0]["source"], "stored");
    assert!(!response.body_text().contains(key));
    assert_eq!(body["offline"], false);
    assert_eq!(
        read_stored_keys(Some(&data_dir))
            .get("openai")
            .and_then(|value| value.to_utf8().ok())
            .as_deref(),
        Some(key)
    );

    // Saving through the route preserves the key and rewrites the canonical
    // small document; clearing removes it.
    let response = common::request(
        server.addr,
        "POST",
        "/api/studio/settings",
        post_settings(
            server.addr,
            TOKEN,
            r#"{"provider":"openrouter","api_key":"sk-or-large-synthetic-0004"}"#,
        ),
    )
    .await;
    assert_eq!(response.status, 200);
    assert_eq!(
        fs::read_to_string(data_dir.join(STUDIO_SETTINGS_FILE_NAME)).unwrap(),
        "{\n  \"api_keys\": {\n    \"openai\": \"sk-large-synthetic-not-a-live-credential-0003\",\n    \"openrouter\": \"sk-or-large-synthetic-0004\"\n  }\n}\n"
    );

    let response = common::request(
        server.addr,
        "POST",
        "/api/studio/settings",
        post_settings(
            server.addr,
            TOKEN,
            r#"{"provider":"openai","api_key":null}"#,
        ),
    )
    .await;
    assert_eq!(response.status, 200);
    assert_eq!(
        fs::read_to_string(data_dir.join(STUDIO_SETTINGS_FILE_NAME)).unwrap(),
        "{\n  \"api_keys\": {\n    \"openrouter\": \"sk-or-large-synthetic-0004\"\n  }\n}\n"
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_utf8_settings_match_the_source_failure_class() {
    // Python `Path.read_text(encoding="utf-8")` raises UnicodeDecodeError
    // (a ValueError) that the source does not catch on reads. Reads become
    // the sanitized 500; a settings write reports the codec message as a 422.
    let work = tempfile::tempdir().unwrap();
    let data_dir = work.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let path = data_dir.join(STUDIO_SETTINGS_FILE_NAME);
    fs::write(&path, b"{\"api_keys\":{}}\xff").unwrap();
    let error = read_stored_keys_checked(Some(&data_dir)).unwrap_err();
    assert_eq!(
        error.python_message(),
        "'utf-8' codec can't decode byte 0xff in position 15: invalid start byte"
    );

    let server = TestServer::start(data_dir.clone(), TOKEN, 16_384, None, HashMap::new()).await;
    for route in [
        "/api/studio/settings",
        "/api/studio/health",
        "/api/studio/providers",
    ] {
        let response = common::request(server.addr, "GET", route, RequestSpec::default()).await;
        assert_eq!(response.status, 500, "{route}");
        assert_eq!(
            response.body_text(),
            "{\"code\":\"internal_error\",\"message\":\"the studio request could not be completed\",\"details\":[]}",
            "{route}"
        );
    }
    let response = common::request(
        server.addr,
        "POST",
        "/api/studio/settings",
        post_settings(
            server.addr,
            TOKEN,
            r#"{"provider":"openai","api_key":null}"#,
        ),
    )
    .await;
    assert_eq!(response.status, 422);
    assert_eq!(response.json()["code"], "invalid_request");
    assert_eq!(
        response.json()["details"][0],
        "'utf-8' codec can't decode byte 0xff in position 15: invalid start byte"
    );
    // The failed write leaves the corrupt file untouched.
    assert_eq!(fs::read(&path).unwrap(), b"{\"api_keys\":{}}\xff");
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn escaped_surrogate_keys_survive_the_routes_like_python() {
    // A Python-accepted stored key containing an escaped lone surrogate
    // (`json.loads` keeps the code point). Rust strings cannot carry it, so
    // the store keeps code points; reads report configured/stored, a write
    // preserves the escape byte-for-byte, and a chat that would need to put
    // it in a header is the source's sanitized 500.
    let work = tempfile::tempdir().unwrap();
    let data_dir = work.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let path = data_dir.join(STUDIO_SETTINGS_FILE_NAME);
    fs::write(
        &path,
        br#"{"api_keys":{"openai":"sk-\ud800","openrouter":"sk-or"}}"#,
    )
    .unwrap();

    let server = TestServer::start(data_dir.clone(), TOKEN, 16_384, None, HashMap::new()).await;

    let settings = common::request(server.addr, "GET", "/api/studio/settings", {
        let mut spec = RequestSpec::default();
        spec.headers
            .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
        spec
    })
    .await;
    assert_eq!(settings.status, 200);
    let body = settings.json();
    assert_eq!(body["credentials"][0]["configured"], true);
    assert_eq!(body["credentials"][0]["source"], "stored");
    assert_eq!(body["providers"][1]["configured"], true);
    assert_eq!(body["offline"], false);
    assert!(!settings.body_text().contains("sk-"));

    let providers = common::request(server.addr, "GET", "/api/studio/providers", {
        let mut spec = RequestSpec::default();
        spec.headers
            .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
        spec
    })
    .await;
    assert_eq!(providers.json()["providers"][1]["configured"], true);
    let health = common::request(server.addr, "GET", "/api/studio/health", {
        let mut spec = RequestSpec::default();
        spec.headers
            .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
        spec
    })
    .await;
    assert_eq!(health.json()["offline"], false);

    // Python's urllib fails to encode the surrogate header, which the route
    // reports as a sanitized 500 before any network call.
    let chat = common::request(
        server.addr,
        "POST",
        "/api/studio/chat",
        post_settings(
            server.addr,
            TOKEN,
            r#"{"provider":"openai","model":"gpt-4o-mini","prompt":"hi","allow_remote":true}"#,
        ),
    )
    .await;
    assert_eq!(chat.status, 500);
    assert_eq!(chat.json()["code"], "internal_error");

    // Storing another provider rewrites the canonical document and keeps the
    // escaped surrogate exactly like `json.dumps(..., ensure_ascii=True)`.
    let stored = common::request(
        server.addr,
        "POST",
        "/api/studio/settings",
        post_settings(
            server.addr,
            TOKEN,
            r#"{"provider":"openrouter","api_key":"sk-or-new"}"#,
        ),
    )
    .await;
    assert_eq!(stored.status, 200);
    assert_eq!(
        fs::read(&path).unwrap(),
        b"{\n  \"api_keys\": {\n    \"openai\": \"sk-\\ud800\",\n    \"openrouter\": \"sk-or-new\"\n  }\n}\n"
    );
    let read_back = read_stored_keys(Some(&data_dir));
    let openai = read_back.get("openai").expect("surrogate key retained");
    assert!(openai.has_surrogates());
    assert_eq!(openai.code_points(), &[0x73, 0x6B, 0x2D, 0xD800]);

    // Clearing it removes the code points and leaves the other key.
    let cleared = common::request(
        server.addr,
        "POST",
        "/api/studio/settings",
        post_settings(
            server.addr,
            TOKEN,
            r#"{"provider":"openai","api_key":null}"#,
        ),
    )
    .await;
    assert_eq!(cleared.status, 200);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "{\n  \"api_keys\": {\n    \"openrouter\": \"sk-or-new\"\n  }\n}\n"
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_and_surrogate_settings_survive_the_routes_like_python() {
    // REVIEW-3 probe with synthetic credentials: Python's `json.loads` keeps
    // the last duplicate `api_keys` member and the last duplicate provider
    // value, and it accepts escaped lone surrogates. The native projection
    // must report the same stored credentials and must not lose the valid key
    // on an unrelated save/clear. Only synthetic values are used.
    let work = tempfile::tempdir().unwrap();
    let data_dir = work.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let path = data_dir.join(STUDIO_SETTINGS_FILE_NAME);
    fs::write(
        &path,
        br#"{"api_keys":{},"api_keys":{"openai":"first","openai":"synthetic-review-key","openrouter":"sk-\ud800"},"unused":"\ud800"}"#,
    )
    .unwrap();

    let server = TestServer::start(data_dir.clone(), TOKEN, 16_384, None, HashMap::new()).await;
    let settings = common::request(server.addr, "GET", "/api/studio/settings", {
        let mut spec = RequestSpec::default();
        spec.headers
            .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
        spec
    })
    .await;
    assert_eq!(settings.status, 200);
    let body = settings.json();
    assert_eq!(body["credentials"][0]["provider"], "openai");
    assert_eq!(body["credentials"][0]["configured"], true);
    assert_eq!(body["credentials"][0]["source"], "stored");
    assert_eq!(body["credentials"][1]["provider"], "openrouter");
    assert_eq!(body["credentials"][1]["configured"], true);
    assert_eq!(body["credentials"][1]["source"], "stored");
    assert_eq!(body["offline"], false);
    assert!(!settings.body_text().contains("synthetic-review-key"));
    assert!(!settings.body_text().contains("sk-"));

    let health = common::request(server.addr, "GET", "/api/studio/health", {
        let mut spec = RequestSpec::default();
        spec.headers
            .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
        spec
    })
    .await;
    assert_eq!(health.status, 200);
    assert_eq!(health.json()["offline"], false);
    let providers = common::request(server.addr, "GET", "/api/studio/providers", {
        let mut spec = RequestSpec::default();
        spec.headers
            .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
        spec
    })
    .await;
    assert_eq!(providers.status, 200);
    assert_eq!(providers.json()["providers"][0]["configured"], true);
    assert_eq!(providers.json()["providers"][1]["configured"], true);

    // An unrelated write rewrites the canonical document and drops the
    // ignored duplicate/surrogate fields exactly like
    // `json.dumps(..., indent=2, sort_keys=True)`.
    let stored = common::request(
        server.addr,
        "POST",
        "/api/studio/settings",
        post_settings(
            server.addr,
            TOKEN,
            r#"{"provider":"openrouter","api_key":"sk-or-unrelated-0007"}"#,
        ),
    )
    .await;
    assert_eq!(stored.status, 200);
    assert_eq!(
        fs::read(&path).unwrap(),
        b"{\n  \"api_keys\": {\n    \"openai\": \"synthetic-review-key\",\n    \"openrouter\": \"sk-or-unrelated-0007\"\n  }\n}\n"
    );

    // Clearing one provider keeps the other surviving duplicate key.
    let cleared = common::request(
        server.addr,
        "POST",
        "/api/studio/settings",
        post_settings(
            server.addr,
            TOKEN,
            r#"{"provider":"openai","api_key":null}"#,
        ),
    )
    .await;
    assert_eq!(cleared.status, 200);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "{\n  \"api_keys\": {\n    \"openrouter\": \"sk-or-unrelated-0007\"\n  }\n}\n"
    );

    // The actual surrogate escape is still preserved byte-for-byte when it is
    // the surviving value and another provider is written.
    fs::write(
        &path,
        br#"{"api_keys":{},"api_keys":{"openai":"synthetic-review-key"},"unused":"\ud800"}"#,
    )
    .unwrap();
    let keys = read_stored_keys(Some(&data_dir));
    assert_eq!(
        keys.get("openai")
            .and_then(|value| value.to_utf8().ok())
            .as_deref(),
        Some("synthetic-review-key")
    );
    write_stored_key(&data_dir, "openrouter", Some("sk-or-kept-0008")).unwrap();
    assert_eq!(
        fs::read(&path).unwrap(),
        b"{\n  \"api_keys\": {\n    \"openai\": \"synthetic-review-key\",\n    \"openrouter\": \"sk-or-kept-0008\"\n  }\n}\n"
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn integer_limited_settings_match_the_source_failure_class_on_the_routes() {
    // REVIEW-3 probe: a valid synthetic key plus an unrelated 4,301-digit
    // integer. CPython's `json.loads` raises `ValueError` (outside the
    // store's `except json.JSONDecodeError`), so reads become the sanitized
    // 500 and a write is a 422 carrying the exact CPython message without
    // touching the file. 4,300 digits remains accepted.
    let work = tempfile::tempdir().unwrap();
    let data_dir = work.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let path = data_dir.join(STUDIO_SETTINGS_FILE_NAME);
    let digits = "7".repeat(4_301);
    let integer_payload =
        format!(r#"{{"api_keys":{{"openai":"sk-synthetic-valid-0009"}},"unused":{digits}}}"#);
    fs::write(&path, &integer_payload).unwrap();

    let server = TestServer::start(data_dir.clone(), TOKEN, 16_384, None, HashMap::new()).await;
    for route in [
        "/api/studio/settings",
        "/api/studio/health",
        "/api/studio/providers",
    ] {
        let response = common::request(server.addr, "GET", route, RequestSpec::default()).await;
        assert_eq!(response.status, 500, "{route}");
        assert_eq!(
            response.body_text(),
            "{\"code\":\"internal_error\",\"message\":\"the studio request could not be completed\",\"details\":[]}",
            "{route}"
        );
    }
    // Chat reads the effective environment before dispatch, like the source.
    let chat = common::request(
        server.addr,
        "POST",
        "/api/studio/chat",
        post_settings(
            server.addr,
            TOKEN,
            r#"{"provider":"local","prompt":"hello"}"#,
        ),
    )
    .await;
    assert_eq!(chat.status, 500);
    assert_eq!(chat.json()["code"], "internal_error");

    let expected_message = "Exceeds the limit (4300 digits) for integer string conversion: value has 4301 digits; use sys.set_int_max_str_digits() to increase the limit";
    let write = common::request(
        server.addr,
        "POST",
        "/api/studio/settings",
        post_settings(
            server.addr,
            TOKEN,
            r#"{"provider":"openrouter","api_key":"sk-or-refused-0010"}"#,
        ),
    )
    .await;
    assert_eq!(write.status, 422);
    assert_eq!(write.json()["code"], "invalid_request");
    assert_eq!(write.json()["details"][0], expected_message);
    // The failed write leaves the original file untouched.
    assert_eq!(fs::read(&path).unwrap(), integer_payload.as_bytes());
    let error = read_stored_keys_checked(Some(&data_dir)).unwrap_err();
    assert_eq!(error.python_exception_name(), "ValueError");
    assert_eq!(error.python_message(), expected_message);

    // CPython's limit is inclusive: exactly 4,300 digits is a legal literal.
    let at_limit = format!(
        r#"{{"api_keys":{{"openai":"sk-synthetic-valid-0011"}},"unused":{}}}"#,
        "7".repeat(4_300)
    );
    fs::write(&path, at_limit).unwrap();
    let settings = common::request(server.addr, "GET", "/api/studio/settings", {
        let mut spec = RequestSpec::default();
        spec.headers
            .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
        spec
    })
    .await;
    assert_eq!(settings.status, 200);
    assert_eq!(settings.json()["credentials"][0]["configured"], true);
    assert_eq!(settings.json()["credentials"][0]["source"], "stored");
    server.stop().await;
}

#[test]
fn malformed_settings_and_unsafe_symlinks_are_handled() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(STUDIO_SETTINGS_FILE_NAME);
    fs::write(&path, "not json").unwrap();
    assert!(read_stored_keys(Some(directory.path())).is_empty());
    fs::write(&path, r#"{"api_keys":{"openai":5}}"#).unwrap();
    assert!(read_stored_keys(Some(directory.path())).is_empty());
    fs::write(&path, r#"{"api_keys":{"openai":"has space"}}"#).unwrap();
    assert!(read_stored_keys(Some(directory.path())).is_empty());

    let outside = directory.path().join("outside.json");
    fs::write(&outside, "outside").unwrap();
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    write_stored_key(directory.path(), "openai", Some("sk-safe-key-0001")).unwrap();
    assert_eq!(fs::read_to_string(&outside).unwrap(), "outside");
    let metadata = fs::symlink_metadata(&path).unwrap();
    assert!(metadata.file_type().is_file());
    assert!(read_stored_keys(Some(directory.path())).contains_key("openai"));
    let leftovers: Vec<PathBuf> = fs::read_dir(directory.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "tmp"))
        .collect();
    assert!(leftovers.is_empty(), "temporary files left: {leftovers:?}");
}

#[cfg(unix)]
#[test]
fn default_data_dir_tightens_only_the_studio_child() {
    let work = tempfile::tempdir().unwrap();
    let parent = work.path().join("parent");
    fs::create_dir(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
    let mut environ = std::collections::BTreeMap::new();
    environ.insert(
        "KALA_AGENT_DATA_DIR".to_owned(),
        parent.display().to_string(),
    );
    let resolved = default_studio_data_dir(&environ);
    assert_eq!(resolved, parent.join("studio"));
    #[cfg(unix)]
    {
        assert_eq!(
            fs::metadata(&resolved).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&parent).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }
}
