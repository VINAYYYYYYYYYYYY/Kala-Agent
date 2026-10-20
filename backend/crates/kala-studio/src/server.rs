//! The Studio HTTP service: middleware order, routes, DTOs and exact bytes.
//!
//! Request processing order is the security policy and matches the Python
//! stack: security headers wrap everything, then Host, body size, Origin,
//! bearer token, static files and finally the router. Static files use the
//! raw path; API routing uses the percent-decoded path.
//!
//! # Which routes are native and which are proxied (ADR-0010)
//!
//! * **Rust-native:** `/health`, `/integrations`, `/providers`, `/settings`
//!   and `/chat`.
//! * **Proxied to the supervised Python geometry worker:** `/preview`,
//!   `/export`, `/handoff`, `/design*`, `/journal*`, `/providers/cad` and
//!   `/import`. The front door enforces the body, response and time limits
//!   and forwards the payload unchanged; the worker owns the geometry, the
//!   design compiler, the journal, the intent parser and the CAD provider
//!   probe. A worker that is down answers 503 on those routes and leaves the
//!   native routes working.
//!
//! # The STEP import ceiling (ADR-0011)
//!
//! The view-only import route has its own body ceiling, the resolved
//! `KALA_STEP_IMPORT_MAX_BYTES` value ([`crate::step_limit`]). The front
//! door and the Python worker it launches always use the same value, so the
//! two sides accept exactly the same body.
//!
//! The native primitive geometry crates are parked behind the non-default
//! `native-primitives` feature (see [`crate::native_backend`]).

use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, HOST, ORIGIN};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tokio::net::TcpListener;

use crate::boundary::{
    HOST_REFUSED_MESSAGE, OriginParse, STUDIO_BEARER_REALM, STUDIO_TOKEN_ENV_VAR,
    TOKEN_REQUIRED_MESSAGE, header_value_text, is_permitted_authority, origins_match,
    parse_host_authority, parse_origin_header, resolve_studio_token,
};
use crate::chat_design;
use crate::errors::{
    FlatError, INTERNAL_ERROR_MESSAGE, METHOD_NOT_ALLOWED_MESSAGE, PAYLOAD_TOO_LARGE_MESSAGE,
    REQUEST_REJECTED_MESSAGE, UNKNOWN_ROUTE_MESSAGE, VALIDATION_MESSAGE,
};
use crate::geometry_backend::{BackendError, BackendHealth, GeometryBackend, WorkerResponse};
use crate::integrations::build_registry;
use crate::json::Json;
use crate::providers::remote::{HttpTransport, ReqwestTransport};
use crate::providers::{
    ChatOutcome, ProviderFailure, any_remote_provider_configured, dispatch, list_provider_records,
};
use crate::pytext;
use crate::schemas::{
    ChatRequest, MAX_MODEL_ID_LENGTH, RequestParseError, UpdateProviderKeyRequest, parse_json_body,
    validate_chat, validate_settings,
};
use crate::settings::{
    CredentialStatus, StoredKeys, WriteKeyError, describe_provider_credentials, effective_environ,
    fallback_settings_dir, read_stored_keys_checked, write_stored_key,
};
use crate::static_files::{StaticLookup, StaticRoot, is_reserved_api_path, lookup, media_type_for};

pub const STUDIO_PREFIX: &str = "/api/studio";
pub const STUDIO_VERSION: &str = "0.1.0";
pub const SERVICE_NAME: &str = "kala-studio";
pub const MIN_REQUEST_BODY_BYTES: usize = 1_024;
pub const MAX_REQUEST_BODY_BYTES: usize = 262_144;

/// The `www-authenticate` challenge of the route-level bearer gate.
///
/// The mutating-method middleware answers with the Studio realm; the
/// read-only design, journal and CAD provider routes answer with the plain
/// challenge the source uses.
pub const ROUTE_BEARER_CHALLENGE: &str = "Bearer";

/// The error code of a refused geometry route.
pub const GEOMETRY_UNAVAILABLE_CODE: &str = "geometry_worker_unavailable";
/// The error message of a refused geometry route.
pub const GEOMETRY_UNAVAILABLE_MESSAGE: &str =
    "the geometry worker is not available; the studio cannot compute geometry without it";

/// Exact security header set from the Python middleware.
pub const SECURITY_HEADERS: &[(&str, &str)] = &[
    ("cache-control", "no-store"),
    (
        "content-security-policy",
        "default-src 'self'; script-src 'self'; connect-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
    ),
    ("cross-origin-opener-policy", "same-origin"),
    ("cross-origin-resource-policy", "same-origin"),
    ("referrer-policy", "no-referrer"),
    ("x-content-type-options", "nosniff"),
    ("x-frame-options", "DENY"),
];

/// Configuration for a standalone Studio service.
pub struct StudioConfig {
    pub data_dir: Option<PathBuf>,
    pub api_token: Option<String>,
    pub max_body_bytes: usize,
    pub static_root: Option<PathBuf>,
    /// `Some` uses this environment view exclusively; `None` reads the
    /// process environment at request time.
    pub environ: Option<HashMap<String, String>>,
    /// The STEP import ceiling of the public import route.
    ///
    /// `None` resolves `KALA_STEP_IMPORT_MAX_BYTES` from the environment
    /// view, exactly like the Python worker, and refuses startup when the
    /// value is unusable. `Some` is the embedding seam: the value is
    /// validated with the same bounds. The front door and the worker process
    /// it launches must always use the same ceiling.
    pub import_max_bytes: Option<usize>,
    /// The parked native STEP seam. It is only used by the parked native
    /// backend; the default build answers the routes from the worker.
    pub step_exporter: Arc<dyn crate::geometry::StepExporter>,
    pub transport: Option<Arc<dyn HttpTransport>>,
    /// The geometry backend. `None` means no worker is attached and the
    /// proxied routes answer 503.
    pub geometry_backend: Option<Arc<dyn GeometryBackend>>,
}

impl Default for StudioConfig {
    fn default() -> Self {
        Self {
            data_dir: None,
            api_token: None,
            max_body_bytes: MAX_REQUEST_BODY_BYTES,
            static_root: None,
            environ: None,
            import_max_bytes: None,
            step_exporter: crate::geometry::default_step_exporter(),
            transport: None,
            geometry_backend: None,
        }
    }
}

impl StudioConfig {
    /// Validate `max_body_bytes` exactly like the Python boundary.
    ///
    /// # Errors
    ///
    /// Returns the Python `ValueError` text for a boolean or out-of-range
    /// value.
    pub fn validate_body_limit(value: usize) -> Result<usize, String> {
        if !(MIN_REQUEST_BODY_BYTES..=MAX_REQUEST_BODY_BYTES).contains(&value) {
            return Err(format!(
                "max_body_bytes must be between {MIN_REQUEST_BODY_BYTES} and {MAX_REQUEST_BODY_BYTES}"
            ));
        }
        Ok(value)
    }
}

/// A startup failure before a socket serves traffic.
#[derive(Debug)]
pub struct StartupError(String);

impl std::fmt::Display for StartupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for StartupError {}

/// The public routes the front door serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RouteKind {
    Health,
    Integrations,
    Providers,
    ProvidersCad,
    Settings,
    Preview,
    Export,
    Handoff,
    Chat,
    Design,
    Import,
    Journal,
    JournalVerify,
    Drawing,
    Section,
}

impl RouteKind {
    /// Whether the route carries its own bearer gate, like the source's
    /// read-only design, journal and CAD provider routes.
    fn requires_route_bearer(self) -> bool {
        matches!(
            self,
            Self::Journal
                | Self::JournalVerify
                | Self::Drawing
                | Self::Section
                | Self::ProvidersCad
        )
    }
}

/// The route a request path names.
fn classify(path: &str) -> Option<(RouteKind, Option<String>)> {
    let kind = match path {
        "/api/studio/health" => RouteKind::Health,
        "/api/studio/integrations" => RouteKind::Integrations,
        "/api/studio/providers" => RouteKind::Providers,
        "/api/studio/providers/cad" => RouteKind::ProvidersCad,
        "/api/studio/settings" => RouteKind::Settings,
        "/api/studio/preview" => RouteKind::Preview,
        "/api/studio/export" => RouteKind::Export,
        "/api/studio/handoff" => RouteKind::Handoff,
        "/api/studio/chat" => RouteKind::Chat,
        "/api/studio/design" => RouteKind::Design,
        "/api/studio/import" => RouteKind::Import,
        "/api/studio/journal" => RouteKind::Journal,
        "/api/studio/journal/verify" => RouteKind::JournalVerify,
        other => {
            if let Some(rest) = other.strip_prefix("/api/studio/design/") {
                if let Some(design_id) = rest.strip_suffix("/drawing") {
                    if !design_id.is_empty() && !design_id.contains('/') {
                        return Some((RouteKind::Drawing, Some(design_id.to_owned())));
                    }
                }
                if let Some(design_id) = rest.strip_suffix("/section") {
                    if !design_id.is_empty() && !design_id.contains('/') {
                        return Some((RouteKind::Section, Some(design_id.to_owned())));
                    }
                }
            }
            return None;
        }
    };
    Some((kind, None))
}

/// The methods a route accepts.
fn allowed_methods(kind: RouteKind) -> &'static [Method] {
    match kind {
        RouteKind::Health
        | RouteKind::Integrations
        | RouteKind::Providers
        | RouteKind::ProvidersCad
        | RouteKind::Journal
        | RouteKind::JournalVerify
        | RouteKind::Drawing
        | RouteKind::Section => &[Method::GET],
        RouteKind::Settings => &[Method::GET, Method::POST],
        RouteKind::Preview
        | RouteKind::Export
        | RouteKind::Handoff
        | RouteKind::Chat
        | RouteKind::Design
        | RouteKind::Import => &[Method::POST],
    }
}

/// The body limit of one route.
///
/// The view-only STEP import has its own ceiling: the resolved
/// `KALA_STEP_IMPORT_MAX_BYTES` value, which is the same value the worker
/// enforces. Every other route keeps the shared Studio limit.
fn body_limit(kind: Option<RouteKind>, shared: usize, import_limit: usize) -> usize {
    match kind {
        Some(RouteKind::Import) => import_limit,
        _ => shared,
    }
}

/// Whether a design identifier is the sha256 digest the source requires.
fn valid_design_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

struct State {
    data_dir: PathBuf,
    token: String,
    max_body_bytes: usize,
    import_max_bytes: usize,
    static_root: Option<StaticRoot>,
    custom_environ: Option<BTreeMap<String, String>>,
    step_exporter: Arc<dyn crate::geometry::StepExporter>,
    transport: Arc<dyn HttpTransport>,
    backend: Option<Arc<dyn GeometryBackend>>,
}

impl State {
    fn base_environ(&self) -> BTreeMap<String, String> {
        match &self.custom_environ {
            Some(environ) => environ.clone(),
            None => crate::boundary::process_environ(),
        }
    }

    /// The environment view and the stored keys read together, like the
    /// source's `effective_environ` + credential status per request.
    fn environ_and_stored(
        &self,
    ) -> Result<(BTreeMap<String, String>, StoredKeys), crate::settings::SettingsReadError> {
        let stored = read_stored_keys_checked(Some(&self.data_dir))?;
        Ok((effective_environ(&self.base_environ(), &stored), stored))
    }
}

/// A bound, ready-to-serve Studio service.
pub struct StudioService {
    state: Arc<State>,
}

impl StudioService {
    /// Build the service, resolve the launch token and create the data dir.
    ///
    /// # Errors
    ///
    /// Returns a sanitized startup error for an invalid body limit, token,
    /// data directory, static root configuration or HTTPS client.
    pub fn new(config: StudioConfig) -> Result<Self, StartupError> {
        let limit =
            StudioConfig::validate_body_limit(config.max_body_bytes).map_err(StartupError)?;
        let state_env: BTreeMap<String, String> = match &config.environ {
            Some(environ) => environ.clone().into_iter().collect(),
            None => crate::boundary::process_environ(),
        };
        // The STEP import ceiling is one setting, shared with the Python
        // worker: the same variable, the same parsing, the same default and
        // the same refusal. The front door enforces it on the public import
        // route and hands the resolved value to the worker it launches, so
        // the two sides always accept exactly the same body.
        let import_max_bytes = match config.import_max_bytes {
            Some(value) => crate::step_limit::validate_step_import_max_bytes(
                i64::try_from(value).unwrap_or(i64::MAX),
            )
            .map_err(StartupError)?,
            None => crate::step_limit::resolve_step_import_max_bytes(&state_env)
                .map_err(StartupError)?,
        };
        let data_dir = match config.data_dir {
            Some(path) => path,
            None => crate::boundary::default_studio_data_dir(&state_env),
        };
        std::fs::create_dir_all(&data_dir).map_err(|_| {
            StartupError("the studio data directory could not be created".to_owned())
        })?;
        let token = match config.api_token {
            Some(token) => {
                // An explicitly supplied token is validated verbatim, like
                // `BearerTokenMiddleware`; only the environment path strips.
                crate::boundary::validate_studio_token(&token).map_err(StartupError)?;
                token
            }
            None => resolve_studio_token(
                &data_dir,
                state_env.get(STUDIO_TOKEN_ENV_VAR).map(String::as_str),
            )
            .map_err(StartupError)?,
        };
        let static_root = config.static_root.and_then(StaticRoot::new);
        let transport = match config.transport {
            Some(transport) => transport,
            None => Arc::new(ReqwestTransport::new().map_err(StartupError)?),
        };
        Ok(Self {
            state: Arc::new(State {
                data_dir,
                token,
                max_body_bytes: limit,
                import_max_bytes,
                static_root,
                custom_environ: config.environ.map(|environ| environ.into_iter().collect()),
                step_exporter: config.step_exporter,
                transport,
                backend: config.geometry_backend,
            }),
        })
    }

    /// The STEP import ceiling this service enforces.
    ///
    /// The value is the one the Python worker was launched with, so the
    /// front door and the worker accept exactly the same body.
    #[must_use]
    pub fn import_max_bytes(&self) -> usize {
        self.state.import_max_bytes
    }

    /// The current STEP capability of the configured exporter.
    #[must_use]
    pub fn step_capability(&self) -> crate::geometry::StepCapability {
        self.state.step_exporter.capability()
    }

    /// The geometry backend the service proxies to.
    #[must_use]
    pub fn geometry_backend(&self) -> Option<Arc<dyn GeometryBackend>> {
        self.state.backend.clone()
    }

    /// Serve HTTP/1.1 until `shutdown` resolves.
    ///
    /// # Errors
    ///
    /// Returns an I/O error only when the accept loop fails fatally.
    pub async fn serve<F>(self, listener: TcpListener, shutdown: F) -> io::Result<()>
    where
        F: Future<Output = ()> + Send,
    {
        let graceful = hyper_util::server::graceful::GracefulShutdown::new();
        let mut shutdown = std::pin::pin!(shutdown);
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (stream, _peer) = match accepted {
                        Ok(value) => value,
                        Err(error) => {
                            if is_transient_accept_error(&error) {
                                continue;
                            }
                            return Err(error);
                        }
                    };
                    let state = Arc::clone(&self.state);
                    let connection = hyper::server::conn::http1::Builder::new()
                        .serve_connection(
                            TokioIo::new(stream),
                            service_fn(move |request| {
                                let state = Arc::clone(&state);
                                async move {
                                    let response = handle_request(state, request).await;
                                    Ok::<_, Infallible>(response)
                                }
                            }),
                        );
                    let watched = graceful.watch(connection);
                    tokio::spawn(async move {
                        let _ = watched.await;
                    });
                }
                () = &mut shutdown => break,
            }
        }
        let _ = tokio::time::timeout(Duration::from_secs(5), graceful.shutdown()).await;
        Ok(())
    }
}

fn is_transient_accept_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::Interrupted
            | io::ErrorKind::WouldBlock
            | io::ErrorKind::TimedOut
    )
}

struct OutResponse {
    status: StatusCode,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    challenge: Option<String>,
}

impl OutResponse {
    fn json(status: StatusCode, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: vec![
                ("content-type".to_owned(), "application/json".to_owned()),
                ("content-length".to_owned(), body.len().to_string()),
            ],
            body,
            challenge: None,
        }
    }

    fn binary(status: StatusCode, content_type: &str, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: vec![
                ("content-type".to_owned(), content_type.to_owned()),
                ("content-length".to_owned(), body.len().to_string()),
            ],
            body,
            challenge: None,
        }
    }

    fn redirect(location: String) -> Self {
        Self {
            status: StatusCode::TEMPORARY_REDIRECT,
            headers: vec![
                ("location".to_owned(), location),
                ("content-length".to_owned(), "0".to_owned()),
            ],
            body: Vec::new(),
            challenge: None,
        }
    }

    fn with_challenge(mut self, challenge: &str) -> Self {
        self.challenge = Some(challenge.to_owned());
        self
    }
}

fn ordered(error: FlatError) -> OutResponse {
    match error.ordered_body().to_bytes() {
        Ok(body) => OutResponse::json(
            StatusCode::from_u16(error.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            body,
        ),
        Err(_) => internal_error(),
    }
}

fn sorted(error: FlatError) -> OutResponse {
    match error.sorted_body().to_sorted_ascii_bytes() {
        Ok(body) => OutResponse::json(
            StatusCode::from_u16(error.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            body,
        ),
        Err(_) => internal_error(),
    }
}

fn internal_error() -> OutResponse {
    let body = FlatError::new(500, "internal_error", INTERNAL_ERROR_MESSAGE, Vec::new())
        .ordered_body()
        .to_bytes()
        .unwrap_or_else(|_| b"{\"code\":\"internal_error\",\"message\":\"the studio request could not be completed\",\"details\":[]}".to_vec());
    OutResponse::json(StatusCode::INTERNAL_SERVER_ERROR, body)
}

async fn handle_request(state: Arc<State>, request: Request<Incoming>) -> Response<Full<Bytes>> {
    let method = request.method().clone();
    let raw_path = request.uri().path().to_owned();
    let raw_query = request.uri().query().map(str::to_owned);
    let decoded_path = percent_decode(&raw_path);
    let classified = classify(&decoded_path);
    let route_kind = classified.as_ref().map(|(kind, _)| *kind);

    // 1. Host guard.
    let hosts = header_values(&request, HOST);
    if hosts.len() != 1 || !is_permitted_authority(&hosts[0]) {
        return finish(
            sorted(FlatError::new(
                421,
                "invalid_host",
                HOST_REFUSED_MESSAGE,
                Vec::new(),
            )),
            method == Method::HEAD,
        );
    }

    // 2. Body size: declared first, then streamed, before Origin/token. The
    //    limit is the route's own: the STEP import has a dedicated one.
    let limit = body_limit(route_kind, state.max_body_bytes, state.import_max_bytes);
    let declared = declared_length(&header_values(&request, CONTENT_LENGTH));
    if declared.is_some_and(|value| value > limit as u128) {
        return finish(too_large(limit), method == Method::HEAD);
    }
    let headers = SavedHeaders::from_headers(request.headers());
    let body = match read_bounded_body(request.into_body(), limit).await {
        Ok(body) => body,
        Err(BodyReadError::TooLarge) => {
            return finish(too_large(limit), method == Method::HEAD);
        }
        Err(BodyReadError::Read) => {
            return finish(
                ordered(FlatError::new(
                    400,
                    "request_rejected",
                    REQUEST_REJECTED_MESSAGE,
                    vec!["There was an error parsing the body".to_owned()],
                )),
                method == Method::HEAD,
            );
        }
    };

    // 3. Same-origin guard.
    let origins = headers.values(ORIGIN);
    if !origins.is_empty() {
        if origins.len() != 1 {
            return finish(cross_origin(), method == Method::HEAD);
        }
        let origin = match parse_origin_header(&origins[0]) {
            OriginParse::Valid(origin) => origin,
            OriginParse::Rejected => {
                return finish(cross_origin(), method == Method::HEAD);
            }
            OriginParse::Malformed => {
                eprintln!("unhandled studio failure: ValueError");
                return finish(internal_error(), method == Method::HEAD);
            }
        };
        let host_values = headers.values(HOST);
        if host_values.len() != 1 {
            return finish(cross_origin(), method == Method::HEAD);
        }
        let Some(authority) = parse_host_authority(&host_values[0]) else {
            return finish(cross_origin(), method == Method::HEAD);
        };
        if !origins_match(&origin, &authority) {
            return finish(cross_origin(), method == Method::HEAD);
        }
    }

    // 4. Bearer token on every mutating method.
    if is_mutating(&method) && !bearer_matches(&headers.values(AUTHORIZATION), &state.token) {
        return finish(
            sorted(FlatError::new(
                401,
                "unauthorized",
                TOKEN_REQUIRED_MESSAGE,
                Vec::new(),
            ))
            .with_challenge(STUDIO_BEARER_REALM),
            method == Method::HEAD,
        );
    }

    // 5. Static files before the router, only for GET/HEAD and never /api.
    if let Some(static_root) = &state.static_root {
        if (method == Method::GET || method == Method::HEAD) && !is_reserved_api_path(&decoded_path)
        {
            return finish(
                serve_static(static_root, &raw_path).await,
                method == Method::HEAD,
            );
        }
    }

    // 6. Router.
    let Some((kind, design_id)) = classified else {
        if let Some(alternate) = redirect_alternate(&decoded_path) {
            if let Some((alternate_kind, _)) = classify(&alternate) {
                if allowed_methods(alternate_kind).contains(&method) {
                    let host = headers.values(HOST).into_iter().next().unwrap_or_default();
                    let mut location = format!("http://{host}{alternate}");
                    if let Some(query) = &raw_query {
                        location.push('?');
                        location.push_str(query);
                    }
                    return finish(OutResponse::redirect(location), method == Method::HEAD);
                }
            }
        }
        return finish(
            ordered(FlatError::new(
                404,
                "not_found",
                UNKNOWN_ROUTE_MESSAGE,
                Vec::new(),
            )),
            method == Method::HEAD,
        );
    };
    if !allowed_methods(kind).contains(&method) {
        return finish(
            ordered(FlatError::new(
                405,
                "method_not_allowed",
                METHOD_NOT_ALLOWED_MESSAGE,
                Vec::new(),
            )),
            method == Method::HEAD,
        );
    }

    // 7. The read-only design, journal and CAD provider routes carry their
    //    own bearer gate, like the source's `_require_bearer`.
    if kind.requires_route_bearer() && !bearer_matches(&headers.values(AUTHORIZATION), &state.token)
    {
        return finish(
            sorted(FlatError::new(
                401,
                "unauthorized",
                TOKEN_REQUIRED_MESSAGE,
                Vec::new(),
            ))
            .with_challenge(ROUTE_BEARER_CHALLENGE),
            method == Method::HEAD,
        );
    }

    let content_type = headers.values(CONTENT_TYPE).into_iter().next();
    let response = route(
        Arc::clone(&state),
        kind,
        design_id.as_deref(),
        &method,
        &body,
        content_type.as_deref(),
        raw_query.as_deref(),
    )
    .await;
    finish(response, method == Method::HEAD)
}

fn too_large(limit: usize) -> OutResponse {
    sorted(FlatError::new(
        413,
        "payload_too_large",
        PAYLOAD_TOO_LARGE_MESSAGE,
        vec![format!("the request body must not exceed {limit} bytes")],
    ))
}

fn cross_origin() -> OutResponse {
    sorted(FlatError::new(
        403,
        "cross_origin_denied",
        "the studio serves no cross-origin client",
        Vec::new(),
    ))
}

fn finish(response: OutResponse, head_request: bool) -> Response<Full<Bytes>> {
    let mut builder = Response::builder().status(response.status);
    let mut headers = response.headers;
    if let Some(challenge) = &response.challenge {
        headers.push(("www-authenticate".to_owned(), challenge.clone()));
    } else if response.status == StatusCode::UNAUTHORIZED {
        headers.push((
            "www-authenticate".to_owned(),
            STUDIO_BEARER_REALM.to_owned(),
        ));
    }
    let present: Vec<String> = headers
        .iter()
        .map(|(name, _)| name.to_ascii_lowercase())
        .collect();
    for (name, value) in SECURITY_HEADERS {
        if !present.iter().any(|existing| existing == name) {
            headers.push(((*name).to_owned(), (*value).to_owned()));
        }
    }
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    let body = if head_request {
        Full::new(Bytes::new())
    } else {
        Full::new(Bytes::from(response.body))
    };
    builder.body(body).unwrap_or_else(|_| {
        Response::new(Full::new(Bytes::from_static(
            b"{\"code\":\"internal_error\",\"message\":\"the studio request could not be completed\",\"details\":[]}",
        )))
    })
}

struct SavedHeaders {
    values: Vec<(String, String)>,
}

impl SavedHeaders {
    fn from_headers(headers: &hyper::HeaderMap) -> Self {
        let mut values = Vec::new();
        for (name, value) in headers {
            values.push((
                name.as_str().to_ascii_lowercase(),
                header_value_text(value.as_bytes()),
            ));
        }
        Self { values }
    }

    fn values(&self, name: hyper::header::HeaderName) -> Vec<String> {
        let wanted = name.as_str();
        self.values
            .iter()
            .filter(|(key, _)| key == wanted)
            .map(|(_, value)| value.clone())
            .collect()
    }
}

fn header_values(request: &Request<Incoming>, name: hyper::header::HeaderName) -> Vec<String> {
    request
        .headers()
        .get_all(name)
        .iter()
        .map(|value| header_value_text(value.as_bytes()))
        .collect()
}

fn declared_length(values: &[String]) -> Option<u128> {
    let raw = values.first()?;
    let candidate = pytext::trim(raw);
    if candidate.is_empty() || !candidate.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    // Python parses an arbitrarily long integer; anything beyond u128 is
    // necessarily larger than the configured cap.
    candidate.parse::<u128>().ok().or(Some(u128::MAX))
}

enum BodyReadError {
    TooLarge,
    Read,
}

async fn read_bounded_body(mut body: Incoming, limit: usize) -> Result<Vec<u8>, BodyReadError> {
    let mut collected = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| BodyReadError::Read)?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        if collected.len() + data.len() > limit {
            return Err(BodyReadError::TooLarge);
        }
        collected.extend_from_slice(&data);
    }
    Ok(collected)
}

fn is_mutating(method: &Method) -> bool {
    matches!(
        *method,
        Method::DELETE | Method::PATCH | Method::POST | Method::PUT
    )
}

fn bearer_matches(values: &[String], expected: &str) -> bool {
    if values.len() != 1 {
        return false;
    }
    let Some((scheme, credential)) = values[0].split_once(' ') else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("bearer") {
        return false;
    }
    let token = pytext::trim(credential);
    if token.is_empty() || token.chars().count() > 512 {
        return false;
    }
    constant_time_eq(token.as_bytes(), expected.as_bytes())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (first, second) in left.iter().zip(right) {
        difference |= first ^ second;
    }
    difference == 0
}

/// Starlette's `redirect_slashes`: a known route with the opposite trailing
/// slash redirects with 307 before method dispatch.
fn redirect_alternate(path: &str) -> Option<String> {
    if path.len() > 1 && path.ends_with('/') {
        Some(path.trim_end_matches('/').to_owned())
    } else if !path.ends_with('/') {
        Some(format!("{path}/"))
    } else {
        None
    }
}

fn percent_decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                decoded.push((high << 4) | low);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

async fn serve_static(root: &StaticRoot, raw_path: &str) -> OutResponse {
    let root = root.clone();
    let raw_path = raw_path.to_owned();
    let lookup_result = tokio::task::spawn_blocking(move || lookup(&root, &raw_path))
        .await
        .unwrap_or(StaticLookup::Missing);
    match lookup_result {
        StaticLookup::File(path) => {
            let read = tokio::task::spawn_blocking(move || {
                let data = std::fs::read(&path)?;
                let media = media_type_for(&path);
                Ok::<_, io::Error>((data, media))
            })
            .await;
            match read {
                Ok(Ok((data, media))) => OutResponse::binary(StatusCode::OK, media, data),
                _ => static_not_found(),
            }
        }
        StaticLookup::Blocked | StaticLookup::Missing => static_not_found(),
    }
}

fn static_not_found() -> OutResponse {
    // StaticFileMiddleware builds this body with json.dumps(sort_keys=True).
    sorted(FlatError::new(
        404,
        "not_found",
        UNKNOWN_ROUTE_MESSAGE,
        Vec::new(),
    ))
}

async fn route(
    state: Arc<State>,
    kind: RouteKind,
    design_id: Option<&str>,
    method: &Method,
    body: &[u8],
    content_type: Option<&str>,
    query: Option<&str>,
) -> OutResponse {
    match kind {
        RouteKind::Health => health(&state).await,
        RouteKind::Integrations => integrations_response(),
        RouteKind::Providers => providers_response(&state),
        RouteKind::Settings => {
            if *method == Method::GET {
                settings_get(&state)
            } else {
                with_request(
                    body,
                    content_type,
                    validate_settings,
                    |request| async move { settings_post(state.clone(), request) },
                )
                .await
            }
        }
        RouteKind::Chat => {
            with_request(body, content_type, validate_chat, |request| async move {
                chat_route(state.clone(), request).await
            })
            .await
        }
        RouteKind::Preview
        | RouteKind::Export
        | RouteKind::Handoff
        | RouteKind::Design
        | RouteKind::Import
        | RouteKind::Journal
        | RouteKind::JournalVerify
        | RouteKind::Drawing
        | RouteKind::Section
        | RouteKind::ProvidersCad => {
            proxy_route(&state, kind, design_id, method, body, content_type, query).await
        }
    }
}

fn not_found() -> OutResponse {
    ordered(FlatError::new(
        404,
        "not_found",
        UNKNOWN_ROUTE_MESSAGE,
        Vec::new(),
    ))
}

async fn with_request<T, F>(
    body: &[u8],
    content_type: Option<&str>,
    validate: impl FnOnce(&serde_json::Map<String, Value>) -> Result<T, FlatError>,
    handler: impl FnOnce(T) -> F,
) -> OutResponse
where
    F: Future<Output = OutResponse>,
{
    let map = match parse_json_body(body, content_type) {
        Ok(map) => map,
        Err(RequestParseError::Missing) => {
            return ordered(FlatError::new(
                422,
                "invalid_request",
                VALIDATION_MESSAGE,
                vec!["body: missing: Field required".to_owned()],
            ));
        }
        Err(RequestParseError::ModelAttributes) => {
            return ordered(FlatError::new(
                422,
                "invalid_request",
                VALIDATION_MESSAGE,
                vec![
                    "body: model_attributes_type: Input should be a valid dictionary or object to extract fields from"
                        .to_owned(),
                ],
            ));
        }
        Err(RequestParseError::InvalidJson(offset)) => {
            return ordered(FlatError::new(
                422,
                "invalid_request",
                VALIDATION_MESSAGE,
                vec![format!("body.{offset}: json_invalid: JSON decode error")],
            ));
        }
        Err(RequestParseError::Rejected) => {
            return ordered(FlatError::new(
                400,
                "request_rejected",
                REQUEST_REJECTED_MESSAGE,
                vec!["There was an error parsing the body".to_owned()],
            ));
        }
    };
    match validate(&map) {
        Ok(request) => handler(request).await,
        Err(error) => ordered(error),
    }
}

async fn health(state: &State) -> OutResponse {
    let (environ, stored) = match state.environ_and_stored() {
        Ok(value) => value,
        Err(error) => {
            eprintln!(
                "unhandled studio failure: {}",
                error.python_exception_name()
            );
            return internal_error();
        }
    };
    let backend_health = match &state.backend {
        Some(backend) => backend.health().await,
        None => BackendHealth::unavailable(),
    };
    let integrations: Vec<Json> = build_registry()
        .iter()
        .map(|row| Json::String(row.id.to_owned()))
        .collect();
    let providers: Vec<Json> = list_provider_records(&environ, &stored)
        .into_iter()
        .map(|row| Json::String(row.id.to_owned()))
        .collect();
    let body = Json::object(vec![
        ("status", Json::String("ok".to_owned())),
        ("service", Json::String(SERVICE_NAME.to_owned())),
        ("version", Json::String(STUDIO_VERSION.to_owned())),
        (
            "offline",
            Json::Bool(!any_remote_provider_configured(&environ, &stored)),
        ),
        ("step_available", Json::Bool(backend_health.step_available)),
        (
            "design_available",
            Json::Bool(backend_health.design_available),
        ),
        ("integrations", Json::Array(integrations)),
        ("providers", Json::Array(providers)),
        (
            "geometry_worker",
            Json::object(vec![
                ("state", Json::String(backend_health.state.clone())),
                (
                    "version",
                    match &backend_health.version {
                        Some(version) => Json::String(version.clone()),
                        None => Json::Null,
                    },
                ),
                (
                    "design_available",
                    Json::Bool(backend_health.design_available),
                ),
            ]),
        ),
    ]);
    json_response(StatusCode::OK, &body)
}

fn integrations_response() -> OutResponse {
    let integrations: Vec<Json> = build_registry().iter().map(integration_json).collect();
    json_response(
        StatusCode::OK,
        &Json::object(vec![("integrations", Json::Array(integrations))]),
    )
}

fn integration_json(row: &crate::integrations::IntegrationRecord) -> Json {
    Json::object(vec![
        ("id", Json::String(row.id.to_owned())),
        ("name", Json::String(row.name.to_owned())),
        ("category", Json::String(row.category.to_owned())),
        (
            "platforms",
            Json::Array(
                row.platforms
                    .iter()
                    .map(|value| Json::String((*value).to_owned()))
                    .collect(),
            ),
        ),
        ("status", Json::String(row.status.to_owned())),
        (
            "capabilities",
            Json::Array(
                row.capabilities
                    .iter()
                    .map(|value| Json::String((*value).to_owned()))
                    .collect(),
            ),
        ),
        ("notes", Json::String(row.notes.to_owned())),
    ])
}

fn providers_response(state: &State) -> OutResponse {
    let (environ, stored) = match state.environ_and_stored() {
        Ok(value) => value,
        Err(error) => {
            eprintln!(
                "unhandled studio failure: {}",
                error.python_exception_name()
            );
            return internal_error();
        }
    };
    let records = list_provider_records(&environ, &stored);
    json_response(
        StatusCode::OK,
        &Json::object(vec![(
            "providers",
            Json::Array(records.iter().map(provider_json).collect()),
        )]),
    )
}

fn provider_json(record: &crate::providers::ProviderRecord) -> Json {
    Json::object(vec![
        ("id", Json::String(record.id.to_owned())),
        ("name", Json::String(record.name.to_owned())),
        ("configured", Json::Bool(record.configured)),
        ("notes", Json::String(record.notes.clone())),
    ])
}

fn settings_get(state: &State) -> OutResponse {
    settings_body(state, &state.base_environ())
}

fn settings_post(state: Arc<State>, request: UpdateProviderKeyRequest) -> OutResponse {
    let data_dir = if state.data_dir.as_os_str().is_empty() {
        let fallback = fallback_settings_dir();
        if let Err(_error) = std::fs::create_dir_all(&fallback) {
            return internal_error();
        }
        fallback
    } else {
        state.data_dir.clone()
    };
    match write_stored_key(
        &data_dir,
        request.provider.as_str(),
        request.api_key.as_deref(),
    ) {
        Ok(_) => {}
        Err(WriteKeyError::Validation(message)) => {
            return sorted(FlatError::new(
                422,
                "invalid_request",
                VALIDATION_MESSAGE,
                vec![message],
            ));
        }
        Err(WriteKeyError::Io(_)) => return internal_error(),
    }
    settings_body(&state, &state.base_environ())
}

fn settings_body(state: &State, base: &BTreeMap<String, String>) -> OutResponse {
    let stored = match read_stored_keys_checked(Some(&state.data_dir)) {
        Ok(stored) => stored,
        Err(error) => {
            eprintln!(
                "unhandled studio failure: {}",
                error.python_exception_name()
            );
            return internal_error();
        }
    };
    let credentials = describe_provider_credentials(base, &stored);
    let environ = effective_environ(base, &stored);
    let providers = list_provider_records(&environ, &stored);
    let body = Json::object(vec![
        (
            "credentials",
            Json::Array(credentials.iter().map(credential_json).collect()),
        ),
        (
            "providers",
            Json::Array(providers.iter().map(provider_json).collect()),
        ),
        (
            "offline",
            Json::Bool(!any_remote_provider_configured(&environ, &stored)),
        ),
    ]);
    json_response(StatusCode::OK, &body)
}

fn credential_json(row: &CredentialStatus) -> Json {
    Json::object(vec![
        ("provider", Json::String(row.provider.to_owned())),
        ("env_var", Json::String(row.env_var.to_owned())),
        ("configured", Json::Bool(row.configured)),
        ("source", Json::String(row.source.as_str().to_owned())),
    ])
}

/// Forward one route to the geometry worker.
async fn proxy_route(
    state: &State,
    kind: RouteKind,
    design_id: Option<&str>,
    method: &Method,
    body: &[u8],
    content_type: Option<&str>,
    query: Option<&str>,
) -> OutResponse {
    let Some(backend) = state.backend.as_ref() else {
        return geometry_unavailable("no geometry worker is attached to this studio");
    };
    if kind == RouteKind::Drawing || kind == RouteKind::Section {
        let Some(design_id) = design_id else {
            return not_found();
        };
        if !valid_design_id(design_id) {
            return sorted(FlatError::new(
                422,
                "invalid_request",
                VALIDATION_MESSAGE,
                vec!["the design id is not a sha256 digest".to_owned()],
            ));
        }
    }
    let outcome = match kind {
        RouteKind::Preview => backend.preview(body, content_type).await,
        RouteKind::Export => backend.export(body, content_type).await,
        RouteKind::Handoff => backend.handoff(body, content_type).await,
        RouteKind::Design => backend.design(body, content_type).await,
        RouteKind::Import => backend.import_step(body, content_type).await,
        RouteKind::Journal => backend.journal_page(query).await,
        RouteKind::JournalVerify => backend.journal_verify().await,
        RouteKind::Drawing => {
            let design_id = design_id.unwrap_or_default();
            backend.drawing(design_id, query).await
        }
        RouteKind::Section => {
            let design_id = design_id.unwrap_or_default();
            backend.section(design_id, query).await
        }
        RouteKind::ProvidersCad => backend.providers_cad().await,
        _ => return not_found(),
    };
    let _ = method;
    match outcome {
        Ok(response) => worker_response(response),
        Err(error) => backend_failure(error),
    }
}

/// Build the client response from the worker's answer.
fn worker_response(response: WorkerResponse) -> OutResponse {
    let status = StatusCode::from_u16(response.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut content_type = "application/json".to_owned();
    let mut headers: Vec<(String, String)> = Vec::new();
    for (name, value) in response.headers {
        if name == "content-type" {
            content_type = value;
            continue;
        }
        headers.push((name, value));
    }
    headers.push(("content-type".to_owned(), content_type));
    headers.push(("content-length".to_owned(), response.body.len().to_string()));
    OutResponse {
        status,
        headers,
        body: response.body,
        challenge: None,
    }
}

/// Map a backend failure onto a sanitized refusal.
fn backend_failure(error: BackendError) -> OutResponse {
    match error {
        BackendError::Unavailable(message) => geometry_unavailable(&message),
        BackendError::Timeout(message) => sorted(FlatError::new(
            504,
            "geometry_worker_timeout",
            "the geometry worker exceeded its bounded timeout",
            vec![crate::errors::truncate_chars(&message, 256)],
        )),
        BackendError::Transport(message) => geometry_unavailable(&message),
        BackendError::TooLarge(message) => sorted(FlatError::new(
            502,
            "geometry_worker_response_too_large",
            "the geometry worker answered more than the front door accepts",
            vec![crate::errors::truncate_chars(&message, 256)],
        )),
        BackendError::Unsupported(route) => sorted(FlatError::new(
            501,
            "geometry_route_unavailable",
            "this studio build does not serve the requested geometry route",
            vec![crate::errors::truncate_chars(&route, 64)],
        )),
    }
}

/// The refusal a missing geometry worker produces.
fn geometry_unavailable(detail: &str) -> OutResponse {
    sorted(FlatError::new(
        503,
        GEOMETRY_UNAVAILABLE_CODE,
        GEOMETRY_UNAVAILABLE_MESSAGE,
        vec![crate::errors::truncate_chars(detail, 256)],
    ))
}

async fn chat_route(state: Arc<State>, request: ChatRequest) -> OutResponse {
    let (environ, stored) = match state.environ_and_stored() {
        Ok(value) => value,
        Err(error) => {
            eprintln!(
                "unhandled studio failure: {}",
                error.python_exception_name()
            );
            return internal_error();
        }
    };

    // The design hook: the worker owns the strict parser, so the front door
    // asks it whether the turn is a design request at all.
    let parsed = match state.backend.as_ref() {
        Some(backend) => chat_design::parse_intent(backend.as_ref(), &request.prompt)
            .await
            .ok(),
        None => None,
    };
    let is_design_request = parsed
        .as_ref()
        .is_some_and(chat_design::ParsedIntent::is_design_request);

    let (outcome, turn) =
        if is_design_request && request.provider != crate::schemas::ProviderId::Local {
            let Some(backend) = state.backend.clone() else {
                return geometry_unavailable("no geometry worker is attached to this studio");
            };
            let transport = Arc::clone(&state.transport);
            let environ = environ.clone();
            let stored = stored.clone();
            let provider = request.provider;
            let model = request.model.clone();
            let allow_remote = request.allow_remote;
            let call = move |prompt: String| {
                let environ = environ.clone();
                let stored = stored.clone();
                let transport = Arc::clone(&transport);
                let model = model.clone();
                async move {
                    dispatch(
                        provider,
                        model.as_deref(),
                        &prompt,
                        allow_remote,
                        &environ,
                        &stored,
                        transport.as_ref(),
                    )
                    .await
                }
            };
            match chat_design::remote_design_turn(
                backend.as_ref(),
                &request.prompt,
                request.cad_provider.as_deref(),
                call,
            )
            .await
            {
                Ok(turn) => {
                    let model = turn
                        .model
                        .clone()
                        .unwrap_or_else(|| request.model.clone().unwrap_or_default());
                    let outcome = ChatOutcome {
                        text: turn.text.clone(),
                        model,
                        offline: false,
                    };
                    (outcome, turn)
                }
                Err(chat_design::DesignTurnError::Provider(failure)) => {
                    return provider_failure(failure);
                }
                Err(chat_design::DesignTurnError::Backend(error)) => {
                    return backend_failure(error);
                }
            }
        } else {
            let outcome = match dispatch(
                request.provider,
                request.model.as_deref(),
                &request.prompt,
                request.allow_remote,
                &environ,
                &stored,
                state.transport.as_ref(),
            )
            .await
            {
                Ok(outcome) => outcome,
                Err(failure) => return provider_failure(failure),
            };
            if is_design_request {
                let Some(backend) = state.backend.clone() else {
                    return geometry_unavailable("no geometry worker is attached to this studio");
                };
                let parsed = parsed.unwrap_or_default();
                let turn = chat_design::local_design_turn(
                    backend.as_ref(),
                    &parsed,
                    request.cad_provider.as_deref(),
                )
                .await;
                let outcome = ChatOutcome {
                    text: turn.text.clone(),
                    ..outcome
                };
                (outcome, turn)
            } else {
                (outcome, chat_design::DesignTurn::default())
            }
        };

    // The journal turn. A worker that is down must not take the native chat
    // route down with it, so a refused append is reported locally and the
    // answer is still returned.
    if let Some(backend) = state.backend.as_ref() {
        if let Err(error) = chat_design::append_chat_turn(
            backend.as_ref(),
            request.provider.as_str(),
            &outcome.model,
            outcome.offline,
            &request.prompt,
            turn.design_id.as_deref(),
            turn.design_requested,
            request.cad_provider.as_deref(),
            &turn.missing,
        )
        .await
        {
            eprintln!("the studio chat turn could not be journalled: {error}");
        }
    }

    chat_success(&request, &outcome, turn.design.as_ref())
}

fn chat_success(
    request: &ChatRequest,
    outcome: &ChatOutcome,
    design: Option<&Value>,
) -> OutResponse {
    // `ChatResponse.model` is bounded to 256 characters. The Python route
    // constructs the response model, so a provider that names a longer model
    // raises a Pydantic `ValidationError` and FastAPI's 500 handler answers
    // with the sanitized internal-error envelope. Truncating or substituting
    // the model would silently widen the source contract.
    if outcome.model.chars().count() > MAX_MODEL_ID_LENGTH {
        eprintln!("unhandled studio failure: ValidationError");
        return internal_error();
    }
    let text: String = outcome.text.chars().take(4_096).collect();
    let design = match design {
        Some(value) => match serde_json::to_string(value) {
            Ok(text) => Json::Raw(text),
            Err(_) => return internal_error(),
        },
        None => Json::Null,
    };
    let body = Json::object(vec![
        ("text", Json::String(text)),
        (
            "provider",
            Json::String(request.provider.as_str().to_owned()),
        ),
        ("model", Json::String(outcome.model.clone())),
        ("offline", Json::Bool(outcome.offline)),
        ("design", design),
    ]);
    json_response(StatusCode::OK, &body)
}

fn provider_failure(failure: ProviderFailure) -> OutResponse {
    let (status, code, message, detail) = match failure {
        ProviderFailure::Unavailable(message) => (
            503,
            "provider_unavailable",
            "the chat provider is not available in this studio build",
            message,
        ),
        ProviderFailure::Configuration { code, message } => (
            400,
            code,
            "the chat provider is not configured for this request",
            message,
        ),
        ProviderFailure::Timeout(message) => (
            504,
            "provider_timeout",
            "the chat provider exceeded its bounded timeout",
            message,
        ),
        ProviderFailure::Network(message) => (
            502,
            "provider_network_error",
            "the studio could not reach the configured provider",
            message,
        ),
        ProviderFailure::Protocol(message) => (
            502,
            "provider_protocol_error",
            "the provider returned an unparseable payload",
            message,
        ),
        ProviderFailure::Subprocess(message) => (
            502,
            "provider_subprocess_error",
            "the chat provider subprocess failed",
            message,
        ),
        ProviderFailure::Internal(message) => {
            eprintln!("unhandled studio provider failure");
            let _ = message;
            return internal_error();
        }
    };
    sorted(FlatError::new(
        status,
        code,
        message,
        vec![crate::errors::truncate_chars(&detail, 256)],
    ))
}

fn json_response(status: StatusCode, body: &Json) -> OutResponse {
    match body.to_bytes() {
        Ok(bytes) => OutResponse::json(status, bytes),
        Err(_) => internal_error(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decoding_matches_starlette() {
        assert_eq!(percent_decode("/api/studio/he%61lth"), "/api/studio/health");
        assert_eq!(percent_decode("/a%2Fb"), "/a/b");
        assert_eq!(percent_decode("/%ZZ"), "/%ZZ");
    }

    #[test]
    fn body_limit_validation_matches_python() {
        assert_eq!(StudioConfig::validate_body_limit(1024).unwrap(), 1024);
        assert!(StudioConfig::validate_body_limit(1023).is_err());
        assert!(StudioConfig::validate_body_limit(262_145).is_err());
    }

    #[test]
    fn the_routes_are_classified() {
        assert_eq!(classify("/api/studio/health").unwrap().0, RouteKind::Health);
        assert_eq!(
            classify("/api/studio/journal/verify").unwrap().0,
            RouteKind::JournalVerify
        );
        let (kind, design_id) = classify("/api/studio/design/abc/drawing").unwrap();
        assert_eq!(kind, RouteKind::Drawing);
        assert_eq!(design_id.as_deref(), Some("abc"));
        assert!(classify("/api/studio/design//drawing").is_none());
        assert!(classify("/api/studio/design/a/b/drawing").is_none());
        assert!(classify("/api/studio/nope").is_none());
    }

    #[test]
    fn the_import_route_has_its_own_body_limit() {
        let shared = 262_144;
        assert_eq!(body_limit(Some(RouteKind::Preview), shared, 64_000), shared);
        assert_eq!(body_limit(Some(RouteKind::Import), shared, 64_000), 64_000);
        // The import ceiling is the worker's own, even when it is below the
        // shared Studio limit.
        assert_eq!(body_limit(Some(RouteKind::Import), shared, 1_024), 1_024);
    }

    #[test]
    fn the_cad_provider_route_carries_the_route_bearer_gate() {
        assert!(RouteKind::ProvidersCad.requires_route_bearer());
        assert!(RouteKind::Journal.requires_route_bearer());
        assert!(!RouteKind::Health.requires_route_bearer());
        assert!(!RouteKind::Providers.requires_route_bearer());
    }

    #[test]
    fn design_identifiers_are_sha256_digests() {
        assert!(valid_design_id(&"a".repeat(64)));
        assert!(!valid_design_id(&"a".repeat(63)));
        assert!(!valid_design_id(&"A".repeat(64)));
        assert!(!valid_design_id(""));
    }
}
