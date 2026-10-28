//! Edit turns and reported token usage (ADR-0012).
//!
//! Every model reply is scripted through the transport seam and every design
//! is compiled by a deterministic stub backend, so no credential, no network
//! and no CAD provider is used. The stub stores the canonical script exactly
//! as the real store does and serves it back over the `script` artifact
//! route, which is what the front door reads for a base design.

mod common;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use sha2::{Digest, Sha256};

use kala_studio::geometry_backend::{
    BackendError, BackendFuture, BackendHealth, GeometryBackend, WorkerResponse,
};
use kala_studio::providers::remote::{
    HttpTransport, RemoteRequest, RemoteResponse, TransportError,
};

use common::{RequestSpec, TOKEN, TestServer};

const PLATE_SCRIPT: &str = r#"{"components":[{"name":"plate","steps":[{"op":"sketch","plane":"XY","profile":"circle","diameter":6,"x":8,"y":8},{"op":"pocket"}]}],"summary":"plate with a 6 mm hole"}"#;
const REVISED_SCRIPT: &str = r#"{"components":[{"name":"plate","steps":[{"op":"sketch","plane":"XY","profile":"circle","diameter":10,"x":8,"y":8},{"op":"pocket"}]}],"summary":"plate with a 10 mm hole"}"#;

// ---------------------------------------------------------------------------
// The scripted provider transport
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ScriptedTransport {
    requests: Mutex<Vec<RemoteRequest>>,
    replies: Mutex<VecDeque<Result<Vec<u8>, TransportError>>>,
}

impl ScriptedTransport {
    /// Queue one OpenAI-compatible reply carrying `content`.
    fn push_reply(&self, content: &str, usage: Option<(u64, u64)>) {
        let mut body = serde_json::json!({
            "model": "mock-served-model",
            "choices": [{"message": {"content": content}}],
        });
        if let Some((input, output)) = usage {
            body["usage"] = serde_json::json!({
                "prompt_tokens": input,
                "completion_tokens": output,
            });
        }
        self.push_raw(serde_json::to_vec(&body).unwrap());
    }

    fn push_raw(&self, body: Vec<u8>) {
        self.replies.lock().unwrap().push_back(Ok(body.clone()));
    }

    fn requests(&self) -> Vec<RemoteRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// The prompt of every model call, in order.
    fn prompts(&self) -> Vec<String> {
        self.requests()
            .iter()
            .map(|request| {
                let body: Value = serde_json::from_slice(&request.body).expect("request JSON");
                body["messages"][0]["content"]
                    .as_str()
                    .expect("the prompt rides in the message content")
                    .to_owned()
            })
            .collect()
    }
}

impl HttpTransport for ScriptedTransport {
    fn post<'a>(
        &'a self,
        request: RemoteRequest,
    ) -> Pin<Box<dyn Future<Output = Result<RemoteResponse, TransportError>> + Send + 'a>> {
        self.requests.lock().unwrap().push(request);
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(TransportError::Network("no scripted reply".to_owned())));
        Box::pin(async move {
            reply.map(|body| RemoteResponse {
                status: 200,
                reason: Some("OK".to_owned()),
                content_length: Some(body.len() as u64),
                body,
            })
        })
    }
}

// ---------------------------------------------------------------------------
// The stub design store
// ---------------------------------------------------------------------------

/// A deterministic geometry backend that stores designs and journals turns.
#[derive(Default)]
struct StubStore {
    /// design id -> the canonical script bytes the `script` artifact serves.
    scripts: Mutex<BTreeMap<String, String>>,
    /// Every `chat.turn` request body the front door appended, decoded.
    journal: Mutex<Vec<Value>>,
    /// How many design requests reached the backend.
    design_calls: AtomicUsize,
}

impl StubStore {
    fn design_calls(&self) -> usize {
        self.design_calls.load(Ordering::SeqCst)
    }

    fn journal_payloads(&self) -> Vec<Value> {
        self.journal
            .lock()
            .unwrap()
            .iter()
            .map(|event| event["payload"].clone())
            .collect()
    }

    fn stored_script(&self, design_id: &str) -> Option<String> {
        self.scripts.lock().unwrap().get(design_id).cloned()
    }
}

fn hex_digest(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn json_response(status: u16, body: Value) -> WorkerResponse {
    WorkerResponse {
        status,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body: serde_json::to_vec(&body).unwrap(),
    }
}

impl GeometryBackend for StubStore {
    fn health(&self) -> BackendFuture<'_, BackendHealth> {
        Box::pin(async {
            BackendHealth {
                state: "ready".to_owned(),
                version: Some("stub".to_owned()),
                design_available: true,
                step_available: false,
            }
        })
    }

    fn intent_parse<'a>(
        &'a self,
        _body: &'a [u8],
        _content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async {
            Ok(json_response(
                200,
                serde_json::json!({"family": null, "intent": null, "missing": [], "notes": []}),
            ))
        })
    }

    fn intent_schema<'a>(&'a self) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async {
            Ok(WorkerResponse {
                status: 200,
                headers: vec![("content-type".to_owned(), "application/json".to_owned())],
                body: br#"{"type":"object","required":["script"],"properties":{"script":{"type":"object","properties":{"summary":{"type":"string"},"components":{"type":"array"}}}}}"#.to_vec(),
            })
        })
    }

    fn intent_validate<'a>(
        &'a self,
        body: &'a [u8],
        _content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            let request: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
            let text = request["text"].as_str().unwrap_or_default();
            let intent = kala_studio::chat_design::extract_design_json_block(text)
                .and_then(|block| serde_json::from_str::<Value>(&block).ok())
                .filter(|value| value.get("script").is_some_and(Value::is_object));
            match intent {
                Some(intent) => Ok(json_response(200, serde_json::json!({"intent": intent}))),
                None => Ok(json_response(
                    422,
                    serde_json::json!({
                        "code": "invalid_request",
                        "message": "the request body or parameters are invalid",
                        "details": ["script: Field required"],
                    }),
                )),
            }
        })
    }

    fn design<'a>(
        &'a self,
        body: &'a [u8],
        _content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            self.design_calls.fetch_add(1, Ordering::SeqCst);
            let request: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
            let provider = request["provider"].as_str().unwrap_or_default().to_owned();
            let script = request["script"].clone();
            let script_text = serde_json::to_string(&script).unwrap_or_default();
            let design_id = hex_digest(&format!("{provider}\u{0}{script_text}"));
            self.scripts
                .lock()
                .unwrap()
                .insert(design_id.clone(), script_text);
            // A summary that says REJECT stands for a build the checks refuse:
            // it lets a test spend the turn's one build repair.
            let rejected = script["summary"]
                .as_str()
                .unwrap_or_default()
                .contains("REJECT");
            let (verified, checks, warnings) = if rejected {
                (
                    false,
                    serde_json::json!([
                        {"id": "cuts_effective", "passed": false, "expected": 1.0, "measured": 0.0}
                    ]),
                    serde_json::json!(["component 'plate': the pocket removed no material"]),
                )
            } else {
                (true, serde_json::json!([]), serde_json::json!([]))
            };
            Ok(json_response(
                200,
                serde_json::json!({
                    "design_id": design_id,
                    "label": "stub plate",
                    "provider": provider,
                    "verified": verified,
                    "measured_by": "Stub",
                    "measured": {
                        "volume_mm3": 1.5,
                        "surface_area_mm2": 2.5,
                        "size_mm": [1.0, 1.0, 1.0],
                    },
                    "script": script,
                    "checks": checks,
                    "warnings": warnings,
                }),
            ))
        })
    }

    fn artifact<'a>(
        &'a self,
        design_id: &'a str,
        kind: &'a str,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            let script = (kind == "script")
                .then(|| self.stored_script(design_id))
                .flatten();
            Ok(match script {
                Some(script) => WorkerResponse {
                    status: 200,
                    headers: vec![("content-type".to_owned(), "application/json".to_owned())],
                    body: script.into_bytes(),
                },
                None => json_response(
                    404,
                    serde_json::json!({
                        "code": "artifact_not_found",
                        "message": "no artifact of that kind is stored for this design",
                        "details": [],
                    }),
                ),
            })
        })
    }

    fn journal_append<'a>(
        &'a self,
        body: &'a [u8],
        _content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            let event: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
            let sequence = {
                let mut journal = self.journal.lock().unwrap();
                journal.push(event);
                journal.len()
            };
            Ok(json_response(
                201,
                serde_json::json!({"sequence": sequence}),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn provider_environment() -> HashMap<String, String> {
    let mut environ = HashMap::new();
    environ.insert(
        "OPENROUTER_API_KEY".to_owned(),
        "sk-or-synthetic-not-a-live-credential-0012".to_owned(),
    );
    environ
}

async fn start() -> (
    TestServer,
    Arc<StubStore>,
    Arc<ScriptedTransport>,
    tempfile::TempDir,
) {
    let work = tempfile::tempdir().unwrap();
    let store = Arc::new(StubStore::default());
    let transport = Arc::new(ScriptedTransport::default());
    let server = TestServer::start_with_backend(
        work.path().join("studio-data"),
        TOKEN,
        64 * 1024,
        None,
        provider_environment(),
        Arc::new(kala_studio::geometry::UnavailableStepExporter),
        Some(Arc::clone(&transport) as Arc<dyn HttpTransport>),
        Arc::clone(&store) as Arc<dyn GeometryBackend>,
    )
    .await;
    (server, store, transport, work)
}

fn chat_spec(body: &Value) -> RequestSpec {
    let mut spec = RequestSpec::default();
    spec.headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    spec.headers
        .push(("content-type".to_owned(), "application/json".to_owned()));
    spec.body = Some(serde_json::to_vec(body).unwrap());
    spec
}

async fn chat(server: &TestServer, body: Value) -> common::TestResponse {
    common::request(server.addr, "POST", "/api/studio/chat", chat_spec(&body)).await
}

async fn artifact(server: &TestServer, design_id: &str, kind: &str) -> common::TestResponse {
    let mut spec = RequestSpec::default();
    spec.headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    common::request(
        server.addr,
        "GET",
        &format!("/api/studio/design/{design_id}/artifact/{kind}"),
        spec,
    )
    .await
}

/// A draft turn that stores [`PLATE_SCRIPT`] as design D.
async fn draft_design(server: &TestServer, transport: &ScriptedTransport) -> String {
    transport.push_reply(
        &format!("```json\n{{\"script\": {PLATE_SCRIPT}}}\n```"),
        None,
    );
    let response = chat(
        server,
        serde_json::json!({
            "provider": "openrouter",
            "model": "mock/model",
            "prompt": "a plate with a 6 mm hole 8 mm from both edges",
            "allow_remote": true,
            "cad_provider": "opencascade",
        }),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    response.json()["design"]["design_id"]
        .as_str()
        .expect("the draft turn built a design")
        .to_owned()
}

fn edit_turn(base: &str, focus: Option<&str>) -> Value {
    let mut body = serde_json::json!({
        "provider": "openrouter",
        "model": "mock/model",
        "prompt": "make this hole 10 mm",
        "allow_remote": true,
        "cad_provider": "opencascade",
        "base_design_id": base,
    });
    if let Some(focus) = focus {
        body["focus"] = Value::String(focus.to_owned());
    }
    body
}

// ---------------------------------------------------------------------------
// The refusals run before any model call
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_edit_turn_without_a_cad_target_or_a_model_is_refused_before_any_call() {
    let (server, store, transport, _work) = start().await;
    let design_id = "a".repeat(64);

    // No CAD target: the edit could never be built.
    let mut body = edit_turn(&design_id, None);
    body.as_object_mut().unwrap().remove("cad_provider");
    let response = chat(&server, body).await;
    assert_eq!(response.status, 400);
    assert_eq!(response.json()["code"], "edit_requires_design_turn");
    assert!(
        response.json()["details"][0]
            .as_str()
            .unwrap_or_default()
            .contains("cad_provider")
    );

    // The local helper has no model to revise the design with.
    let mut body = edit_turn(&design_id, None);
    body["provider"] = Value::String("local".to_owned());
    let response = chat(&server, body).await;
    assert_eq!(response.status, 400);
    assert_eq!(response.json()["code"], "edit_requires_model");

    // A selected feature needs the design it belongs to.
    let response = chat(
        &server,
        serde_json::json!({
            "provider": "openrouter",
            "model": "mock/model",
            "prompt": "make this hole 10 mm",
            "allow_remote": true,
            "cad_provider": "opencascade",
            "focus": "the 6 mm hole",
        }),
    )
    .await;
    assert_eq!(response.status, 400);
    assert_eq!(response.json()["code"], "focus_requires_base_design");

    // None of the three touched the store or a provider.
    assert_eq!(store.design_calls(), 0);
    assert!(transport.requests().is_empty());
    assert!(store.journal_payloads().is_empty());
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_or_malformed_design_identifier_is_not_found() {
    let (server, store, transport, _work) = start().await;
    for design_id in ["a".repeat(64), "not-a-design".to_owned()] {
        let response = chat(&server, edit_turn(&design_id, None)).await;
        assert_eq!(
            response.status,
            404,
            "design {design_id}: {}",
            response.body_text()
        );
        assert_eq!(response.json()["code"], "design_not_found");
        assert!(transport.requests().is_empty());
    }
    assert_eq!(store.design_calls(), 0);
    server.stop().await;
}

// ---------------------------------------------------------------------------
// The edit turn itself
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_edit_turn_carries_the_stored_script_and_returns_a_new_design() {
    let (server, store, transport, _work) = start().await;
    let base_design_id = draft_design(&server, &transport).await;
    // The draft turn is journalled; clear our view so the edit assertions are
    // about the edit turn itself.
    let stored_before = store.stored_script(&base_design_id).expect("stored base");
    store.journal.lock().unwrap().clear();
    transport.requests.lock().unwrap().clear();

    let focus = "the 6 mm hole at x=8, y=8";
    transport.push_reply(
        &format!("```json\n{{\"script\": {REVISED_SCRIPT}}}\n```"),
        Some((321, 45)),
    );
    let response = chat(&server, edit_turn(&base_design_id, Some(focus))).await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    let document = response.json();

    // The response always carries the usage block.
    assert_eq!(document["usage"]["input_tokens"], 321);
    assert_eq!(document["usage"]["output_tokens"], 45);

    // A new design, never the base design.
    let new_design_id = document["design"]["design_id"]
        .as_str()
        .expect("the edit turn built a design")
        .to_owned();
    assert_ne!(new_design_id, base_design_id);
    assert_eq!(
        store.stored_script(&base_design_id).as_deref(),
        Some(stored_before.as_str()),
        "the base design is never mutated"
    );

    // The prompt carries the instruction, the schema, the stored script, the
    // sentence, the selected feature and the request, in that order.
    let prompts = transport.prompts();
    assert_eq!(prompts.len(), 1, "one model call for a clean edit turn");
    let prompt = &prompts[0];
    let instruction = prompt
        .find(kala_studio::chat_design::DESIGN_INTENT_INSTRUCTION)
        .expect("the unchanged instruction");
    let schema = prompt.find("DesignIntent JSON Schema:").expect("schema");
    let base = prompt.find("CURRENT DESIGN:").expect("the base section");
    let script = prompt
        .find(&stored_before)
        .expect("the stored script bytes");
    let sentence = prompt
        .find(
            "Return the COMPLETE revised DesignIntent and keep everything the request does not change.",
        )
        .expect("the edit sentence");
    let feature = prompt.find("SELECTED FEATURE:").expect("the feature");
    let focus_at = prompt.find(focus).expect("the focus text");
    let request = prompt.find("User request:").expect("the request");
    assert!(instruction < schema, "{prompt}");
    assert!(schema < base, "{prompt}");
    assert!(base < script, "{prompt}");
    assert!(script < sentence, "{prompt}");
    assert!(sentence < feature, "{prompt}");
    assert!(feature < focus_at, "{prompt}");
    assert!(focus_at < request, "{prompt}");
    assert!(
        prompt.ends_with("User request:\nmake this hole 10 mm"),
        "{prompt}"
    );

    // The artifact route serves exactly the bytes the prompt carried.
    let artifact_response = artifact(&server, &base_design_id, "script").await;
    assert_eq!(artifact_response.status, 200);
    assert_eq!(artifact_response.body_text(), stored_before);

    // The journal records the base id, the focus length and the usage -- never
    // the focus text.
    let payloads = store.journal_payloads();
    assert_eq!(payloads.len(), 1);
    let payload = &payloads[0];
    assert_eq!(payload["base_design_id"], base_design_id);
    assert_eq!(payload["focus_chars"], focus.chars().count());
    assert_eq!(payload["usage"]["input_tokens"], 321);
    assert_eq!(payload["usage"]["output_tokens"], 45);
    assert_eq!(payload["design_id"], new_design_id);
    let raw = serde_json::to_string(payload).unwrap();
    assert!(
        !raw.contains("x=8"),
        "the focus text is never journalled: {raw}"
    );
    assert!(
        !raw.contains("make this hole"),
        "the prompt is never journalled"
    );

    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_validation_repair_carries_the_same_base_section_and_its_usage() {
    let (server, store, transport, _work) = start().await;
    let base_design_id = draft_design(&server, &transport).await;
    let stored = store.stored_script(&base_design_id).expect("stored base");
    transport.requests.lock().unwrap().clear();
    store.journal.lock().unwrap().clear();

    // The first reply carries no script, so the turn spends its one repair.
    transport.push_reply("```json\n{\"script\": null}\n```", Some((11, 4)));
    transport.push_reply(
        &format!("```json\n{{\"script\": {REVISED_SCRIPT}}}\n```"),
        Some((13, 6)),
    );
    let focus = "the 6 mm hole";
    let response = chat(&server, edit_turn(&base_design_id, Some(focus))).await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    let document = response.json();
    assert_eq!(document["usage"]["input_tokens"], 24);
    assert_eq!(document["usage"]["output_tokens"], 10);

    let prompts = transport.prompts();
    assert_eq!(prompts.len(), 2, "one call and one repair");
    for prompt in &prompts {
        assert!(prompt.contains("CURRENT DESIGN:"), "{prompt}");
        assert!(prompt.contains(&stored), "{prompt}");
        assert!(
            prompt.contains(
                "Return the COMPLETE revised DesignIntent and keep everything the request does not change."
            ),
            "{prompt}"
        );
        assert!(
            prompt.contains("SELECTED FEATURE:\nthe 6 mm hole"),
            "{prompt}"
        );
    }
    // The repair carries its own previous JSON and the exact error as well.
    assert!(prompts[1].contains("The JSON block you sent:"));
    assert!(prompts[1].contains("script: Field required"));

    // Both calls are one turn: the journal carries their sum.
    let payloads = store.journal_payloads();
    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0]["usage"]["input_tokens"], 24);
    assert_eq!(payloads[0]["usage"]["output_tokens"], 10);
    assert_eq!(payloads[0]["base_design_id"], base_design_id);

    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_build_repair_carries_the_base_section_and_its_usage() {
    let (server, store, transport, _work) = start().await;
    let base_design_id = draft_design(&server, &transport).await;
    transport.requests.lock().unwrap().clear();
    store.journal.lock().unwrap().clear();

    let reject_script =
        r#"{"components":[{"name":"plate","steps":[]}],"summary":"REJECT this plate"}"#;
    transport.push_reply(
        &format!("```json\n{{\"script\": {reject_script}}}\n```"),
        Some((7, 3)),
    );
    transport.push_reply(
        &format!("```json\n{{\"script\": {REVISED_SCRIPT}}}\n```"),
        Some((9, 5)),
    );
    let response = chat(&server, edit_turn(&base_design_id, Some("the 6 mm hole"))).await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    let document = response.json();
    assert_eq!(
        document["design"]["verified"], true,
        "the repaired design is the turn's design"
    );
    assert_eq!(document["usage"]["input_tokens"], 16);
    assert_eq!(document["usage"]["output_tokens"], 8);

    let prompts = transport.prompts();
    assert_eq!(prompts.len(), 2, "one attempt and one build repair");
    assert!(
        prompts[1].contains("the build was rejected"),
        "{}",
        prompts[1]
    );
    assert!(prompts[1].contains("cuts_effective"), "{}", prompts[1]);
    assert!(prompts[1].contains("CURRENT DESIGN:"), "{}", prompts[1]);
    assert!(prompts[1].contains("SELECTED FEATURE:\nthe 6 mm hole"));
    assert!(prompts[1].ends_with("User request:\nmake this hole 10 mm"));

    let payloads = store.journal_payloads();
    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0]["usage"]["input_tokens"], 16);
    assert_eq!(payloads[0]["usage"]["output_tokens"], 8);
    server.stop().await;
}

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_malformed_usage_block_never_fails_a_turn_and_is_never_a_number() {
    let (server, _store, transport, _work) = start().await;

    for usage in [
        serde_json::json!({"prompt_tokens": "lots", "completion_tokens": -3}),
        serde_json::json!({"prompt_tokens": 12.5, "completion_tokens": null}),
        serde_json::json!([1, 2]),
        Value::Null,
    ] {
        let body = serde_json::json!({
            "model": "mock-served-model",
            "choices": [{"message": {"content": "hi"}}],
            "usage": usage,
        });
        transport.push_raw(serde_json::to_vec(&body).unwrap());
        let response = chat(
            &server,
            serde_json::json!({
                "provider": "openrouter",
                "model": "mock/model",
                "prompt": "hello",
                "allow_remote": true,
            }),
        )
        .await;
        assert_eq!(response.status, 200, "{}", response.body_text());
        assert_eq!(
            response.json()["usage"],
            Value::Null,
            "a malformed usage is not reported, never estimated"
        );
    }

    // A partially reported block keeps the one number it did report.
    transport.push_raw(
        serde_json::to_vec(&serde_json::json!({
            "model": "mock-served-model",
            "choices": [{"message": {"content": "hi"}}],
            "usage": {"prompt_tokens": 17},
        }))
        .unwrap(),
    );
    let response = chat(
        &server,
        serde_json::json!({
            "provider": "openrouter",
            "model": "mock/model",
            "prompt": "hello",
            "allow_remote": true,
        }),
    )
    .await;
    assert_eq!(response.status, 200);
    assert_eq!(
        response.json()["usage"],
        serde_json::json!({"input_tokens": 17, "output_tokens": null})
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_local_helper_reports_null_usage_and_adds_no_usage_to_the_journal() {
    let (server, store, _transport, _work) = start().await;
    let response = chat(
        &server,
        serde_json::json!({"provider": "local", "prompt": "a box 10x20x30 mm"}),
    )
    .await;
    assert_eq!(response.status, 200);
    assert_eq!(response.json()["usage"], Value::Null);
    let payloads = store.journal_payloads();
    assert_eq!(payloads.len(), 1);
    assert!(
        payloads[0].get("usage").is_none(),
        "an unreported usage adds no journal key: {}",
        payloads[0]
    );
    assert!(payloads[0].get("base_design_id").is_none());
    server.stop().await;
}

// ---------------------------------------------------------------------------
// Request shape
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_focus_is_bounded_trimmed_and_free_of_control_characters() {
    let (server, store, transport, _work) = start().await;
    let base_design_id = draft_design(&server, &transport).await;
    store.journal.lock().unwrap().clear();

    let too_long = "x".repeat(1_025);
    let response = chat(&server, edit_turn(&base_design_id, Some(&too_long))).await;
    assert_eq!(response.status, 422);
    assert_eq!(
        response.json()["details"][0],
        "body.focus: string_too_long: String should have at most 1024 characters"
    );

    let response = chat(&server, edit_turn(&base_design_id, Some("a\u{1}b"))).await;
    assert_eq!(response.status, 422);
    assert_eq!(
        response.json()["details"][0],
        "body.focus: value_error: Value error, the focus must not contain control characters other than newline"
    );

    // A newline is allowed, and the text is trimmed.
    transport.push_reply(
        &format!("```json\n{{\"script\": {REVISED_SCRIPT}}}\n```"),
        None,
    );
    let response = chat(
        &server,
        edit_turn(&base_design_id, Some("  the top hole\nand its wall  ")),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    let prompt = transport.prompts().last().cloned().unwrap_or_default();
    assert!(
        prompt.contains("SELECTED FEATURE:\nthe top hole\nand its wall\n\nUser request:"),
        "{prompt}"
    );

    // A whitespace-only focus is absent, not an error.
    let response = chat(
        &server,
        serde_json::json!({"provider": "local", "prompt": "a box 10x20x30 mm", "focus": "   "}),
    )
    .await;
    assert_eq!(
        response.status,
        200,
        "a blank focus is not a selected feature: {}",
        response.body_text()
    );

    let response = chat(
        &server,
        serde_json::json!({
            "provider": "openrouter",
            "model": "mock/model",
            "prompt": "hello",
            "allow_remote": true,
            "base_design_id": 12,
        }),
    )
    .await;
    assert_eq!(response.status, 422);
    assert!(
        response.json()["details"][0]
            .as_str()
            .unwrap_or_default()
            .starts_with("body.base_design_id: string_type")
    );

    server.stop().await;
}
