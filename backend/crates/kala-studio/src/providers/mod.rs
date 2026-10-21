//! Provider dispatch, fixed-endpoint remote provider and transport seam.

pub mod catalog;
pub mod claude_cli;
pub mod cli_runtime;
pub mod codex_cli;
pub mod local;
pub mod opencode_cli;
pub mod remote;

use std::collections::BTreeMap;

use crate::schemas::ProviderId;
use crate::settings::StoredKeys;

pub use remote::{
    DESIGN_BUILD_TIMEOUT_SECONDS, DESIGN_TURN_BUILD_ATTEMPTS, HttpTransport,
    OPENAI_DEFAULT_ENDPOINT, OPENROUTER_DEFAULT_ENDPOINT, ReqwestTransport,
    STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS, STUDIO_REQUEST_TIMEOUT_SECONDS,
    design_turn_budget_seconds,
};

pub use cli_runtime::{CliProbe, CliSpec};

/// The provider ids the Studio publishes, in the published order.
///
/// `codex` is the historical id of the Codex CLI route. It stays in the list
/// so an old saved preference still resolves; it names the same CLI as
/// `codex_cli`.
pub const PROVIDER_IDS: &[&str] = &[
    "local",
    "openai",
    "openrouter",
    "codex",
    "claude_code",
    "codex_cli",
    "opencode_cli",
];

/// The source the Studio reports for a subscription CLI's credential.
///
/// The credential is the operator's own CLI login. Kala never sees it, never
/// stores it and never forwards it.
pub const SUBSCRIPTION_SOURCE: &str = "subscription (CLI)";

/// One row in `GET /api/studio/providers`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderRecord {
    pub id: &'static str,
    pub name: &'static str,
    pub configured: bool,
    pub notes: String,
    /// The availability probe's verdict, when the Studio probed this
    /// provider. `None` means the Studio did not probe it, never "available".
    pub available: Option<bool>,
    /// Why the provider is unavailable, when it is.
    pub reason: Option<String>,
    /// Where the credential comes from, when the Studio names a source.
    pub source: Option<&'static str>,
}

/// A chat reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatOutcome {
    /// The display text. It is bounded to
    /// [`MAX_RESPONSE_TEXT_LENGTH`](super::remote::MAX_RESPONSE_TEXT_LENGTH)
    /// characters, exactly like the Python `ChatResponse.text` parity bound.
    pub text: String,
    /// The model's full reply, before the display bound.
    ///
    /// A design inside a reply can be longer than the displayed prose, so
    /// the design JSON is extracted from this field. It is still bounded by
    /// the transport: 1 MB for the HTTP body and 1 MB of CLI standard output.
    pub reply: String,
    pub model: String,
    pub offline: bool,
}

impl ChatOutcome {
    /// Build one reply: the display text keeps the 4 096-character parity
    /// bound and the full reply is kept for design extraction.
    #[must_use]
    pub fn new(text: String, model: String, offline: bool) -> Self {
        let display: String = text
            .chars()
            .take(crate::providers::remote::MAX_RESPONSE_TEXT_LENGTH)
            .collect();
        Self {
            text: display,
            reply: text,
            model,
            offline,
        }
    }
}

/// A typed provider failure. The route maps each variant onto the same
/// statuses and codes as the Python server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderFailure {
    Unavailable(String),
    Configuration {
        code: &'static str,
        message: String,
    },
    Timeout(String),
    Network(String),
    Protocol(String),
    Subprocess(String),
    /// A failure the Python route does not map, and therefore returns as a
    /// sanitized 500 (for example an oversized response body).
    Internal(String),
}

impl ProviderFailure {
    #[must_use]
    pub fn configuration(message: impl Into<String>) -> Self {
        Self::Configuration {
            code: "provider_not_configured",
            message: message.into(),
        }
    }

    #[must_use]
    pub fn missing_credential(message: impl Into<String>) -> Self {
        Self::Configuration {
            code: "missing_api_key",
            message: message.into(),
        }
    }

    /// The envelope code the route answers for this failure.
    ///
    /// This is the single mapping shared by the chat route, the design turn's
    /// repair reason and the CLI providers, so a failure always names the same
    /// code wherever it is reported.
    #[must_use]
    pub fn envelope_code(&self) -> &'static str {
        match self {
            Self::Unavailable(_) => "provider_unavailable",
            Self::Configuration { code, .. } => code,
            Self::Timeout(_) => "provider_timeout",
            Self::Network(_) => "provider_network_error",
            Self::Protocol(_) => "provider_protocol_error",
            Self::Subprocess(_) => "provider_subprocess_error",
            Self::Internal(_) => "internal_error",
        }
    }
}

/// Whether a provider is configured by the stored settings or the
/// environment, exactly like the source overlay.
fn provider_configured(
    environ: &BTreeMap<String, String>,
    stored: &StoredKeys,
    provider: &str,
) -> bool {
    if crate::settings::stored_key_configured(stored, provider) {
        return true;
    }
    let env_name = crate::settings::PROVIDER_ENV_VARS
        .iter()
        .find(|(candidate, _)| *candidate == provider)
        .map(|(_, name)| *name)
        .unwrap_or("");
    !env_name.is_empty()
        && environ
            .get(env_name)
            .is_some_and(|value| !crate::pytext::trim(value).is_empty())
}

/// The notes of one subscription-CLI provider.
fn subscription_notes(name: &str, binary: &str, extra: &str) -> String {
    format!(
        "{name}. Runs the operator's own `{binary}` CLI as a sandboxed subprocess: no shell, \
         no tools, a private working directory, a sanitized environment and a bounded run. \
         {extra} Kala never sees or stores the CLI's credential. Requires \
         allow_remote=true at request time."
    )
}

/// The probe verdict of one subscription CLI, when the Studio has one.
fn probe_for<'a>(clis: &'a [CliProbe], id: &str) -> Option<&'a CliProbe> {
    clis.iter().find(|probe| probe.id == id)
}

/// One subscription-CLI provider row.
fn subscription_record(
    id: &'static str,
    name: &'static str,
    notes: String,
    probe: Option<&CliProbe>,
) -> ProviderRecord {
    let usable = probe.is_some_and(CliProbe::usable);
    // A probe that is still running is reported as unknown, never as
    // unavailable: `available: null` and the honest "checking" reason.
    let checking = probe.is_some_and(|probe| probe.checking);
    ProviderRecord {
        id,
        name,
        configured: usable,
        notes,
        available: match probe {
            Some(_) if checking => None,
            Some(_) => Some(usable),
            None => None,
        },
        reason: match probe {
            Some(_) if checking => {
                Some(crate::providers::cli_runtime::PROBE_CHECKING_REASON.to_owned())
            }
            Some(probe) if !usable => Some(probe.reason.clone()),
            _ => None,
        },
        source: Some(SUBSCRIPTION_SOURCE),
    }
}

/// Probe the three subscription CLIs on this host.
///
/// The probe is cheap and cached: it asks each CLI for its version and for
/// its own authentication status, and it never reads a credential file. Only
/// the exit status of the status command is used, so no account name, address
/// or token can reach a response.
///
/// This form waits for every CLI. The HTTP routes use
/// [`probe_subscription_clis_cached`] instead, which never waits for a CLI.
pub async fn probe_subscription_clis(environ: &BTreeMap<String, String>) -> Vec<CliProbe> {
    let (claude, codex, opencode) = tokio::join!(
        cli_runtime::probe(&claude_cli::SPEC, environ),
        cli_runtime::probe(&codex_cli::SPEC, environ),
        cli_runtime::probe(&opencode_cli::SPEC, environ),
    );
    vec![claude, codex, opencode]
}

/// The three subscription verdicts, answered from the cache.
///
/// No route may wait for a CLI: a cached (even stale) verdict is returned at
/// once and refreshed in the background, and a first probe that has not
/// finished inside the route budget is reported as `checking`.
pub async fn probe_subscription_clis_cached(environ: &BTreeMap<String, String>) -> Vec<CliProbe> {
    cli_runtime::probe_subscription_clis_cached(
        &[claude_cli::SPEC, codex_cli::SPEC, opencode_cli::SPEC],
        environ,
    )
    .await
}

/// Return the sanitized provider list in the published order.
///
/// The subscription rows carry the availability probe's verdict: a CLI that
/// is installed and signed in is `configured`; one that is missing or not
/// signed in is reported as unavailable with the honest reason.
#[must_use]
pub fn list_provider_records(
    environ: &BTreeMap<String, String>,
    stored: &StoredKeys,
    clis: &[CliProbe],
) -> Vec<ProviderRecord> {
    let openai_configured = provider_configured(environ, stored, "openai");
    let openrouter_configured = provider_configured(environ, stored, "openrouter");
    let codex_probe = probe_for(clis, codex_cli::PROVIDER_ID);
    vec![
        ProviderRecord {
            id: "local",
            name: "Local Deterministic Helper",
            configured: true,
            notes: "Offline deterministic mechanical-engineering helper. Extracts dimensions and returns nominal analytic volume/surface-area estimates, and documents the built-in OpenCascade (OCP) STEP/STL estimate, neutral CAD/CAE handoff, and standalone build workflow. Opens no network connection and runs no solver.".to_owned(),
            available: None,
            reason: None,
            source: None,
        },
        ProviderRecord {
            id: "openai",
            name: "OpenAI (Chat Completions)",
            configured: openai_configured,
            notes: format!(
                "API-key route; uses the standard Chat Completions endpoint at {OPENAI_DEFAULT_ENDPOINT}. Requires allow_remote=true at request time."
            ),
            available: None,
            reason: None,
            source: None,
        },
        ProviderRecord {
            id: "openrouter",
            name: "OpenRouter (Chat Completions)",
            configured: openrouter_configured,
            notes: format!(
                "API-key route; uses the public OpenRouter endpoint at {OPENROUTER_DEFAULT_ENDPOINT}. Requires allow_remote=true at request time."
            ),
            available: None,
            reason: None,
            source: None,
        },
        subscription_record(
            "codex",
            "Codex CLI (ChatGPT subscription)",
            subscription_notes(
                "Codex CLI (ChatGPT subscription)",
                "codex",
                "The turn runs in a read-only sandbox with no session transcript and without the operator's own Codex configuration.",
            ),
            codex_probe,
        ),
        subscription_record(
            claude_cli::PROVIDER_ID,
            "Claude Code CLI (Anthropic subscription)",
            subscription_notes(
                "Claude Code CLI (Anthropic subscription)",
                "claude",
                "The turn runs with every built-in tool removed and with the operator's hooks, plugins, skills and MCP servers switched off.",
            ),
            probe_for(clis, claude_cli::PROVIDER_ID),
        ),
        subscription_record(
            codex_cli::PROVIDER_ID,
            "Codex CLI (ChatGPT subscription)",
            subscription_notes(
                "Codex CLI (ChatGPT subscription)",
                "codex",
                "The turn runs in a read-only sandbox with no session transcript and without the operator's own Codex configuration.",
            ),
            codex_probe,
        ),
        subscription_record(
            opencode_cli::PROVIDER_ID,
            "OpenCode CLI (subscription)",
            subscription_notes(
                "OpenCode CLI (subscription)",
                "opencode",
                "The turn runs as an agent that may not call any tool, in a private working directory.",
            ),
            probe_for(clis, opencode_cli::PROVIDER_ID),
        ),
    ]
}

/// Dispatch one chat request.
///
/// `timeout` is the whole-request budget for a remote OpenAI-compatible call.
/// A plain chat passes [`remote::STUDIO_REQUEST_TIMEOUT_SECONDS`]; the design
/// path passes [`remote::STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS`]. The CLI
/// providers keep their own 180 s budget: the argument is unused on their
/// route, so it cannot lengthen or shorten a subprocess.
///
/// # Errors
///
/// Returns the typed provider failure the route maps onto a sanitized body.
#[allow(clippy::too_many_arguments)]
pub async fn dispatch(
    provider: ProviderId,
    model: Option<&str>,
    prompt: &str,
    allow_remote: bool,
    environ: &BTreeMap<String, String>,
    stored: &StoredKeys,
    transport: &dyn HttpTransport,
    timeout: std::time::Duration,
) -> Result<ChatOutcome, ProviderFailure> {
    match provider {
        ProviderId::Local => local::local_chat(prompt, model),
        ProviderId::OpenAi => {
            let Some(model) = model.filter(|value| !value.is_empty()) else {
                return Err(ProviderFailure::configuration(
                    "openai provider requires a model id (e.g. 'gpt-4o-mini')",
                ));
            };
            remote::openai_compat_chat(
                "openai",
                model,
                prompt,
                allow_remote,
                environ,
                stored,
                transport,
                timeout,
            )
            .await
        }
        ProviderId::OpenRouter => {
            let Some(model) = model.filter(|value| !value.is_empty()) else {
                return Err(ProviderFailure::configuration(
                    "openrouter provider requires a model id",
                ));
            };
            remote::openai_compat_chat(
                "openrouter",
                model,
                prompt,
                allow_remote,
                environ,
                stored,
                transport,
                timeout,
            )
            .await
        }
        ProviderId::Codex => {
            if !allow_remote {
                return Err(ProviderFailure::configuration(
                    cli_runtime::consent_message(codex_cli::LEGACY_PROVIDER_ID),
                ));
            }
            codex_cli::chat(model, prompt, environ).await
        }
        ProviderId::CodexCli => {
            if !allow_remote {
                return Err(ProviderFailure::configuration(
                    cli_runtime::consent_message(codex_cli::PROVIDER_ID),
                ));
            }
            codex_cli::chat(model, prompt, environ).await
        }
        ProviderId::ClaudeCode => {
            if !allow_remote {
                return Err(ProviderFailure::configuration(
                    cli_runtime::consent_message(claude_cli::PROVIDER_ID),
                ));
            }
            claude_cli::chat(model, prompt, environ).await
        }
        ProviderId::OpenCodeCli => {
            if !allow_remote {
                return Err(ProviderFailure::configuration(
                    cli_runtime::consent_message(opencode_cli::PROVIDER_ID),
                ));
            }
            opencode_cli::chat(model, prompt, environ).await
        }
    }
}

/// Whether either build-reachable remote provider has an API credential.
///
/// This is the API-key half of the "is a remote turn possible" question.
/// [`any_remote_route_available`] is the whole answer the Studio reports as
/// `offline`.
#[must_use]
pub fn any_remote_provider_configured(
    environ: &BTreeMap<String, String>,
    stored: &StoredKeys,
) -> bool {
    provider_configured(environ, stored, "openai")
        || provider_configured(environ, stored, "openrouter")
}

/// Whether a remote turn is *known* to be possible right now.
///
/// A remote route is available when an OpenAI-compatible credential is
/// configured **or** a subscription CLI is installed and signed in. The
/// `offline` field of `/health` and `/settings` is the negation of this
/// answer; it is therefore never "offline" while a usable CLI is on the
/// host. A probe that has not produced a verdict yet does not count, so the
/// field can stay conservative for at most the route probe budget.
#[must_use]
pub fn any_remote_route_available(
    environ: &BTreeMap<String, String>,
    stored: &StoredKeys,
    clis: &[CliProbe],
) -> bool {
    any_remote_provider_configured(environ, stored) || clis.iter().any(CliProbe::usable)
}

#[cfg(test)]
mod tests {
    use super::ProviderFailure;

    #[test]
    fn every_provider_failure_names_its_envelope_code() {
        assert_eq!(
            ProviderFailure::Unavailable("off".to_owned()).envelope_code(),
            "provider_unavailable"
        );
        assert_eq!(
            ProviderFailure::configuration("no key").envelope_code(),
            "provider_not_configured"
        );
        assert_eq!(
            ProviderFailure::missing_credential("no key").envelope_code(),
            "missing_api_key"
        );
        assert_eq!(
            ProviderFailure::Configuration {
                code: "provider_rate_limited",
                message: "slow down".to_owned(),
            }
            .envelope_code(),
            "provider_rate_limited"
        );
        assert_eq!(
            ProviderFailure::Timeout("slow".to_owned()).envelope_code(),
            "provider_timeout"
        );
        assert_eq!(
            ProviderFailure::Network("down".to_owned()).envelope_code(),
            "provider_network_error"
        );
        assert_eq!(
            ProviderFailure::Protocol("garbage".to_owned()).envelope_code(),
            "provider_protocol_error"
        );
        assert_eq!(
            ProviderFailure::Subprocess("crashed".to_owned()).envelope_code(),
            "provider_subprocess_error"
        );
        assert_eq!(
            ProviderFailure::Internal("too large".to_owned()).envelope_code(),
            "internal_error"
        );
    }
}
