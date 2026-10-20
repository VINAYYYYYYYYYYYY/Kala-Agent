//! The connection layer of the local Kala HTTP adapter.
//!
//! The layer is the transport's part of the API: it judges the request
//! boundary, bounds the body, and hands the request to a router. It holds no
//! engineering rule, writes no database row, calls no provider, and never
//! opens an outbound connection.
//!
//! The order of the judgements is the policy, and it is the order the Python
//! source uses:
//!
//! 1. the security headers wrap everything, so a refusal from any stage is as
//!    complete as a success;
//! 2. the Host guard runs before a body is read, so a rebound name costs
//!    nothing;
//! 3. the size limit runs before the token is judged, so the limit answers
//!    identically for every method and for an unauthenticated caller;
//! 4. the token is judged before routing, so a mutating method can never reach
//!    a handler unauthenticated;
//! 5. the cross-origin guard refuses a browser-originated request before it
//!    can drive a handler.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Incoming};
use hyper::{Request, Response, StatusCode};

use crate::boundary::{Boundary, Refusal};
use crate::errors;
use crate::middleware::{
    SECURITY_HEADERS, apply_security_headers, header_value, header_values, raw_headers,
    refusal_body,
};

/// The future a router returns.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The router the connection layer hands a permitted request to.
pub trait Router: Send + Sync + 'static {
    /// Answer one permitted request.
    fn handle<'a>(&'a self, request: Request<Full<Bytes>>) -> BoxFuture<'a, Response<Full<Bytes>>>;
}

/// The state of one launch: the request boundary and the router.
#[derive(Clone)]
pub struct ApiState {
    /// The request boundary of this launch.
    pub boundary: Boundary,
    /// The router the permitted requests reach.
    pub router: Arc<dyn Router>,
}

impl ApiState {
    /// Build the state of one launch.
    ///
    /// # Errors
    ///
    /// Returns a sanitized message when the launch token is malformed or the
    /// body limit is outside the accepted range.
    pub fn new(
        api_token: &str,
        max_body_bytes: i64,
        router: Arc<dyn Router>,
    ) -> Result<Self, String> {
        Ok(Self {
            boundary: Boundary::new(api_token, max_body_bytes)?,
            router,
        })
    }
}

/// Answer one request.
///
/// The body is read only after the Host guard has passed, and it is read
/// under the limit, so an oversized body never reaches a handler.
///
/// # Errors
///
/// The function itself does not fail: every failure becomes a sanitized
/// response.
pub async fn handle_request<B>(state: &ApiState, request: Request<B>) -> Response<Full<Bytes>>
where
    B: Body<Data = Bytes> + Unpin,
    B::Error: std::fmt::Display,
{
    let method = request.method().as_str().to_owned();
    let headers = raw_headers(request.headers());

    // 1. The Host guard runs before the body is read.
    if let Some(refusal) = state.boundary.check_host(&headers) {
        return refusal_response(&refusal);
    }

    // 2. The size limit answers every method, declared length first.
    if let Some(refusal) = state.boundary.check_declared_length(&headers) {
        return refusal_response(&refusal);
    }
    let limit = state.boundary.max_body_bytes();
    let (parts, stream) = request.into_parts();
    let body = match read_bounded_body(stream, limit).await {
        Ok(body) => body,
        Err(BodyOutcome::TooLarge) => {
            let refusal = state
                .boundary
                .check_body_length(limit.saturating_add(1))
                .expect("a length over the limit is refused");
            return refusal_response(&refusal);
        }
        Err(BodyOutcome::Failed) => {
            // The Python source lets a transport failure escape the
            // middleware stack, and the server error middleware answers it
            // with the sanitized 500 of the error handlers.
            return internal_error_response();
        }
    };

    // 3. The token is judged before routing.
    if let Some(refusal) = state.boundary.check_token(&method, &headers) {
        return refusal_response(&refusal);
    }

    // 4. The cross-origin guard refuses a browser-originated request.
    if let Some(refusal) = state.boundary.check_origin(&headers) {
        return refusal_response(&refusal);
    }

    // 5. The router answers.
    let request = Request::from_parts(parts, Full::new(body));
    let mut response = state.router.handle(request).await;
    apply_security_headers(response.headers_mut());
    response
}

/// Why the body could not be read.
enum BodyOutcome {
    /// The body was longer than the limit.
    TooLarge,
    /// The transport failed while the body was read.
    Failed,
}

/// Read a body under the limit.
async fn read_bounded_body<B>(body: B, limit: usize) -> Result<Bytes, BodyOutcome>
where
    B: Body<Data = Bytes> + Unpin,
    B::Error: std::fmt::Display,
{
    let mut body = body;
    let mut collected: Vec<u8> = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| BodyOutcome::Failed)?;
        if let Ok(data) = frame.into_data() {
            if collected.len().saturating_add(data.len()) > limit {
                return Err(BodyOutcome::TooLarge);
            }
            collected.extend_from_slice(&data);
        }
    }
    Ok(Bytes::from(collected))
}

/// Answer a refusal with the sanitized body and the full header set.
///
/// A refusal raised below the router never reaches the error handlers, so it
/// carries the security headers itself. The body is the same sanitized shape
/// every other error uses, and it echoes nothing that was submitted.
#[must_use]
pub fn refusal_response(refusal: &Refusal) -> Response<Full<Bytes>> {
    let details: Vec<&str> = refusal.details.iter().map(String::as_str).collect();
    let body = refusal_body(refusal.code, &refusal.message, &details);
    let length = body.len();
    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() =
        StatusCode::from_u16(refusal.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let headers = response.headers_mut();
    headers.insert(
        hyper::header::CONTENT_LENGTH,
        hyper::header::HeaderValue::from(length),
    );
    headers.insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("application/json"),
    );
    for (name, value) in &refusal.extra_headers {
        if let (Ok(name), Ok(value)) = (
            hyper::header::HeaderName::from_bytes(name.as_bytes()),
            hyper::header::HeaderValue::from_str(value),
        ) {
            headers.append(name, value);
        }
    }
    for (name, value) in SECURITY_HEADERS {
        headers.insert(
            hyper::header::HeaderName::from_bytes(name.as_bytes()).expect("static header name"),
            hyper::header::HeaderValue::from_static(value),
        );
    }
    response
}

/// Answer a framework or routing error with the sanitized body.
///
/// This is the ordered layout the Python error handlers write. The security
/// headers are attached here as well as in the transport, because the server
/// error handler answers a failed request from outside the middleware stack.
#[must_use]
pub fn error_response(status: u16, body: &crate::envelope::ErrorResponse) -> Response<Full<Bytes>> {
    let bytes = body.ordered_bytes();
    let length = bytes.len();
    let mut response = Response::new(Full::new(Bytes::from(bytes)));
    *response.status_mut() =
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let headers = response.headers_mut();
    headers.insert(
        hyper::header::CONTENT_LENGTH,
        hyper::header::HeaderValue::from(length),
    );
    headers.insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("application/json"),
    );
    for (name, value) in SECURITY_HEADERS {
        headers.insert(
            hyper::header::HeaderName::from_bytes(name.as_bytes()).expect("static header name"),
            hyper::header::HeaderValue::from_static(value),
        );
    }
    response
}

/// Answer an unexpected failure with a bare 500.
#[must_use]
pub fn internal_error_response() -> Response<Full<Bytes>> {
    let (status, body) = errors::handle_unexpected_error();
    error_response(status, &body)
}

/// The header values the transport received for one header.
#[must_use]
pub fn request_header_value(headers: &[(Vec<u8>, Vec<u8>)], name: &str) -> Option<String> {
    header_value(headers, name)
}

/// Every header value the transport received for one header.
#[must_use]
pub fn request_header_values(headers: &[(Vec<u8>, Vec<u8>)], name: &str) -> Vec<String> {
    header_values(headers, name)
}

/// The hyper body the server reads.
pub type ServerBody = Incoming;

/// Serve HTTP/1.1 on a listener until the shutdown future resolves.
///
/// The server binds nothing and trusts nothing: the connection layer judges
/// every request that arrives.
///
/// # Errors
///
/// Returns an I/O error only when the accept loop fails fatally.
pub async fn serve<F>(
    state: Arc<ApiState>,
    listener: tokio::net::TcpListener,
    shutdown: F,
) -> std::io::Result<()>
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
                let state = Arc::clone(&state);
                let connection = hyper::server::conn::http1::Builder::new()
                    .serve_connection(
                        hyper_util::rt::TokioIo::new(stream),
                        hyper::service::service_fn(move |request| {
                            let state = Arc::clone(&state);
                            let is_head = request.method() == hyper::Method::HEAD;
                            async move {
                                let response = handle_request(&state, request).await;
                                let response = if is_head {
                                    strip_head_body(response)
                                } else {
                                    response
                                };
                                Ok::<_, std::convert::Infallible>(response)
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
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), graceful.shutdown()).await;
    Ok(())
}

/// Drop the body of a response to a HEAD request, keeping its length.
fn strip_head_body(response: Response<Full<Bytes>>) -> Response<Full<Bytes>> {
    let (parts, body) = response.into_parts();
    let length = body.clone().into_inner().map_or(0, |inner| inner.len());
    let mut response = Response::from_parts(parts, Full::new(Bytes::new()));
    if !response
        .headers()
        .contains_key(hyper::header::CONTENT_LENGTH)
    {
        response.headers_mut().insert(
            hyper::header::CONTENT_LENGTH,
            hyper::header::HeaderValue::from(length),
        );
    }
    response
}

/// Whether an accept error is worth retrying.
fn is_transient_accept_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::Interrupted
            | std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::TimedOut
    )
}
