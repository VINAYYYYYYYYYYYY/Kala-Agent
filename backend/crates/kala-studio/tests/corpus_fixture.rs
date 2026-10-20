//! Replay the frozen 56-case Studio HTTP corpus against the native service.
//!
//! Every deterministic native-supported case is compared byte-for-byte for
//! the body and value-for-value for stable/security headers. The two STEP
//! downloads are asserted against the fixture's `body_structural` contract
//! instead of the informational OCP byte digest: the native analytic writer
//! emits a real ISO 10303-21 solid whose bytes are deterministic while
//! OpenCascade's writer embeds a timestamp and its own entity numbering, so
//! byte equality is explicitly not the fixture contract for those two cases.
//!
//! The eight cases whose route the front door proxies to the supervised
//! geometry worker (ADR-0010) are replayed as well. This build attaches no
//! worker, so they answer the truthful refusal the offline build produces;
//! their contract against the live worker is the live worker differential's
//! (`worker_differential.py`) and the corpus replay harness's
//! (`replay_binary_corpus.py`), which run against the real worker.
//!
//! The manifest is part of the contract: the test asserts the regenerated
//! corpus digest, the number of cases and routes, and that the corpus files
//! on disk are the ones the manifest describes. Nothing is skipped or
//! rewritten.
//!
//! The replay needs the parked native primitive geometry, so it runs in the
//! `native-primitives` build. The default build has no native geometry at
//! all; the live Python differential covers the same corpus through the
//! supervised geometry worker.

#![cfg(feature = "native-primitives")]

mod common;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

use common::{RequestSpec, TOKEN, TestServer, empty_environ};

/// The digest of the regenerated corpus.
///
/// The corpus is regenerated whenever a product source changes, so this
/// constant moves with the corpus and a stale corpus fails the test instead
/// of passing silently.
const CORPUS_SHA256: &str = "53dcb9da105589b5a92d37b16d63c7fe2d5e60b7630549102127e019c95dff28";

/// The number of Studio cases in the regenerated corpus.
const CORPUS_CASES: usize = 56;

/// The number of Studio routes the manifest declares.
const STUDIO_ROUTES: usize = 15;

/// The cases whose health document the Rust front door extends.
const HEALTH_BODY_CASES: &[&str] = &[
    "studio.health",
    "studio.health.online",
    "studio.health.offline_again",
    "studio.valid.same_origin",
];

/// The two STEP downloads carry `body_contract: "structural"`: the fixture
/// itself says byte equality is not the contract there.
const STEP_STRUCTURAL_CASES: &[&str] =
    &["studio.export.step_box", "studio.handoff.step_solidworks"];

/// Health cases whose single `step_available` field depends on the seam. In
/// the default build the field is `true` and the body is byte-exact; in the
/// `native-step`-disabled build the same documented difference from the
/// Python/OCP fixture is asserted truthfully.
const HEALTH_CASES: &[&str] = &[
    "studio.health",
    "studio.health.online",
    "studio.health.offline_again",
    "studio.valid.same_origin",
];

/// Whether the front door proxies this route to the geometry worker.
///
/// The answer is the internal worker route the front door names in its
/// refusal, so the test asserts the route the front door chose.
fn worker_route(path: &str) -> Option<&'static str> {
    let route = path.split('?').next().unwrap_or(path);
    if route == "/api/studio/design" {
        return Some("design");
    }
    if route == "/api/studio/journal" {
        return Some("journal");
    }
    if route == "/api/studio/journal/verify" {
        return Some("journal/verify");
    }
    if route == "/api/studio/providers/cad" {
        return Some("providers/cad");
    }
    if let Some(rest) = route.strip_prefix("/api/studio/design/") {
        if let Some(design_id) = rest.strip_suffix("/drawing") {
            if !design_id.is_empty() && !design_id.contains('/') {
                return Some("drawing");
            }
        }
        if let Some(design_id) = rest.strip_suffix("/section") {
            if !design_id.is_empty() && !design_id.contains('/') {
                return Some("section");
            }
        }
    }
    None
}

/// The corpus and the manifest on disk.
fn corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus/v1")
}

/// The manifest's digest of the corpus files, recomputed from the files.
///
/// The rule is the generator's: the SQLite fixture's file bytes are not
/// asserted (the SQLite library version changes them), so its entry is
/// recorded as `not-asserted`; every other file contributes its own digest.
fn recomputed_corpus_digest(root: &Path, manifest: &Value) -> String {
    let files = manifest["files"].as_array().expect("manifest files");
    let mut entries: Vec<(String, String)> = files
        .iter()
        .map(|entry| {
            let path = entry["path"].as_str().expect("file path").to_owned();
            let digest = if path.ends_with(".sqlite3") {
                "not-asserted".to_owned()
            } else {
                hex(&Sha256::digest(
                    fs::read(root.join(&path)).expect("corpus file"),
                ))
            };
            (path, digest)
        })
        .collect();
    entries.sort();
    let listed: Vec<Vec<&str>> = entries
        .iter()
        .map(|(path, digest)| vec![path.as_str(), digest.as_str()])
        .collect();
    hex(&Sha256::digest(
        serde_json::to_vec(&listed).expect("digest list"),
    ))
}

#[test]
fn the_manifest_describes_the_checked_in_corpus() {
    let root = corpus_root();
    let manifest: Value =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).expect("manifest"))
            .expect("manifest JSON");
    let studio: Value =
        serde_json::from_slice(&fs::read(root.join("studio_http.json")).expect("studio corpus"))
            .expect("corpus JSON");

    // The corpus digest the regenerated corpus carries.
    assert_eq!(
        manifest["corpus_sha256"].as_str(),
        Some(CORPUS_SHA256),
        "the corpus digest moved; regenerate the corpus and this constant"
    );
    // The digest of the files on disk is the digest the manifest claims.
    assert_eq!(
        recomputed_corpus_digest(&root, &manifest),
        CORPUS_SHA256,
        "the corpus files on disk are not the ones the manifest describes"
    );

    let cases = studio["service"]["cases"].as_array().expect("cases");
    assert_eq!(
        cases.len(),
        CORPUS_CASES,
        "the regenerated corpus case count"
    );

    let declared = manifest["coverage"]["studio_http"]["declared"]
        .as_array()
        .expect("declared routes");
    assert_eq!(
        declared.len(),
        STUDIO_ROUTES,
        "the declared Studio route count"
    );
    let missing = manifest["coverage"]["studio_http"]["missing"]
        .as_array()
        .expect("missing routes");
    assert!(missing.is_empty(), "uncovered declared routes: {missing:?}");

    // Every case declares how its body may be compared.
    let mut contracts: HashSet<&str> = HashSet::new();
    for case in cases {
        let contract = case["expected"]["body_comparison"].as_str().unwrap_or("");
        assert!(
            ["exact", "fields", "structural"].contains(&contract),
            "{}: unknown body comparison contract {contract}",
            case["id"]
        );
        contracts.insert(contract);
    }
    assert_eq!(
        contracts.len(),
        3,
        "the corpus declares every body contract"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replay_all_56_studio_cases() {
    // The STEP exporter is linked by the `native-primitives` feature, which
    // the `native-step` alias turns on as well. The exporter is what decides
    // whether the two STEP downloads are real files or the truthful
    // `step_unavailable` refusal, so the feature that links it is the one the
    // expectation follows.
    let native_step = cfg!(feature = "native-primitives");
    let root = corpus_root();
    let fixture_path = root.join("studio_http.json");
    let fixture: Value = serde_json::from_slice(&fs::read(&fixture_path).expect("studio corpus"))
        .expect("corpus JSON");
    let cases = fixture["service"]["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), CORPUS_CASES);

    let work = tempfile::tempdir().unwrap();
    let static_dir = work.path().join("static");
    fs::create_dir(&static_dir).unwrap();
    fs::write(
        static_dir.join("index.html"),
        "<!doctype html><html><body>corpus</body></html>",
    )
    .unwrap();
    fs::write(static_dir.join("asset.txt"), "asset\n").unwrap();
    let data_dir = work.path().join("studio-data");
    let server = TestServer::start_with_backend(
        data_dir,
        TOKEN,
        16_384,
        Some(static_dir),
        empty_environ(),
        kala_studio::geometry::default_step_exporter(),
        None,
        std::sync::Arc::new(kala_studio::native_backend::NativePrimitivesBackend::new(
            kala_studio::geometry::default_step_exporter(),
            std::env::temp_dir().join("kala-studio-work"),
        )),
    )
    .await;

    let mut exact = 0usize;
    let mut structural = 0usize;
    let mut dependency = 0usize;
    let mut proxied = 0usize;
    let mut failures = Vec::new();
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let request = &case["request"];
        let method = request["method"].as_str().unwrap();
        let path = request["path"].as_str().unwrap();
        let mut spec = RequestSpec::default();
        if let Some(headers) = request["headers"].as_object() {
            for (name, value) in headers {
                spec.headers
                    .push((name.clone(), value.as_str().unwrap().to_owned()));
            }
        }
        match request["body"]["kind"].as_str() {
            Some("json") => {
                spec.body =
                    Some(serde_json::to_vec(&request["body"]["value"]).expect("request body"));
            }
            Some("raw_bytes") => {
                let length = request["body"]["length"].as_u64().unwrap() as usize;
                spec.body = Some(vec![b'x'; length]);
            }
            Some("chunked") => {
                let mut chunks = Vec::new();
                for chunk in request["body"]["chunks"].as_array().unwrap() {
                    let length = chunk["length"].as_u64().unwrap() as usize;
                    chunks.push(vec![b'x'; length]);
                }
                spec.chunked = Some(chunks);
            }
            Some(other) => panic!("unexpected body kind {other}"),
            None => {}
        }
        let response = common::request(server.addr, method, path, spec).await;
        let expected = &case["expected"];
        if let Some(route) = worker_route(path) {
            proxied += 1;
            check_worker_proxied(&response, route, id, &mut failures);
            continue;
        }
        if STEP_STRUCTURAL_CASES.contains(&id) {
            if native_step {
                structural += 1;
                check_step_structural(&response, expected, id, &mut failures);
            } else {
                dependency += 1;
                check_step_unavailable(&response, expected, id, &mut failures);
            }
            continue;
        }
        if !native_step && HEALTH_CASES.contains(&id) {
            dependency += 1;
            check_health_without_step(&response, expected, id, &mut failures);
            continue;
        }
        exact += 1;
        if response.status != expected["status"].as_u64().unwrap() as u16 {
            failures.push(format!(
                "{id}: status {} != {}",
                response.status, expected["status"]
            ));
        }
        if let Some(text) = expected["body_text"].as_str() {
            // Both sides are normalised: the frozen corpus records the
            // Python service's `design_available` and `design` fields, and
            // the native front door publishes its own worker state.
            let actual =
                normalize_body(id, &response.body_text()).unwrap_or_else(|| response.body_text());
            let wanted = normalize_body(id, text).unwrap_or_else(|| text.to_owned());
            if actual != wanted {
                failures.push(format!(
                    "{id}: body mismatch\nexpected: {text}\nactual:   {}",
                    response.body_text()
                ));
            }
        } else if let Some(expected_sha) = expected["body_sha256"].as_str() {
            let actual = hex(&Sha256::digest(&response.body));
            if actual != expected_sha
                || response.body.len() != expected["body_length"].as_u64().unwrap() as usize
            {
                failures.push(format!(
                    "{id}: body digest/length mismatch ({actual}, {} bytes)",
                    response.body.len()
                ));
            }
        }
        if let Some(headers) = expected["headers"].as_object() {
            for (name, value) in headers {
                // The ADR-0010 additions change the length of the health and
                // chat documents, so the frozen length is not a contract for
                // those two documents.
                if name == "content-length" && adr_extended(id) {
                    continue;
                }
                let actual = response.header(name);
                if actual != Some(value.as_str().unwrap()) {
                    failures.push(format!(
                        "{id}: header {name} {:?} != {:?}",
                        actual,
                        value.as_str().unwrap()
                    ));
                }
            }
        }
        if let Some(absent) = expected["headers_absent"].as_array() {
            for name in absent {
                if response.header(name.as_str().unwrap()).is_some() {
                    failures.push(format!("{id}: leaked header {}", name.as_str().unwrap()));
                }
            }
        }
    }
    server.stop().await;
    if native_step {
        assert_eq!(exact, 46, "exact native-supported fixture cases");
        assert_eq!(structural, 2, "structural STEP fixture cases");
        assert_eq!(dependency, 0, "no dependency cases in the native build");
        assert_eq!(proxied, 8, "the worker-proxied fixture cases");
    } else {
        assert_eq!(exact, 42, "exact cases in the unavailable-step build");
        assert_eq!(
            structural, 0,
            "no structural STEP cases without the feature"
        );
        assert_eq!(dependency, 6, "truthful STEP dependency cases");
        assert_eq!(proxied, 8, "the worker-proxied fixture cases");
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The front door recognises the worker route and this build refuses it.
///
/// The offline build attaches no geometry worker, so the proxied routes
/// answer the truthful refusal: a 501 that names the internal worker route
/// the front door would have called. The front door must not answer 404
/// (the route exists) and must not leak a worker detail.
fn check_worker_proxied(
    response: &common::TestResponse,
    route: &str,
    id: &str,
    failures: &mut Vec<String>,
) {
    let body = response.json();
    if response.status != 501
        || body["code"] != "geometry_route_unavailable"
        || body["details"]
            .as_array()
            .map(|details| details.iter().any(|detail| detail == route))
            != Some(true)
    {
        failures.push(format!(
            "{id}: expected the truthful worker-route refusal for {route}, got {} {}",
            response.status,
            response.body_text()
        ));
    }
    for absent in ["server", "x-powered-by", "set-cookie"] {
        if response.header(absent).is_some() {
            failures.push(format!("{id}: leaked header {absent}"));
        }
    }
}

/// The `native-step`-disabled build keeps the documented truthful divergence:
/// the health `step_available` field is false and the two STEP downloads
/// answer the real `step_unavailable` 503 envelope.
fn check_health_without_step(
    response: &common::TestResponse,
    expected: &Value,
    id: &str,
    failures: &mut Vec<String>,
) {
    let expected_body: Value =
        serde_json::from_str(expected["body_text"].as_str().unwrap()).unwrap();
    let mut expected_without_step = expected_body.clone();
    expected_without_step
        .as_object_mut()
        .unwrap()
        .remove("step_available");
    let mut actual = response.json();
    let step_available = actual
        .as_object_mut()
        .unwrap()
        .remove("step_available")
        .and_then(|value| value.as_bool())
        .unwrap();
    if response.status != 200 || actual != expected_without_step || step_available {
        failures.push(format!("{id}: health divergence is not truthful"));
    }
    if response.header("content-type") != Some("application/json") {
        failures.push(format!("{id}: health content-type mismatch"));
    }
}

fn check_step_unavailable(
    response: &common::TestResponse,
    expected: &Value,
    id: &str,
    failures: &mut Vec<String>,
) {
    let body = response.json();
    if response.status != 503
        || body["code"] != "step_unavailable"
        || body["details"]
            .as_array()
            .is_none_or(|details| details.is_empty())
    {
        failures.push(format!(
            "{id}: expected truthful step_unavailable 503, got {} {}",
            response.status,
            response.body_text()
        ));
    }
    for absent in expected["headers_absent"].as_array().into_iter().flatten() {
        if response.header(absent.as_str().unwrap()).is_some() {
            failures.push(format!("{id}: leaked header {}", absent.as_str().unwrap()));
        }
    }
}

/// Check the fixture's `body_structural` contract for a STEP download.
///
/// The fixture declares `byte_equality_contract: false` and asks for the
/// stable fields instead: an ISO 10303-21 file with exactly one manifold
/// solid. Stable headers (media type, download filename, security set) and a
/// self-consistent `x-studio-handoff` length are verified; the OCP
/// `content-length`/digest are informational and are not compared.
fn check_step_structural(
    response: &common::TestResponse,
    expected: &Value,
    id: &str,
    failures: &mut Vec<String>,
) {
    let contract = &expected["body_structural"];
    if response.status != 200 {
        failures.push(format!("{id}: status {} != 200", response.status));
    }
    if response.header("content-type") != Some("application/step") {
        failures.push(format!(
            "{id}: content-type {:?} != application/step",
            response.header("content-type")
        ));
    }
    let name = expected["headers"]["content-disposition"].as_str().unwrap();
    if response.header("content-disposition") != Some(name) {
        failures.push(format!("{id}: content-disposition mismatch"));
    }
    if contract["starts_with_iso_10303_21"] == Value::Bool(true)
        && !response.body.starts_with(b"ISO-10303-21;")
    {
        failures.push(format!("{id}: body does not start with ISO-10303-21;"));
    }
    let expected_breps = contract["manifold_solid_brep_count"].as_u64().unwrap_or(0) as usize;
    let actual_breps = count_occurrences(&response.body, b"MANIFOLD_SOLID_BREP");
    if actual_breps != expected_breps {
        failures.push(format!(
            "{id}: MANIFOLD_SOLID_BREP count {actual_breps} != {expected_breps}"
        ));
    }
    if !response.body.ends_with(b"END-ISO-10303-21;\n") {
        failures.push(format!("{id}: STEP body trailer missing"));
    }
    if !response.body.iter().all(u8::is_ascii) {
        failures.push(format!("{id}: STEP body is not ASCII"));
    }
    let declared = response.header("content-length");
    if declared != Some(response.body.len().to_string().as_str()) {
        failures.push(format!("{id}: content-length mismatch"));
    }
    if let Some(expected_handoff) = expected["headers"]["x-studio-handoff"].as_str() {
        let prefix = expected_handoff
            .split(";bytes=")
            .next()
            .unwrap_or(expected_handoff);
        let actual_handoff = response.header("x-studio-handoff").unwrap_or("");
        if actual_handoff != format!("{prefix};bytes={}", response.body.len()) {
            failures.push(format!(
                "{id}: x-studio-handoff {actual_handoff:?} is inconsistent with {} bytes",
                response.body.len()
            ));
        }
    }
    for absent in ["server", "x-powered-by", "set-cookie"] {
        if response.header(absent).is_some() {
            failures.push(format!("{id}: leaked header {absent}"));
        }
    }
    for (name, value) in expected["headers"].as_object().unwrap() {
        if name == "content-length" || name == "x-studio-handoff" {
            continue;
        }
        if response.header(name) != Some(value.as_str().unwrap()) {
            failures.push(format!("{id}: header {name} mismatch"));
        }
    }
    let duplicates = response
        .headers
        .iter()
        .map(|(name, _)| name)
        .collect::<Vec<_>>();
    let unique: HashSet<_> = duplicates.iter().collect();
    if unique.len() != duplicates.len() {
        failures.push(format!("{id}: duplicate response headers"));
    }
}

/// The ADR-0010 additions the frozen corpus predates.
///
/// * `health` gained `design_available` (ADR-0009) and the Rust front door
///   publishes `geometry_worker` (ADR-0010).
/// * `chat` gained `design` (ADR-0009).
///
/// The corpus is the frozen nine-route artifact, so the two documents are
/// compared without those fields. Every other field is still compared byte
/// for byte, and the comparison stays a byte comparison whenever no field
/// has to be removed.
fn normalize_body(id: &str, body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    let object = value.as_object()?;
    let mut rebuilt = serde_json::Map::new();
    for (key, entry) in object {
        let drop = if HEALTH_BODY_CASES.contains(&id) {
            key == "design_available" || key == "geometry_worker"
        } else {
            key == "design"
        };
        if drop {
            continue;
        }
        rebuilt.insert(key.clone(), entry.clone());
    }
    serde_json::to_string(&Value::Object(rebuilt)).ok()
}

/// Whether the case's body carries the ADR-0010 additions.
fn adr_extended(id: &str) -> bool {
    HEALTH_BODY_CASES.contains(&id) || id.starts_with("studio.chat.")
}

fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}
