//! The OpenAI Codex CLI chat provider.
//!
//! The Codex CLI is the operator's own, already-authenticated command-line
//! client for their ChatGPT subscription. Kala never holds the credential:
//! the CLI reads its own login from its own `CODEX_HOME`, and Kala only runs
//! the CLI.
//!
//! # The exact argv
//!
//! ```text
//! codex exec --sandbox read-only --skip-git-repo-check --ephemeral \
//!            --ignore-user-config --json [--model <model>]
//! ```
//!
//! * `exec` is the CLI's non-interactive mode; it reads the prompt from
//!   standard input when no prompt argument is given.
//! * `--sandbox read-only` confines every command the model may run to a
//!   read-only sandbox, so the turn cannot change anything on the host.
//! * `--skip-git-repo-check` lets the turn run in the private temporary
//!   directory, which is not a repository.
//! * `--ephemeral` keeps the turn out of the CLI's session store, so a chat
//!   turn leaves no transcript behind.
//! * `--ignore-user-config` does not load `$CODEX_HOME/config.toml`. The
//!   operator's MCP servers, hooks and custom instructions therefore stay
//!   out of the turn, and the CLI falls back to its own default model. The
//!   login still comes from `CODEX_HOME`.
//! * `--json` prints the turn's events as JSON lines. The assistant's final
//!   message is the `item.completed` event whose item is an
//!   `agent_message`.
//! * `--model` is passed through only when the request names a model; the
//!   alias `default` means the CLI's own default model and is therefore not
//!   passed at all.
//!
//! The prompt is written to the child's standard input and is never placed
//! on the argv. The child runs in a private temporary directory with a
//! sanitized environment; see [`super::cli_runtime`] for the shared contract.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::Value;

use super::cli_runtime::{
    CliInvocation, CliSpec, consent_message, failure_excerpt, failure_text, program_or_reason, run,
    timeout_seconds,
};
use super::{ChatOutcome, ChatUsage, ProviderFailure};

/// The provider id of the Codex CLI route.
pub const PROVIDER_ID: &str = "codex_cli";

/// The historical provider id of the Codex CLI route.
///
/// The Studio published `codex` before the subscription routes existed. The
/// id stays as an alias so an old saved preference still resolves.
pub const LEGACY_PROVIDER_ID: &str = "codex";

/// The identity and the fixed probe commands of the Codex CLI.
pub const SPEC: CliSpec = CliSpec {
    id: PROVIDER_ID,
    binary: "codex",
    name: "Codex CLI",
    path_override_env: "KALA_STUDIO_CODEX_CLI_PATH",
    version_args: &["--version"],
    auth_args: &["login", "status"],
    env_passthrough: &["CODEX_HOME"],
};

/// The model alias that means "the CLI's own default model".
const DEFAULT_MODEL: &str = "default";

/// The exact argv of one Codex CLI turn.
#[must_use]
pub fn build_argv(model: Option<&str>) -> Vec<String> {
    let mut argv: Vec<String> = vec![
        "exec".to_owned(),
        "--sandbox".to_owned(),
        "read-only".to_owned(),
        "--skip-git-repo-check".to_owned(),
        "--ephemeral".to_owned(),
        "--ignore-user-config".to_owned(),
        "--json".to_owned(),
    ];
    if let Some(model) = model.filter(|value| !value.is_empty() && *value != DEFAULT_MODEL) {
        argv.push("--model".to_owned());
        argv.push(model.to_owned());
    }
    argv
}

/// Run one Codex CLI turn.
///
/// # Errors
///
/// Returns the typed provider failure the Studio route maps onto a sanitized
/// status. The CLI's own text is bounded and never carries a credential.
pub async fn chat(
    model: Option<&str>,
    prompt: &str,
    environ: &BTreeMap<String, String>,
) -> Result<ChatOutcome, ProviderFailure> {
    let program = program_or_reason(&SPEC, environ)?;
    let workdir = tempfile::tempdir().map_err(|_| {
        ProviderFailure::Internal("a private working directory could not be created".to_owned())
    })?;
    let argv = build_argv(model);
    let timeout = Duration::from_secs(timeout_seconds(environ));
    let run = run(CliInvocation {
        spec: &SPEC,
        program: &program,
        argv: &argv,
        stdin: prompt.as_bytes(),
        timeout,
        environ,
        workdir: workdir.path(),
        extra_env: &[],
    })
    .await?;
    if !run.succeeded() {
        return Err(ProviderFailure::Subprocess(failure_message(&run)));
    }
    let (text, failure, usage) = parse_events(&run.stdout)?;
    if let Some(failure) = failure {
        return Err(ProviderFailure::Subprocess(failure));
    }
    let Some(text) = text else {
        return Err(ProviderFailure::Protocol(
            "the Codex CLI produced no assistant message".to_owned(),
        ));
    };
    if text.trim().is_empty() {
        return Err(ProviderFailure::Protocol(
            "the Codex CLI produced no assistant message".to_owned(),
        ));
    }
    Ok(ChatOutcome::new(
        text,
        model
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| DEFAULT_MODEL.to_owned()),
        false,
    )
    .with_usage(usage))
}

/// The bounded failure text of a CLI run that did not exit with status zero.
fn failure_message(run: &super::cli_runtime::CliRun) -> String {
    let stderr = failure_excerpt(&run.stderr);
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = failure_excerpt(&run.stdout);
    if !stdout.is_empty() {
        return stdout;
    }
    format!(
        "the Codex CLI exited with status {}",
        run.status.unwrap_or(-1)
    )
}

/// Read the JSON lines the CLI prints with `--json`.
///
/// The assistant's final message is the last `item.completed` event whose
/// item is an `agent_message`. A `turn.failed` or `error` event is the
/// failure the operator sees.
///
/// The token counts come from the turn's own report: the `turn.completed`
/// event's `usage` block when the CLI sends one, otherwise the last
/// `token_count` event (`{"type":"token_count","info":{"total_token_usage":…}}`
/// or its `event_msg`/`payload` spelling). A `turn.completed` block wins
/// because it is the turn's own total; two `token_count` events are
/// cumulative snapshots, so the last one is the whole turn.
fn parse_events(
    stdout: &[u8],
) -> Result<(Option<String>, Option<String>, ChatUsage), ProviderFailure> {
    let text = String::from_utf8_lossy(stdout);
    let mut answer: Option<String> = None;
    let mut failure: Option<String> = None;
    let mut completed_usage: Option<ChatUsage> = None;
    let mut counted_usage: Option<ChatUsage> = None;
    let mut parsed_any = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(Value::Object(event)) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        parsed_any = true;
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        let is_token_count = kind == "token_count"
            || (kind == "event_msg"
                && event
                    .get("payload")
                    .and_then(Value::as_object)
                    .and_then(|payload| payload.get("type"))
                    .and_then(Value::as_str)
                    == Some("token_count"));
        if is_token_count {
            if let Some(usage) = token_count_usage(&event) {
                counted_usage = Some(usage);
            }
        }
        let candidate = match kind {
            "item.completed" => {
                let mut message = None;
                if let Some(item) = event.get("item") {
                    if item.get("type").and_then(Value::as_str) == Some("agent_message") {
                        if let Some(text) = item.get("text").and_then(Value::as_str) {
                            message = Some(text.to_owned());
                        }
                    }
                }
                if let Some(text) = message {
                    answer = Some(text);
                }
                None
            }
            "turn.completed" => {
                if let Some(usage) = reported_usage(event.get("usage")) {
                    completed_usage = Some(usage);
                }
                None
            }
            "turn.failed" => Some(event_message(event.get("error"))),
            "error" => Some(event_message(event.get("message"))),
            _ => None,
        };
        if let Some(text) = candidate {
            failure = failure.take().or(Some(text));
        }
    }
    if !parsed_any {
        return Err(ProviderFailure::Protocol(
            "the Codex CLI output carried no JSON event".to_owned(),
        ));
    }
    let usage = completed_usage.or(counted_usage).unwrap_or(ChatUsage::NONE);
    Ok((answer, failure, usage))
}

/// The input and output counts of one Codex `usage` object.
///
/// `cached_input_tokens` is a subset of `input_tokens`, so it is never added;
/// `reasoning_output_tokens` is a subset of `output_tokens` for the same
/// reason.
fn usage_counts(object: &serde_json::Map<String, Value>) -> ChatUsage {
    ChatUsage {
        input_tokens: super::usage::count(object.get("input_tokens")),
        output_tokens: super::usage::count(object.get("output_tokens")),
    }
}

/// The usage block of a value, when it reports any count at all.
fn reported_usage(value: Option<&Value>) -> Option<ChatUsage> {
    let usage = usage_counts(value?.as_object()?);
    usage.is_reported().then_some(usage)
}

/// The counts of one `token_count` event, in either documented shape.
fn token_count_usage(event: &serde_json::Map<String, Value>) -> Option<ChatUsage> {
    let info = event.get("info").and_then(Value::as_object).or_else(|| {
        event
            .get("payload")
            .and_then(Value::as_object)
            .and_then(|payload| payload.get("info"))
            .and_then(Value::as_object)
    });
    let total = info
        .and_then(|info| info.get("total_token_usage"))
        .and_then(Value::as_object)
        .or_else(|| event.get("total_token_usage").and_then(Value::as_object));
    let nested = total.map(usage_counts).filter(ChatUsage::is_reported);
    nested.or_else(|| {
        let direct = usage_counts(event);
        direct.is_reported().then_some(direct)
    })
}

/// The bounded text of a failure event.
fn event_message(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => failure_text(text),
        Some(Value::Object(object)) => object
            .get("message")
            .and_then(Value::as_str)
            .map(failure_text)
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// The refusal the route shows when the operator did not consent.
#[must_use]
pub fn consent_refusal() -> String {
    consent_message(PROVIDER_ID)
}
