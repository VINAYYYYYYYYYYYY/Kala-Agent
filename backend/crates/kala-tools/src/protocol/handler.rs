//! Tool handlers, the worker bootstrap and the child environment.
//!
//! A tool runs out of process. The handler describes how to start the worker,
//! the bootstrap reads one request from the worker's standard input and
//! writes one response to its standard output, and the child environment pins
//! every key that bounds the child so a tool can never be pointed at another
//! home directory, a host import path or a writable temporary location.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use kala_contracts::ToolDescriptor;

use crate::protocol::MAX_JSON_BYTES;
use crate::protocol::PROTOCOL_VERSION;
use crate::protocol::envelope::{ToolRequestEnvelope, ToolResponseEnvelope};
use crate::protocol::error::ToolProtocolError;
use crate::protocol::workspace::resolve_path;

/// The handler protocol.
///
/// A tool declares its descriptor and knows how to run one request.
pub trait ToolHandler {
    /// The descriptor of the tool this handler serves.
    fn descriptor(&self) -> &ToolDescriptor;

    /// Run one request.
    fn invoke(
        &self,
        request: &ToolRequestEnvelope,
    ) -> Result<ToolResponseEnvelope, ToolProtocolError>;
}

/// The bootstrap a worker process runs.
///
/// The text is the Python module's `WORKER_BOOTSTRAP` verbatim: the worker is
/// a Python process that imports the product's own protocol module.
pub const WORKER_BOOTSTRAP: &str = r#"import importlib
import sys
from kala_agent.tools.protocol import (
    ToolErrorCode,
    ToolResponseEnvelope,
    ToolRunStatus,
    read_request_stream,
    write_response_stream,
)
entry_point = sys.argv[1]
module_name, _, attribute = entry_point.partition(':')
module = importlib.import_module(module_name)
target = getattr(module, attribute)
request = read_request_stream(sys.stdin.buffer)
channel = sys.stdout
sys.stdout = sys.stderr
try:
    runner = target if callable(target) else getattr(target, 'invoke')
    envelope = runner(request)
except BaseException as error:
    envelope = ToolResponseEnvelope.failed_envelope(
        call_id=request.call_id,
        tool_id=request.tool_id,
        tool_version=request.tool_version,
        status=ToolRunStatus.FAILED,
        error_code=ToolErrorCode.INTERNAL_ERROR,
        error_message=type(error).__name__,
    )
sys.stdout = channel
write_response_stream(envelope, channel)"#;

/// The environment keys the executor owns.
///
/// A profile may not replace any of them.
pub const PROTECTED_CHILD_ENVIRONMENT_KEYS: [&str; 13] = [
    "HOME",
    "KALA_TOOL_PROTOCOL",
    "LANG",
    "PATH",
    "PYTHONDONTWRITEBYTECODE",
    "PYTHONHASHSEED",
    "PYTHONIOENCODING",
    "PYTHONPATH",
    "PYTHONNOUSERSITE",
    "PYTHONSAFEPATH",
    "TMP",
    "TMPDIR",
    "TEMP",
];

/// A handler that runs its tool in a child process.
#[derive(Clone, Debug, PartialEq)]
pub struct SubprocessToolHandler {
    descriptor: ToolDescriptor,
    entry_point: String,
    interpreter_args: Vec<String>,
}

impl SubprocessToolHandler {
    /// Build a subprocess handler, validating the entry point and the
    /// interpreter arguments.
    pub fn new(
        descriptor: ToolDescriptor,
        entry_point: &str,
        interpreter_args: &[&str],
    ) -> Result<Self, ToolProtocolError> {
        let (module_name, attribute) = entry_point.split_once(':').ok_or_else(|| {
            ToolProtocolError::envelope("entry_point must be formatted as module:attribute")
        })?;
        if module_name.is_empty() || attribute.is_empty() {
            return Err(ToolProtocolError::envelope(
                "entry_point must be formatted as module:attribute",
            ));
        }
        for part in module_name.split('.').chain(std::iter::once(attribute)) {
            if !is_python_identifier(part) {
                return Err(ToolProtocolError::envelope(
                    "entry_point components must be identifiers",
                ));
            }
        }
        for item in interpreter_args {
            if !item.starts_with('-') || matches!(*item, "-c" | "-E" | "-I" | "-m") {
                return Err(ToolProtocolError::envelope(
                    "interpreter_args cannot replace the worker bootstrap",
                ));
            }
        }
        Ok(Self {
            descriptor,
            entry_point: entry_point.to_owned(),
            interpreter_args: interpreter_args
                .iter()
                .map(|item| (*item).to_owned())
                .collect(),
        })
    }

    /// The descriptor of the tool.
    pub fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    /// The `module:attribute` entry point of the worker.
    pub fn entry_point(&self) -> &str {
        &self.entry_point
    }

    /// The interpreter arguments.
    pub fn interpreter_args(&self) -> &[String] {
        &self.interpreter_args
    }
}

impl ToolHandler for SubprocessToolHandler {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn invoke(
        &self,
        _request: &ToolRequestEnvelope,
    ) -> Result<ToolResponseEnvelope, ToolProtocolError> {
        Err(ToolProtocolError::protocol(
            "subprocess handlers are dispatched by the executor and cannot run in process",
        ))
    }
}

/// Build the command that starts a tool worker.
///
/// The Python implementation falls back to `sys.executable`; the Rust port has
/// no such interpreter, so the caller must name one. An empty or missing
/// interpreter is refused.
pub fn build_worker_command(
    handler: &SubprocessToolHandler,
    python_executable: Option<&str>,
) -> Result<Vec<String>, ToolProtocolError> {
    let Some(interpreter) = python_executable else {
        return Err(ToolProtocolError::protocol(
            "no python interpreter is available for the tool worker",
        ));
    };
    if interpreter.is_empty() {
        return Err(ToolProtocolError::protocol(
            "no python interpreter is available for the tool worker",
        ));
    }
    let mut command = vec![interpreter.to_owned()];
    command.extend(handler.interpreter_args().iter().cloned());
    command.push("-c".to_owned());
    command.push(WORKER_BOOTSTRAP.to_owned());
    command.push(handler.entry_point().to_owned());
    Ok(command)
}

/// Build the child's environment, refusing any override of a fixed key.
///
/// The executor owns every key that bounds the child: the working home, the
/// import roots, the loader path and the temporary directory. A profile may
/// not replace any of them, so a tool can never be pointed at another home
/// directory, a host import path or a writable temporary location.
pub fn build_child_environment(
    workspace: &Path,
    import_roots: &[&Path],
    extra: Option<&BTreeMap<String, String>>,
) -> Result<BTreeMap<String, String>, ToolProtocolError> {
    let resolved_workspace = resolve_path(workspace).to_string_lossy().replace('\\', "/");
    // The import roots keep the platform separator, exactly like the Python
    // `str(Path(root).resolve(strict=False))`.
    let roots: Vec<String> = import_roots
        .iter()
        .map(|root| resolve_path(root).to_string_lossy().into_owned())
        .collect();
    let mut environment = BTreeMap::new();
    environment.insert("HOME".to_owned(), resolved_workspace.clone());
    environment.insert("KALA_TOOL_PROTOCOL".to_owned(), PROTOCOL_VERSION.to_owned());
    environment.insert("LANG".to_owned(), "C.UTF-8".to_owned());
    environment.insert("PATH".to_owned(), "/usr/local/bin:/usr/bin:/bin".to_owned());
    environment.insert("PYTHONDONTWRITEBYTECODE".to_owned(), "1".to_owned());
    environment.insert("PYTHONHASHSEED".to_owned(), "0".to_owned());
    environment.insert("PYTHONIOENCODING".to_owned(), "utf-8".to_owned());
    environment.insert("PYTHONNOUSERSITE".to_owned(), "1".to_owned());
    environment.insert("PYTHONPATH".to_owned(), roots.join(":"));
    environment.insert("PYTHONSAFEPATH".to_owned(), "1".to_owned());
    environment.insert("TMPDIR".to_owned(), resolved_workspace);
    if let Some(extra) = extra {
        for (key, value) in extra {
            if !is_simple_environment_key(key) {
                return Err(ToolProtocolError::envelope(
                    "environment keys must be simple names",
                ));
            }
            if PROTECTED_CHILD_ENVIRONMENT_KEYS.contains(&key.as_str()) {
                return Err(ToolProtocolError::envelope(format!(
                    "environment key {key} is protected by the executor"
                )));
            }
            if value.contains('\u{0}') {
                return Err(ToolProtocolError::envelope(
                    "environment values must be null-free strings",
                ));
            }
            environment.insert(key.clone(), value.clone());
        }
    }
    Ok(environment)
}

/// Whether a name matches `^[A-Za-z_][A-Za-z0-9_]{0,63}$`.
fn is_simple_environment_key(key: &str) -> bool {
    let mut characters = key.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    if key.len() > 64 {
        return false;
    }
    characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

/// Whether a name is a valid Python identifier.
fn is_python_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !(first == '_' || first.is_alphabetic()) {
        return false;
    }
    characters.all(|character| character == '_' || character.is_alphanumeric())
}

/// Read one request envelope from a worker's standard input.
pub fn read_request_stream(
    stream: &mut impl Read,
) -> Result<ToolRequestEnvelope, ToolProtocolError> {
    let mut data = Vec::new();
    let limit = MAX_JSON_BYTES + 1;
    let mut buffer = [0u8; 8192];
    while data.len() < limit {
        let wanted = (limit - data.len()).min(buffer.len());
        let read = stream.read(&mut buffer[..wanted])?;
        if read == 0 {
            break;
        }
        data.extend_from_slice(&buffer[..read]);
    }
    if data.len() > MAX_JSON_BYTES {
        return Err(ToolProtocolError::envelope(
            "request envelope exceeds the bounded size",
        ));
    }
    ToolRequestEnvelope::from_json(&data)
}

/// Write one response envelope to a worker's standard output.
pub fn write_response_stream(
    envelope: &ToolResponseEnvelope,
    stream: &mut impl Write,
) -> Result<(), ToolProtocolError> {
    let mut payload = envelope.to_json()?;
    payload.push('\n');
    stream.write_all(payload.as_bytes())?;
    stream.flush()?;
    Ok(())
}
