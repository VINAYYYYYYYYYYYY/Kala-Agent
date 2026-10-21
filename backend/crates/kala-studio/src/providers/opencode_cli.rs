//! The OpenCode CLI chat provider.
//!
//! OpenCode is the operator's own command-line agent. Kala never holds a
//! credential: OpenCode reads its own provider logins from its own data
//! directory, and Kala only runs the CLI.
//!
//! # The exact argv
//!
//! ```text
//! opencode run --standalone --format json --agent kala-chat [--model <provider/model>]
//! ```
//!
//! * `run` is OpenCode's non-interactive mode. It reads the prompt from
//!   standard input when no message argument is given.
//! * `--standalone` runs a private server for the turn instead of joining the
//!   operator's background service, so a chat turn cannot disturb or inherit
//!   a running session.
//! * `--format json` prints the turn's events as JSON lines. The assistant's
//!   text is the `text` event's `part.text`.
//! * `--agent kala-chat` selects the agent the turn writes into the private
//!   working directory. That agent denies every tool, so the model cannot
//!   read a file, run a command or fetch a URL.
//! * `--model` is passed through only when the request names a model. The
//!   alias `default` means OpenCode's own default model and is therefore not
//!   passed at all. OpenCode names its models `provider/model`.
//!
//! # The sandboxing configuration
//!
//! The turn runs in a private temporary directory that holds an
//! `opencode.json` (and the same document in `OPENCODE_CONFIG_CONTENT`).
//! The document gives the `kala-chat` agent a permission rule that denies
//! every action on every resource, and the same rule at the top level so it
//! reaches every agent. The deprecated `permission` and `tools` spellings
//! carry the same intent, so the rule holds whichever configuration shape
//! the installed OpenCode reads.
//!
//! The prompt is written to the child's standard input and is never placed
//! on the argv. The child runs with a sanitized environment; see
//! [`super::cli_runtime`] for the shared contract.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::Value;

use super::cli_runtime::{
    CliInvocation, CliSpec, consent_message, failure_excerpt, failure_text, program_or_reason, run,
    timeout_seconds,
};
use super::{ChatOutcome, ProviderFailure};

/// The provider id of the OpenCode CLI route.
pub const PROVIDER_ID: &str = "opencode_cli";

/// The identity and the fixed probe commands of the OpenCode CLI.
pub const SPEC: CliSpec = CliSpec {
    id: PROVIDER_ID,
    name: "OpenCode CLI",
    binary: "opencode",
    path_override_env: "KALA_STUDIO_OPENCODE_CLI_PATH",
    version_args: &["--version"],
    auth_args: &["auth", "list", "--format", "json"],
    env_passthrough: &[],
};

/// The model alias that means "the CLI's own default model".
const DEFAULT_MODEL: &str = "default";

/// The name of the agent the turn runs as.
const AGENT_NAME: &str = "kala-chat";

/// The name of the configuration document written into the working directory.
const CONFIG_FILE_NAME: &str = "opencode.json";

/// The environment variable that carries the same document inline.
const CONFIG_CONTENT_ENV: &str = "OPENCODE_CONFIG_CONTENT";

/// The sandboxing configuration of the turn.
///
/// Every action is denied on every resource. The document is written both as
/// the deprecated `permission` object and as the current `permissions` rule
/// list, and the `tools` map denies every built-in tool by name, so the
/// intent survives whichever shape the installed OpenCode reads.
pub const SANDBOX_CONFIG: &str = r#"{
  "$schema": "https://opencode.ai/config.json",
  "permission": { "*": "deny" },
  "permissions": [ { "action": "*", "resource": "*", "effect": "deny" } ],
  "agent": {
    "kala-chat": {
      "description": "Kala Studio chat turn: no tools, no file access, no network fetch.",
      "mode": "primary",
      "permission": { "*": "deny" },
      "permissions": [ { "action": "*", "resource": "*", "effect": "deny" } ],
      "tools": {
        "bash": false,
        "edit": false,
        "write": false,
        "patch": false,
        "read": false,
        "grep": false,
        "glob": false,
        "list": false,
        "todowrite": false,
        "webfetch": false,
        "websearch": false,
        "task": false,
        "question": false,
        "lsp": false,
        "skill": false,
        "external_directory": false,
        "doom_loop": false
      }
    }
  }
}"#;

/// The exact argv of one OpenCode turn.
#[must_use]
pub fn build_argv(model: Option<&str>) -> Vec<String> {
    let mut argv: Vec<String> = vec![
        "run".to_owned(),
        "--standalone".to_owned(),
        "--format".to_owned(),
        "json".to_owned(),
        "--agent".to_owned(),
        AGENT_NAME.to_owned(),
    ];
    if let Some(model) = model.filter(|value| !value.is_empty() && *value != DEFAULT_MODEL) {
        argv.push("--model".to_owned());
        argv.push(model.to_owned());
    }
    argv
}

/// Run one OpenCode turn.
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
    write_config(workdir.path())?;
    let argv = build_argv(model);
    let timeout = Duration::from_secs(timeout_seconds(environ));
    let extra_env = vec![(CONFIG_CONTENT_ENV.to_owned(), SANDBOX_CONFIG.to_owned())];
    let run = run(CliInvocation {
        spec: &SPEC,
        program: &program,
        argv: &argv,
        stdin: prompt.as_bytes(),
        timeout,
        environ,
        workdir: workdir.path(),
        extra_env: &extra_env,
    })
    .await?;
    if !run.succeeded() {
        return Err(ProviderFailure::Subprocess(failure_message(&run)));
    }
    let (text, failure) = parse_events(&run.stdout)?;
    if let Some(failure) = failure {
        return Err(ProviderFailure::Subprocess(failure));
    }
    let Some(text) = text else {
        return Err(ProviderFailure::Protocol(
            "the OpenCode CLI produced no assistant text".to_owned(),
        ));
    };
    if text.trim().is_empty() {
        return Err(ProviderFailure::Protocol(
            "the OpenCode CLI produced no assistant text".to_owned(),
        ));
    }
    Ok(ChatOutcome::new(
        text,
        model
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| DEFAULT_MODEL.to_owned()),
        false,
    ))
}

/// Write the sandboxing configuration into the private working directory.
fn write_config(workdir: &std::path::Path) -> Result<(), ProviderFailure> {
    std::fs::write(workdir.join(CONFIG_FILE_NAME), SANDBOX_CONFIG).map_err(|_| {
        ProviderFailure::Internal(
            "the OpenCode sandboxing configuration could not be written".to_owned(),
        )
    })
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
        "the OpenCode CLI exited with status {}",
        run.status.unwrap_or(-1)
    )
}

/// Read the JSON lines the CLI prints with `--format json`.
///
/// The assistant's text is the concatenation of the `text` events' text. An
/// `error` event is the failure the operator sees.
fn parse_events(stdout: &[u8]) -> Result<(Option<String>, Option<String>), ProviderFailure> {
    let text = String::from_utf8_lossy(stdout);
    let mut answer = String::new();
    let mut failure: Option<String> = None;
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
        let candidate = match kind {
            "text" => {
                if let Some(part) = event.get("part") {
                    if let Some(chunk) = part.get("text").and_then(Value::as_str) {
                        answer.push_str(chunk);
                    }
                }
                None
            }
            "error" => Some(event_message(event.get("error"))),
            _ => None,
        };
        if let Some(text) = candidate {
            failure = failure.take().or(Some(text));
        }
    }
    if !parsed_any {
        return Err(ProviderFailure::Protocol(
            "the OpenCode CLI output carried no JSON event".to_owned(),
        ));
    }
    let answer = if answer.is_empty() {
        None
    } else {
        Some(answer)
    };
    Ok((answer, failure))
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
