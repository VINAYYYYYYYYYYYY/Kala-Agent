//! Shared helpers for the native integration tests.

#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use kala_contracts::ArtifactClassification;
use kala_json::{CanonicalValue, JsonText};
use kala_runtime::gateway::{
    ModelArtifact, ModelCapability, ModelError, ModelMessage, ModelRequest, ModelRole,
    ProviderRequest, ProviderResponse, ProviderTransport, TransportFailure, bounded_identifier,
};

/// The repository root.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("repository root")
}

/// The live Python oracle interpreter.
pub fn python_executable() -> PathBuf {
    if let Ok(path) = std::env::var("KALA_PYTHON") {
        return PathBuf::from(path);
    }
    let venv = repo_root().join(".venv/bin/python");
    assert!(
        venv.exists(),
        "live Python oracle requires {} (set KALA_PYTHON to override)",
        venv.display()
    );
    venv
}

/// Run the gateway oracle and return its parsed result document.
pub fn run_oracle(scenario: &CanonicalValue) -> CanonicalValue {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/oracle.py");
    let mut command = Command::new(python_executable());
    command
        .arg(script)
        .arg("gateway")
        .env(
            "PYTHONPATH",
            repo_root().join("src").to_string_lossy().to_string(),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn the Python oracle");
    let limits = kala_json::Limits {
        max_string_length: 1_048_576,
        max_json_bytes: 16_777_216,
        ..kala_json::Limits::protocol()
    };
    let payload = kala_json::canonical_dumps(scenario, &limits).expect("canonical scenario");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(payload.as_bytes())
        .expect("write the scenario");
    let output = child.wait_with_output().expect("wait for the oracle");
    assert!(
        output.status.success(),
        "the Python oracle failed with {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("the oracle stdout is UTF-8");
    parse(&text)
}

/// Parse canonical JSON text.
///
/// The result documents carry bounded transcript bodies of 64 KiB, which is
/// above the protocol's 32 KiB string limit, so the harness uses relaxed
/// limits.
pub fn parse(text: &str) -> CanonicalValue {
    let limits = kala_json::Limits {
        max_string_length: 1_048_576,
        max_json_bytes: 16_777_216,
        ..kala_json::Limits::protocol()
    };
    kala_json::parse_strict(text.as_bytes(), &limits)
        .unwrap_or_else(|error| panic!("parse the result document: {error}"))
}

/// Start the mock provider and return the child and the chosen port.
pub fn start_mock_provider() -> (Child, u16) {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mock_provider.py");
    let mut child = Command::new(python_executable())
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the mock provider");
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = std::io::BufReader::new(stdout);
    let mut line = String::new();
    {
        use std::io::BufRead as _;
        reader.read_line(&mut line).expect("read the mock port");
    }
    let port: u16 = line.trim().parse().expect("a mock provider port");
    (child, port)
}

/// Fetch the recorded requests from the mock provider.
pub fn mock_log(port: u16) -> CanonicalValue {
    let body = http_request(port, "GET", "/__log");
    parse(&body)
}

/// Clear the recorded requests of the mock provider.
pub fn reset_mock(port: u16) {
    let body = http_request(port, "POST", "/__reset");
    assert_eq!(body.trim(), "{}");
}

/// A minimal HTTP/1.1 client for the mock provider control endpoints.
///
/// The blocking reqwest client cannot be created inside a Tokio runtime, so
/// the control plane uses a plain socket.
fn http_request(port: u16, method: &str, path: &str) -> String {
    use std::io::{Read as _, Write as _};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\
         Content-Length: 0\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .expect("write the request");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("read the response");
    let text = String::from_utf8_lossy(&response).to_string();
    text.split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or_default()
        .to_owned()
}

/// A JSON object helper.
pub fn object(entries: Vec<(&str, CanonicalValue)>) -> CanonicalValue {
    CanonicalValue::object(entries)
}

/// A JSON array helper.
pub fn array(values: Vec<CanonicalValue>) -> CanonicalValue {
    CanonicalValue::Array(values)
}

/// A JSON string helper.
pub fn text(value: &str) -> CanonicalValue {
    CanonicalValue::Text(JsonText::new(value))
}

/// A JSON integer helper.
pub fn int(value: i64) -> CanonicalValue {
    CanonicalValue::integer(value)
}

/// A JSON boolean helper.
pub fn bool_value(value: bool) -> CanonicalValue {
    CanonicalValue::Bool(value)
}

/// Read a field of a JSON object.
pub fn get<'a>(value: &'a CanonicalValue, key: &str) -> Option<&'a CanonicalValue> {
    match value {
        CanonicalValue::Object(entries) => entries.get(&JsonText::new(key)),
        _ => None,
    }
}

/// Read a string field of a JSON object.
pub fn text_field(value: &CanonicalValue, key: &str) -> String {
    get(value, key)
        .and_then(|item| item.as_text())
        .map(|item| item.to_string_lossy())
        .unwrap_or_default()
}

/// Read an optional string field of a JSON object.
pub fn optional_text_field(value: &CanonicalValue, key: &str) -> Option<String> {
    get(value, key)
        .and_then(|item| item.as_text())
        .map(|item| item.to_string_lossy())
}

/// Convert a `serde_json` value into the canonical value model.
pub fn serde_to_canonical(value: &serde_json::Value) -> CanonicalValue {
    match value {
        serde_json::Value::Null => CanonicalValue::Null,
        serde_json::Value::Bool(value) => CanonicalValue::Bool(*value),
        serde_json::Value::String(value) => CanonicalValue::text(value.clone()),
        serde_json::Value::Number(number) => {
            let text = number.to_string();
            if let Ok(integer) = text.parse::<i128>() {
                CanonicalValue::integer(integer)
            } else {
                CanonicalValue::Float(text.parse::<f64>().unwrap_or(0.0))
            }
        }
        serde_json::Value::Array(values) => {
            CanonicalValue::Array(values.iter().map(serde_to_canonical).collect())
        }
        serde_json::Value::Object(entries) => CanonicalValue::object(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), serde_to_canonical(value))),
        ),
    }
}

/// Build a model request from the scenario document.
pub fn build_request(payload: &CanonicalValue) -> Result<ModelRequest, ModelError> {
    let request_id = text_field(payload, "request_id");
    let mut messages = Vec::new();
    if let Some(CanonicalValue::Array(items)) = get(payload, "messages") {
        for item in items {
            let role = text_field(item, "role");
            let content = text_field(item, "content");
            messages.push(ModelMessage::from_text(&role, &content)?);
        }
    }
    let mut capabilities = Vec::new();
    if let Some(CanonicalValue::Array(items)) = get(payload, "capabilities") {
        for item in items {
            let name = item
                .as_text()
                .map(|value| value.to_string_lossy())
                .unwrap_or_default();
            let capability = ModelCapability::parse(&name).ok_or_else(|| {
                ModelError::capability(format!("unknown model capability: '{name}'"))
            })?;
            capabilities.push(capability);
        }
    }
    let schema = match get(payload, "response_schema") {
        Some(value @ CanonicalValue::Object(_)) => Some(value.clone()),
        _ => None,
    };
    let mut artifacts = Vec::new();
    if let Some(CanonicalValue::Array(items)) = get(payload, "artifacts") {
        for item in items {
            let classification = ArtifactClassification::parse(&text_field(item, "classification"))
                .ok_or_else(|| {
                    ModelError::request("classification is not an artifact classification")
                })?;
            let size_bytes = get(item, "size_bytes")
                .and_then(|value| value.as_integer())
                .and_then(|value| value.to_string().parse::<i128>().ok())
                .unwrap_or(0);
            let mut artifact = ModelArtifact::new(
                &text_field(item, "artifact_id"),
                &text_field(item, "digest"),
                classification,
                &text_field(item, "media_type"),
                size_bytes,
            )?;
            artifact =
                artifact.with_preview_text(optional_text_field(item, "preview_text").as_deref())?;
            artifact = artifact.with_path(optional_text_field(item, "path").as_deref())?;
            if let Some(metadata) = get(item, "metadata") {
                artifact = artifact.with_metadata(metadata)?;
            }
            artifacts.push(artifact);
        }
    }
    let seed = get(payload, "seed")
        .and_then(|value| value.as_integer())
        .and_then(|value| value.to_string().parse::<i128>().ok());
    let max_output_tokens = get(payload, "max_output_tokens")
        .and_then(|value| value.as_integer())
        .and_then(|value| value.to_string().parse::<i128>().ok())
        .unwrap_or(1_024);
    let timeout_seconds = get(payload, "timeout_seconds")
        .and_then(|value| value.as_float())
        .unwrap_or(60.0);
    ModelRequest::from_parts(
        &request_id,
        messages,
        capabilities,
        schema.as_ref(),
        artifacts,
        seed,
        max_output_tokens,
        timeout_seconds,
    )
}

/// A safe identifier for the tests.
pub fn identifier(value: &str) -> String {
    bounded_identifier(value, "identifier").expect("a safe identifier")
}

/// The role of a message.
pub fn role(value: &str) -> ModelRole {
    ModelRole::parse(value).expect("a model role")
}

/// A transport that answers with a fixed status and body.
///
/// The Python implementation injects an `httpx.MockTransport` for its tests;
/// the Rust port injects a `ProviderTransport` instead.
pub fn mock_transport(status: u16, body: &'static [u8]) -> Arc<dyn ProviderTransport> {
    Arc::new(MockTransport {
        status,
        body: body.to_vec(),
    })
}

/// A fixed provider transport.
pub struct MockTransport {
    status: u16,
    body: Vec<u8>,
}

#[async_trait::async_trait]
impl ProviderTransport for MockTransport {
    async fn post(&self, request: ProviderRequest) -> Result<ProviderResponse, TransportFailure> {
        let mut seen = String::new();
        for (name, value) in &request.headers {
            seen.push_str(name);
            seen.push('=');
            // The credential is never echoed, only its presence.
            seen.push_str(if name == "authorization" {
                "<redacted>"
            } else {
                value
            });
            seen.push(';');
        }
        assert!(
            !seen.contains("Bearer "),
            "the credential must never be echoed"
        );
        Ok(ProviderResponse {
            status: self.status,
            body: self.body.clone(),
        })
    }
}
