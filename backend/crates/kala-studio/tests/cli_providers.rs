//! The three subscription-CLI chat providers, exercised with fake CLIs.
//!
//! No test in this file ever calls a real CLI. Every case writes a small
//! bash script into a temporary directory and points the provider at it
//! with the documented path-override environment variable. The script
//! records its own argv, its own environment and its standard input, so
//! the assertions can prove that the turn really used an argv array, that
//! the prompt never rode on the argv and that nothing of Kala's own
//! configuration reached the child.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use kala_studio::providers::ProviderFailure;
use kala_studio::providers::claude_cli;
use kala_studio::providers::cli_runtime;
use kala_studio::providers::codex_cli;
use kala_studio::providers::opencode_cli;

const TOKEN: &str = "cli-provider-test-token-0123456789abcdef";

/// A fake CLI that records how it was called.
struct FakeCli {
    _dir: tempfile::TempDir,
    program: PathBuf,
    argv_file: PathBuf,
    env_file: PathBuf,
    stdin_file: PathBuf,
    cwd_file: PathBuf,
}

impl FakeCli {
    /// Write a fake CLI that runs `body` after recording the call.
    fn new(name: &str, body: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join(name);
        let argv_file = dir.path().join("argv.txt");
        let env_file = dir.path().join("env.txt");
        let stdin_file = dir.path().join("stdin.txt");
        let cwd_file = dir.path().join("cwd.txt");
        let script = format!(
            "#!/usr/bin/env bash\n\
             set -u\n\
             printf '%s\\n' \"$@\" > '{argv}'\n\
             env | sort > '{env}'\n\
             pwd > '{cwd}'\n\
             cat > '{stdin}'\n\
             {body}\n",
            argv = argv_file.display(),
            env = env_file.display(),
            cwd = cwd_file.display(),
            stdin = stdin_file.display(),
        );
        std::fs::write(&program, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self {
            _dir: dir,
            program,
            argv_file,
            env_file,
            stdin_file,
            cwd_file,
        }
    }

    /// The argv the CLI was started with, one argument per line.
    fn argv(&self) -> Vec<String> {
        let text = std::fs::read_to_string(&self.argv_file).unwrap_or_default();
        text.lines().map(str::to_owned).collect()
    }

    /// The environment the CLI saw.
    fn env(&self) -> String {
        std::fs::read_to_string(&self.env_file).unwrap_or_default()
    }

    /// The prompt the CLI read from standard input.
    fn stdin(&self) -> String {
        std::fs::read_to_string(&self.stdin_file).unwrap_or_default()
    }

    /// The directory the CLI ran in.
    fn cwd(&self) -> String {
        std::fs::read_to_string(&self.cwd_file)
            .unwrap_or_default()
            .trim()
            .to_owned()
    }

    /// Whether the CLI was started at all.
    fn called(&self) -> bool {
        self.argv_file.exists()
    }
}

/// The environment a test hands to the provider.
///
/// The PATH holds only the system directories, so a real CLI in the
/// operator's home is never reachable even if the path override were
/// ignored. The secret names are present on purpose: the assertion proves
/// they do not reach the child.
fn environment() -> BTreeMap<String, String> {
    let mut environ = BTreeMap::new();
    environ.insert("PATH".to_owned(), "/usr/bin:/bin".to_owned());
    environ.insert("HOME".to_owned(), "/home/synthetic-cli-user".to_owned());
    environ.insert("LANG".to_owned(), "C.UTF-8".to_owned());
    environ.insert(
        "OPENAI_API_KEY".to_owned(),
        "sk-synthetic-must-not-reach-the-cli".to_owned(),
    );
    environ.insert(
        "OPENROUTER_API_KEY".to_owned(),
        "sk-or-synthetic-must-not-reach-the-cli".to_owned(),
    );
    environ.insert(
        "KALA_STUDIO_TOKEN".to_owned(),
        "synthetic-token-must-not-reach-the-cli".to_owned(),
    );
    environ.insert(
        "KALA_AGENT_DATA_DIR".to_owned(),
        "/synthetic/kala-data-must-not-reach-the-cli".to_owned(),
    );
    environ
}

/// Point one provider at a fake CLI.
fn with_fake(spec_env: &str, fake: &FakeCli, environ: &mut BTreeMap<String, String>) {
    environ.insert(
        spec_env.to_owned(),
        fake.program.to_string_lossy().into_owned(),
    );
}

const CLAUDE_RESULT: &str = concat!(
    r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":12,"#,
    r#""num_turns":1,"result":"the fake claude answered","session_id":"s","#,
    r#""total_cost_usd":0.01,"modelUsage":{"claude-sonnet-4-5":{"inputTokens":1}}}"#
);

const CODEX_EVENTS: &str = concat!(
    r#"{"type":"thread.started","thread_id":"t"}"#,
    "\n",
    r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"the fake codex answered"}}"#,
    "\n",
    r#"{"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":2}}"#,
    "\n",
);

const OPENCODE_EVENTS: &str = concat!(
    r#"{"type":"step_start","timestamp":1,"sessionID":"s","part":{"type":"step-start"}}"#,
    "\n",
    r#"{"type":"text","timestamp":2,"sessionID":"s","part":{"id":"p","type":"text","text":"the fake opencode answered"}}"#,
    "\n",
);

#[tokio::test]
async fn claude_code_turn_uses_the_documented_argv_and_the_prompt_on_stdin() {
    let fake = FakeCli::new("claude", &format!("cat <<'JSON'\n{CLAUDE_RESULT}\nJSON"));
    let mut environ = environment();
    with_fake(claude_cli::SPEC.path_override_env, &fake, &mut environ);
    let outcome = claude_cli::chat(Some("sonnet"), "design a plate", &environ)
        .await
        .unwrap();
    assert_eq!(outcome.text, "the fake claude answered");
    assert_eq!(outcome.model, "claude-sonnet-4-5");
    assert!(!outcome.offline);

    assert_eq!(
        fake.argv(),
        vec![
            "--print",
            "--output-format",
            "json",
            "--safe-mode",
            "--tools",
            "",
            "--strict-mcp-config",
            "--permission-mode",
            "dontAsk",
            "--no-session-persistence",
            "--model",
            "sonnet",
        ]
    );
    // The prompt is on standard input and nowhere else.
    assert_eq!(fake.stdin(), "design a plate");
    assert!(
        !fake
            .argv()
            .iter()
            .any(|item| item.contains("design a plate"))
    );
}

#[tokio::test]
async fn claude_code_without_a_model_leaves_the_default_to_the_cli() {
    let fake = FakeCli::new("claude", &format!("cat <<'JSON'\n{CLAUDE_RESULT}\nJSON"));
    let mut environ = environment();
    with_fake(claude_cli::SPEC.path_override_env, &fake, &mut environ);
    let outcome = claude_cli::chat(None, "hello", &environ).await.unwrap();
    assert_eq!(outcome.text, "the fake claude answered");
    assert!(!fake.argv().iter().any(|item| item == "--model"));
    // The alias `default` means the CLI's own default model too.
    let outcome = claude_cli::chat(Some("default"), "hello", &environ)
        .await
        .unwrap();
    assert!(!fake.argv().iter().any(|item| item == "--model"));
    assert_eq!(outcome.model, "claude-sonnet-4-5");
}

#[tokio::test]
async fn codex_cli_turn_uses_the_documented_argv_and_reads_the_jsonl() {
    let fake = FakeCli::new("codex", &format!("cat <<'JSON'\n{CODEX_EVENTS}\nJSON"));
    let mut environ = environment();
    with_fake(codex_cli::SPEC.path_override_env, &fake, &mut environ);
    let outcome = codex_cli::chat(Some("gpt-5-codex"), "design a plate", &environ)
        .await
        .unwrap();
    assert_eq!(outcome.text, "the fake codex answered");
    assert_eq!(outcome.model, "gpt-5-codex");
    assert!(!outcome.offline);
    // The `turn.completed` usage block is this turn's report: 1 in, 2 out.
    assert_eq!(outcome.usage.input_tokens, Some(1));
    assert_eq!(outcome.usage.output_tokens, Some(2));

    assert_eq!(
        fake.argv(),
        vec![
            "exec",
            "--sandbox",
            "read-only",
            "--skip-git-repo-check",
            "--ephemeral",
            "--ignore-user-config",
            "--json",
            "--model",
            "gpt-5-codex",
        ]
    );
    assert_eq!(fake.stdin(), "design a plate");
    assert!(
        !fake
            .argv()
            .iter()
            .any(|item| item.contains("design a plate"))
    );
}

#[tokio::test]
async fn opencode_cli_turn_uses_the_documented_argv_and_the_sandbox_agent() {
    let fake = FakeCli::new(
        "opencode",
        &format!("cat <<'JSON'\n{OPENCODE_EVENTS}\nJSON"),
    );
    let mut environ = environment();
    with_fake(opencode_cli::SPEC.path_override_env, &fake, &mut environ);
    let outcome = opencode_cli::chat(Some("anthropic/claude-sonnet-4-5"), "hello", &environ)
        .await
        .unwrap();
    assert_eq!(outcome.text, "the fake opencode answered");
    assert_eq!(outcome.model, "anthropic/claude-sonnet-4-5");
    // This fixture prints no step-finish event, so no count is reported --
    // and never estimated.
    assert!(!outcome.usage.is_reported());

    assert_eq!(
        fake.argv(),
        vec![
            "run",
            "--standalone",
            "--format",
            "json",
            "--agent",
            "kala-chat",
            "--model",
            "anthropic/claude-sonnet-4-5",
        ]
    );
    assert_eq!(fake.stdin(), "hello");
    // The sandboxing configuration reaches the child, both inline and as a
    // document in the private working directory.
    let env = fake.env();
    assert!(env.contains("OPENCODE_CONFIG_CONTENT="), "{env}");
    assert!(
        env.contains("\"effect\": \"deny\"") || env.contains("\"effect\":\"deny\""),
        "{env}"
    );
}

/// Each CLI reports the token counts it actually prints (ADR-0012).
#[tokio::test]
async fn each_cli_reports_the_usage_it_actually_prints() {
    // Claude Code bills one prompt with three input fields; the input count is
    // their sum.
    let claude_usage = concat!(
        r#"{"type":"result","subtype":"success","is_error":false,"result":"ok","#,
        r#""usage":{"input_tokens":3,"cache_creation_input_tokens":1200,"#,
        r#""cache_read_input_tokens":5000,"output_tokens":100,"#,
        r#""server_tool_use":{"web_search_requests":0}},"#,
        r#""modelUsage":{"claude-sonnet-4-5":{"inputTokens":3}}}"#
    );
    let fake = FakeCli::new("claude", &format!("cat <<'JSON'\n{claude_usage}\nJSON"));
    let mut environ = environment();
    with_fake(claude_cli::SPEC.path_override_env, &fake, &mut environ);
    let outcome = claude_cli::chat(None, "hello", &environ).await.unwrap();
    assert_eq!(outcome.usage.input_tokens, Some(6_203));
    assert_eq!(outcome.usage.output_tokens, Some(100));

    // A result without a usage block stays "not reported".
    let fake = FakeCli::new("claude", &format!("cat <<'JSON'\n{CLAUDE_RESULT}\nJSON"));
    with_fake(claude_cli::SPEC.path_override_env, &fake, &mut environ);
    let outcome = claude_cli::chat(None, "hello", &environ).await.unwrap();
    assert!(!outcome.usage.is_reported());

    // Codex's newer `event_msg`/`token_count` shape, with no
    // `turn.completed` block.
    let codex_token_count = concat!(
        r#"{"type":"thread.started","thread_id":"t"}"#,
        "\n",
        r#"{"type":"item.completed","item":{"type":"agent_message","text":"counted"}}"#,
        "\n",
        r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":40,"output_tokens":20,"reasoning_output_tokens":5,"total_tokens":120}}}}"#,
        "\n",
    );
    let fake = FakeCli::new("codex", &format!("cat <<'JSON'\n{codex_token_count}\nJSON"));
    with_fake(codex_cli::SPEC.path_override_env, &fake, &mut environ);
    let outcome = codex_cli::chat(Some("gpt-5-codex"), "hello", &environ)
        .await
        .unwrap();
    assert_eq!(outcome.usage.input_tokens, Some(100));
    assert_eq!(outcome.usage.output_tokens, Some(20));

    // OpenCode reports one `tokens` block per step-finish; the turn's counts
    // are their sum.
    let opencode_usage = concat!(
        r#"{"type":"text","part":{"type":"text","text":"counted"}}"#,
        "\n",
        r#"{"type":"step_finish","part":{"type":"step-finish","tokens":{"input":11,"output":7,"reasoning":0,"cache":{"read":0,"write":0}}}}"#,
        "\n",
        r#"{"type":"step_finish","part":{"type":"step-finish","tokens":{"input":5,"output":3}}}"#,
        "\n",
    );
    let fake = FakeCli::new("opencode", &format!("cat <<'JSON'\n{opencode_usage}\nJSON"));
    with_fake(opencode_cli::SPEC.path_override_env, &fake, &mut environ);
    let outcome = opencode_cli::chat(None, "hello", &environ).await.unwrap();
    assert_eq!(outcome.usage.input_tokens, Some(16));
    assert_eq!(outcome.usage.output_tokens, Some(10));

    // A usage block of the wrong shape never fails the turn.
    let malformed = concat!(
        r#"{"type":"result","is_error":false,"result":"ok","#,
        r#""usage":{"input_tokens":"many","output_tokens":-1}}"#
    );
    let fake = FakeCli::new("claude", &format!("cat <<'JSON'\n{malformed}\nJSON"));
    with_fake(claude_cli::SPEC.path_override_env, &fake, &mut environ);
    let outcome = claude_cli::chat(None, "hello", &environ).await.unwrap();
    assert_eq!(outcome.text, "ok");
    assert!(!outcome.usage.is_reported());
}

#[tokio::test]
async fn the_child_environment_carries_no_kala_secret() {
    let fake = FakeCli::new("claude", &format!("cat <<'JSON'\n{CLAUDE_RESULT}\nJSON"));
    let mut environ = environment();
    with_fake(claude_cli::SPEC.path_override_env, &fake, &mut environ);
    claude_cli::chat(None, "hello", &environ).await.unwrap();
    let env = fake.env();
    assert!(env.contains("PATH="), "{env}");
    assert!(env.contains("HOME="), "{env}");
    for secret in [
        "OPENAI_API_KEY",
        "OPENROUTER_API_KEY",
        "KALA_STUDIO_TOKEN",
        "KALA_AGENT_DATA_DIR",
    ] {
        assert!(!env.contains(secret), "{secret} reached the child: {env}");
    }
}

#[tokio::test]
async fn a_missing_cli_is_reported_as_unavailable() {
    let mut environ = environment();
    environ.insert(
        claude_cli::SPEC.path_override_env.to_owned(),
        "/nonexistent/kala/claude".to_owned(),
    );
    // The PATH holds only the system directories, so no real CLI is found.
    let failure = claude_cli::chat(None, "hello", &environ).await.unwrap_err();
    match failure {
        ProviderFailure::Unavailable(message) => {
            assert!(message.contains("not found on PATH"), "{message}");
        }
        other => panic!("unexpected failure: {other:?}"),
    }
}

#[tokio::test]
async fn a_failing_cli_reports_its_own_bounded_text() {
    let fake = FakeCli::new(
        "claude",
        "echo 'Invalid API key - please run /login' >&2\nexit 1",
    );
    let mut environ = environment();
    with_fake(claude_cli::SPEC.path_override_env, &fake, &mut environ);
    let failure = claude_cli::chat(None, "hello", &environ).await.unwrap_err();
    match failure {
        ProviderFailure::Subprocess(message) => {
            assert!(message.contains("please run /login"), "{message}");
        }
        other => panic!("unexpected failure: {other:?}"),
    }
}

#[tokio::test]
async fn a_failure_the_cli_prints_as_its_result_is_reported() {
    // The CLI's own documentation: a failure inside a run is printed as the
    // result on standard output, not on standard error.
    let fake = FakeCli::new(
        "claude",
        "cat <<'JSON'\n{\"type\":\"result\",\"subtype\":\"error_during_execution\",\
         \"is_error\":true,\"result\":\"Invalid API key - please run /login\"}\nJSON\nexit 1",
    );
    let mut environ = environment();
    with_fake(claude_cli::SPEC.path_override_env, &fake, &mut environ);
    let failure = claude_cli::chat(None, "hello", &environ).await.unwrap_err();
    match failure {
        ProviderFailure::Subprocess(message) => {
            assert!(message.contains("please run /login"), "{message}");
        }
        other => panic!("unexpected failure: {other:?}"),
    }
}

#[tokio::test]
async fn a_cli_that_reports_an_error_event_is_a_subprocess_failure() {
    let fake = FakeCli::new(
        "codex",
        "cat <<'JSON'\n{\"type\":\"error\",\"message\":\"not logged in\"}\nJSON\nexit 1",
    );
    let mut environ = environment();
    with_fake(codex_cli::SPEC.path_override_env, &fake, &mut environ);
    let failure = codex_cli::chat(None, "hello", &environ).await.unwrap_err();
    match failure {
        ProviderFailure::Subprocess(message) => {
            assert!(message.contains("not logged in"), "{message}");
        }
        other => panic!("unexpected failure: {other:?}"),
    }
}

#[tokio::test]
async fn output_beyond_the_bound_is_refused() {
    let fake = FakeCli::new(
        "opencode",
        "python3 -c \"import sys; sys.stdout.write('x' * 2000000)\"",
    );
    let mut environ = environment();
    with_fake(opencode_cli::SPEC.path_override_env, &fake, &mut environ);
    let failure = opencode_cli::chat(None, "hello", &environ)
        .await
        .unwrap_err();
    match failure {
        ProviderFailure::Internal(message) => {
            assert!(message.contains("bounded size"), "{message}");
        }
        other => panic!("unexpected failure: {other:?}"),
    }
}

#[tokio::test]
async fn a_cli_that_overruns_is_killed_and_reported_as_a_timeout() {
    let fake = FakeCli::new("claude", "sleep 30");
    let mut environ = environment();
    with_fake(claude_cli::SPEC.path_override_env, &fake, &mut environ);
    environ.insert(cli_runtime::CLI_TIMEOUT_ENV.to_owned(), "1".to_owned());
    let failure = claude_cli::chat(None, "hello", &environ).await.unwrap_err();
    match failure {
        ProviderFailure::Timeout(message) => {
            assert!(message.contains("bounded timeout"), "{message}");
        }
        other => panic!("unexpected failure: {other:?}"),
    }
}

#[tokio::test]
async fn the_probe_reports_a_missing_cli_without_running_it() {
    let mut environ = environment();
    environ.insert(
        claude_cli::SPEC.path_override_env.to_owned(),
        "/nonexistent/kala/claude".to_owned(),
    );
    cli_runtime::clear_probe_cache();
    let probe = cli_runtime::probe(&claude_cli::SPEC, &environ).await;
    assert!(!probe.installed);
    assert!(!probe.authenticated);
    assert!(!probe.usable());
    assert!(
        probe.reason.contains("not found on PATH"),
        "{}",
        probe.reason
    );
}

#[tokio::test]
async fn the_probe_reports_the_version_and_the_login_state() {
    let fake = FakeCli::new(
        "claude",
        "if [ \"${1:-}\" = \"--version\" ]; then echo '2.1.303 (Claude Code)'; exit 0; fi\n\
         echo '{\"loggedIn\":true}'\n\
         exit 0",
    );
    let mut environ = environment();
    with_fake(claude_cli::SPEC.path_override_env, &fake, &mut environ);
    cli_runtime::clear_probe_cache();
    let probe = cli_runtime::probe(&claude_cli::SPEC, &environ).await;
    assert!(probe.installed);
    assert!(probe.authenticated);
    assert!(probe.usable());
    assert!(probe.reason.contains("2.1.303"), "{}", probe.reason);
}

#[tokio::test]
async fn the_probe_reports_a_cli_that_is_not_signed_in() {
    let fake = FakeCli::new(
        "codex",
        "if [ \"${1:-}\" = \"--version\" ]; then echo 'codex-cli 0.163.0'; exit 0; fi\n\
         echo 'Not logged in'\n\
         exit 1",
    );
    let mut environ = environment();
    with_fake(codex_cli::SPEC.path_override_env, &fake, &mut environ);
    cli_runtime::clear_probe_cache();
    let probe = cli_runtime::probe(&codex_cli::SPEC, &environ).await;
    assert!(probe.installed, "{}", probe.reason);
    assert!(!probe.authenticated);
    assert!(!probe.usable());
    assert!(probe.reason.contains("not signed in"), "{}", probe.reason);
    assert!(probe.reason.contains("0.163.0"), "{}", probe.reason);
}

// --- The route ------------------------------------------------------------

/// One chat turn over the loopback route.
async fn chat(addr: std::net::SocketAddr, body: serde_json::Value) -> common::TestResponse {
    common::request(
        addr,
        "POST",
        "/api/studio/chat",
        common::RequestSpec {
            headers: vec![
                ("authorization".to_owned(), format!("Bearer {TOKEN}")),
                ("content-type".to_owned(), "application/json".to_owned()),
            ],
            body: Some(body.to_string().into_bytes()),
            chunked: None,
        },
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_route_refuses_a_cli_turn_without_remote_consent() {
    let work = tempfile::tempdir().unwrap();
    let fake = FakeCli::new("claude", &format!("cat <<'JSON'\n{CLAUDE_RESULT}\nJSON"));
    let mut environ = environment();
    with_fake(claude_cli::SPEC.path_override_env, &fake, &mut environ);
    let server = common::TestServer::start(
        work.path().join("data"),
        TOKEN,
        64 * 1024,
        None,
        environ.into_iter().collect(),
    )
    .await;
    let response = chat(
        server.addr,
        serde_json::json!({"provider":"claude_code","prompt":"hello"}),
    )
    .await;
    assert_eq!(response.status, 400);
    assert_eq!(response.json()["code"], "provider_not_configured");
    assert_eq!(
        response.json()["details"][0],
        "provider=claude_code requires allow_remote=true at request time"
    );
    // The CLI was never started.
    assert!(!fake.called());
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_route_runs_the_cli_when_the_operator_consented() {
    let work = tempfile::tempdir().unwrap();
    let fake = FakeCli::new("claude", &format!("cat <<'JSON'\n{CLAUDE_RESULT}\nJSON"));
    let mut environ = environment();
    with_fake(claude_cli::SPEC.path_override_env, &fake, &mut environ);
    let server = common::TestServer::start(
        work.path().join("data"),
        TOKEN,
        64 * 1024,
        None,
        environ.into_iter().collect(),
    )
    .await;
    let response = chat(
        server.addr,
        serde_json::json!({
            "provider":"claude_code",
            "prompt":"design a plate",
            "allow_remote":true
        }),
    )
    .await;
    assert_eq!(response.status, 200);
    assert_eq!(response.json()["text"], "the fake claude answered");
    assert_eq!(response.json()["provider"], "claude_code");
    assert_eq!(response.json()["offline"], false);
    assert_eq!(fake.stdin(), "design a plate");
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_route_refuses_the_codex_alias_without_remote_consent() {
    let work = tempfile::tempdir().unwrap();
    let server = common::TestServer::start(
        work.path().join("data"),
        TOKEN,
        64 * 1024,
        None,
        environment().into_iter().collect(),
    )
    .await;
    let response = chat(
        server.addr,
        serde_json::json!({"provider":"codex","prompt":"hello"}),
    )
    .await;
    assert_eq!(response.status, 400);
    assert_eq!(
        response.json()["details"][0],
        "provider=codex requires allow_remote=true at request time"
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_provider_list_names_the_subscription_routes() {
    let work = tempfile::tempdir().unwrap();
    let server = common::TestServer::start(
        work.path().join("data"),
        TOKEN,
        64 * 1024,
        None,
        environment().into_iter().collect(),
    )
    .await;
    let response = common::request(
        server.addr,
        "GET",
        "/api/studio/providers",
        common::RequestSpec {
            headers: vec![("authorization".to_owned(), format!("Bearer {TOKEN}"))],
            body: None,
            chunked: None,
        },
    )
    .await;
    assert_eq!(response.status, 200);
    let document = response.json();
    let rows = document["providers"].as_array().unwrap();
    let ids: Vec<String> = rows
        .iter()
        .map(|row| row["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        ids,
        vec![
            "local",
            "openai",
            "openrouter",
            "codex",
            "claude_code",
            "codex_cli",
            "opencode_cli"
        ]
    );
    // The subscription rows name their source and carry the probe verdict.
    let claude = rows.iter().find(|row| row["id"] == "claude_code").unwrap();
    assert_eq!(claude["source"], "subscription (CLI)");
    assert_eq!(claude["available"], false);
    assert!(claude["reason"].as_str().unwrap().contains("not found"));
    // The API-key rows are not probed, so they carry no verdict.
    let local = &rows[0];
    assert_eq!(local["available"], serde_json::Value::Null);
    assert_eq!(local["source"], serde_json::Value::Null);
    server.stop().await;
}

/// The turn runs in a private working directory, not in the project.
#[tokio::test]
async fn the_working_directory_is_private() {
    let fake = FakeCli::new("claude", &format!("cat <<'JSON'\n{CLAUDE_RESULT}\nJSON"));
    let mut environ = environment();
    with_fake(claude_cli::SPEC.path_override_env, &fake, &mut environ);
    claude_cli::chat(None, "hello", &environ).await.unwrap();
    let cwd = fake.cwd();
    assert!(
        !cwd.is_empty(),
        "the CLI did not report its working directory"
    );
    assert!(
        Path::new(&cwd).starts_with(std::env::temp_dir()),
        "the turn ran in {cwd}, not in a private temporary directory"
    );
    assert!(
        !cwd.contains("Projects") && !cwd.contains("kala-agent"),
        "the turn ran in {cwd}, not in a private temporary directory"
    );
}

// --- The process tree (B3) ------------------------------------------------

/// Whether a pid is still present in the process table.
///
/// Linux only: the experiment needs an observable pid, and that is where the
/// audit's reproduction ran. The process-group contract is the same on every
/// Unix.
fn pid_is_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// Wait at most two seconds for every pid to disappear.
async fn wait_until_gone(pids: &[u32]) -> bool {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        if pids.iter().all(|pid| !pid_is_alive(*pid)) {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    pids.iter().all(|pid| !pid_is_alive(*pid))
}

/// A fake CLI that starts two long-lived grandchildren and records their
/// pids, then waits. It is the audit's surviving-tree case.
fn tree_fake(name: &str, pid_file: &Path) -> FakeCli {
    FakeCli::new(
        name,
        &format!(
            "sleep 300 & echo $! >> '{pids}'\n\
             sleep 300 & echo $! >> '{pids}'\n\
             wait",
            pids = pid_file.display()
        ),
    )
}

fn read_pids(path: &Path) -> Vec<u32> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .collect()
}

/// Wait until the fake recorded its two grandchildren.
async fn wait_for_pids(path: &Path) -> Vec<u32> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let pids = read_pids(path);
        if pids.len() >= 2 {
            return pids;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the fake CLI never recorded its children"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_dropped_turn_kills_the_whole_cli_process_tree() {
    let work = tempfile::tempdir().unwrap();
    let pid_file = work.path().join("pids.txt");
    let fake = tree_fake("claude", &pid_file);
    let mut environ = environment();
    with_fake(claude_cli::SPEC.path_override_env, &fake, &mut environ);

    let task = tokio::spawn(async move { claude_cli::chat(None, "hello", &environ).await });
    let pids = wait_for_pids(&pid_file).await;
    for pid in &pids {
        assert!(pid_is_alive(*pid), "the grandchild {pid} was never started");
    }
    // The client disconnected: the request future is dropped, and with it the
    // process-tree guard.
    task.abort();
    let _ = task.await;
    assert!(
        wait_until_gone(&pids).await,
        "grandchildren survived the dropped turn: {pids:?}"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_timed_out_turn_kills_the_whole_cli_process_tree() {
    let work = tempfile::tempdir().unwrap();
    let pid_file = work.path().join("pids.txt");
    let fake = tree_fake("claude", &pid_file);
    let mut environ = environment();
    with_fake(claude_cli::SPEC.path_override_env, &fake, &mut environ);
    environ.insert(cli_runtime::CLI_TIMEOUT_ENV.to_owned(), "1".to_owned());

    let failure = claude_cli::chat(None, "hello", &environ).await.unwrap_err();
    assert!(
        matches!(failure, ProviderFailure::Timeout(_)),
        "{failure:?}"
    );
    let pids = read_pids(&pid_file);
    assert_eq!(pids.len(), 2, "the fake recorded {pids:?}");
    assert!(
        wait_until_gone(&pids).await,
        "grandchildren survived the timeout: {pids:?}"
    );
}

// --- Route latency and the honest offline flag (B13, B16) -----------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hanging_cli_never_blocks_the_provider_or_settings_route() {
    let work = tempfile::tempdir().unwrap();
    let hanging = FakeCli::new("claude", "sleep 300");
    let mut environ = environment();
    with_fake(claude_cli::SPEC.path_override_env, &hanging, &mut environ);
    environ.remove("PATH");
    environ.insert("PATH".to_owned(), "/usr/bin:/bin".to_owned());
    cli_runtime::clear_probe_cache();
    let server = common::TestServer::start(
        work.path().join("data"),
        TOKEN,
        64 * 1024,
        None,
        environ.into_iter().collect(),
    )
    .await;
    let started = std::time::Instant::now();
    let response = common::request(
        server.addr,
        "GET",
        "/api/studio/providers",
        common::RequestSpec {
            headers: vec![("authorization".to_owned(), format!("Bearer {TOKEN}"))],
            body: None,
            chunked: None,
        },
    )
    .await;
    let elapsed = started.elapsed();
    assert_eq!(response.status, 200);
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "/providers took {elapsed:?} with a hanging CLI"
    );
    let claude = response.json()["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "claude_code")
        .cloned()
        .unwrap();
    assert_eq!(claude["configured"], false);
    assert_eq!(claude["available"], serde_json::Value::Null);
    assert_eq!(
        claude["reason"],
        kala_studio::providers::cli_runtime::PROBE_CHECKING_REASON
    );

    let started = std::time::Instant::now();
    let settings = common::request(
        server.addr,
        "GET",
        "/api/studio/settings",
        common::RequestSpec {
            headers: vec![("authorization".to_owned(), format!("Bearer {TOKEN}"))],
            body: None,
            chunked: None,
        },
    )
    .await;
    let elapsed = started.elapsed();
    assert_eq!(settings.status, 200);
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "/settings took {elapsed:?} with a hanging CLI"
    );

    let started = std::time::Instant::now();
    let health = common::request(
        server.addr,
        "GET",
        "/api/studio/health",
        common::RequestSpec {
            headers: vec![("authorization".to_owned(), format!("Bearer {TOKEN}"))],
            body: None,
            chunked: None,
        },
    )
    .await;
    let elapsed = started.elapsed();
    assert_eq!(health.status, 200);
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "/health took {elapsed:?} with a hanging CLI"
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_signed_in_cli_makes_the_studio_online() {
    // No API key is configured. The signed-in subscription CLI is a real
    // remote route, so "offline" would be a lie (B16).
    let work = tempfile::tempdir().unwrap();
    let fake = FakeCli::new(
        "claude",
        "if [ \"${1:-}\" = \"--version\" ]; then echo '2.1.303 (Claude Code)'; exit 0; fi\nexit 0",
    );
    let mut environ = environment();
    environ.remove("OPENAI_API_KEY");
    environ.remove("OPENROUTER_API_KEY");
    with_fake(claude_cli::SPEC.path_override_env, &fake, &mut environ);
    cli_runtime::clear_probe_cache();
    let server = common::TestServer::start(
        work.path().join("data"),
        TOKEN,
        64 * 1024,
        None,
        environ.into_iter().collect(),
    )
    .await;

    let health = common::request(
        server.addr,
        "GET",
        "/api/studio/health",
        common::RequestSpec {
            headers: vec![("authorization".to_owned(), format!("Bearer {TOKEN}"))],
            body: None,
            chunked: None,
        },
    )
    .await;
    assert_eq!(health.status, 200);
    assert_eq!(health.json()["offline"], false);

    let settings = common::request(
        server.addr,
        "GET",
        "/api/studio/settings",
        common::RequestSpec {
            headers: vec![("authorization".to_owned(), format!("Bearer {TOKEN}"))],
            body: None,
            chunked: None,
        },
    )
    .await;
    assert_eq!(settings.status, 200);
    assert_eq!(settings.json()["offline"], false);
    let claude = settings.json()["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "claude_code")
        .cloned()
        .unwrap();
    assert_eq!(claude["available"], true);

    // Without the CLI the same studio is honestly offline.
    cli_runtime::clear_probe_cache();
    let mut offline_environ = environment();
    offline_environ.remove("OPENAI_API_KEY");
    offline_environ.remove("OPENROUTER_API_KEY");
    let offline = common::TestServer::start(
        work.path().join("offline-data"),
        TOKEN,
        64 * 1024,
        None,
        offline_environ.into_iter().collect(),
    )
    .await;
    let health = common::request(
        offline.addr,
        "GET",
        "/api/studio/health",
        common::RequestSpec {
            headers: vec![("authorization".to_owned(), format!("Bearer {TOKEN}"))],
            body: None,
            chunked: None,
        },
    )
    .await;
    assert_eq!(health.json()["offline"], true);
    server.stop().await;
    offline.stop().await;
}
