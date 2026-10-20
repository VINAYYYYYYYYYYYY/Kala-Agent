//! Shared helpers for the native tool protocol tests.

#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use kala_contracts::ToolDescriptor;
use kala_contracts::model_json_limits;
use kala_json::{CanonicalObject, CanonicalValue, JsonText, canonical_dumps, parse_strict};
use kala_tools::protocol::{
    ToolHandler, ToolProtocolError, ToolRequestEnvelope, ToolResponseEnvelope,
};
use kala_tools::registry::{KERNEL_PACKAGE_DIRECTORY, RegistryHandler};

/// A tool handler for the registry tests.
///
/// The handler can pretend to be defined by the kernel package and it can
/// change the descriptor it exposes, which is what the Python implementation
/// models with a mutated handler attribute.
pub struct TestHandler {
    descriptor: ToolDescriptor,
    replacement: Mutex<Option<ToolDescriptor>>,
    first_party: bool,
}

impl TestHandler {
    /// Build a handler that serves the descriptor.
    pub fn new(descriptor: ToolDescriptor) -> Self {
        Self {
            descriptor,
            replacement: Mutex::new(None),
            first_party: false,
        }
    }

    /// Build a handler that claims to be defined by the kernel package.
    pub fn first_party(descriptor: ToolDescriptor) -> Self {
        Self {
            descriptor,
            replacement: Mutex::new(None),
            first_party: true,
        }
    }

    /// Change the descriptor the handler exposes.
    pub fn replace_descriptor(&self, descriptor: ToolDescriptor) {
        let mut replacement = self.replacement.lock().expect("handler");
        *replacement = Some(descriptor);
    }
}

impl ToolHandler for TestHandler {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn invoke(
        &self,
        _request: &ToolRequestEnvelope,
    ) -> Result<ToolResponseEnvelope, ToolProtocolError> {
        Err(ToolProtocolError::protocol("the test handler does not run"))
    }
}

impl RegistryHandler for TestHandler {
    fn descriptor_snapshot(&self) -> ToolDescriptor {
        let replacement = self.replacement.lock().expect("handler");
        match replacement.as_ref() {
            None => self.descriptor.clone(),
            Some(descriptor) => descriptor.clone(),
        }
    }

    fn defining_package(&self) -> &'static str {
        if self.first_party {
            KERNEL_PACKAGE_DIRECTORY
        } else {
            ""
        }
    }
}

/// A request envelope for the simulator tests.
pub fn request(
    workspace: &Path,
    arguments: &CanonicalObject,
    call_id: &str,
) -> ToolRequestEnvelope {
    ToolRequestEnvelope::new(
        kala_tools::protocol::PROTOCOL_VERSION,
        call_id,
        "mechanical.reference-simulator",
        "1.0.0",
        arguments,
        &workspace.to_string_lossy().replace('\\', "/"),
        30.0,
        &num_bigint::BigInt::from(1_000_000),
        &num_bigint::BigInt::from(1_000_000),
        &[],
    )
    .expect("request envelope")
}

/// Create a symbolic link to a directory.
pub fn symlink_directory(target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(target, link)
    }
}

/// The repository root.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates")
        .parent()
        .expect("backend")
        .parent()
        .expect("repo")
        .to_path_buf()
}

/// The interpreter that runs the live Python oracle.
pub fn python_executable() -> PathBuf {
    if let Ok(value) = std::env::var("KALA_TOOLS_PYTHON") {
        return PathBuf::from(value);
    }
    let candidate = repo_root().join(".venv/bin/python");
    assert!(
        candidate.exists(),
        "live Python oracle missing: {} (set KALA_TOOLS_PYTHON)",
        candidate.display()
    );
    candidate
}

/// The canonical JSON text of a value, using the unbounded model limits.
///
/// The corpus that feeds the oracle is not itself a tool protocol payload, so
/// it must not be bounded by the protocol limits.
pub fn canonical(value: &CanonicalValue) -> String {
    canonical_dumps(value, &model_json_limits()).expect("canonical corpus")
}

/// Run the test-only Python oracle and return its parsed results.
pub fn run_oracle(
    mode: &str,
    cases: &CanonicalValue,
    workspace_root: Option<&Path>,
) -> Vec<CanonicalValue> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/oracle.py");
    let mut command = Command::new(python_executable());
    command
        .arg("-I")
        .arg(script)
        .arg(mode)
        .env("PYTHONPATH", repo_root().join("src"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(root) = workspace_root {
        command.env("KALA_TOOLS_WORKSPACE_ROOT", root);
    }
    let mut child = command.spawn().expect("spawn Python oracle");
    let payload = canonical(cases);
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(payload.as_bytes())
        .expect("write oracle input");
    let output = child.wait_with_output().expect("wait for Python oracle");
    assert!(
        output.status.success(),
        "Python oracle failed with {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed = parse_strict(&output.stdout, &model_json_limits()).expect("oracle output is JSON");
    let CanonicalValue::Array(items) = &parsed else {
        panic!("oracle output is not an array: {parsed:?}");
    };
    items.clone()
}

/// The oracle shape of a successful result.
pub fn ok_result(output: CanonicalValue) -> CanonicalValue {
    CanonicalValue::object([("ok", CanonicalValue::Bool(true)), ("output", output)])
}

/// The oracle shape of a failed result.
pub fn error_result(error: &ToolProtocolError) -> CanonicalValue {
    CanonicalValue::object([
        ("ok", CanonicalValue::Bool(false)),
        ("error", CanonicalValue::text(error.python_class_name())),
        ("message", CanonicalValue::text(error.to_string())),
    ])
}

/// The Python exception classes of the protocol hierarchy.
pub const PROTOCOL_ERROR_CLASSES: [&str; 5] = [
    "ToolProtocolError",
    "JsonEnvelopeError",
    "ToolEnvelopeError",
    "ToolWorkspaceError",
    "SchemaValidationError",
];

/// Compare a native result with the oracle result, byte for byte.
///
/// When the oracle reports a Python exception outside the protocol hierarchy
/// the exception class cannot match, because Rust has no equivalent class.
/// The failure itself and its message must still match exactly.
pub fn assert_same(native: &CanonicalValue, oracle: &CanonicalValue, label: &str) {
    let oracle_class = match oracle {
        CanonicalValue::Object(entries) => match entries.get(&kala_json::JsonText::new("error")) {
            Some(CanonicalValue::Text(class)) => Some(class.to_string_lossy()),
            _ => None,
        },
        _ => None,
    };
    let native = match oracle_class {
        Some(class) if !PROTOCOL_ERROR_CLASSES.contains(&class.as_str()) => {
            let CanonicalValue::Object(entries) = native else {
                panic!("native result is not an object")
            };
            let mut entries = entries.clone();
            entries.insert(
                kala_json::JsonText::new("error"),
                CanonicalValue::text(&class),
            );
            CanonicalValue::Object(entries)
        }
        _ => native.clone(),
    };
    let native_bytes = canonical(&native);
    let oracle_bytes = canonical(oracle);
    assert_eq!(
        native_bytes, oracle_bytes,
        "native and oracle differ for {label}\nnative: {native_bytes}\noracle: {oracle_bytes}"
    );
}

/// A deterministic pseudo random number generator.
pub struct Lcg(u64);

impl Lcg {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }

    pub fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    pub fn pick<'a, T>(&mut self, values: &'a [T]) -> &'a T {
        &values[(self.next() as usize) % values.len()]
    }
}

/// The hexadecimal encoding of a byte string.
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Build a tool call for the executor tests.
///
/// The call is the accepted contract model, so the arguments are canonical
/// JSON text, exactly like the Python `CanonicalJsonObject`.
pub fn tool_call(
    call_id: &str,
    tool_id: &str,
    tool_version: &str,
    arguments_json: &str,
    requested_capabilities: &[&str],
    side_effect_level: &str,
    timeout_seconds: f64,
) -> kala_contracts::ToolCall {
    use kala_contracts::ContractModel;
    let mut entries = CanonicalObject::new();
    entries.insert(JsonText::new("call_id"), CanonicalValue::text(call_id));
    entries.insert(JsonText::new("tool_id"), CanonicalValue::text(tool_id));
    entries.insert(
        JsonText::new("tool_version"),
        CanonicalValue::text(tool_version),
    );
    entries.insert(
        JsonText::new("arguments_json"),
        CanonicalValue::text(arguments_json),
    );
    entries.insert(
        JsonText::new("requested_capabilities"),
        CanonicalValue::array(
            requested_capabilities
                .iter()
                .map(|value| CanonicalValue::text(*value))
                .collect::<Vec<_>>(),
        ),
    );
    entries.insert(
        JsonText::new("side_effect_level"),
        CanonicalValue::text(side_effect_level),
    );
    entries.insert(
        JsonText::new("timeout_seconds"),
        CanonicalValue::Float(timeout_seconds),
    );
    entries.insert(
        JsonText::new("issued_at"),
        CanonicalValue::text("2026-05-06T07:08:09+00:00"),
    );
    kala_contracts::ToolCall::from_value(&CanonicalValue::Object(entries)).expect("tool call")
}

/// The import root the Python worker reads the product from.
pub fn product_source_root() -> PathBuf {
    repo_root().join("src")
}

/// The interpreter layout of the live Python interpreter.
///
/// The strict jail binds the host paths the interpreter needs read-only. The
/// facts are read from the live interpreter, so the jail of the native
/// executor holds the same paths as the jail of the Python executor.
pub fn python_interpreter_layout() -> kala_sandbox::InterpreterLayout {
    let script = r#"
import json, os, sys, sysconfig
print(json.dumps({
    "executable": sys.executable,
    "prefix": sys.prefix,
    "base_prefix": sys.base_prefix,
    "exec_prefix": sys.exec_prefix,
    "base_exec_prefix": sys.base_exec_prefix,
    "module_directory": os.path.dirname(os.__file__),
    "stdlib": sysconfig.get_paths().get("stdlib"),
    "platstdlib": sysconfig.get_paths().get("platstdlib"),
    "purelib": sysconfig.get_paths().get("purelib"),
    "platlib": sysconfig.get_paths().get("platlib"),
}))
"#;
    let output = std::process::Command::new(python_executable())
        .arg("-I")
        .arg("-c")
        .arg(script)
        .output()
        .expect("run the interpreter");
    assert!(
        output.status.success(),
        "the interpreter layout failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let decoded = parse_strict(&output.stdout, &model_json_limits()).expect("interpreter layout");
    let CanonicalValue::Object(entries) = &decoded else {
        panic!("the interpreter layout is not an object");
    };
    let text = |name: &str| -> Option<String> {
        match entries.get(&JsonText::new(name)) {
            Some(CanonicalValue::Text(value)) => Some(value.to_string_lossy().to_owned()),
            _ => None,
        }
    };
    kala_sandbox::InterpreterLayout {
        executable: PathBuf::from(text("executable").expect("sys.executable")),
        prefix: PathBuf::from(text("prefix").expect("sys.prefix")),
        base_prefix: PathBuf::from(text("base_prefix").expect("sys.base_prefix")),
        exec_prefix: PathBuf::from(text("exec_prefix").expect("sys.exec_prefix")),
        base_exec_prefix: PathBuf::from(text("base_exec_prefix").expect("sys.base_exec_prefix")),
        module_directory: PathBuf::from(text("module_directory").expect("the module directory")),
        stdlib: text("stdlib").map(PathBuf::from),
        platstdlib: text("platstdlib").map(PathBuf::from),
        purelib: text("purelib").map(PathBuf::from),
        platlib: text("platlib").map(PathBuf::from),
    }
}
