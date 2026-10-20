//! Security and edge-case evidence for the native command line.
//!
//! These cases are the port of `tests/unit/test_cli.py`'s boundary
//! expectations plus the properties the module promises: the launch token is
//! printed only by `token` and never written to a log or another file, a
//! credential-shaped environment value is never printed, a configuration
//! refusal never echoes the rejected value, a malformed usage never creates the
//! data directory, and the one loopback-only command refuses before a socket
//! exists.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use kala_api::config::{SettingValue, Settings, settings_from_env, settings_from_values};
use kala_application::KalaApplication;
use kala_cli::{EXIT_ERROR, EXIT_OK, EXIT_USAGE, build_serve_target, main};
use num_bigint::BigInt;

const TOKEN: &str = "unit-test-launch-token-0123456789abcdef";
const SECRET: &str = "sk-cli-secret-value-must-not-be-printed-9876543210";

fn settings_for(data_dir: &Path) -> Settings {
    let mut values = BTreeMap::new();
    values.insert(
        "data_dir".to_owned(),
        SettingValue::Text(data_dir.display().to_string()),
    );
    settings_from_values(&values).expect("settings")
}

fn environment(data_dir: &Path) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    values.insert(
        "KALA_AGENT_DATA_DIR".to_owned(),
        data_dir.display().to_string(),
    );
    values.insert("KALA_AGENT_MODEL_PROVIDER".to_owned(), "local".to_owned());
    values.insert("KALA_AGENT_MODEL_ID".to_owned(), "local".to_owned());
    values
}

fn run(
    argv: &[&str],
    environ: &BTreeMap<String, String>,
    application: Option<&KalaApplication>,
) -> (i32, String, String) {
    let argv: Vec<String> = argv.iter().map(|value| (*value).to_owned()).collect();
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    let code = main(Some(argv), application, Some(environ), &mut out, &mut err);
    (
        code,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

fn files_containing(root: &Path, needle: &[u8]) -> Vec<PathBuf> {
    let mut matches = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(bytes) = fs::read(&path) {
                if bytes.windows(needle.len()).any(|window| window == needle) {
                    matches.push(path);
                }
            }
        }
    }
    matches
}

#[test]
fn serve_binds_loopback_only() {
    assert_eq!(
        build_serve_target(None, None).expect("defaults"),
        kala_cli::ServeTarget {
            host: "127.0.0.1".to_owned(),
            port: 8000,
        }
    );
    assert_eq!(
        build_serve_target(Some("::1"), Some(&BigInt::from(9_001)))
            .expect("ipv6")
            .url(),
        "http://[::1]:9001"
    );
    assert_eq!(
        build_serve_target(Some("127.0.0.1"), Some(&BigInt::from(8_931)))
            .expect("ipv4")
            .url(),
        "http://127.0.0.1:8931"
    );
    for refused in [
        "0.0.0.0",
        "::",
        "[::]",
        "*",
        "192.168.1.10",
        "example.com",
        "",
        "  ",
    ] {
        let error = build_serve_target(Some(refused), None).expect_err("must refuse");
        assert!(error.message().contains("loopback"), "{refused}");
    }
    for refused in [0i64, -1, 65_536, 1_000_000] {
        let error =
            build_serve_target(Some("127.0.0.1"), Some(&BigInt::from(refused))).expect_err("port");
        assert!(error.message().contains("port"), "{refused}");
    }
    let huge = BigInt::parse_bytes(b"999999999999999999999999999", 10).expect("big");
    let error = build_serve_target(Some("127.0.0.1"), Some(&huge)).expect_err("huge port");
    assert_eq!(
        error.message(),
        "the port must be between 1 and 65535",
        "the Python message"
    );
}

#[test]
fn serve_refuses_a_non_loopback_address_before_minting_a_token() {
    let temp = tempfile::tempdir().expect("temp");
    let data = temp.path().join("data");
    let environ = environment(&data);
    for refused in ["0.0.0.0", "::", "192.168.1.10", "example.com", "  "] {
        let (code, stdout, stderr) = run(&["serve", "--host", refused], &environ, None);
        assert_eq!(code, EXIT_ERROR, "{refused}");
        assert!(stdout.is_empty(), "{refused}");
        assert!(
            stderr.contains("loopback interface only"),
            "{refused}: {stderr}"
        );
        assert!(
            !data.join("api-token").exists(),
            "{refused}: the refusal happened before the token was minted"
        );
    }
    // A refused port is refused too, and still before the token exists.
    let (code, stdout, stderr) = run(&["serve", "--port", "0"], &environ, None);
    assert_eq!(code, EXIT_ERROR);
    assert!(stdout.is_empty());
    assert!(stderr.contains("the port must be between 1 and 65535"));
    assert!(!data.join("api-token").exists());
}

#[test]
fn serve_refuses_a_borrowed_application_without_closing_it() {
    let temp = tempfile::tempdir().expect("temp");
    let data = temp.path().join("data");
    let application = KalaApplication::open(settings_for(&data)).expect("application");
    let environ = environment(&data);
    let (code, stdout, stderr) = run(&["serve", "--port", "8931"], &environ, Some(&application));
    assert_eq!(code, EXIT_ERROR);
    assert!(stdout.is_empty());
    assert!(stderr.contains("application supplied by a caller"));
    assert!(!application.closed(), "a borrowed application stays open");
    application.close().expect("close");
}

#[test]
fn token_prints_the_stored_launch_token_and_nothing_else() {
    let temp = tempfile::tempdir().expect("temp");
    let data = temp.path().join("data");
    let environ = environment(&data);
    let (code, stdout, stderr) = run(&["token"], &environ, None);
    assert_eq!(code, EXIT_ERROR);
    assert!(stdout.is_empty());
    let body: serde_json::Value = serde_json::from_str(&stderr).expect("json error");
    assert_eq!(body["error"]["code"], "application_error");
    assert!(
        body["error"]["message"]
            .as_str()
            .expect("message")
            .contains("start the local api")
    );
    assert!(!stderr.contains("Traceback"));

    fs::write(data.join("api-token"), TOKEN).expect("token");
    let (code, stdout, stderr) = run(&["token"], &environ, None);
    assert_eq!(code, EXIT_OK, "{stderr}");
    assert!(stderr.is_empty());
    let body: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    assert_eq!(body["command"], "token");
    assert_eq!(body["status"], "issued");
    assert_eq!(body["data"]["token"], TOKEN);
    assert_eq!(body["data"]["authentication"], "bearer");
    assert_eq!(
        body["data"]["path"].as_str().expect("path"),
        data.join("api-token").display().to_string()
    );

    // An invalid stored token is no token at all, as in the Python boundary.
    fs::write(data.join("api-token"), "short").expect("token");
    let (code, _, stderr) = run(&["token"], &environ, None);
    assert_eq!(code, EXIT_ERROR);
    assert!(stderr.contains("start the local api"));
}

#[test]
fn no_command_prints_the_launch_token() {
    let temp = tempfile::tempdir().expect("temp");
    let data = temp.path().join("data");
    let environ = environment(&data);
    let (code, _, stderr) = run(&["init"], &environ, None);
    assert_eq!(code, EXIT_OK, "{stderr}");
    fs::write(data.join("api-token"), TOKEN).expect("token");
    for argv in [
        vec!["init"],
        vec!["domains"],
        vec!["events"],
        vec!["evaluate", "--domain", "mechanical"],
        vec!["run-mechanical", "--design-id", "token-run"],
        vec!["workflow-status", "--workflow-id", "workflow-token-run"],
        vec!["approve", "--workflow-id", "workflow-token-run"],
        vec!["serve", "--host", "0.0.0.0"],
    ] {
        let (code, stdout, stderr) = run(&argv, &environ, None);
        assert!(code == EXIT_OK || code == EXIT_ERROR, "{argv:?}");
        assert!(!stdout.contains(TOKEN), "{argv:?} printed the token");
        assert!(!stderr.contains(TOKEN), "{argv:?} printed the token");
    }
    let leaking = files_containing(&data, TOKEN.as_bytes());
    assert_eq!(
        leaking,
        vec![data.join("api-token")],
        "the token is stored only in the token file"
    );
}

#[test]
fn secrets_in_the_environment_are_never_printed() {
    let temp = tempfile::tempdir().expect("temp");
    let data = temp.path().join("data");
    let mut environ = environment(&data);
    environ.insert("KALA_AGENT_API_KEY".to_owned(), SECRET.to_owned());
    environ.insert("OPENAI_API_KEY".to_owned(), SECRET.to_owned());
    environ.insert(
        "KALA_AGENT_MODEL_ENDPOINT".to_owned(),
        "http://127.0.0.1:11434/v1".to_owned(),
    );
    for argv in [
        vec!["init"],
        vec!["domains"],
        vec!["events"],
        vec!["evaluate", "--domain", "mechanical"],
    ] {
        let (code, stdout, stderr) = run(&argv, &environ, None);
        assert_eq!(code, EXIT_OK, "{argv:?}: {stderr}");
        assert!(!stdout.contains(SECRET), "{argv:?}");
        assert!(!stderr.contains(SECRET), "{argv:?}");
        assert!(!stdout.contains("11434"), "{argv:?}");
        let body: serde_json::Value = serde_json::from_str(&stdout).expect("json");
        // Only `init` reports the local facts; every other command must not
        // disclose an endpoint either.
        let offline = body["data"]
            .get("offline")
            .cloned()
            .unwrap_or(serde_json::Value::Bool(true));
        assert_eq!(offline, true);
        if let Some(egress) = body["data"].get("egress_configured") {
            assert_eq!(egress, false);
        }
        if let Some(declared) = body["data"].get("provider_endpoint_declared") {
            assert_eq!(declared, true);
        }
    }
    assert!(data.is_dir());
}

#[test]
fn invalid_configuration_is_reported_without_values() {
    let temp = tempfile::tempdir().expect("temp");
    let broken: BTreeMap<String, String> = [
        ("KALA_AGENT_DATA_DIR", "relative/path"),
        ("KALA_AGENT_WORKER_COUNT", "0"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    let (code, stdout, stderr) = run(&["init"], &broken, None);
    assert_eq!(code, EXIT_ERROR);
    assert!(stdout.is_empty());
    let body: serde_json::Value = serde_json::from_str(&stderr).expect("json error");
    assert_eq!(body["error"]["code"], "invalid_configuration");
    assert_eq!(
        body["error"]["message"],
        "the local configuration is invalid: data_dir:value_error,worker_count:greater_than_equal"
    );
    assert!(!stderr.contains("relative/path"));
    assert!(!stderr.contains("Traceback"));

    let empty: BTreeMap<String, String> = BTreeMap::new();
    let (code, _, stderr) = run(&["--data-dir", "relative", "init"], &empty, None);
    assert_eq!(code, EXIT_ERROR);
    let body: serde_json::Value = serde_json::from_str(&stderr).expect("json error");
    assert_eq!(body["error"]["code"], "invalid_configuration");
    assert!(!temp.path().join("relative").exists());
}

#[test]
fn malformed_usage_exits_two_and_creates_nothing() {
    let temp = tempfile::tempdir().expect("temp");
    let data = temp.path().join("data");
    let environ = environment(&data);
    for argv in [
        vec!["init", "--unknown-flag"],
        vec!["--bogus", "init"],
        vec!["events", "--limit", "not-a-number"],
        vec!["evaluate"],
        vec!["not-a-command"],
        vec![],
    ] {
        let (code, stdout, stderr) = run(&argv, &environ, None);
        assert_eq!(code, EXIT_USAGE, "{argv:?}");
        assert!(stdout.is_empty(), "{argv:?}");
        assert!(
            stderr.starts_with("usage: kala-agent"),
            "{argv:?}: {stderr}"
        );
        assert!(stderr.contains("error:"), "{argv:?}");
    }
    assert!(!data.exists(), "a usage error never creates the data dir");
}

#[test]
fn invalid_actor_and_reason_fail_before_the_data_directory_exists() {
    let temp = tempfile::tempdir().expect("temp");
    let data = temp.path().join("data");
    let environ = environment(&data);
    let long_actor = "a".repeat(65);
    let long_reason = "r".repeat(513);
    for argv in [
        vec!["run-mechanical", "--design-id", "x", "--actor", ""],
        vec!["run-mechanical", "--design-id", "x", "--actor", &long_actor],
        vec![
            "run-mechanical",
            "--design-id",
            "x",
            "--reason",
            &long_reason,
        ],
    ] {
        let (code, stdout, stderr) = run(&argv, &environ, None);
        assert_eq!(code, EXIT_ERROR, "{argv:?}");
        assert!(stdout.is_empty());
        let body: serde_json::Value = serde_json::from_str(&stderr).expect("json error");
        assert_eq!(body["error"]["code"], "application_error");
        assert!(!data.exists(), "the data dir must not be created: {argv:?}");
    }
}

#[test]
fn identifier_digest_and_parameter_limits_are_enforced() {
    let temp = tempfile::tempdir().expect("temp");
    let data = temp.path().join("data");
    let environ = environment(&data);
    let cases: Vec<(Vec<&str>, &str)> = vec![
        (
            vec![
                "prepare-revision",
                "--design-id",
                "../escape",
                "--domain",
                "mechanical",
            ],
            "invalid_identifier",
        ),
        (
            vec![
                "commit-revision",
                "--request-id",
                "commit-x",
                "--state-digest",
                "nope",
            ],
            "application_error",
        ),
        (
            vec![
                "commit-revision",
                "--request-id",
                "commit-x",
                "--action-digest",
                "AABB",
            ],
            "application_error",
        ),
        (
            vec!["evaluate", "--domain", "mechanical", "--param", "broken"],
            "application_error",
        ),
        (
            vec![
                "evaluate",
                "--domain",
                "mechanical",
                "--parameters-json",
                "[1]",
            ],
            "application_error",
        ),
        (vec!["events", "--limit", "0"], "invalid_parameters"),
        (
            vec!["events", "--after-sequence", "-1"],
            "invalid_parameters",
        ),
        (
            vec!["workflow-status", "--workflow-id", "kernel-state:policy"],
            "reserved_identifier",
        ),
    ];
    for (argv, expected) in cases {
        let (code, stdout, stderr) = run(&argv, &environ, None);
        assert_eq!(code, EXIT_ERROR, "{argv:?}");
        assert!(stdout.is_empty(), "{argv:?}");
        let body: serde_json::Value = serde_json::from_str(&stderr).expect("json error");
        assert_eq!(body["error"]["code"], expected, "{argv:?}: {stderr}");
        assert!(!stderr.contains("Traceback"));
    }

    let mut too_many = vec![
        "evaluate".to_owned(),
        "--domain".to_owned(),
        "mechanical".to_owned(),
    ];
    for index in 0..257 {
        too_many.push("--param".to_owned());
        too_many.push(format!("p{index}=1"));
    }
    let borrowed: Vec<&str> = too_many.iter().map(String::as_str).collect();
    let (code, _, stderr) = run(&borrowed, &environ, None);
    assert_eq!(code, EXIT_ERROR);
    assert!(stderr.contains("too many parameter overrides"), "{stderr}");

    let oversized = "x".repeat(17_000);
    let (code, _, stderr) = run(
        &[
            "evaluate",
            "--domain",
            "mechanical",
            "--param",
            &format!("value={oversized}"),
        ],
        &environ,
        None,
    );
    assert_eq!(code, EXIT_ERROR);
    assert!(stderr.contains("bounded size"), "{stderr}");
}

#[test]
fn reserved_identifiers_are_refused_before_work() {
    let temp = tempfile::tempdir().expect("temp");
    let data = temp.path().join("data");
    let environ = environment(&data);
    let (code, _, stderr) = run(
        &[
            "run-mechanical",
            "--design-id",
            "probe",
            "--workflow-id",
            "kernel-state:budget",
        ],
        &environ,
        None,
    );
    assert_eq!(code, EXIT_ERROR);
    let body: serde_json::Value = serde_json::from_str(&stderr).expect("json error");
    assert_eq!(body["error"]["code"], "reserved_identifier");
    // The reserved identifier is refused before the workflow is created.
    let journal = data.join("kala.sqlite3");
    if journal.exists() {
        let connection = rusqlite::Connection::open_with_flags(
            &journal,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .expect("open");
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM events WHERE aggregate_id = 'kernel-state:budget'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(count, 0);
    }
}

#[test]
fn a_tampered_journal_is_a_local_failure() {
    let temp = tempfile::tempdir().expect("temp");
    let data = temp.path().join("data");
    let environ = environment(&data);
    let (code, _, stderr) = run(&["run-mechanical", "--design-id", "tamper"], &environ, None);
    assert_eq!(code, EXIT_OK, "{stderr}");
    let connection = rusqlite::Connection::open(data.join("kala.sqlite3")).expect("open");
    let triggers: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='trigger'")
        .expect("prepare")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("rows");
    for trigger in triggers {
        connection
            .execute(&format!("DROP TRIGGER {trigger}"), [])
            .expect("drop trigger");
    }
    connection
        .execute(
            "UPDATE events SET payload_json = '{}' WHERE sequence = 1",
            [],
        )
        .expect("tamper");
    drop(connection);
    let (code, stdout, stderr) = run(&["init"], &environ, None);
    assert_eq!(code, EXIT_ERROR);
    assert!(stdout.is_empty());
    let body: serde_json::Value = serde_json::from_str(&stderr).expect("json error");
    assert_eq!(body["error"]["code"], "local_failure");
    assert_eq!(body["error"]["message"], "JournalIntegrityError");
}

#[test]
fn injected_application_is_never_closed() {
    let temp = tempfile::tempdir().expect("temp");
    let data = temp.path().join("data");
    let application = KalaApplication::open(settings_for(&data)).expect("application");
    let environ = environment(&data);
    let (code, stdout, stderr) = run(&["domains"], &environ, Some(&application));
    assert_eq!(code, EXIT_OK, "{stderr}");
    let body: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    assert_eq!(body["command"], "domains");
    assert!(!application.closed(), "a borrowed application stays open");
    application.close().expect("close");
    assert!(application.closed());
}

#[test]
fn settings_from_env_keeps_the_seven_field_allowlist() {
    let temp = tempfile::tempdir().expect("temp");
    let data = temp.path().join("data");
    let mut environ = environment(&data);
    environ.insert("KALA_AGENT_API_KEY".to_owned(), SECRET.to_owned());
    let settings = settings_from_env(&environ).expect("settings");
    assert_eq!(settings.data_dir, data);
    assert!(settings.model_endpoint.is_none());
}
