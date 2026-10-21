//! Geometry route bytes: previews, STL exports, every registry handoff, the
//! truthful STEP 503 seam, and a test-only exporter plumbing check.
//!
//! The parked native primitive geometry is the subject of this file, so it
//! runs in the `native-primitives` build. The default build has no native
//! geometry; the live Python differential covers the same routes through the
//! supervised geometry worker.

#![cfg(feature = "native-primitives")]

mod common;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use kala_studio::geometry::{StepCapability, StepError, StepExporter};
#[cfg(feature = "native-step")]
use kala_studio::integrations::build_registry;
use kala_studio::native_backend::NativePrimitivesBackend;
use kala_studio::schemas::PrimitiveKind;
use serde_json::json;

use common::{RequestSpec, TOKEN, TestServer};

fn post(body: &serde_json::Value) -> RequestSpec {
    let mut spec = RequestSpec::default();
    spec.headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    spec.headers
        .push(("content-type".to_owned(), "application/json".to_owned()));
    spec.body = Some(serde_json::to_vec(body).unwrap());
    spec
}

/// The parked native backend over one STEP exporter.
fn native_backend(
    exporter: Arc<dyn StepExporter>,
) -> Arc<dyn kala_studio::geometry_backend::GeometryBackend> {
    Arc::new(NativePrimitivesBackend::new(
        exporter,
        std::env::temp_dir().join("kala-studio-work"),
    ))
}

async fn start() -> (TestServer, tempfile::TempDir) {
    let work = tempfile::tempdir().unwrap();
    let server = TestServer::start_with_backend(
        work.path().join("data"),
        TOKEN,
        64 * 1024,
        None,
        HashMap::new(),
        kala_studio::geometry::default_step_exporter(),
        None,
        native_backend(kala_studio::geometry::default_step_exporter()),
    )
    .await;
    (server, work)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preview_and_stl_bytes_match_the_geometry_crate() {
    let (server, _work) = start().await;
    let cases = [
        ("box", json!({})),
        ("cylinder", json!({})),
        ("sphere", json!({})),
        (
            "box",
            json!({"size_x": 20, "size_y": 10, "size_z": 5, "subdivisions": 2}),
        ),
        (
            "cylinder",
            json!({"radius": 2.5, "height": 7.5, "radial_segments": 12, "height_segments": 3}),
        ),
        ("sphere", json!({"radius": 3.5, "rings": 12, "sectors": 16})),
    ];
    for (kind, parameters) in cases {
        let mesh = kala_geometry::build_primitive_json(kind, Some(&parameters)).unwrap();
        let preview = common::request(
            server.addr,
            "POST",
            "/api/studio/preview",
            post(&json!({"kind": kind, "parameters": parameters})),
        )
        .await;
        assert_eq!(preview.status, 200);
        assert_eq!(preview.header("content-type"), Some("application/json"));
        let body = preview.json();
        assert_eq!(body["engine"], "analytic");
        assert_eq!(
            body["warnings"][0],
            "analytic tessellation is a preview representation; not CAD-grade NURBS or B-rep"
        );
        let positions: Vec<f64> = body["positions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_f64().unwrap())
            .collect();
        let normals: Vec<f64> = body["normals"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_f64().unwrap())
            .collect();
        let indices: Vec<u64> = body["indices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_u64().unwrap())
            .collect();
        assert_eq!(positions, mesh.positions(), "positions for {kind}");
        assert_eq!(normals, mesh.normals(), "normals for {kind}");
        assert_eq!(
            indices,
            mesh.indices()
                .iter()
                .map(|value| u64::from(*value))
                .collect::<Vec<_>>(),
            "indices for {kind}"
        );
        let bounds = kala_geometry::compute_bounds(mesh.positions()).unwrap();
        assert_eq!(
            body["bounds"]["min"],
            json!([bounds.min[0], bounds.min[1], bounds.min[2]])
        );
        assert_eq!(
            body["bounds"]["max"],
            json!([bounds.max[0], bounds.max[1], bounds.max[2]])
        );
        assert_eq!(
            preview.header("content-length"),
            Some(preview.body.len().to_string().as_str())
        );

        let export = common::request(
            server.addr,
            "POST",
            "/api/studio/export",
            post(&json!({"kind": kind, "format": "stl", "parameters": parameters})),
        )
        .await;
        assert_eq!(export.status, 200);
        assert_eq!(export.header("content-type"), Some("model/stl"));
        assert_eq!(
            export.header("content-disposition").unwrap(),
            format!("attachment; filename=\"kala-studio-{kind}.stl\"")
        );
        assert_eq!(export.body, kala_geometry::write_binary_stl(&mesh));
    }
    server.stop().await;
}

#[cfg(feature = "native-step")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_handoff_capability_has_real_bytes_or_a_truthful_refusal() {
    let (server, _work) = start().await;
    let box_mesh = kala_geometry::build_primitive_json("box", None).unwrap();
    let stl = kala_geometry::write_binary_stl(&box_mesh);
    let mut stl_handoffs = 0;
    let mut step_handoffs = 0;
    for record in build_registry() {
        for capability in record.capabilities {
            let response = common::request(
                server.addr,
                "POST",
                "/api/studio/handoff",
                post(&json!({
                    "integration_id": record.id,
                    "kind": "box",
                    "format": capability
                })),
            )
            .await;
            if *capability == "stl" {
                stl_handoffs += 1;
                assert_eq!(response.status, 200);
                assert_eq!(response.header("content-type"), Some("model/stl"));
                assert_eq!(
                    response.header("content-disposition").unwrap(),
                    format!("attachment; filename=\"{}.stl\"", record.id)
                );
                assert_eq!(
                    response.header("x-studio-handoff").unwrap(),
                    format!("{};format=stl;bytes={}", record.id, stl.len())
                );
                assert_eq!(response.body, stl);
            } else {
                step_handoffs += 1;
                assert_eq!(response.status, 200, "{} step", record.id);
                assert_eq!(response.header("content-type"), Some("application/step"));
                assert_eq!(
                    response.header("content-disposition").unwrap(),
                    format!("attachment; filename=\"{}.step\"", record.id)
                );
                assert_eq!(
                    response.header("x-studio-handoff").unwrap(),
                    format!("{};format=step;bytes={}", record.id, response.body.len())
                );
                assert!(
                    response.body.starts_with(b"ISO-10303-21;"),
                    "{} step must be a real ISO 10303-21 file",
                    record.id
                );
            }
            for (name, value) in kala_studio::server::SECURITY_HEADERS {
                assert!(
                    response
                        .headers
                        .iter()
                        .any(|(key, actual)| key == name && actual == value),
                    "missing security header {name}"
                );
            }
        }
    }
    assert_eq!(stl_handoffs, 2);
    assert_eq!(step_handoffs, 10);

    // Direct STEP export keeps the same real branch.
    let response = common::request(
        server.addr,
        "POST",
        "/api/studio/export",
        post(&json!({"kind": "box", "format": "step"})),
    )
    .await;
    assert_eq!(response.status, 200);
    assert_eq!(response.header("content-type"), Some("application/step"));
    assert_eq!(
        response.header("content-disposition").unwrap(),
        "attachment; filename=\"kala-studio-box.step\""
    );
    assert!(response.body.starts_with(b"ISO-10303-21;"));
    let response = common::request(
        server.addr,
        "POST",
        "/api/studio/handoff",
        post(&json!({"integration_id": "openscad", "kind": "box", "format": "step"})),
    )
    .await;
    assert_eq!(response.status, 409);
    assert_eq!(response.json()["code"], "handoff_format_unsupported");
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn step_export_keeps_validation_overflow_and_error_contracts() {
    let (server, _work) = start().await;
    // Invalid dimensions keep the Python validation text and the 422 branch
    // before any STEP work happens (the route validates the mesh first).
    let invalid = common::request(
        server.addr,
        "POST",
        "/api/studio/export",
        post(&json!({"kind": "box", "format": "step", "parameters": {"size_x": 0.01}})),
    )
    .await;
    assert_eq!(invalid.status, 422);
    assert_eq!(
        invalid.body_text(),
        "{\"code\":\"invalid_request\",\"details\":[\"size_x must be between 0.1 and 2000.0 millimetres\"],\"message\":\"the request body or parameters are invalid\"}"
    );

    let mut spec = RequestSpec::default();
    spec.headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    spec.headers
        .push(("content-type".to_owned(), "application/json".to_owned()));
    spec.body = Some(br#"{"kind":"box","format":"step","parameters":{"size_x":1e400}}"#.to_vec());
    let non_finite = common::request(server.addr, "POST", "/api/studio/export", spec).await;
    assert_eq!(non_finite.status, 422);
    assert_eq!(
        non_finite.json()["details"][0],
        "size_x must be a finite number"
    );

    // A dimension the Python `float()` would overflow keeps the legacy
    // sanitized 500 (the source raises `OverflowError`, not `ValueError`).
    let mut spec = RequestSpec::default();
    spec.headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    spec.headers
        .push(("content-type".to_owned(), "application/json".to_owned()));
    spec.body = Some(
        format!(
            r#"{{"kind":"box","format":"step","parameters":{{"size_x":{},"size_y":1,"size_z":1}}}}"#,
            "9".repeat(400)
        )
        .into_bytes(),
    );
    let overflow = common::request(server.addr, "POST", "/api/studio/export", spec).await;
    assert_eq!(overflow.status, 500);
    assert_eq!(overflow.json()["code"], "internal_error");

    // The unsupported handoff capability keeps its 409 before STEP work.
    let handoff = common::request(
        server.addr,
        "POST",
        "/api/studio/handoff",
        post(&json!({"integration_id": "openscad", "kind": "box", "format": "step"})),
    )
    .await;
    assert_eq!(handoff.status, 409);
    assert_eq!(handoff.json()["code"], "handoff_format_unsupported");
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unavailable_exporter_keeps_the_truthful_503_and_health_false() {
    let work = tempfile::tempdir().unwrap();
    let server = TestServer::start_with_backend(
        work.path().join("data"),
        TOKEN,
        64 * 1024,
        None,
        HashMap::new(),
        Arc::new(kala_studio::geometry::UnavailableStepExporter),
        None,
        native_backend(Arc::new(kala_studio::geometry::UnavailableStepExporter)),
    )
    .await;
    let health = common::request(server.addr, "GET", "/api/studio/health", {
        let mut spec = RequestSpec::default();
        spec.headers
            .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
        spec
    })
    .await;
    assert_eq!(health.json()["step_available"], false);

    let export = common::request(
        server.addr,
        "POST",
        "/api/studio/export",
        post(&json!({"kind": "box", "format": "step"})),
    )
    .await;
    assert_eq!(export.status, 503);
    let body = export.json();
    assert_eq!(body["code"], "step_unavailable");
    assert_eq!(
        body["message"],
        kala_studio::geometry::STEP_UNAVAILABLE_MESSAGE
    );
    assert_eq!(export.header("content-type"), Some("application/json"));

    let handoff = common::request(
        server.addr,
        "POST",
        "/api/studio/handoff",
        post(&json!({"integration_id": "solidworks", "kind": "box", "format": "step"})),
    )
    .await;
    assert_eq!(handoff.status, 503);
    assert_eq!(handoff.json()["code"], "step_unavailable");
    server.stop().await;
}

struct MockStepExporter;

impl StepExporter for MockStepExporter {
    fn capability(&self) -> StepCapability {
        StepCapability {
            available: true,
            exporter_name: Some("test-only-mock".to_owned()),
        }
    }

    fn export_primitive(
        &self,
        _kind: PrimitiveKind,
        _parameters: Option<&serde_json::Value>,
        _workdir: &Path,
    ) -> Result<Vec<u8>, StepError> {
        Ok(b"ISO-10303-21;\nMOCK-NOT-PRODUCTION;\n".to_vec())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_only_exporter_proves_the_step_response_plumbing() {
    let work = tempfile::tempdir().unwrap();
    let server = TestServer::start_with_backend(
        work.path().join("data"),
        TOKEN,
        64 * 1024,
        None,
        HashMap::new(),
        Arc::new(MockStepExporter),
        None,
        native_backend(Arc::new(MockStepExporter)),
    )
    .await;
    let health = common::request(server.addr, "GET", "/api/studio/health", {
        let mut spec = RequestSpec::default();
        spec.headers
            .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
        spec
    })
    .await;
    assert_eq!(health.json()["step_available"], true);

    let mock = b"ISO-10303-21;\nMOCK-NOT-PRODUCTION;\n";
    let export = common::request(
        server.addr,
        "POST",
        "/api/studio/export",
        post(&json!({"kind": "box", "format": "step"})),
    )
    .await;
    assert_eq!(export.status, 200);
    assert_eq!(export.header("content-type"), Some("application/step"));
    assert_eq!(
        export.header("content-disposition").unwrap(),
        "attachment; filename=\"kala-studio-box.step\""
    );
    assert_eq!(export.body, mock);

    let handoff = common::request(
        server.addr,
        "POST",
        "/api/studio/handoff",
        post(&json!({"integration_id": "solidworks", "kind": "box", "format": "step"})),
    )
    .await;
    assert_eq!(handoff.status, 200);
    assert_eq!(
        handoff.header("x-studio-handoff").unwrap(),
        format!("solidworks;format=step;bytes={}", mock.len())
    );
    assert_eq!(handoff.body, mock);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn validation_overflow_and_unknown_integration_branches() {
    let (server, _work) = start().await;
    let response = common::request(
        server.addr,
        "POST",
        "/api/studio/preview",
        post(&json!({"kind": "box", "parameters": {"size_x": 0.01}})),
    )
    .await;
    assert_eq!(response.status, 422);
    assert_eq!(
        response.body_text(),
        "{\"code\":\"invalid_request\",\"details\":[\"size_x must be between 0.1 and 2000.0 millimetres\"],\"message\":\"the request body or parameters are invalid\"}"
    );

    let huge = format!(
        r#"{{"kind":"box","parameters":{{"size_x":{},"size_y":1,"size_z":1}}}}"#,
        "9".repeat(400)
    );
    let mut spec = RequestSpec::default();
    spec.headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    spec.headers
        .push(("content-type".to_owned(), "application/json".to_owned()));
    spec.body = Some(huge.into_bytes());
    let response = common::request(server.addr, "POST", "/api/studio/preview", spec).await;
    assert_eq!(response.status, 500);
    assert_eq!(response.json()["code"], "internal_error");

    let response = common::request(server.addr, "POST", "/api/studio/preview", {
        let mut spec = RequestSpec::default();
        spec.headers
            .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
        spec.headers
            .push(("content-type".to_owned(), "application/json".to_owned()));
        spec.body = Some(br#"{"kind":"box","parameters":{"size_z":1e400}}"#.to_vec());
        spec
    })
    .await;
    assert_eq!(response.status, 422);
    assert_eq!(
        response.json()["details"][0],
        "size_z must be a finite number"
    );

    let response = common::request(
        server.addr,
        "POST",
        "/api/studio/handoff",
        post(&json!({"integration_id": "nope", "kind": "box", "format": "step"})),
    )
    .await;
    assert_eq!(response.status, 404);
    assert_eq!(response.json()["code"], "unknown_integration");
    server.stop().await;
}
