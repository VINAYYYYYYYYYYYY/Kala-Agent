//! Chat with designs: the strict parser, the CAD provider choice and the
//! journal turn, against the real Python geometry worker.
//!
//! The test drives the real `kala-studio` binary with the real supervised
//! worker. It is Unix-only; no Windows worker lifecycle is proven here.
//!
//! A missing interpreter fails this test loudly. `KALA_SKIP_WORKER_LIFECYCLE=1`
//! is the only opt-out, and it is loud too.

#![cfg(unix)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use common::{RequestSpec, TOKEN};

const SKIP_ENV: &str = "KALA_SKIP_WORKER_LIFECYCLE";

/// The ADR-0011 end-to-end sentence the live differential also uses.
const STITCH_PROMPT: &str = "Design a machined aluminum L-bracket for a 40 mm mounting plate, 8 mm wall thickness, with a 10 mm centered bore and 4x M5 corner mounting holes.";

/// The helper prose the worker's strict parser produces for that sentence.
///
/// Captured from the live Python public server; the live differential
/// (`tests/worker_differential.py`) compares the whole reply byte for byte.
const STITCH_HELPER_TEXT: &str = "Local helper. Design intent parsed from declared values. 4x M5 mapped to 5.5 mm clearance holes (ISO 273 medium). Plate width 60 mm and depth 45 mm are canonical template defaults; state them to override. Compiling and measuring with OpenCASCADE.";

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .expect("the crate lives under <repo>/backend/crates")
}

fn venv_python() -> PathBuf {
    repository_root().join(".venv").join("bin").join("python")
}

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_kala-studio")
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

async fn read_banner(child: &mut Child) -> serde_json::Value {
    let stdout = child.stdout.take().expect("piped stdout");
    let mut lines = BufReader::new(stdout).lines();
    let mut text = String::new();
    tokio::time::timeout(Duration::from_secs(30), async {
        while let Ok(Some(line)) = lines.next_line().await {
            text.push_str(&line);
            text.push('\n');
            if line.trim() == "}" {
                return;
            }
        }
    })
    .await
    .expect("banner before 30s");
    serde_json::from_str(&text).expect("banner JSON")
}

async fn wait_ready(port: u16) {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the studio did not become ready");
}

async fn wait_for_worker(port: u16) -> serde_json::Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(90);
    while std::time::Instant::now() < deadline {
        let response = common::request(
            format!("127.0.0.1:{port}").parse().unwrap(),
            "GET",
            "/api/studio/health",
            RequestSpec::default(),
        )
        .await;
        if response.status == 200 {
            let document = response.json();
            if document["geometry_worker"]["state"] == "ready" {
                return document;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the geometry worker did not become ready");
}

fn chat_request(body: serde_json::Value) -> RequestSpec {
    let mut spec = RequestSpec::default();
    spec.headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    spec.headers
        .push(("content-type".to_owned(), "application/json".to_owned()));
    spec.body = Some(serde_json::to_vec(&body).unwrap());
    spec
}

async fn chat(port: u16, body: serde_json::Value) -> common::TestResponse {
    common::request(
        format!("127.0.0.1:{port}").parse().unwrap(),
        "POST",
        "/api/studio/chat",
        chat_request(body),
    )
    .await
}

async fn start_studio() -> (Child, u16, tempfile::TempDir) {
    if std::env::var(SKIP_ENV).is_ok() {
        eprintln!("{SKIP_ENV} is set: the chat-with-design test is skipped loudly");
        std::process::exit(0);
    }
    let python = venv_python();
    assert!(
        python.is_file(),
        "the geometry worker interpreter is missing: {}",
        python.display()
    );
    let work = tempfile::tempdir().unwrap();
    let home = work.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let port = free_port();
    let command = serde_json::json!([
        python.to_string_lossy(),
        "-m",
        "kala_agent.studio",
        "--worker"
    ])
    .to_string();
    let mut child = Command::new(binary())
        .arg("--port")
        .arg(port.to_string())
        .arg("--data-dir")
        .arg(work.path().join("data"))
        .env_clear()
        .env("PATH", "")
        .env("HOME", &home)
        .env("KALA_STUDIO_TOKEN", TOKEN)
        .env("KALA_STUDIO_GEOMETRY_WORKER", command)
        .env("PYTHONPATH", repository_root().join("src"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kala-studio");
    let _banner = read_banner(&mut child).await;
    wait_ready(port).await;
    wait_for_worker(port).await;
    (child, port, work)
}

/// Every `studio-journal.sqlite3` below `root`, found iteratively.
fn journal_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if file_type.is_dir() {
                pending.push(path);
            } else if path.file_name().and_then(|name| name.to_str())
                == Some("studio-journal.sqlite3")
            {
                found.push(path);
            }
        }
    }
    found
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_design_prompt_without_a_cad_provider_asks_the_operator() {
    let (mut child, port, _work) = start_studio().await;
    let response = chat(
        port,
        serde_json::json!({
            "provider": "local",
            "prompt": "make me an L-bracket 60 x 45 x 40 mm with a 6 mm wall"
        }),
    )
    .await;
    assert_eq!(response.status, 200);
    let document = response.json();
    let text = document["text"].as_str().unwrap_or_default();
    assert!(
        text.contains("Choose a CAD provider before I build this design"),
        "the turn must ask which provider to use: {text}"
    );
    assert!(
        text.contains("opencascade (opt-in)"),
        "the turn must name the providers: {text}"
    );
    assert!(
        text.contains("never selects a provider automatically"),
        "the turn must state that no provider is chosen automatically: {text}"
    );
    assert!(
        text.contains("No design was built"),
        "the turn must say that nothing was built: {text}"
    );
    assert!(
        document["design"].is_null(),
        "no design may be compiled without an explicit provider"
    );
    let _ = child.kill().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_design_prompt_with_opencascade_compiles_the_design() {
    let (mut child, port, _work) = start_studio().await;
    let response = chat(
        port,
        serde_json::json!({
            "provider": "local",
            "prompt": "make me an L-bracket 60 x 45 x 40 mm with a 6 mm wall",
            "cad_provider": "opencascade"
        }),
    )
    .await;
    assert_eq!(response.status, 200);
    let document = response.json();
    let design = &document["design"];
    assert!(
        design.is_object(),
        "the turn must carry the design: {document}"
    );
    let design_id = design["design_id"].as_str().unwrap_or_default();
    assert_eq!(design_id.len(), 64, "the design id is a sha256 digest");
    assert_eq!(design["verified"], true);
    let volume = design["measured"]["volume_mm3"]
        .as_f64()
        .unwrap_or_default();
    assert!(
        (volume - 28_439.999999999993).abs() < 1e-6,
        "the measured volume must match the source: {volume}"
    );
    assert_eq!(
        design["measured"]["surface_area_mm2"].as_f64(),
        Some(11_148.0)
    );
    assert_eq!(
        design_id,
        "9ec29435f7b95f19641f548c1d29e398f0cbb4115e9e5e38c5d8ef07c83f4bf8"
    );
    let text = document["text"].as_str().unwrap_or_default();
    assert!(
        text.contains("Measured volume"),
        "the turn must report the measurement: {text}"
    );
    assert!(
        text.contains("B-Rep verified"),
        "the turn must report the verdict"
    );
    let _ = child.kill().await;
}

/// The worker's live CAD provider rows, through the front door.
async fn cad_provider_rows(port: u16) -> Vec<serde_json::Value> {
    let mut spec = RequestSpec::default();
    spec.headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    let response = common::request(
        format!("127.0.0.1:{port}").parse().unwrap(),
        "GET",
        "/api/studio/providers/cad",
        spec,
    )
    .await;
    assert_eq!(
        response.status, 200,
        "the CAD provider probe must answer before the chat parity turn"
    );
    response.json()["providers"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn provider_row<'a>(rows: &'a [serde_json::Value], id: &str) -> &'a serde_json::Value {
    rows.iter()
        .find(|row| row["id"].as_str() == Some(id))
        .expect("the provider probe names every ADR-0011 provider")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_stitch_sentence_matches_python_for_every_cad_provider() {
    let (mut child, port, _work) = start_studio().await;
    let rows = cad_provider_rows(port).await;

    // FreeCAD is a real external build on this host. The reference answer was
    // captured from the live Python public server for the same sentence and
    // the same provider; the live differential (`worker_differential.py`)
    // compares the whole reply and design document against it directly.
    let freecad = provider_row(&rows, "freecad");
    if freecad["available"].as_bool() == Some(true) {
        let response = chat(
            port,
            serde_json::json!({
                "provider": "local",
                "prompt": STITCH_PROMPT,
                "cad_provider": "freecad"
            }),
        )
        .await;
        assert_eq!(response.status, 200);
        let document = response.json();
        let design = &document["design"];
        assert!(
            design.is_object(),
            "the freecad turn must build through the worker: {document}"
        );
        assert_eq!(design["provider"], "freecad");
        assert_eq!(design["provider_version"], "1.1.4");
        assert_eq!(design["measured_by"], "FreeCAD");
        assert_eq!(design["verified"], true);
        assert_eq!(
            design["design_id"],
            "a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7"
        );
        assert_eq!(
            design["measured"]["volume_mm3"].as_f64(),
            Some(35_571.416_047)
        );
        assert_eq!(
            design["measured"]["surface_area_mm2"].as_f64(),
            Some(11_889.101_731)
        );
        assert_eq!(design["measured"]["solid_count"], 1);
        assert_eq!(design["measured"]["valid"], true);
        assert_eq!(
            document["text"].as_str().unwrap_or_default(),
            format!(
                "{STITCH_HELPER_TEXT} L-bracket 60 x 45 x 40: B-Rep verified; measured by FreeCAD. Measured volume 35571.416 mm^3, surface area 11889.102 mm^2, envelope 60.000 x 45.000 x 40.000 mm. design_id a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7."
            )
        );
    } else {
        eprintln!(
            "a real FreeCADCmd is not available on this host: the FreeCAD chat parity \
             assertions are skipped loudly ({})",
            freecad["reason"].as_str().unwrap_or("no reason")
        );
    }

    // The opt-in OpenCascade builder is the same sentence through the same
    // pass-through, with its own identity and measurements.
    let response = chat(
        port,
        serde_json::json!({
            "provider": "local",
            "prompt": STITCH_PROMPT,
            "cad_provider": "opencascade"
        }),
    )
    .await;
    assert_eq!(response.status, 200);
    let document = response.json();
    let design = &document["design"];
    assert_eq!(design["provider"], "opencascade");
    assert_eq!(design["provider_version"], "8.0.1.1");
    assert_eq!(design["measured_by"], "OpenCascade (opt-in)");
    assert_eq!(design["verified"], true);
    assert_eq!(
        design["design_id"],
        "594f408b3223505c51bfdef61cfb935fc28f8ec461aa62fefc62c9b731932c92"
    );
    assert_eq!(
        design["measured"]["volume_mm3"].as_f64(),
        Some(35_571.416_047_113_315)
    );
    assert_eq!(
        document["text"].as_str().unwrap_or_default(),
        format!(
            "{STITCH_HELPER_TEXT} L-bracket 60 x 45 x 40: B-Rep verified; measured by OpenCascade (opt-in). Measured volume 35571.416 mm^3, surface area 11889.102 mm^2, envelope 60.000 x 45.000 x 40.000 mm. design_id 594f408b3223505c51bfdef61cfb935fc28f8ec461aa62fefc62c9b731932c92."
        )
    );

    // Solid Edge cannot run on this Linux host. The worker's honest refusal is
    // relayed in the source's wording: the reason, the providers that can
    // build here and "No design was built." There is no fallback.
    let solidedge = provider_row(&rows, "solidedge");
    if std::env::consts::OS == "linux" && solidedge["available"].as_bool() == Some(false) {
        let response = chat(
            port,
            serde_json::json!({
                "provider": "local",
                "prompt": STITCH_PROMPT,
                "cad_provider": "solidedge"
            }),
        )
        .await;
        assert_eq!(response.status, 200);
        let document = response.json();
        assert!(
            document["design"].is_null(),
            "a refused provider must not fall back to another one: {document}"
        );
        assert_eq!(
            document["text"].as_str().unwrap_or_default(),
            format!(
                "{STITCH_HELPER_TEXT} The solidedge CAD provider is not available: Solid Edge build requires the Windows bridge (not reachable); this host is linux, so the ADR-0008 bridge cannot run here. Available: opencascade, freecad. No design was built."
            )
        );
    } else {
        eprintln!(
            "the exact Solid Edge refusal is host-specific: the refusal parity assertion \
             is skipped loudly (host={}, available={})",
            std::env::consts::OS,
            solidedge["available"]
        );
    }
    let _ = child.kill().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_design_prompt_with_missing_values_asks_for_them() {
    let (mut child, port, _work) = start_studio().await;
    let response = chat(
        port,
        serde_json::json!({
            "provider": "local",
            "prompt": "make me a bracket"
        }),
    )
    .await;
    assert_eq!(response.status, 200);
    let document = response.json();
    let text = document["text"].as_str().unwrap_or_default();
    assert!(
        text.contains("I need these declared values"),
        "the turn must ask for the missing values: {text}"
    );
    assert!(text.contains("width (mm)"), "the turn must name the values");
    assert!(document["design"].is_null());
    let _ = child.kill().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_chat_turn_is_journalled_through_the_worker() {
    let (mut child, port, work) = start_studio().await;
    let data_dir = work.path().join("data");
    let home = work.path().join("home");
    let response = chat(
        port,
        serde_json::json!({
            "provider": "local",
            "prompt": "a box 10x20x30 mm"
        }),
    )
    .await;
    assert_eq!(response.status, 200);

    let mut journal = RequestSpec::default();
    journal
        .headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    let page = common::request(
        format!("127.0.0.1:{port}").parse().unwrap(),
        "GET",
        "/api/studio/journal?after=0&limit=10",
        journal,
    )
    .await;
    assert_eq!(page.status, 200);
    let document = page.json();
    let events = document["events"].as_array().expect("journal events");
    assert_eq!(events.len(), 1, "the chat turn is journalled once");
    let event = &events[0];
    assert_eq!(event["event_type"], "chat.turn");
    assert_eq!(event["aggregate_id"], "studio");
    let payload = &event["payload"];
    assert_eq!(payload["provider"], "local");
    assert_eq!(payload["design_requested"], false);
    assert_eq!(payload["cad_provider"], serde_json::Value::Null);
    assert!(
        payload["prompt_sha256"]
            .as_str()
            .is_some_and(|d| d.len() == 64),
        "the payload carries the prompt digest, never the prompt"
    );
    assert!(
        payload.get("prompt").is_none(),
        "the prompt is never journalled"
    );

    // The worker writes exactly the front door's journal, under the front
    // door's --data-dir. There is no second journal in a home directory or a
    // per-worker fallback directory, and the empty HOME stays empty.
    let expected = data_dir.join("studio-journal.sqlite3");
    assert!(
        expected.is_file(),
        "the journal must live at {}",
        expected.display()
    );
    assert_eq!(
        journal_files(work.path()),
        vec![expected],
        "exactly one journal, under the front door's data dir"
    );
    assert_eq!(
        std::fs::read_dir(&home).unwrap().count(),
        0,
        "the studio and its worker must not write into HOME"
    );
    let _ = child.kill().await;
}
