//! OpenAI-compatible remote chat with a fixed allowlist and bounded transport.
//!
//! Endpoints are hard-coded, redirects are refused, the credential is read
//! only from the documented environment name, the request timeout is 30
//! seconds, and the response body is capped at 1,000,000 bytes. A
//! [`HttpTransport`] seam exists so tests can verify the wire request and
//! error mapping without a live provider; the production implementation is
//! [`ReqwestTransport`].

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde_json::Value;

use crate::json::quote_ascii;
use crate::json_input::{JsonInputError, decode_json_bytes};
use crate::pytext;
use crate::schemas::has_overwide_integer;

use super::{ChatOutcome, ProviderFailure};

pub const OPENAI_DEFAULT_ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";
pub const OPENROUTER_DEFAULT_ENDPOINT: &str = "https://openrouter.ai/api/v1/chat/completions";
pub const STUDIO_REQUEST_TIMEOUT_SECONDS: u64 = 30;
pub const MAX_HTTP_RESPONSE_BYTES: usize = 1_000_000;
pub const MAX_KEY_LENGTH: usize = 256;
pub const MAX_RESPONSE_TEXT_LENGTH: usize = 4_096;

const REFERER: &str = "https://kala-agent.local/studio";

/// One outbound HTTP request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteRequest {
    pub url: &'static str,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub timeout: Duration,
}

/// One HTTP response after bounded reading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteResponse {
    pub status: u16,
    pub reason: Option<String>,
    pub content_length: Option<u64>,
    pub body: Vec<u8>,
}

/// Transport failures with no credential or URL text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransportError {
    Timeout,
    Network(String),
    TooLarge(String),
    /// A configuration failure that the Python source does not map to a
    /// provider error and therefore answers with a sanitized 500 (for
    /// example an outbound header that cannot be encoded, or a malformed
    /// proxy URL).
    Internal(String),
}

/// Isolated transport seam for tests and the production reqwest client.
pub trait HttpTransport: Send + Sync {
    fn post<'a>(
        &'a self,
        request: RemoteRequest,
    ) -> Pin<Box<dyn Future<Output = Result<RemoteResponse, TransportError>> + Send + 'a>>;
}

/// Production HTTPS transport backed by reqwest + rustls.
#[derive(Debug)]
pub struct ReqwestTransport {
    /// The direct client (no proxy attached).
    client: reqwest::Client,
    /// One client per proxy URL, built lazily like Python's per-request
    /// `build_opener`.
    proxied: std::sync::Mutex<std::collections::HashMap<String, reqwest::Client>>,
}

/// Build one provider client: no redirects, fixed timeout, Python proxy
/// semantics resolved per request, and the source's system trust model.
fn build_client(proxy: Option<&str>) -> Result<reqwest::Client, String> {
    let builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(STUDIO_REQUEST_TIMEOUT_SECONDS))
        .no_proxy();
    let builder = crate::tls::configure_roots(builder)?;
    let builder = match proxy {
        Some(proxy_url) => builder.proxy(
            reqwest::Proxy::https(proxy_url)
                .map_err(|_| "the configured proxy could not be parsed".to_owned())?,
        ),
        None => builder,
    };
    builder
        .build()
        .map_err(|_| "the provider HTTPS client could not be initialised".to_owned())
}

impl ReqwestTransport {
    /// Build a client that never follows redirects and has a fixed timeout.
    ///
    /// Proxy handling is *not* delegated to reqwest's system matcher: the
    /// transport resolves Python's `urllib` environment rules per request
    /// (`src/proxy.rs`) and attaches the resulting proxy explicitly, so
    /// `ALL_PROXY`, uppercase/lowercase precedence and the CGI
    /// `HTTP_PROXY` suppression behave like the source.
    ///
    /// # Errors
    ///
    /// Returns a sanitized message when the client cannot be constructed.
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            client: build_client(None)?,
            proxied: std::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }

    /// The client to use for one proxy URL, built once per URL.
    fn client_for_proxy(&self, proxy_url: &str) -> Result<reqwest::Client, TransportError> {
        let mut cache = self
            .proxied
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(client) = cache.get(proxy_url) {
            return Ok(client.clone());
        }
        let client = build_client(Some(proxy_url)).map_err(TransportError::Internal)?;
        cache.insert(proxy_url.to_owned(), client.clone());
        Ok(client)
    }
}

/// The value `urllib` would use as `req.host` for proxy bypass.
fn request_host(url: &reqwest::Url) -> String {
    let Some(host) = url.host_str() else {
        return String::new();
    };
    let default_port = match url.scheme() {
        "https" => Some(443),
        "http" => Some(80),
        _ => None,
    };
    match url.port() {
        Some(port) if Some(port) != default_port => format!("{host}:{port}"),
        _ => host.to_owned(),
    }
}

impl HttpTransport for ReqwestTransport {
    fn post<'a>(
        &'a self,
        request: RemoteRequest,
    ) -> Pin<Box<dyn Future<Output = Result<RemoteResponse, TransportError>> + Send + 'a>> {
        Box::pin(async move {
            // `urllib` reads the process environment per request
            // (`build_opener` constructs a fresh `ProxyHandler`), so the same
            // resolution runs here rather than at client construction.
            let client = match reqwest::Url::parse(request.url) {
                Ok(url) if url.scheme() == "https" => {
                    let environ = crate::boundary::process_environ();
                    match crate::proxy::proxy_for(&environ, "https", &request_host(&url)) {
                        Some(proxy_url) => self.client_for_proxy(&proxy_url)?,
                        None => self.client.clone(),
                    }
                }
                _ => self.client.clone(),
            };
            let mut builder = client
                .post(request.url)
                .timeout(request.timeout)
                .body(request.body);
            for (name, value) in request.headers {
                builder = builder.header(name, value);
            }
            let response = match builder.send().await {
                Ok(response) => response,
                Err(error) => return Err(map_reqwest_error(&error)),
            };
            let status = response.status().as_u16();
            let reason = response
                .status()
                .canonical_reason()
                .map(std::string::ToString::to_string);
            let content_length = response.content_length();
            if content_length.is_some_and(|value| value > MAX_HTTP_RESPONSE_BYTES as u64) {
                return Err(TransportError::TooLarge(
                    "the provider response is larger than the bounded size".to_owned(),
                ));
            }
            let mut response = response;
            let mut body = Vec::new();
            loop {
                match response.chunk().await {
                    Ok(Some(chunk)) => {
                        if body.len() + chunk.len() > MAX_HTTP_RESPONSE_BYTES {
                            return Err(TransportError::TooLarge(
                                "the provider response exceeded the bounded size during read"
                                    .to_owned(),
                            ));
                        }
                        body.extend_from_slice(&chunk);
                    }
                    Ok(None) => break,
                    Err(error) => return Err(map_reqwest_error(&error)),
                }
            }
            Ok(RemoteResponse {
                status,
                reason,
                content_length,
                body,
            })
        })
    }
}

fn map_reqwest_error(error: &reqwest::Error) -> TransportError {
    if error.is_timeout() {
        return TransportError::Timeout;
    }
    let debug = format!("{error:?}");
    let class = if error.is_connect() {
        if debug.to_ascii_lowercase().contains("dns") {
            "gaierror"
        } else {
            "ConnectionRefusedError"
        }
    } else if debug.to_ascii_lowercase().contains("tls")
        || debug.to_ascii_lowercase().contains("certificate")
    {
        "SSLError"
    } else {
        "URLError"
    };
    TransportError::Network(class.to_owned())
}

fn endpoint_for(provider: &str) -> Result<&'static str, ProviderFailure> {
    match provider {
        "openai" => Ok(OPENAI_DEFAULT_ENDPOINT),
        "openrouter" => Ok(OPENROUTER_DEFAULT_ENDPOINT),
        _ => Err(ProviderFailure::configuration(format!(
            "no fixed endpoint registered for provider '{provider}'"
        ))),
    }
}

fn credential(
    provider: &str,
    environ: &BTreeMap<String, String>,
    stored: &crate::settings::StoredKeys,
) -> Result<String, ProviderFailure> {
    let name = match provider {
        "openai" => "OPENAI_API_KEY",
        "openrouter" => "OPENROUTER_API_KEY",
        _ => "",
    };
    // The source overlays stored keys on the environment, so a stored value
    // wins. A stored value with a lone surrogate is returned by the source
    // `_credential` and then fails while urllib encodes the header
    // (`UnicodeEncodeError`), which the route reports as a sanitized 500.
    if let Some(value) = stored.get(provider) {
        if !value.code_points().is_empty() {
            return match value.to_utf8() {
                Ok(text) => {
                    if text.chars().count() > MAX_KEY_LENGTH {
                        Err(ProviderFailure::configuration(format!(
                            "{name} is longer than the bounded length"
                        )))
                    } else {
                        Ok(text)
                    }
                }
                Err(_) => Err(ProviderFailure::Internal(
                    "the provider credential cannot be encoded as a header".to_owned(),
                )),
            };
        }
    }
    if !name.is_empty() {
        if let Some(value) = environ.get(name) {
            let stripped = pytext::trim(value);
            if !stripped.is_empty() {
                if stripped.chars().count() > MAX_KEY_LENGTH {
                    return Err(ProviderFailure::configuration(format!(
                        "{name} is longer than the bounded length"
                    )));
                }
                return Ok(stripped.to_owned());
            }
        }
    }
    Err(ProviderFailure::missing_credential(format!(
        "provider '{provider}' is not configured (set the documented env key)"
    )))
}

fn request_body(model: &str, prompt: &str) -> Vec<u8> {
    let body = format!(
        "{{\"model\": {}, \"messages\": [{{\"role\": \"user\", \"content\": {}}}], \"temperature\": 0.2, \"stream\": false}}",
        quote_ascii(model),
        quote_ascii(prompt)
    );
    body.into_bytes()
}

fn request_headers(key: &str) -> Vec<(String, String)> {
    vec![
        ("Authorization".to_owned(), format!("Bearer {key}")),
        ("Content-Type".to_owned(), "application/json".to_owned()),
        ("Accept".to_owned(), "application/json".to_owned()),
        ("User-Agent".to_owned(), "kala-studio/0.1".to_owned()),
        ("HTTP-Referer".to_owned(), REFERER.to_owned()),
        ("X-Title".to_owned(), "kala-studio".to_owned()),
    ]
}

/// Send one bounded chat request to the fixed provider endpoint.
///
/// # Errors
///
/// Returns the typed provider failure the Studio route maps to a sanitized
/// status. No endpoint or credential appears in a failure message.
pub async fn openai_compat_chat(
    provider: &str,
    model: &str,
    prompt: &str,
    allow_remote: bool,
    environ: &BTreeMap<String, String>,
    stored: &crate::settings::StoredKeys,
    transport: &dyn HttpTransport,
) -> Result<ChatOutcome, ProviderFailure> {
    if !allow_remote {
        return Err(ProviderFailure::configuration(
            "remote calls require explicit allow_remote=true",
        ));
    }
    let endpoint = endpoint_for(provider)?;
    let key = credential(provider, environ, stored)?;
    let request = RemoteRequest {
        url: endpoint,
        headers: request_headers(&key),
        body: request_body(model, prompt),
        timeout: Duration::from_secs(STUDIO_REQUEST_TIMEOUT_SECONDS),
    };
    let response = match transport.post(request).await {
        Ok(response) => response,
        Err(TransportError::Timeout) => {
            return Err(ProviderFailure::Timeout(
                "the provider request exceeded its bounded timeout".to_owned(),
            ));
        }
        Err(TransportError::Network(class)) => {
            return Err(ProviderFailure::Network(format!(
                "the provider network call failed: {class}"
            )));
        }
        Err(TransportError::TooLarge(message)) | Err(TransportError::Internal(message)) => {
            return Err(ProviderFailure::Internal(message));
        }
    };
    if (300..400).contains(&response.status) {
        return Err(ProviderFailure::Network(
            "redirects to a different origin are refused".to_owned(),
        ));
    }
    if !(200..300).contains(&response.status) {
        return Err(ProviderFailure::Protocol(format!(
            "the provider returned HTTP {}: {}",
            response.status,
            response.reason.unwrap_or_default()
        )));
    }
    if response
        .content_length
        .is_some_and(|value| value > MAX_HTTP_RESPONSE_BYTES as u64)
    {
        return Err(ProviderFailure::Internal(
            "the provider response is larger than the bounded size".to_owned(),
        ));
    }
    if response.body.len() > MAX_HTTP_RESPONSE_BYTES {
        return Err(ProviderFailure::Internal(
            "the provider response exceeded the bounded size during read".to_owned(),
        ));
    }
    let (text, observed_model) = read_response(&response.body)?;
    let text: String = text.chars().take(MAX_RESPONSE_TEXT_LENGTH).collect();
    Ok(ChatOutcome {
        text,
        model: observed_model.unwrap_or_else(|| model.to_owned()),
        offline: false,
    })
}

fn read_response(data: &[u8]) -> Result<(String, Option<String>), ProviderFailure> {
    // CPython `json.loads(bytes)` detects UTF-8 BOM, UTF-16 and UTF-32; the
    // source provider parser inherits that accepted domain. A decode failure
    // propagates as `UnicodeDecodeError` in Python and becomes the route's
    // sanitized 500, not a protocol 502.
    let text = match decode_json_bytes(data) {
        Ok(text) => text,
        Err(JsonInputError::Encoding | JsonInputError::Surrogate(_)) => {
            return Err(ProviderFailure::Internal(
                "the provider response was not valid UTF-8".to_owned(),
            ));
        }
        Err(JsonInputError::Syntax(_)) => {
            return Err(ProviderFailure::Protocol(
                "the provider response was not valid JSON".to_owned(),
            ));
        }
    };
    let substituted = crate::json_input::substitute_lone_surrogates(&text);
    let parse_source = substituted.as_deref().unwrap_or(&text);
    let normalised = crate::json_input::normalise_json_bytes(parse_source).map_err(|_| {
        ProviderFailure::Protocol("the provider response was not valid JSON".to_owned())
    })?;
    let payload: Value = serde_json::from_slice(&normalised).map_err(|_| {
        ProviderFailure::Protocol("the provider response was not valid JSON".to_owned())
    })?;
    if has_overwide_integer(&payload) {
        // CPython raises a plain `ValueError` for its integer digit limit,
        // which the source route does not map and therefore returns as 500.
        return Err(ProviderFailure::Internal(
            "the provider response exceeded the integer digit limit".to_owned(),
        ));
    }
    let Value::Object(object) = payload else {
        return Err(ProviderFailure::Protocol(
            "the provider response was not a JSON object".to_owned(),
        ));
    };
    let Some(Value::Array(choices)) = object.get("choices") else {
        return Err(ProviderFailure::Protocol(
            "the provider response did not declare any choices".to_owned(),
        ));
    };
    let Some(first) = choices.first() else {
        return Err(ProviderFailure::Protocol(
            "the provider response did not declare any choices".to_owned(),
        ));
    };
    let Value::Object(first) = first else {
        return Err(ProviderFailure::Protocol(
            "the provider response had a malformed choice".to_owned(),
        ));
    };
    let mut content = "";
    if let Some(Value::Object(message)) = first.get("message") {
        if let Some(Value::String(value)) = message.get("content") {
            content = value;
        }
    }
    if content.is_empty() {
        return Err(ProviderFailure::Protocol(
            "the provider choice had no readable message content".to_owned(),
        ));
    }
    // Pydantic's `ChatResponse.text` rejects a lone surrogate; the source
    // slices the text to 4096 characters before constructing the model.
    if content
        .chars()
        .take(MAX_RESPONSE_TEXT_LENGTH)
        .any(|character| character == crate::json_input::LONE_SURROGATE_SENTINEL)
    {
        return Err(ProviderFailure::Internal(
            "the provider response contained a lone surrogate".to_owned(),
        ));
    }
    let observed_model = match object.get("model") {
        Some(Value::String(value)) if !value.is_empty() => Some(value.clone()),
        _ => None,
    };
    if observed_model
        .as_deref()
        .is_some_and(|model| model.contains(crate::json_input::LONE_SURROGATE_SENTINEL))
    {
        return Err(ProviderFailure::Internal(
            "the provider response contained a lone surrogate".to_owned(),
        ));
    }
    Ok((content.to_owned(), observed_model))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeTransport {
        response: Mutex<Option<Result<RemoteResponse, TransportError>>>,
        last: Mutex<Option<RemoteRequest>>,
        captured: bool,
    }

    impl FakeTransport {
        fn new(response: Result<RemoteResponse, TransportError>) -> Self {
            Self {
                response: Mutex::new(Some(response)),
                last: Mutex::new(None),
                captured: false,
            }
        }
    }

    impl HttpTransport for FakeTransport {
        fn post<'a>(
            &'a self,
            request: RemoteRequest,
        ) -> Pin<Box<dyn Future<Output = Result<RemoteResponse, TransportError>> + Send + 'a>>
        {
            *self.last.lock().unwrap() = Some(request);
            let response = self
                .response
                .lock()
                .unwrap()
                .take()
                .unwrap_or(Err(TransportError::Network("unused".to_owned())));
            let _ = self.captured;
            Box::pin(async move { response })
        }
    }

    fn environment() -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        map.insert("OPENAI_API_KEY".to_owned(), "sk-test".to_owned());
        map
    }

    #[tokio::test]
    async fn wire_request_matches_python_shape() {
        let transport = FakeTransport::new(Ok(RemoteResponse {
            status: 200,
            reason: Some("OK".to_owned()),
            content_length: Some(74),
            body: br#"{"model":"gpt-4o-mini","choices":[{"message":{"content":"hi"}}]}"#.to_vec(),
        }));
        let outcome = openai_compat_chat(
            "openai",
            "gpt-4o-mini",
            "hello",
            true,
            &environment(),
            &crate::settings::StoredKeys::new(),
            &transport,
        )
        .await
        .unwrap();
        assert_eq!(outcome.text, "hi");
        assert_eq!(outcome.model, "gpt-4o-mini");
        let request = transport.last.lock().unwrap().clone().unwrap();
        assert_eq!(request.url, OPENAI_DEFAULT_ENDPOINT);
        assert_eq!(
            String::from_utf8(request.body).unwrap(),
            "{\"model\": \"gpt-4o-mini\", \"messages\": [{\"role\": \"user\", \"content\": \"hello\"}], \"temperature\": 0.2, \"stream\": false}"
        );
        assert_eq!(request.headers[0].1, "Bearer sk-test");
    }

    #[tokio::test]
    async fn protocol_and_network_failures_are_typed() {
        let transport = FakeTransport::new(Ok(RemoteResponse {
            status: 500,
            reason: Some("Internal Server Error".to_owned()),
            content_length: None,
            body: Vec::new(),
        }));
        let failure = openai_compat_chat(
            "openai",
            "m",
            "p",
            true,
            &environment(),
            &crate::settings::StoredKeys::new(),
            &transport,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(failure, ProviderFailure::Protocol(message) if message.contains("HTTP 500"))
        );

        let transport = FakeTransport::new(Err(TransportError::Timeout));
        let failure = openai_compat_chat(
            "openai",
            "m",
            "p",
            true,
            &environment(),
            &crate::settings::StoredKeys::new(),
            &transport,
        )
        .await
        .unwrap_err();
        assert!(matches!(failure, ProviderFailure::Timeout(_)));
    }

    #[tokio::test]
    async fn redirects_are_refused() {
        let transport = FakeTransport::new(Ok(RemoteResponse {
            status: 302,
            reason: Some("Found".to_owned()),
            content_length: None,
            body: Vec::new(),
        }));
        let failure = openai_compat_chat(
            "openai",
            "m",
            "p",
            true,
            &environment(),
            &crate::settings::StoredKeys::new(),
            &transport,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            failure,
            ProviderFailure::Network(message) if message.contains("redirects")
        ));
    }
}
