//! The public surface of `kala-cli`, and the JSON dialect the commands speak.
//!
//! These are the fast, Python-free checks: the constants and messages the
//! module publishes, the argparse-compatible parser's outcomes, the
//! `json.dumps(indent=2, sort_keys=True)` output form, `json.loads` input
//! permissiveness, and the loopback launch helpers.

use std::collections::BTreeMap;

use kala_api::config::{SettingValue, Settings, settings_from_values};
use kala_application::KalaApplication;
use kala_cli::json;
use kala_cli::parser::{self, Command, Outcome};
use kala_cli::{
    DEFAULT_EVENT_LIMIT, DISCLAIMER, EPILOG, EXIT_ERROR, EXIT_FAILED_OUTCOME, EXIT_OK,
    EXIT_SERVE_STARTUP_FAILURE, EXIT_USAGE, MAX_ACTOR_LENGTH, MAX_DIGEST_LENGTH, MAX_EVENT_LIMIT,
    MAX_PARAMETER_BYTES, MAX_PARAMETERS_ENTRIES, MAX_REASON_LENGTH, NON_LOOPBACK_HOST_MESSAGE,
    PROGRAM_NAME, SERVE_HOST, SERVE_PORT, build_serve_plan, load_uvicorn,
};
use kala_json::CanonicalValue;
use num_bigint::BigInt;
use tempfile::TempDir;

fn settings_for(data_dir: &std::path::Path) -> Settings {
    let mut values = BTreeMap::new();
    values.insert(
        "data_dir".to_owned(),
        SettingValue::Text(data_dir.display().to_string()),
    );
    settings_from_values(&values).expect("settings")
}

fn argv(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn the_public_constants_match_the_python_module() {
    assert_eq!(PROGRAM_NAME, "kala-agent");
    assert_eq!(
        (EXIT_OK, EXIT_ERROR, EXIT_USAGE, EXIT_FAILED_OUTCOME),
        (0, 1, 2, 3)
    );
    assert_eq!(MAX_PARAMETER_BYTES, 16_384);
    assert_eq!(MAX_REASON_LENGTH, 512);
    assert_eq!(MAX_ACTOR_LENGTH, 64);
    assert_eq!(MAX_PARAMETERS_ENTRIES, 256);
    assert_eq!(MAX_DIGEST_LENGTH, 64);
    assert_eq!(DEFAULT_EVENT_LIMIT, 50);
    assert_eq!(MAX_EVENT_LIMIT, 1_000);
    assert_eq!((SERVE_HOST, SERVE_PORT), ("127.0.0.1", 8000));
    assert_eq!(
        DISCLAIMER,
        "Bounded surrogate reference arithmetic only; not CAD, CAE, a code check, or an engineering certification. No authentication and no network access."
    );
    assert_eq!(
        EPILOG,
        "Exit codes: 0 completed, 1 error, 2 usage, 3 completed with a failed outcome."
    );
    assert_eq!(
        NON_LOOPBACK_HOST_MESSAGE,
        "serve binds the loopback interface only; a routable or wildcard address is refused"
    );
    assert_eq!(EXIT_SERVE_STARTUP_FAILURE, 3);
    assert!(load_uvicorn().is_ok(), "the native server is compiled in");
}

#[test]
fn the_serve_plan_mints_the_launch_token_without_the_token() {
    let temp = TempDir::new().expect("temp");
    let data = temp.path().join("data");
    let application = KalaApplication::open(settings_for(&data)).expect("application");
    let plan = build_serve_plan(&application, None, Some(&BigInt::from(8_777))).expect("plan");
    assert_eq!(plan.target.host, "127.0.0.1");
    assert_eq!(plan.target.port, 8_777);
    assert_eq!(plan.token_path, data.join("api-token"));
    let token = kala_api::boundary::read_api_token(&data).expect("stored token");
    assert!(kala_api::boundary::validate_api_token(&token).is_ok());
    // The plan is a plain value; it never renders the secret.
    let rendered = format!("{plan:?}");
    assert!(!rendered.contains(&token));
    application.close().expect("close");
}

#[test]
fn the_parser_accepts_the_argparse_surface() {
    match parser::parse(&argv(&["--data-dir", "/tmp/kala", "init"])) {
        Outcome::Run(invocation) => {
            assert_eq!(invocation.data_dir.as_deref(), Some("/tmp/kala"));
            assert_eq!(invocation.command, Command::Init);
        }
        other => panic!("unexpected: {other:?}"),
    }
    match parser::parse(&argv(&[
        "evaluate",
        "--dom",
        "mechanical",
        "--param",
        "x=1",
    ])) {
        Outcome::Run(invocation) => match invocation.command {
            Command::Evaluate {
                domain, parameters, ..
            } => {
                assert_eq!(domain, "mechanical");
                assert_eq!(parameters, vec!["x=1".to_owned()]);
            }
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("unexpected: {other:?}"),
    }
    match parser::parse(&argv(&[
        "events",
        "--limit",
        "1_0",
        "--after-sequence",
        "-1",
    ])) {
        Outcome::Run(invocation) => match invocation.command {
            Command::Events {
                limit,
                after_sequence,
                ..
            } => {
                assert_eq!(limit, BigInt::from(10));
                assert_eq!(after_sequence, BigInt::from(-1));
            }
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("unexpected: {other:?}"),
    }
    // Defaults.
    match parser::parse(&argv(&["run-mechanical"])) {
        Outcome::Run(invocation) => match invocation.command {
            Command::RunMechanical {
                design_id, actor, ..
            } => {
                assert_eq!(design_id, "mechanical-reference");
                assert_eq!(actor, "local-user");
            }
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn the_parser_renders_argparse_help_and_errors() {
    for argv in [argv(&["--help"]), argv(&["evaluate", "--help"])] {
        match parser::parse(&argv) {
            Outcome::Help(text) => {
                assert!(text.starts_with("usage: kala-agent"), "{text}");
                assert!(text.contains("options:"), "{text}");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
    for (argv, expected) in [
        (argv(&[]), "the following arguments are required: command"),
        (
            argv(&["evaluate"]),
            "the following arguments are required: --domain",
        ),
        (
            argv(&["evaluate", "--domain"]),
            "argument --domain: expected one argument",
        ),
        (
            argv(&["events", "--limit", "x"]),
            "argument --limit: invalid int value: 'x'",
        ),
        (
            argv(&["commit-revision", "--request-id", "x", "--decision", "nope"]),
            "argument --decision: invalid choice: 'nope' (choose from 'approve', 'reject')",
        ),
        (
            argv(&["evaluate", "--d"]),
            "ambiguous option: --d could match --domain, --design-id",
        ),
        (
            argv(&["init", "--unknown"]),
            "unrecognized arguments: --unknown",
        ),
    ] {
        match parser::parse(&argv) {
            Outcome::Usage(error) => {
                assert_eq!(error.message, expected, "{argv:?}");
                assert!(error.render().starts_with("usage: kala-agent"));
                assert!(error.render().ends_with(&format!(": error: {expected}\n")));
            }
            other => panic!("unexpected for {argv:?}: {other:?}"),
        }
    }
    // A help action exits before trailing extras are reported.
    assert!(matches!(
        parser::parse(&argv(&["init", "-h", "extra"])),
        Outcome::Help(_)
    ));
}

#[test]
fn the_output_dialect_is_json_dumps_indent_two_sorted() {
    let value = CanonicalValue::object([
        ("z", CanonicalValue::from(1)),
        ("a", CanonicalValue::array([CanonicalValue::from("x")])),
        (
            "empty",
            CanonicalValue::Object(kala_json::CanonicalObject::new()),
        ),
        ("emptyArray", CanonicalValue::Array(Vec::new())),
        ("truth", CanonicalValue::Bool(true)),
        ("nothing", CanonicalValue::Null),
        ("float", CanonicalValue::Float(2.5)),
    ]);
    assert_eq!(
        json::pretty(&value),
        "{\n  \"a\": [\n    \"x\"\n  ],\n  \"empty\": {},\n  \"emptyArray\": [],\n  \"float\": 2.5,\n  \"nothing\": null,\n  \"truth\": true,\n  \"z\": 1\n}"
    );
    assert_eq!(
        json::default_dumps(&value),
        "{\"a\": [\"x\"], \"empty\": {}, \"emptyArray\": [], \"float\": 2.5, \"nothing\": null, \"truth\": true, \"z\": 1}"
    );
    // CPython's ensure_ascii escaping, including supplementary characters.
    let text = CanonicalValue::text("a\"b\\c\u{0000}\u{03ba}\u{1F600}");
    assert_eq!(
        json::pretty(&text),
        "\"a\\\"b\\\\c\\u0000\\u03ba\\ud83d\\ude00\""
    );
}

#[test]
fn the_input_dialect_is_json_loads() {
    // Duplicate keys keep the last value.
    let value = json::loads(r#"{"a": 1, "a": 2}"#).expect("json");
    assert_eq!(
        value,
        CanonicalValue::object([("a", CanonicalValue::from(2))])
    );
    // Non-finite literals, as CPython accepts them.
    assert_eq!(
        json::loads("-Infinity").expect("json"),
        CanonicalValue::Float(f64::NEG_INFINITY)
    );
    assert!(matches!(
        json::loads("NaN").expect("json"),
        CanonicalValue::Float(value) if value.is_nan()
    ));
    // Large integers keep full precision.
    assert_eq!(
        json::loads("1180591620717411303424").expect("json"),
        CanonicalValue::Integer(BigInt::parse_bytes(b"1180591620717411303424", 10).expect("big"))
    );
    // A lone surrogate escape is preserved, not replaced.
    let lone = json::loads(r#""\ud800""#).expect("json");
    let CanonicalValue::Text(ref text) = lone else {
        panic!("not text")
    };
    assert!(text.has_surrogates());
    // Malformed input is refused.
    for bad in [
        "",
        "{",
        "[1,]",
        "1 trailing",
        "01",
        "+1",
        ".5",
        "1.",
        "\"\\x\"",
    ] {
        assert!(json::loads(bad).is_err(), "{bad}");
    }
}

#[test]
fn main_runs_a_command_and_reports_its_exit_code() {
    let temp = TempDir::new().expect("temp");
    let data = temp.path().join("data");
    let mut environ = BTreeMap::new();
    environ.insert("KALA_AGENT_DATA_DIR".to_owned(), data.display().to_string());
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    let code = kala_cli::main(
        Some(argv(&["init"])),
        None,
        Some(&environ),
        &mut out,
        &mut err,
    );
    assert_eq!(code, EXIT_OK);
    assert!(err.is_empty());
    let body: serde_json::Value = serde_json::from_slice(&out).expect("json");
    assert_eq!(body["command"], "init");
    assert_eq!(body["status"], "ready");

    // A usage error writes only stderr.
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    let code = kala_cli::main(
        Some(argv(&["init", "--bogus"])),
        None,
        Some(&environ),
        &mut out,
        &mut err,
    );
    assert_eq!(code, EXIT_USAGE);
    assert!(out.is_empty());
    assert!(String::from_utf8_lossy(&err).contains("unrecognized arguments"));
}
