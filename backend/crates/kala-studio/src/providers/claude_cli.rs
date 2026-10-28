//! The Claude Code CLI chat provider.
//!
//! Claude Code is the operator's own, already-authenticated command-line
//! client for their Claude subscription. Kala never holds the credential: the
//! CLI reads its own login from its own configuration directory, and Kala
//! only runs the CLI.
//!
//! # The exact argv
//!
//! ```text
//! claude --print --output-format json --safe-mode --tools "" \
//!        --strict-mcp-config --permission-mode dontAsk \
//!        --no-session-persistence [--model <model>]
//! ```
//!
//! * `--print` runs the non-interactive mode the CLI documents for scripted
//!   and SDK callers.
//! * `--output-format json` makes the CLI answer with one JSON object whose
//!   `result` field holds the assistant's final text.
//! * `--safe-mode` switches off the customizations a normal session loads -
//!   `CLAUDE.md`, skills, plugins, hooks, MCP servers, custom commands and
//!   agents - while leaving the login, the model selection and the
//!   permissions alone. A chat turn must not run the operator's hooks.
//! * `--tools ""` removes every built-in tool, so the model cannot read a
//!   file, run a command or fetch a URL. This is the hard guarantee that the
//!   turn is a plain question and answer.
//! * `--strict-mcp-config` makes sure no MCP server is connected, even when
//!   the operator configured one.
//! * `--permission-mode dontAsk` denies anything that would still ask for a
//!   permission, so the turn can never block on a question.
//! * `--no-session-persistence` keeps the turn out of the CLI's session
//!   store, so a chat turn leaves no transcript behind.
//! * `--model` is passed through only when the request names a model; the
//!   alias `default` means the CLI's own default model and is therefore not
//!   passed at all.
//!
//! The prompt is written to the child's standard input and is never placed
//! on the argv, so it cannot leak through the process table. The child runs
//! in a private temporary directory with a sanitized environment; see
//! [`super::cli_runtime`] for the shared contract.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::Value;

use super::cli_runtime::{
    CLI_TIMEOUT_ENV, CliInvocation, CliSpec, consent_message, failure_excerpt, failure_text,
    program_or_reason, run, timeout_seconds,
};
use super::{ChatOutcome, ChatUsage, ProviderFailure};

/// The provider id of the Claude Code route.
pub const PROVIDER_ID: &str = "claude_code";

/// The identity and the fixed probe commands of the Claude Code CLI.
pub const SPEC: CliSpec = CliSpec {
    id: PROVIDER_ID,
    name: "Claude Code CLI",
    binary: "claude",
    path_override_env: "KALA_STUDIO_CLAUDE_CLI_PATH",
    version_args: &["--version"],
    auth_args: &["auth", "status"],
    env_passthrough: &["CLAUDE_CONFIG_DIR"],
};

/// The model alias that means "the CLI's own default model".
const DEFAULT_MODEL: &str = "default";

/// The exact argv of one Claude Code turn.
#[must_use]
pub fn build_argv(model: Option<&str>) -> Vec<String> {
    let mut argv: Vec<String> = vec![
        "--print".to_owned(),
        "--output-format".to_owned(),
        "json".to_owned(),
        "--safe-mode".to_owned(),
        "--tools".to_owned(),
        String::new(),
        "--strict-mcp-config".to_owned(),
        "--permission-mode".to_owned(),
        "dontAsk".to_owned(),
        "--no-session-persistence".to_owned(),
    ];
    if let Some(model) = model.filter(|value| !value.is_empty() && *value != DEFAULT_MODEL) {
        argv.push("--model".to_owned());
        argv.push(model.to_owned());
    }
    argv
}

/// Run one Claude Code turn.
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
    let (text, observed, usage) = parse_result(&run.stdout)?;
    if text.trim().is_empty() {
        return Err(ProviderFailure::Protocol(
            "the Claude Code CLI produced no assistant text".to_owned(),
        ));
    }
    Ok(ChatOutcome::new(
        text,
        observed
            .or_else(|| model.filter(|value| !value.is_empty()).map(str::to_owned))
            .unwrap_or_else(|| DEFAULT_MODEL.to_owned()),
        false,
    )
    .with_usage(usage))
}

/// The bounded failure text of a CLI run that did not exit with status zero.
///
/// The CLI's own documentation says that a failure inside a run - a missing
/// login, for example - is printed as the result on standard output, so the
/// JSON result is read first when it is there.
fn failure_message(run: &super::cli_runtime::CliRun) -> String {
    if let Ok(Value::Object(object)) = serde_json::from_slice::<Value>(&run.stdout) {
        if let Some(result) = object.get("result").and_then(Value::as_str) {
            let detail = failure_text(result);
            if !detail.is_empty() {
                return detail;
            }
        }
    }
    let stderr = failure_excerpt(&run.stderr);
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = failure_excerpt(&run.stdout);
    if !stdout.is_empty() {
        return stdout;
    }
    format!(
        "the Claude Code CLI exited with status {}",
        run.status.unwrap_or(-1)
    )
}

/// Parse the JSON object the CLI prints with `--output-format json`.
///
/// The object carries the assistant's final text in `result`. A run that
/// failed reports `is_error`, and its text is the failure the operator sees.
/// The model that answered is read from `modelUsage`, the per-model cost
/// breakdown the CLI publishes.
///
/// The `usage` block bills one prompt as three input fields (`input_tokens`,
/// `cache_read_input_tokens`, `cache_creation_input_tokens`); the input count
/// is their sum. The output count is `output_tokens`. A block the CLI did not
/// send, or a field it did not report, stays "not reported".
fn parse_result(stdout: &[u8]) -> Result<(String, Option<String>, ChatUsage), ProviderFailure> {
    let text = String::from_utf8_lossy(stdout);
    let payload: Value = serde_json::from_str(text.trim()).map_err(|_| {
        ProviderFailure::Protocol("the Claude Code CLI output was not valid JSON".to_owned())
    })?;
    let Value::Object(object) = payload else {
        return Err(ProviderFailure::Protocol(
            "the Claude Code CLI output was not a JSON object".to_owned(),
        ));
    };
    if object.get("is_error").and_then(Value::as_bool) == Some(true) {
        let detail = object
            .get("result")
            .and_then(Value::as_str)
            .map(failure_text)
            .unwrap_or_default();
        let detail = if detail.is_empty() {
            "the Claude Code CLI reported a failed turn".to_owned()
        } else {
            detail
        };
        return Err(ProviderFailure::Subprocess(detail));
    }
    let result = object
        .get("result")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ProviderFailure::Protocol("the Claude Code CLI output carried no result".to_owned())
        })?;
    let observed = object
        .get("modelUsage")
        .and_then(Value::as_object)
        .and_then(|usage| usage.keys().next().cloned())
        .or_else(|| {
            object
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    Ok((result.to_owned(), observed, usage_of(&object)))
}

/// The token counts of one `--output-format json` result.
fn usage_of(result: &serde_json::Map<String, Value>) -> ChatUsage {
    let Some(usage) = result.get("usage").and_then(Value::as_object) else {
        return ChatUsage::NONE;
    };
    ChatUsage {
        input_tokens: super::usage::sum([
            super::usage::count(usage.get("input_tokens")),
            super::usage::count(usage.get("cache_read_input_tokens")),
            super::usage::count(usage.get("cache_creation_input_tokens")),
        ]),
        output_tokens: super::usage::count(usage.get("output_tokens")),
    }
}

/// The environment variable that configures the wall-clock budget.
pub const TIMEOUT_ENV: &str = CLI_TIMEOUT_ENV;

/// The refusal the route shows when the operator did not consent.
#[must_use]
pub fn consent_refusal() -> String {
    consent_message(PROVIDER_ID)
}
