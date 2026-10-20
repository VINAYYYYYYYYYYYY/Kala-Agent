//! The minimal router the connection layer is exercised with.
//!
//! The Python source keeps its routes in `kala_agent.api.app`, which needs the
//! composition root and therefore belongs to a later cell. This router is the
//! transport's own test surface: it answers one read, echoes one body, and
//! reports every other path through the same sanitized error envelope the
//! Python error handlers write.
//!
//! It is deliberately tiny. It exists so the boundary, the body limit and the
//! error envelope can be driven end to end without the composition root, and
//! so the live Python differential has a route that means the same thing on
//! both sides.

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::header::{CONTENT_LENGTH, CONTENT_TYPE};
use hyper::{Method, Request, Response, StatusCode};

use crate::errors;
use crate::server::{BoxFuture, Router};

/// The path the probe router answers.
pub const PROBE_PATH: &str = "/probe";

/// The body the probe router answers a read with.
pub const PROBE_BODY: &[u8] = b"{\"ok\":true}";

/// The minimal router of the connection layer.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProbeRouter;

impl Router for ProbeRouter {
    fn handle<'a>(&'a self, request: Request<Full<Bytes>>) -> BoxFuture<'a, Response<Full<Bytes>>> {
        Box::pin(async move {
            let method = request.method().clone();
            let path = request.uri().path().to_owned();
            if path != PROBE_PATH {
                let (status, body) = errors::handle_http_exception(404, "Not Found");
                return crate::server::error_response(status, &body);
            }
            match method {
                Method::GET => {
                    let mut response = Response::new(Full::new(Bytes::from_static(PROBE_BODY)));
                    *response.status_mut() = StatusCode::OK;
                    let headers = response.headers_mut();
                    headers.insert(CONTENT_LENGTH, PROBE_BODY.len().into());
                    headers.insert(CONTENT_TYPE, "application/json".parse().unwrap());
                    response
                }
                Method::POST => {
                    let body = request
                        .into_body()
                        .collect()
                        .await
                        .map(http_body_util::Collected::to_bytes)
                        .unwrap_or_else(|_| Bytes::new());
                    let mut response = Response::new(Full::new(body.clone()));
                    *response.status_mut() = StatusCode::OK;
                    let headers = response.headers_mut();
                    headers.insert(CONTENT_LENGTH, body.len().into());
                    headers.insert(CONTENT_TYPE, "application/octet-stream".parse().unwrap());
                    response
                }
                _ => {
                    let (status, body) = errors::handle_http_exception(405, "Method Not Allowed");
                    crate::server::error_response(status, &body)
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::Boundary;
    use crate::server::{ApiState, handle_request};
    use std::sync::Arc;

    fn state() -> ApiState {
        ApiState::new(
            "unit-test-launch-token-0123456789abcdef",
            4096,
            Arc::new(ProbeRouter),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn the_probe_router_answers_one_read() {
        let mut request = Request::builder()
            .method(Method::GET)
            .uri(PROBE_PATH)
            .body(Full::new(Bytes::new()))
            .unwrap();
        request
            .headers_mut()
            .insert("host", "127.0.0.1".parse().unwrap());
        let response = handle_request(&state(), request).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], "application/json");
        assert_eq!(response.headers()["x-frame-options"], "DENY");
    }

    #[tokio::test]
    async fn an_unknown_path_is_reported_safely() {
        let mut request = Request::builder()
            .method(Method::GET)
            .uri("/missing")
            .body(Full::new(Bytes::new()))
            .unwrap();
        request
            .headers_mut()
            .insert("host", "127.0.0.1".parse().unwrap());
        let response = handle_request(&state(), request).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            String::from_utf8(body.to_vec()).unwrap(),
            "{\"error\":{\"code\":\"not_found\",\"message\":\"the requested resource does not exist\"},\"details\":[]}"
        );
    }

    #[tokio::test]
    async fn the_boundary_still_guards_the_probe_router() {
        let mut request = Request::builder()
            .method(Method::GET)
            .uri(PROBE_PATH)
            .body(Full::new(Bytes::new()))
            .unwrap();
        request
            .headers_mut()
            .insert("host", "evil.example".parse().unwrap());
        let response = handle_request(&state(), request).await;
        assert_eq!(response.status(), StatusCode::MISDIRECTED_REQUEST);
    }

    #[test]
    fn the_boundary_is_built_from_the_launch_token() {
        assert!(Boundary::new("short", 4096).is_err());
        assert!(Boundary::new("unit-test-launch-token-0123456789abcdef", 0).is_err());
        assert!(Boundary::new("unit-test-launch-token-0123456789abcdef", 4096).is_ok());
    }
}
