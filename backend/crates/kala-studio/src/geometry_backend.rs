//! The geometry backend seam and the Python worker implementation.
//!
//! The front door never computes geometry. Every geometry, design, journal
//! and import route is answered by a [`GeometryBackend`], and the production
//! implementation ([`PythonWorkerBackend`]) forwards the request to the
//! supervised Python/OCP worker over an authenticated loopback connection.
//!
//! The seam exists so the routing, the limits and the refusals can be tested
//! without a Python process, and so a later native OCCT adapter can replace
//! the worker without touching the HTTP surface.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::RwLock;

use crate::worker::{GeometryWorker, WORKER_HOST, WORKER_INTERNAL_PREFIX};

/// The bounded time one proxied worker request may take.
pub const WORKER_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
/// The largest worker response body the front door accepts.
pub const MAX_WORKER_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
/// The largest journal page the worker is asked for.
pub const MAX_JOURNAL_LIMIT: usize = 500;
/// The default journal page size.
pub const DEFAULT_JOURNAL_LIMIT: usize = 200;
/// The response headers the front door copies from the worker.
pub const FORWARDED_RESPONSE_HEADERS: &[&str] = &[
    "content-type",
    "content-disposition",
    "x-studio-handoff",
    "x-studio-design-id",
    "x-studio-digest",
    "x-studio-artifact-kind",
    "www-authenticate",
];

/// A boxed future, so the backend trait stays object safe.
pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One worker answer, forwarded to the client unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerResponse {
    /// The HTTP status the worker reported.
    pub status: u16,
    /// The headers the front door forwards.
    pub headers: Vec<(String, String)>,
    /// The body bytes, unchanged.
    pub body: Vec<u8>,
}

impl WorkerResponse {
    /// Whether the worker answered with a 2xx status.
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The body parsed as JSON, when it is JSON.
    #[must_use]
    pub fn json(&self) -> Option<Value> {
        serde_json::from_slice(&self.body).ok()
    }
}

/// The health facts the front door publishes for the geometry backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendHealth {
    /// The lifecycle state of the backend.
    pub state: String,
    /// The version the backend reports, when it has one.
    pub version: Option<String>,
    /// Whether the backend can compile a design right now.
    pub design_available: bool,
    /// Whether the backend can export STEP right now.
    pub step_available: bool,
}

impl BackendHealth {
    /// The health of a backend that is not attached.
    #[must_use]
    pub fn unavailable() -> Self {
        Self {
            state: "stopped".to_owned(),
            version: None,
            design_available: false,
            step_available: false,
        }
    }
}

/// Why a geometry backend could not answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendError {
    /// The worker is not running; the front door answers 503.
    Unavailable(String),
    /// The worker did not answer inside the bounded timeout.
    Timeout(String),
    /// The worker could not be reached.
    Transport(String),
    /// The worker answered with more bytes than the front door accepts.
    TooLarge(String),
    /// The backend does not implement the route.
    Unsupported(String),
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message)
            | Self::Timeout(message)
            | Self::Transport(message)
            | Self::TooLarge(message)
            | Self::Unsupported(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for BackendError {}

/// One geometry, design, journal or import route.
///
/// Every method mirrors exactly one worker route. The bodies are the raw
/// request bytes and the answers are the raw worker bytes: the front door
/// forwards the payload unchanged so a field the worker adds is never
/// silently dropped.
pub trait GeometryBackend: Send + Sync {
    /// The health facts the front door publishes in `/health`.
    fn health(&self) -> BackendFuture<'_, BackendHealth> {
        Box::pin(async { BackendHealth::unavailable() })
    }

    /// `POST /internal/v1/preview`.
    fn preview<'a>(
        &'a self,
        _body: &'a [u8],
        _content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async { Err(BackendError::Unsupported("preview".to_owned())) })
    }

    /// `POST /internal/v1/export`.
    fn export<'a>(
        &'a self,
        _body: &'a [u8],
        _content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async { Err(BackendError::Unsupported("export".to_owned())) })
    }

    /// `POST /internal/v1/handoff`.
    fn handoff<'a>(
        &'a self,
        _body: &'a [u8],
        _content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async { Err(BackendError::Unsupported("handoff".to_owned())) })
    }

    /// `POST /internal/v1/design`.
    fn design<'a>(
        &'a self,
        _body: &'a [u8],
        _content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async { Err(BackendError::Unsupported("design".to_owned())) })
    }

    /// `GET /internal/v1/design/{design_id}/drawing`.
    fn drawing<'a>(
        &'a self,
        _design_id: &'a str,
        _query: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async { Err(BackendError::Unsupported("drawing".to_owned())) })
    }

    /// `GET /internal/v1/design/{design_id}/section`.
    fn section<'a>(
        &'a self,
        _design_id: &'a str,
        _query: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async { Err(BackendError::Unsupported("section".to_owned())) })
    }

    /// `POST /internal/v1/import` — the view-only STEP import.
    fn import_step<'a>(
        &'a self,
        _body: &'a [u8],
        _content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async { Err(BackendError::Unsupported("import".to_owned())) })
    }

    /// `POST /internal/v1/intent/parse`.
    fn intent_parse<'a>(
        &'a self,
        _body: &'a [u8],
        _content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async { Err(BackendError::Unsupported("intent/parse".to_owned())) })
    }

    /// `POST /internal/v1/intent/validate`.
    fn intent_validate<'a>(
        &'a self,
        _body: &'a [u8],
        _content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async { Err(BackendError::Unsupported("intent/validate".to_owned())) })
    }

    /// `GET /internal/v1/intent/schema`.
    fn intent_schema<'a>(&'a self) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async { Err(BackendError::Unsupported("intent/schema".to_owned())) })
    }

    /// `GET /internal/v1/journal`.
    fn journal_page<'a>(
        &'a self,
        _query: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async { Err(BackendError::Unsupported("journal".to_owned())) })
    }

    /// `GET /internal/v1/journal/verify`.
    fn journal_verify<'a>(&'a self) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async { Err(BackendError::Unsupported("journal/verify".to_owned())) })
    }

    /// `POST /internal/v1/journal/append`.
    fn journal_append<'a>(
        &'a self,
        _body: &'a [u8],
        _content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async { Err(BackendError::Unsupported("journal/append".to_owned())) })
    }

    /// `GET /internal/v1/providers/cad`.
    fn providers_cad<'a>(&'a self) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async { Err(BackendError::Unsupported("providers/cad".to_owned())) })
    }
}

/// The production backend: the supervised Python geometry worker.
pub struct PythonWorkerBackend {
    worker: Arc<GeometryWorker>,
    client: reqwest::Client,
    design_schema: RwLock<Option<Arc<DesignSchema>>>,
}

impl PythonWorkerBackend {
    /// Build a backend bound to one supervised worker.
    ///
    /// # Errors
    ///
    /// Returns a message when the HTTP client cannot be created.
    pub fn new(worker: Arc<GeometryWorker>) -> Result<Arc<Self>, String> {
        let client = reqwest::Client::builder()
            .timeout(WORKER_REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| format!("the geometry worker client could not be created: {error}"))?;
        Ok(Arc::new(Self {
            worker,
            client,
            design_schema: RwLock::new(None),
        }))
    }

    /// The supervised worker this backend talks to.
    #[must_use]
    pub fn worker(&self) -> &Arc<GeometryWorker> {
        &self.worker
    }

    /// The DesignIntent property names the worker's schema declares.
    ///
    /// The schema is fetched once, lazily, from `GET /intent/schema`. It
    /// decides whether the front door may add the ADR-0011 `provider` field to
    /// a design request: a worker whose schema does not know the field would
    /// refuse the request as an extra input.
    pub async fn design_schema(&self) -> Option<Arc<DesignSchema>> {
        if let Some(schema) = self.design_schema.read().await.clone() {
            return Some(schema);
        }
        let response = self.intent_schema().await.ok()?;
        let text = String::from_utf8(response.body).ok()?;
        let schema = Arc::new(DesignSchema::parse(&text)?);
        *self.design_schema.write().await = Some(Arc::clone(&schema));
        Some(schema)
    }

    /// Send one request to the worker.
    async fn request(
        &self,
        method: &str,
        path: &str,
        query: Option<&str>,
        body: Option<&[u8]>,
        content_type: Option<&str>,
    ) -> Result<WorkerResponse, BackendError> {
        let Some(endpoint) = self.worker.endpoint().await else {
            return Err(BackendError::Unavailable(
                "the geometry worker is not running".to_owned(),
            ));
        };
        let mut url = format!("http://{WORKER_HOST}:{}{path}", endpoint.port);
        if let Some(query) = query {
            url.push('?');
            url.push_str(query);
        }
        let method = reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::GET);
        let mut builder = self.client.request(method, &url).header(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", endpoint.token),
        );
        if let Some(content_type) = content_type {
            builder = builder.header(reqwest::header::CONTENT_TYPE, content_type);
        }
        if let Some(body) = body {
            builder = builder.body(body.to_vec());
        }
        let response = builder.send().await.map_err(|error| {
            BackendError::Transport(format!("the geometry worker is unreachable: {error}"))
        })?;
        let status = response.status().as_u16();
        let mut headers = Vec::new();
        for name in FORWARDED_RESPONSE_HEADERS {
            if let Some(value) = response.headers().get(*name) {
                if let Ok(text) = value.to_str() {
                    headers.push(((*name).to_owned(), text.to_owned()));
                }
            }
        }
        let mut collected = Vec::new();
        let mut stream = response;
        loop {
            let chunk = stream.chunk().await.map_err(|error| {
                BackendError::Transport(format!("the geometry worker response failed: {error}"))
            })?;
            let Some(chunk) = chunk else { break };
            if collected.len() + chunk.len() > MAX_WORKER_RESPONSE_BYTES {
                return Err(BackendError::TooLarge(format!(
                    "the geometry worker answered more than {MAX_WORKER_RESPONSE_BYTES} bytes"
                )));
            }
            collected.extend_from_slice(&chunk);
        }
        Ok(WorkerResponse {
            status,
            headers,
            body: collected,
        })
    }
}

/// The declared properties of the worker's DesignIntent schema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesignSchema {
    properties: Vec<String>,
}

impl DesignSchema {
    /// Parse a JSON Schema document and keep its property names.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let parsed: Value = serde_json::from_str(text).ok()?;
        let properties = parsed.get("properties")?.as_object()?;
        Some(Self {
            properties: properties.keys().cloned().collect(),
        })
    }

    /// Whether the schema declares the named property.
    #[must_use]
    pub fn declares(&self, name: &str) -> bool {
        self.properties.iter().any(|property| property == name)
    }
}

impl GeometryBackend for PythonWorkerBackend {
    fn health(&self) -> BackendFuture<'_, BackendHealth> {
        Box::pin(async move {
            let status = self.worker.status().await;
            // The worker's OCP kernel provides both the design compiler and
            // the STEP writer, so one flag answers both capabilities.
            let available = status.design_available;
            BackendHealth {
                state: status.state.as_str().to_owned(),
                version: status.version,
                design_available: available,
                step_available: available,
            }
        })
    }

    fn preview<'a>(
        &'a self,
        body: &'a [u8],
        content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            self.request(
                "POST",
                &format!("{WORKER_INTERNAL_PREFIX}/preview"),
                None,
                Some(body),
                content_type,
            )
            .await
        })
    }

    fn export<'a>(
        &'a self,
        body: &'a [u8],
        content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            self.request(
                "POST",
                &format!("{WORKER_INTERNAL_PREFIX}/export"),
                None,
                Some(body),
                content_type,
            )
            .await
        })
    }

    fn handoff<'a>(
        &'a self,
        body: &'a [u8],
        content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            self.request(
                "POST",
                &format!("{WORKER_INTERNAL_PREFIX}/handoff"),
                None,
                Some(body),
                content_type,
            )
            .await
        })
    }

    fn design<'a>(
        &'a self,
        body: &'a [u8],
        content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            self.request(
                "POST",
                &format!("{WORKER_INTERNAL_PREFIX}/design"),
                None,
                Some(body),
                content_type,
            )
            .await
        })
    }

    fn drawing<'a>(
        &'a self,
        design_id: &'a str,
        query: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            self.request(
                "GET",
                &format!("{WORKER_INTERNAL_PREFIX}/design/{design_id}/drawing"),
                query,
                None,
                None,
            )
            .await
        })
    }

    fn section<'a>(
        &'a self,
        design_id: &'a str,
        query: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            self.request(
                "GET",
                &format!("{WORKER_INTERNAL_PREFIX}/design/{design_id}/section"),
                query,
                None,
                None,
            )
            .await
        })
    }

    fn import_step<'a>(
        &'a self,
        body: &'a [u8],
        content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            self.request(
                "POST",
                &format!("{WORKER_INTERNAL_PREFIX}/import"),
                None,
                Some(body),
                content_type,
            )
            .await
        })
    }

    fn intent_parse<'a>(
        &'a self,
        body: &'a [u8],
        content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            self.request(
                "POST",
                &format!("{WORKER_INTERNAL_PREFIX}/intent/parse"),
                None,
                Some(body),
                content_type,
            )
            .await
        })
    }

    fn intent_validate<'a>(
        &'a self,
        body: &'a [u8],
        content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            self.request(
                "POST",
                &format!("{WORKER_INTERNAL_PREFIX}/intent/validate"),
                None,
                Some(body),
                content_type,
            )
            .await
        })
    }

    fn intent_schema<'a>(&'a self) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            self.request(
                "GET",
                &format!("{WORKER_INTERNAL_PREFIX}/intent/schema"),
                None,
                None,
                None,
            )
            .await
        })
    }

    fn journal_page<'a>(
        &'a self,
        query: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            self.request(
                "GET",
                &format!("{WORKER_INTERNAL_PREFIX}/journal"),
                query,
                None,
                None,
            )
            .await
        })
    }

    fn journal_verify<'a>(&'a self) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            self.request(
                "GET",
                &format!("{WORKER_INTERNAL_PREFIX}/journal/verify"),
                None,
                None,
                None,
            )
            .await
        })
    }

    fn journal_append<'a>(
        &'a self,
        body: &'a [u8],
        content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            self.request(
                "POST",
                &format!("{WORKER_INTERNAL_PREFIX}/journal/append"),
                None,
                Some(body),
                content_type,
            )
            .await
        })
    }

    fn providers_cad<'a>(&'a self) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            self.request(
                "GET",
                &format!("{WORKER_INTERNAL_PREFIX}/providers/cad"),
                None,
                None,
                None,
            )
            .await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_design_schema_exposes_its_properties() {
        let schema = DesignSchema::parse(
            r#"{"type":"object","properties":{"template":{},"script":{},"provider":{}}}"#,
        )
        .unwrap();
        assert!(schema.declares("provider"));
        assert!(!schema.declares("material"));
    }

    #[test]
    fn an_unparsable_schema_declares_nothing() {
        assert!(DesignSchema::parse("not json").is_none());
    }
}
