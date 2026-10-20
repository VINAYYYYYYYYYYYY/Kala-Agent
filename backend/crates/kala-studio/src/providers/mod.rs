//! Provider dispatch, fixed-endpoint remote provider and transport seam.

pub mod local;
pub mod remote;

use std::collections::BTreeMap;

use crate::schemas::ProviderId;
use crate::settings::StoredKeys;

pub use remote::{
    HttpTransport, OPENAI_DEFAULT_ENDPOINT, OPENROUTER_DEFAULT_ENDPOINT, ReqwestTransport,
    STUDIO_REQUEST_TIMEOUT_SECONDS,
};

/// One row in `GET /api/studio/providers`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderRecord {
    pub id: &'static str,
    pub name: &'static str,
    pub configured: bool,
    pub notes: String,
}

/// A chat reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatOutcome {
    pub text: String,
    pub model: String,
    pub offline: bool,
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
}

pub const CODEX_UNAVAILABLE_MESSAGE: &str = "provider=codex is not available in this studio build; the verified Codex CLI subprocess integration is not shipped yet, so no request is sent. Use provider=openai or provider=openrouter with an API key for remote chat.";

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

/// Return the sanitized provider list in frozen order.
#[must_use]
pub fn list_provider_records(
    environ: &BTreeMap<String, String>,
    stored: &StoredKeys,
) -> Vec<ProviderRecord> {
    let openai_configured = provider_configured(environ, stored, "openai");
    let openrouter_configured = provider_configured(environ, stored, "openrouter");
    vec![
        ProviderRecord {
            id: "local",
            name: "Local Deterministic Helper",
            configured: true,
            notes: "Offline deterministic mechanical-engineering helper. Extracts dimensions and returns nominal analytic volume/surface-area estimates, and documents the built-in OpenCascade (OCP) STEP/STL estimate, neutral CAD/CAE handoff, and standalone build workflow. Opens no network connection and runs no solver.".to_owned(),
        },
        ProviderRecord {
            id: "openai",
            name: "OpenAI (Chat Completions)",
            configured: openai_configured,
            notes: format!(
                "API-key route; uses the standard Chat Completions endpoint at {OPENAI_DEFAULT_ENDPOINT}. Requires allow_remote=true at request time."
            ),
        },
        ProviderRecord {
            id: "openrouter",
            name: "OpenRouter (Chat Completions)",
            configured: openrouter_configured,
            notes: format!(
                "API-key route; uses the public OpenRouter endpoint at {OPENROUTER_DEFAULT_ENDPOINT}. Requires allow_remote=true at request time."
            ),
        },
        ProviderRecord {
            id: "codex",
            name: "Codex CLI (ChatGPT subscription)",
            configured: false,
            notes: "Codex CLI (ChatGPT subscription). This build does not ship the verified codex subprocess integration, so the provider is always unavailable and a request returns a 503. No request is sent.".to_owned(),
        },
    ]
}

/// Dispatch one chat request.
///
/// # Errors
///
/// Returns the typed provider failure the route maps onto a sanitized body.
pub async fn dispatch(
    provider: ProviderId,
    model: Option<&str>,
    prompt: &str,
    allow_remote: bool,
    environ: &BTreeMap<String, String>,
    stored: &StoredKeys,
    transport: &dyn HttpTransport,
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
            )
            .await
        }
        ProviderId::Codex => {
            if !allow_remote {
                return Err(ProviderFailure::configuration(
                    "provider=codex requires allow_remote=true at request time",
                ));
            }
            Err(ProviderFailure::Unavailable(
                CODEX_UNAVAILABLE_MESSAGE.to_owned(),
            ))
        }
    }
}

/// Whether either build-reachable remote provider has a credential.
#[must_use]
pub fn any_remote_provider_configured(
    environ: &BTreeMap<String, String>,
    stored: &StoredKeys,
) -> bool {
    provider_configured(environ, stored, "openai")
        || provider_configured(environ, stored, "openrouter")
}
