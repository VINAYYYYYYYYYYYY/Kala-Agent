//! Model-authored designs through a mock OpenRouter-compatible provider and
//! the real native CAD providers.
//!
//! Every provider reply is scripted, so no credential and no network are
//! used. The design itself is compiled by the real OpenCascade adapter and,
//! on a host with `FreeCADCmd`, by the real FreeCAD subprocess. The test
//! proves the plate-with-holes prompt that used to come back as prose now
//! produces a ConstructionScript and a measured part.

#![cfg(unix)]

mod common;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use kala_studio::geometry_backend::GeometryBackend;
use kala_studio::native_geometry::NativeGeometryBackend;
use kala_studio::providers::remote::{
    HttpTransport, OPENROUTER_DEFAULT_ENDPOINT, RemoteRequest, RemoteResponse, TransportError,
};

use common::{RequestSpec, TOKEN, TestServer};

const PLATE_PROMPT: &str =
    "aluminium mounting plate 80 x 40 x 8 mm with four 6 mm through holes 8 mm from the edges";
const PLATE_VOLUME_MM3: f64 = 24_695.221_316_345;
const PLATE_AREA_MM2: f64 = 8_696.991_118;

fn plate_script() -> &'static str {
    r#"{"script":{"summary":"80 x 40 x 8 mm plate with four 6 mm through holes","components":[{"name":"plate","steps":[{"op":"sketch","plane":"XY","profile":"rectangle","width":80,"height":40},{"op":"extrude","length":8},{"op":"sketch","plane":"XY","profile":"circle","diameter":6,"x":8,"y":8},{"op":"pocket"},{"op":"sketch","plane":"XY","profile":"circle","diameter":6,"x":72,"y":8},{"op":"pocket"},{"op":"sketch","plane":"XY","profile":"circle","diameter":6,"x":72,"y":32},{"op":"pocket"},{"op":"sketch","plane":"XY","profile":"circle","diameter":6,"x":8,"y":32},{"op":"pocket"}]}]}}"#
}

fn provider_body(content: &str) -> Vec<u8> {
    serde_json::json!({
        "model": "mock-served-model",
        "choices": [{"message": {"content": content}}],
    })
    .to_string()
    .into_bytes()
}

#[derive(Default)]
struct ScriptedTransport {
    requests: Mutex<Vec<RemoteRequest>>,
    replies: Mutex<VecDeque<ScriptedReply>>,
}

enum ScriptedReply {
    Text(String),
    Timeout,
}

impl ScriptedTransport {
    fn push(&self, reply: impl Into<String>) {
        self.replies
            .lock()
            .unwrap()
            .push_back(ScriptedReply::Text(reply.into()));
    }

    fn push_timeout(&self) {
        self.replies
            .lock()
            .unwrap()
            .push_back(ScriptedReply::Timeout);
    }

    fn requests(&self) -> Vec<RemoteRequest> {
        self.requests.lock().unwrap().clone()
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
            .unwrap_or_else(|| ScriptedReply::Text("no scripted reply".to_owned()));
        Box::pin(async move {
            let ScriptedReply::Text(reply) = reply else {
                return Err(TransportError::Timeout);
            };
            let body = provider_body(&reply);
            Ok(RemoteResponse {
                status: 200,
                reason: Some("OK".to_owned()),
                content_length: Some(body.len() as u64),
                body,
            })
        })
    }
}

fn provider_environment() -> HashMap<String, String> {
    let mut environ = HashMap::new();
    environ.insert(
        "OPENROUTER_API_KEY".to_owned(),
        "sk-or-corpus-synthetic-not-a-live-credential-0003".to_owned(),
    );
    environ
}

fn native_backend(work: &Path) -> Arc<dyn GeometryBackend> {
    let home = work.join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(work.join("geometry-work")).unwrap();
    let mut environ = BTreeMap::new();
    environ.insert(
        "PATH".to_owned(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".to_owned()),
    );
    environ.insert("HOME".to_owned(), home.display().to_string());
    NativeGeometryBackend::new(
        work.join("geometry-data"),
        environ,
        64 * 1024 * 1024,
        kala_studio::geometry::default_step_exporter(),
        work.join("geometry-work"),
    )
    .expect("the native geometry backend opens in a temp directory")
}

async fn freecad_available(backend: &Arc<dyn GeometryBackend>) -> bool {
    let Ok(response) = backend.providers_cad().await else {
        return false;
    };
    let Some(document) = response.json() else {
        return false;
    };
    document["providers"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|row| row["id"] == "freecad" && row["available"] == true)
}

async fn start(
    work: &Path,
    backend: Arc<dyn GeometryBackend>,
    transport: Arc<ScriptedTransport>,
) -> TestServer {
    TestServer::start_with_backend(
        work.join("studio-data"),
        TOKEN,
        64 * 1024,
        None,
        provider_environment(),
        kala_studio::geometry::default_step_exporter(),
        Some(transport),
        backend,
    )
    .await
}

fn chat_spec(body: serde_json::Value) -> RequestSpec {
    let mut spec = RequestSpec::default();
    spec.headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    spec.headers
        .push(("content-type".to_owned(), "application/json".to_owned()));
    spec.body = Some(serde_json::to_vec(&body).unwrap());
    spec
}

async fn chat(server: &TestServer, cad_provider: &str) -> common::TestResponse {
    chat_with_prompt(server, cad_provider, PLATE_PROMPT).await
}

async fn chat_with_prompt(
    server: &TestServer,
    cad_provider: &str,
    prompt: &str,
) -> common::TestResponse {
    common::request(
        server.addr,
        "POST",
        "/api/studio/chat",
        chat_spec(serde_json::json!({
            "provider": "openrouter",
            "model": "minimax/minimax-m3.1-flash:free",
            "prompt": prompt,
            "allow_remote": true,
            "cad_provider": cad_provider,
        })),
    )
    .await
}

fn stepped_post_script() -> &'static str {
    r#"{"script":{"summary":"stepped 30/20 mm round post, 40 and 25 mm long","components":[{"name":"post","steps":[{"op":"sketch","plane":"XY","profile":"rectangle","width":15,"height":40},{"op":"revolve","angle":360},{"op":"sketch","plane":"XY","profile":"rectangle","width":10,"height":25,"y":40},{"op":"revolve","angle":360}]}]}}"#
}

fn placed_assembly_script() -> &'static str {
    r#"{"script":{"summary":"40 x 30 x 5 mm base with a 12 mm boss placed on top","components":[{"name":"base","steps":[{"op":"sketch","plane":"XY","profile":"rectangle","width":40,"height":30},{"op":"extrude","length":5}]},{"name":"boss","steps":[{"op":"sketch","plane":"XY","profile":"circle","diameter":12},{"op":"extrude","length":10}]}],"assembly":[{"op":"place","component":"base"},{"op":"place","component":"boss","x":20,"y":15,"z":5}]}}"#
}

fn assert_plate_measurement(design: &serde_json::Value, provider: &str) {
    assert_eq!(design["provider"], provider);
    assert_eq!(design["verified"], true);
    assert_eq!(design["measured"]["solid_count"], 1);
    let volume = design["measured"]["volume_mm3"]
        .as_f64()
        .unwrap_or_default();
    assert!(
        (volume - PLATE_VOLUME_MM3).abs() < 1e-3,
        "{provider} volume {volume} does not match the 80 x 40 x 8 plate with four 6 mm holes"
    );
    let area = design["measured"]["surface_area_mm2"]
        .as_f64()
        .unwrap_or_default();
    assert!(
        (area - PLATE_AREA_MM2).abs() < 1e-2,
        "{provider} surface area {area} does not match the plate"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_model_authored_plate_builds_on_opencascade_and_freecad() {
    let work = tempfile::tempdir().unwrap();
    let backend = native_backend(work.path());
    let freecad = freecad_available(&backend).await;
    let transport = Arc::new(ScriptedTransport::default());
    let server = start(work.path(), Arc::clone(&backend), Arc::clone(&transport)).await;

    transport.push(plate_script());
    let response = chat(&server, "opencascade").await;
    assert_eq!(response.status, 200);
    let document = response.json();
    assert_plate_measurement(&document["design"], "opencascade");
    let text = document["text"].as_str().unwrap_or_default();
    assert!(text.contains("Measured volume 24695.221"), "{text}");
    assert!(text.contains("B-Rep verified"), "{text}");
    assert!(!text.contains("No part was built"), "{text}");

    if freecad {
        transport.push(plate_script());
        let response = chat(&server, "freecad").await;
        assert_eq!(response.status, 200);
        let document = response.json();
        assert_plate_measurement(&document["design"], "freecad");
        assert_eq!(document["design"]["measured_by"], "FreeCAD");
        let text = document["text"].as_str().unwrap_or_default();
        assert!(text.contains("Measured volume 24695.221"), "{text}");
        assert!(text.contains("measured by FreeCAD"), "{text}");
    } else {
        eprintln!(
            "FreeCADCmd is not available on this host: the real FreeCAD half of the \
             model-authored plate test is skipped loudly"
        );
    }

    let requests = transport.requests();
    assert_eq!(requests.len(), if freecad { 2 } else { 1 });
    for request in &requests {
        assert_eq!(request.url, OPENROUTER_DEFAULT_ENDPOINT);
        let body = String::from_utf8(request.body.clone()).unwrap();
        assert!(
            body.contains("When no template matches, author a ConstructionScript"),
            "{body}"
        );
        assert!(
            body.contains("a rectangle's x/y is its lower-left corner"),
            "{body}"
        );
        assert!(
            body.contains("Every pocket must cut into material that its own component already has"),
            "{body}"
        );
        assert!(
            body.contains("A design may use up to 500 steps in total across up to 64 components"),
            "{body}"
        );
        assert!(body.contains("DesignIntent JSON Schema"), "{body}");
        assert!(body.contains("80 x 40 x 8 mm"), "{body}");
    }
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_invalid_reply_is_repaired_with_the_failing_field_schema() {
    let work = tempfile::tempdir().unwrap();
    let backend = native_backend(work.path());
    let transport = Arc::new(ScriptedTransport::default());
    transport.push(
        r#"{"template":{"template":"l_bracket","width":-3,"depth":45,"height":40,"wall":8}}"#,
    );
    transport.push(plate_script());
    let server = start(work.path(), Arc::clone(&backend), Arc::clone(&transport)).await;

    let response = chat(&server, "opencascade").await;
    assert_eq!(response.status, 200);
    let document = response.json();
    assert_plate_measurement(&document["design"], "opencascade");

    let requests = transport.requests();
    assert_eq!(requests.len(), 2, "exactly one repair round-trip");
    let repair = String::from_utf8(requests[1].body.clone()).unwrap();
    assert!(repair.contains("could not be validated"), "{repair}");
    assert!(repair.contains("template.l_bracket.width"), "{repair}");
    assert!(
        repair.contains("Schema for template.l_bracket.width"),
        "{repair}"
    );
    assert!(repair.contains("exclusiveMinimum"), "{repair}");
    assert!(
        repair.contains("Keep every value the user stated"),
        "{repair}"
    );
    // Every model call is stateless: the repair call must see the same
    // instruction, the same full schema and its OWN previous JSON, or it
    // re-authors the design blind.
    assert!(
        repair.contains("When no template matches, author a ConstructionScript"),
        "{repair}"
    );
    assert!(repair.contains("Worked examples"), "{repair}");
    assert!(repair.contains("DesignIntent JSON Schema:"), "{repair}");
    assert!(
        repair.contains("The JSON block you sent:"),
        "the repair prompt carries the JSON the model itself sent: {repair}"
    );
    assert!(
        repair.contains(r#"{\"template\":{\"template\":\"l_bracket\",\"width\":-3"#),
        "the repair prompt carries the model's own previous JSON: {repair}"
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invalid_twice_says_no_part_was_built() {
    let work = tempfile::tempdir().unwrap();
    let backend = native_backend(work.path());
    let transport = Arc::new(ScriptedTransport::default());
    transport.push(r#"{"template": 3}"#);
    transport.push(r#"{"script": {"components": []}}"#);
    let server = start(work.path(), Arc::clone(&backend), Arc::clone(&transport)).await;

    let response = chat(&server, "opencascade").await;
    assert_eq!(response.status, 200);
    let document = response.json();
    assert!(
        document["design"].is_null(),
        "an invalid output must not produce a design: {document}"
    );
    let text = document["text"].as_str().unwrap_or_default();
    assert!(text.contains("No part was built"), "{text}");
    assert!(text.contains("after one repair attempt"), "{text}");
    // BOTH errors are named: the user sees what went wrong the first time
    // too, not only what went wrong the second time.
    assert!(
        text.contains(
            "the first reply was not a valid design (template: Input should be a valid \
             dictionary or object to extract fields from)"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "and the reply after one repair attempt was not a valid design either \
             (script.summary: Field required;"
        ),
        "{text}"
    );
    assert!(!text.contains("Measured volume"), "{text}");
    assert_eq!(transport.requests().len(), 2);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_repair_that_asks_for_values_is_not_an_error() {
    let work = tempfile::tempdir().unwrap();
    let backend = native_backend(work.path());
    let transport = Arc::new(ScriptedTransport::default());
    transport.push(r#"{"template": 3}"#);
    transport.push("What is the wall thickness?");
    let server = start(work.path(), Arc::clone(&backend), Arc::clone(&transport)).await;

    let response = chat(&server, "opencascade").await;
    assert_eq!(response.status, 200);
    let document = response.json();
    assert!(document["design"].is_null(), "{document}");
    let text = document["text"].as_str().unwrap_or_default();
    assert!(text.contains("What is the wall thickness?"), "{text}");
    assert!(text.contains("No part was built"), "{text}");
    assert!(text.contains("asks for values"), "{text}");
    assert!(!text.contains("not a valid design"), "{text}");
    assert_eq!(transport.requests().len(), 2);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_instruction_revolve_and_assembly_examples_build() {
    let work = tempfile::tempdir().unwrap();
    let backend = native_backend(work.path());
    let transport = Arc::new(ScriptedTransport::default());
    transport.push(stepped_post_script());
    transport.push(placed_assembly_script());
    let server = start(work.path(), Arc::clone(&backend), Arc::clone(&transport)).await;

    let response = chat_with_prompt(&server, "opencascade", "a stepped round post").await;
    assert_eq!(response.status, 200);
    let document = response.json();
    let design = &document["design"];
    assert_eq!(design["provider"], "opencascade");
    assert_eq!(design["verified"], true);
    assert_eq!(design["measured"]["solid_count"], 1);
    let volume = design["measured"]["volume_mm3"]
        .as_f64()
        .unwrap_or_default();
    let expected = 11_500.0 * std::f64::consts::PI;
    assert!(
        (volume - expected).abs() < 1e-3,
        "the 30/20 stepped post volume {volume} is not {expected}"
    );

    let response = chat_with_prompt(&server, "opencascade", "a base with a boss on top").await;
    assert_eq!(response.status, 200);
    let document = response.json();
    let design = &document["design"];
    assert_eq!(design["provider"], "opencascade");
    assert_eq!(design["verified"], true);
    assert_eq!(
        design["measured"]["solid_count"], 2,
        "the placed assembly has two occurrences"
    );
    let volume = design["measured"]["volume_mm3"]
        .as_f64()
        .unwrap_or_default();
    let expected = 6_000.0 + 360.0 * std::f64::consts::PI;
    assert!(
        (volume - expected).abs() < 1e-3,
        "the placed base and boss volume {volume} is not {expected}"
    );
    server.stop().await;
}

async fn plain_chat(server: &TestServer, prompt: &str) -> common::TestResponse {
    common::request(
        server.addr,
        "POST",
        "/api/studio/chat",
        chat_spec(serde_json::json!({
            "provider": "openrouter",
            "model": "minimax/minimax-m3.1-flash:free",
            "prompt": prompt,
            "allow_remote": true,
        })),
    )
    .await
}

fn journal_spec() -> RequestSpec {
    let mut spec = RequestSpec::default();
    spec.headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    spec
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn design_turns_and_their_repair_carry_the_180_second_budget() {
    let work = tempfile::tempdir().unwrap();
    let backend = native_backend(work.path());
    let transport = Arc::new(ScriptedTransport::default());
    transport.push(r#"{"template": 3}"#);
    transport.push(plate_script());
    let server = start(work.path(), Arc::clone(&backend), Arc::clone(&transport)).await;

    let response = chat(&server, "opencascade").await;
    assert_eq!(response.status, 200);
    assert_plate_measurement(&response.json()["design"], "opencascade");

    let requests = transport.requests();
    assert_eq!(requests.len(), 2, "the first call and the repair call");
    for request in &requests {
        assert_eq!(
            request.timeout,
            std::time::Duration::from_secs(180),
            "every remote design call must carry the 180 s design budget"
        );
    }

    // A plain chat keeps the 30 s budget exactly.
    transport.push("plain reply");
    let response = plain_chat(&server, "What is a STEP file?").await;
    assert_eq!(response.status, 200);
    let requests = transport.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[2].timeout,
        std::time::Duration::from_secs(30),
        "a plain chat must keep the 30 s budget"
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_timed_out_design_turn_is_journalled_once_with_its_code() {
    let work = tempfile::tempdir().unwrap();
    let backend = native_backend(work.path());
    let transport = Arc::new(ScriptedTransport::default());
    transport.push_timeout();
    let server = start(work.path(), Arc::clone(&backend), Arc::clone(&transport)).await;

    let response = chat(&server, "opencascade").await;
    assert_eq!(response.status, 504);
    assert_eq!(
        response.body_text(),
        "{\"code\":\"provider_timeout\",\"details\":[\"the provider request exceeded its \
         bounded timeout\"],\"message\":\"the chat provider exceeded its bounded timeout\"}"
    );

    let page = common::request(
        server.addr,
        "GET",
        "/api/studio/journal?after=0&limit=50",
        journal_spec(),
    )
    .await;
    assert_eq!(page.status, 200);
    let document = page.json();
    let events = document["events"].as_array().expect("journal events");
    let turns: Vec<&serde_json::Value> = events
        .iter()
        .filter(|event| event["event_type"] == "chat.turn")
        .collect();
    assert_eq!(
        turns.len(),
        1,
        "a failed design turn is journalled exactly once: {events:?}"
    );
    let payload = &turns[0]["payload"];
    assert_eq!(payload["provider"], "openrouter");
    assert_eq!(payload["offline"], false);
    assert_eq!(payload["design_requested"], true);
    assert_eq!(payload["design_id"], serde_json::Value::Null);
    assert_eq!(payload["cad_provider"], "opencascade");
    assert_eq!(payload["missing"], serde_json::json!([]));
    assert_eq!(payload["failure"], "provider_timeout");
    assert_eq!(
        payload["prompt_chars"],
        serde_json::json!(PLATE_PROMPT.chars().count())
    );
    assert_eq!(
        payload["prompt_sha256"].as_str().map(str::len),
        Some(64),
        "the payload carries the prompt digest, never the prompt"
    );
    assert!(
        payload.get("prompt").is_none(),
        "the prompt is never journalled"
    );
    assert_eq!(
        transport.requests().len(),
        1,
        "the turn stopped at the first timing-out call"
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_missing_dimension_question_is_not_repaired() {
    let work = tempfile::tempdir().unwrap();
    let backend = native_backend(work.path());
    let transport = Arc::new(ScriptedTransport::default());
    transport.push("What is the plate thickness and the hole diameter?");
    let server = start(work.path(), Arc::clone(&backend), Arc::clone(&transport)).await;

    let response = chat(&server, "opencascade").await;
    assert_eq!(response.status, 200);
    let document = response.json();
    assert!(document["design"].is_null(), "{document}");
    let text = document["text"].as_str().unwrap_or_default();
    assert!(text.contains("What is the plate thickness"), "{text}");
    assert!(text.contains("No part was built"), "{text}");
    assert!(!text.contains("after one repair attempt"), "{text}");
    assert_eq!(
        transport.requests().len(),
        1,
        "a question for missing values must not spend a repair turn"
    );
    server.stop().await;
}

/// A plate with one hole sketched entirely outside the plate: the build
/// succeeds, but the cut removed no material, so the design is not verified.
fn ineffective_cut_script() -> &'static str {
    r#"{"script":{"summary":"80 x 40 x 8 mm plate with one 6 mm hole outside the plate","components":[{"name":"plate","steps":[{"op":"sketch","plane":"XY","profile":"rectangle","width":80,"height":40},{"op":"extrude","length":8},{"op":"sketch","plane":"XY","profile":"circle","diameter":6,"x":200,"y":200},{"op":"pocket"}]}]}}"#
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unverified_build_is_repaired_with_the_build_feedback() {
    let work = tempfile::tempdir().unwrap();
    let backend = native_backend(work.path());
    let transport = Arc::new(ScriptedTransport::default());
    transport.push(ineffective_cut_script());
    transport.push(plate_script());
    let server = start(work.path(), Arc::clone(&backend), Arc::clone(&transport)).await;

    let response = chat(&server, "opencascade").await;
    assert_eq!(response.status, 200);
    let document = response.json();
    // The second attempt is the turn's design, and it verifies.
    assert_plate_measurement(&document["design"], "opencascade");

    let text = document["text"].as_str().unwrap_or_default();
    assert!(
        text.contains("The first attempt was rejected"),
        "the text says the first attempt was rejected: {text}"
    );
    assert!(text.contains("cuts_effective"), "{text}");
    assert!(text.contains("removed no material"), "{text}");
    assert!(text.contains("B-Rep verified"), "{text}");

    let requests = transport.requests();
    assert_eq!(requests.len(), 2, "the first call and the one build repair");
    let repair = String::from_utf8(requests[1].body.clone()).unwrap();
    assert!(repair.contains("the build was rejected"), "{repair}");
    assert!(repair.contains("cuts_effective"), "{repair}");
    assert!(
        repair.contains("component 'plate': pocket at step index 3 on XY removed no material"),
        "{repair}"
    );
    assert!(repair.contains(PLATE_PROMPT), "{repair}");
    // Every model call is stateless: the repair call must still see the
    // supported subset, the plane rule and the worked examples.
    assert!(
        repair.contains("When no template matches, author a ConstructionScript"),
        "{repair}"
    );
    assert!(
        repair.contains("Never invent a dimension the user did not state"),
        "{repair}"
    );
    assert!(repair.contains("Worked examples"), "{repair}");
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_spent_validation_repair_buys_no_build_repair() {
    let work = tempfile::tempdir().unwrap();
    let backend = native_backend(work.path());
    let transport = Arc::new(ScriptedTransport::default());
    // The first reply is not a valid DesignIntent, so the validation repair
    // is spent. The repair builds, but unverified.
    transport.push(r#"{"template": 3}"#);
    transport.push(ineffective_cut_script());
    let server = start(work.path(), Arc::clone(&backend), Arc::clone(&transport)).await;

    let response = chat(&server, "opencascade").await;
    assert_eq!(response.status, 200);
    let document = response.json();
    assert_eq!(document["design"]["verified"], false, "{document}");
    let text = document["text"].as_str().unwrap_or_default();
    assert!(text.contains("B-Rep NOT verified"), "{text}");
    assert!(
        !text.contains("The first attempt was rejected"),
        "a turn that spent its repair on the validation never repairs the build: {text}"
    );
    assert_eq!(
        transport.requests().len(),
        2,
        "the whole turn never spends more than one repair call"
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unavailable_cad_provider_buys_no_build_repair() {
    let work = tempfile::tempdir().unwrap();
    let backend = native_backend(work.path());
    let transport = Arc::new(ScriptedTransport::default());
    transport.push(ineffective_cut_script());
    let server = start(work.path(), Arc::clone(&backend), Arc::clone(&transport)).await;

    // Solid Edge cannot build on Linux: the refusal is not the model's fault,
    // so the turn never spends a repair call on it.
    let response = chat(&server, "solidedge").await;
    assert_eq!(response.status, 200);
    let document = response.json();
    assert!(document["design"].is_null(), "{document}");
    let text = document["text"].as_str().unwrap_or_default();
    assert!(text.contains("not available"), "{text}");
    assert_eq!(
        transport.requests().len(),
        1,
        "a provider that cannot build here never buys a repair call"
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_repair_that_is_still_unverified_never_claims_verified() {
    let work = tempfile::tempdir().unwrap();
    let backend = native_backend(work.path());
    let transport = Arc::new(ScriptedTransport::default());
    transport.push(ineffective_cut_script());
    transport.push(ineffective_cut_script());
    let server = start(work.path(), Arc::clone(&backend), Arc::clone(&transport)).await;

    let response = chat(&server, "opencascade").await;
    assert_eq!(response.status, 200);
    let document = response.json();
    assert_eq!(
        document["design"]["verified"], false,
        "the returned design says it is not verified: {document}"
    );
    let text = document["text"].as_str().unwrap_or_default();
    assert!(text.contains("The first attempt was rejected"), "{text}");
    assert!(text.contains("B-Rep NOT verified"), "{text}");
    assert!(
        !text.contains("B-Rep verified"),
        "the text never claims a verification the design does not carry: {text}"
    );
    assert_eq!(transport.requests().len(), 2);
    server.stop().await;
}

/// A script the builder refuses: a helix is a valid DesignIntent step, but
/// the Studio OCP compiler does not support it.
fn refused_script() -> &'static str {
    r#"{"script":{"summary":"a shaft with a helix","components":[{"name":"shaft","steps":[{"op":"sketch","plane":"XY","profile":"circle","diameter":10},{"op":"extrude","length":20},{"op":"helix","diameter":10,"pitch":2,"length":10}]}]}}"#
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_repair_call_keeps_the_first_build() {
    let work = tempfile::tempdir().unwrap();
    let backend = native_backend(work.path());
    let transport = Arc::new(ScriptedTransport::default());
    // The first reply builds unverified, so the turn spends its one repair
    // call on the build. The repair call itself times out.
    transport.push(ineffective_cut_script());
    transport.push_timeout();
    let server = start(work.path(), Arc::clone(&backend), Arc::clone(&transport)).await;

    let response = chat(&server, "opencascade").await;
    assert_eq!(
        response.status,
        200,
        "a failed repair call is not a turn failure: {}",
        response.body_text()
    );
    let document = response.json();
    let design = &document["design"];
    assert!(
        !design.is_null(),
        "the first build was already made and must not be thrown away: {document}"
    );
    assert_eq!(
        design["verified"], false,
        "the first design says it is not verified: {document}"
    );
    let design_id = design["design_id"].as_str().unwrap_or_default().to_owned();

    let text = document["text"].as_str().unwrap_or_default();
    assert!(text.contains("The first attempt was rejected"), "{text}");
    assert!(text.contains("cuts_effective"), "{text}");
    assert!(text.contains("removed no material"), "{text}");
    assert!(
        text.contains("the repair attempt failed: provider_timeout"),
        "the turn says why the repair did not happen: {text}"
    );
    assert!(text.contains("B-Rep NOT verified"), "{text}");
    assert!(
        !text.contains("B-Rep verified"),
        "the text never claims a verification the design does not carry: {text}"
    );

    // The turn is journalled exactly once, and it points at the first design.
    let page = common::request(
        server.addr,
        "GET",
        "/api/studio/journal?after=0&limit=50",
        journal_spec(),
    )
    .await;
    assert_eq!(page.status, 200);
    let events = page.json();
    let turns: Vec<&serde_json::Value> = events["events"]
        .as_array()
        .expect("journal events")
        .iter()
        .filter(|event| event["event_type"] == "chat.turn")
        .collect();
    assert_eq!(
        turns.len(),
        1,
        "the turn is journalled exactly once: {events:?}"
    );
    assert_eq!(
        turns[0]["payload"]["design_id"],
        serde_json::json!(design_id)
    );
    assert_eq!(
        transport.requests().len(),
        2,
        "the turn stopped at the timing-out repair call"
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_repair_call_after_a_refusal_says_no_part_was_built() {
    let work = tempfile::tempdir().unwrap();
    let backend = native_backend(work.path());
    let transport = Arc::new(ScriptedTransport::default());
    // The first script is refused by the builder, so the turn spends its one
    // repair call on the build. The repair call itself times out.
    transport.push(refused_script());
    transport.push_timeout();
    let server = start(work.path(), Arc::clone(&backend), Arc::clone(&transport)).await;

    let response = chat(&server, "opencascade").await;
    assert_eq!(
        response.status,
        200,
        "a failed repair call is not a turn failure: {}",
        response.body_text()
    );
    let document = response.json();
    assert!(
        document["design"].is_null(),
        "a refused script never built a part: {document}"
    );
    let text = document["text"].as_str().unwrap_or_default();
    assert!(text.contains("The first attempt was rejected"), "{text}");
    assert!(text.contains("the builder refused the script"), "{text}");
    assert!(
        text.contains("the repair attempt failed: provider_timeout"),
        "the turn says why the repair did not happen: {text}"
    );
    assert!(text.contains("No part was built"), "{text}");
    assert_eq!(transport.requests().len(), 2);
    server.stop().await;
}

/// The 200 x 200 x 5 mm plate with a 10 x 10 grid of Ø3 through holes.
///
/// The design JSON is longer than the 4 096-character display bound, which is
/// exactly the reply the old provider truncation cut mid-object.
fn hundred_hole_plate_script() -> String {
    let mut steps = String::from(
        r#"{"op":"sketch","plane":"XY","profile":"rectangle","width":200,"height":200},{"op":"extrude","length":5}"#,
    );
    for row in 0..10 {
        for column in 0..10 {
            let x = 20 + 16 * column;
            let y = 20 + 16 * row;
            steps.push_str(&format!(
                r#",{{"op":"sketch","plane":"XY","profile":"circle","diameter":3,"x":{x},"y":{y}}},{{"op":"pocket"}}"#
            ));
        }
    }
    format!(
        r#"{{"script":{{"summary":"200 x 200 x 5 mm plate with a 10 x 10 grid of 3 mm through holes","components":[{{"name":"plate","steps":[{steps}]}}]}}}}"#
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_design_longer_than_the_display_bound_still_builds() {
    let work = tempfile::tempdir().unwrap();
    let backend = native_backend(work.path());
    let transport = Arc::new(ScriptedTransport::default());
    let server = start(work.path(), Arc::clone(&backend), Arc::clone(&transport)).await;

    let design = hundred_hole_plate_script();
    assert!(design.len() > 6_000, "{}", design.len());
    transport.push(format!(
        "Here is the plate. It has a 10 x 10 grid of through holes.\n\n```json\n{design}\n```"
    ));

    let response = chat(&server, "opencascade").await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    let document = response.json();
    let design_row = &document["design"];
    assert_eq!(design_row["verified"], true, "{design_row}");
    let volume = design_row["measured"]["volume_mm3"]
        .as_f64()
        .unwrap_or_default();
    let expected = 200.0 * 200.0 * 5.0 - 100.0 * std::f64::consts::PI * 1.5 * 1.5 * 5.0;
    assert!(
        (volume - expected).abs() < 1e-3,
        "the 100-hole plate volume is {volume}, expected {expected}"
    );
    let text = document["text"].as_str().unwrap_or_default();
    assert!(
        text.chars().count() <= 4_096,
        "the displayed text is {} characters",
        text.chars().count()
    );
    assert!(text.contains("Here is the plate"), "{text}");
    assert!(!text.contains("```json"), "{text}");
    assert_eq!(transport.requests().len(), 1, "the reply needs no repair");
    server.stop().await;
}
