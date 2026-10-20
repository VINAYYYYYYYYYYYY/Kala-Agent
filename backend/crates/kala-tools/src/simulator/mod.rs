//! The mechanical reference simulator.
//!
//! This module is the Rust port of `kala_agent.tools.simulator`. It owns the
//! deterministic closed form reference calculation for a prismatic bar, the
//! two tool descriptors that expose it, the handlers that run it, the
//! reference report it publishes and the misbehaviour entry points the
//! executor's security tests use.
//!
//! The simulator is an arithmetic reference, not an engineering validation.
//! It is the one tool the kernel ships, and the pure evaluator is the one
//! registration the registry accepts as a trusted in-process evaluator.

mod error;
mod parameters;
mod report;

use std::path::Path;
use std::sync::Arc;

use kala_contracts::{
    ArtifactClassification, ArtifactRef, ContractModel, ToolDescriptor, ToolRunStatus,
};
use kala_json::{CanonicalObject, CanonicalValue, JsonText};

use crate::protocol::{
    SubprocessToolHandler, ToolErrorCode, ToolHandler, ToolProtocolError, ToolRequestEnvelope,
    ToolResponseEnvelope, canonical_json_digest,
};
use crate::registry::{KERNEL_PACKAGE_DIRECTORY, RegistryHandler, ToolRegistry};

pub use error::SimulatorError;
pub use parameters::{
    MAX_REPORTED_MASS_KG, MAX_REPORTED_SLENDERNESS, MAX_REPORTED_VOLUME_MM3, OPERATIONS,
    SimulatorParameters, compute_metrics, parse_parameters, report_payload, within_limits,
};
pub use report::{write_report, write_workspace_file};

/// The identifier of the simulator tool.
pub const SIMULATOR_TOOL_ID: &str = "mechanical.reference-simulator";

/// The version of the simulator tool.
pub const SIMULATOR_TOOL_VERSION: &str = "1.0.0";

/// The worker entry point of the simulator.
///
/// The entry point names the Python worker of the Python implementation. The
/// Rust port keeps the name so the descriptor, the constants and the
/// differential stay comparable; the Rust executor maps the entry point to
/// its own worker.
pub const SIMULATOR_ENTRY_POINT: &str = "kala_agent.tools.simulator:invoke";

/// The directory the reference report is published in.
pub const SIMULATOR_REPORT_DIRECTORY: &str = "outputs";

/// The name of the reference report.
pub const SIMULATOR_REPORT_NAME: &str = "reference-report.json";

/// The capabilities the simulator requires.
pub const SIMULATOR_CAPABILITIES: [&str; 4] = [
    "artifact.read",
    "artifact.write",
    "design.read",
    "tool.execute",
];

/// The identifier of the pure reference evaluator.
pub const EVALUATOR_TOOL_ID: &str = "mechanical.reference-evaluator";

/// The version of the pure reference evaluator.
pub const EVALUATOR_TOOL_VERSION: &str = "1.0.0";

/// The capabilities the pure reference evaluator requires.
pub const EVALUATOR_CAPABILITIES: [&str; 0] = [];

/// The entry points of the misbehaviour tools.
///
/// The executor's security tests use them to prove that a slow, crashing,
/// flooding, lying, escaping or child-leaving tool is contained.
pub const MISBEHAVIOR_ENTRY_POINTS: [&str; 8] = [
    "kala_agent.tools.simulator:crash_invoke",
    "kala_agent.tools.simulator:escape_invoke",
    "kala_agent.tools.simulator:flood_invoke",
    "kala_agent.tools.simulator:hang_invoke",
    "kala_agent.tools.simulator:liar_invoke",
    "kala_agent.tools.simulator:lingering_child_invoke",
    "kala_agent.tools.simulator:stray_child_invoke",
    "kala_agent.tools.simulator:unrecognized_code_invoke",
];

/// The time a misbehaviour tool sleeps, in seconds.
pub const MISBEHAVIOR_SLEEP_SECONDS: f64 = 30.0;

/// The exit code a crashing tool leaves behind.
pub const MISBEHAVIOR_EXIT_CODE: i32 = 9;

/// The number of chunks a flooding tool writes.
pub const MISBEHAVIOR_FLOOD_CHUNKS: usize = 32;

/// The size of one chunk a flooding tool writes.
pub const MISBEHAVIOR_FLOOD_CHUNK_BYTES: usize = 65_536;

/// The name of the file an escaping tool writes outside its workspace.
pub const MISBEHAVIOR_ESCAPE_NAME: &str = "escaped-report.json";

/// The name of the file a stray child writes after the call reported.
pub const MISBEHAVIOR_STRAY_CHILD_NAME: &str = "stray-child-effect.txt";

/// The delay before a stray child writes its side effect, in seconds.
pub const MISBEHAVIOR_STRAY_CHILD_DELAY_SECONDS: f64 = 1.0;

/// The source of the stray child of the Python implementation.
///
/// The Rust port spawns a child of the host shell instead, because the crate
/// never depends on an interpreter.
pub const STRAY_CHILD_SOURCE: &str = r#"
import pathlib, sys, time
time.sleep(float(sys.argv[1]))
target = pathlib.Path(sys.argv[2])
try:
    target.write_bytes(b"late side effect")
except OSError:
    pass
"#;

/// The length of the error message a failing tool reports.
const MAX_ERROR_MESSAGE_LENGTH: usize = 1_024;

/// The input schema of the simulator and of the evaluator.
pub fn input_schema() -> CanonicalValue {
    let mut properties = CanonicalObject::new();
    properties.insert(
        JsonText::new("density_kg_m3"),
        number_schema(2_700.0, 20_000.0, 100.0),
    );
    properties.insert(JsonText::new("height_mm"), number_schema(10.0, 500.0, 1.0));
    properties.insert(
        JsonText::new("length_mm"),
        number_schema(120.0, 5_000.0, 10.0),
    );
    let mut operation = CanonicalObject::new();
    operation.insert(
        JsonText::new("default"),
        CanonicalValue::text("beam_volume"),
    );
    operation.insert(
        JsonText::new("enum"),
        CanonicalValue::array(OPERATIONS.iter().map(|name| CanonicalValue::text(*name))),
    );
    operation.insert(JsonText::new("type"), CanonicalValue::text("string"));
    properties.insert(
        JsonText::new("operation"),
        CanonicalValue::Object(operation),
    );
    properties.insert(JsonText::new("width_mm"), number_schema(20.0, 500.0, 1.0));
    let mut write_report = CanonicalObject::new();
    write_report.insert(JsonText::new("default"), CanonicalValue::Bool(false));
    write_report.insert(JsonText::new("type"), CanonicalValue::text("boolean"));
    properties.insert(
        JsonText::new("write_report"),
        CanonicalValue::Object(write_report),
    );
    let mut schema = CanonicalObject::new();
    schema.insert(
        JsonText::new("additionalProperties"),
        CanonicalValue::Bool(false),
    );
    schema.insert(
        JsonText::new("properties"),
        CanonicalValue::Object(properties),
    );
    schema.insert(
        JsonText::new("required"),
        CanonicalValue::array([CanonicalValue::text("operation")]),
    );
    schema.insert(
        JsonText::new("title"),
        CanonicalValue::text("mechanical.reference-simulator.input"),
    );
    schema.insert(JsonText::new("type"), CanonicalValue::text("object"));
    CanonicalValue::Object(schema)
}

/// The output schema of the simulator and of the evaluator.
pub fn output_schema() -> CanonicalValue {
    let mut properties = CanonicalObject::new();
    for name in [
        "mass_kg",
        "second_moment_mm4",
        "slenderness_ratio",
        "surface_area_mm2",
        "volume_mm3",
    ] {
        let mut property = CanonicalObject::new();
        property.insert(JsonText::new("type"), CanonicalValue::text("number"));
        properties.insert(JsonText::new(name), CanonicalValue::Object(property));
    }
    let mut operation = CanonicalObject::new();
    operation.insert(
        JsonText::new("enum"),
        CanonicalValue::array(OPERATIONS.iter().map(|name| CanonicalValue::text(*name))),
    );
    operation.insert(JsonText::new("type"), CanonicalValue::text("string"));
    properties.insert(
        JsonText::new("operation"),
        CanonicalValue::Object(operation),
    );
    let mut report_path = CanonicalObject::new();
    report_path.insert(
        JsonText::new("type"),
        CanonicalValue::array([CanonicalValue::text("string"), CanonicalValue::text("null")]),
    );
    properties.insert(
        JsonText::new("report_path"),
        CanonicalValue::Object(report_path),
    );
    let mut within_limits = CanonicalObject::new();
    within_limits.insert(JsonText::new("type"), CanonicalValue::text("boolean"));
    properties.insert(
        JsonText::new("within_limits"),
        CanonicalValue::Object(within_limits),
    );
    let mut schema = CanonicalObject::new();
    schema.insert(
        JsonText::new("additionalProperties"),
        CanonicalValue::Bool(false),
    );
    schema.insert(
        JsonText::new("properties"),
        CanonicalValue::Object(properties),
    );
    schema.insert(
        JsonText::new("required"),
        CanonicalValue::array([
            CanonicalValue::text("mass_kg"),
            CanonicalValue::text("operation"),
            CanonicalValue::text("report_path"),
            CanonicalValue::text("second_moment_mm4"),
            CanonicalValue::text("slenderness_ratio"),
            CanonicalValue::text("surface_area_mm2"),
            CanonicalValue::text("volume_mm3"),
            CanonicalValue::text("within_limits"),
        ]),
    );
    schema.insert(
        JsonText::new("title"),
        CanonicalValue::text("mechanical.reference-simulator.output"),
    );
    schema.insert(JsonText::new("type"), CanonicalValue::text("object"));
    CanonicalValue::Object(schema)
}

/// One numeric property of the input schema.
fn number_schema(default: f64, maximum: f64, minimum: f64) -> CanonicalValue {
    let mut property = CanonicalObject::new();
    property.insert(JsonText::new("default"), CanonicalValue::Float(default));
    property.insert(JsonText::new("maximum"), CanonicalValue::Float(maximum));
    property.insert(JsonText::new("minimum"), CanonicalValue::Float(minimum));
    property.insert(JsonText::new("type"), CanonicalValue::text("number"));
    CanonicalValue::Object(property)
}

/// The descriptor of the simulator tool.
///
/// The simulator writes a report, so it declares a local write and the
/// capabilities that write needs.
pub fn simulator_descriptor(
    tool_id: &str,
    version: &str,
) -> Result<ToolDescriptor, SimulatorError> {
    descriptor_value(
        tool_id,
        version,
        "Mechanical reference simulator",
        "Deterministic closed-form reference calculation for a prismatic bar. \
         It is an arithmetic reference, not an engineering validation.",
        "local_write",
        &SIMULATOR_CAPABILITIES,
        20.0,
    )
}

/// The handler of the simulator tool.
///
/// The tool runs out of process, like every tool that may have a side
/// effect.
pub fn simulator_handler(
    tool_id: &str,
    version: &str,
) -> Result<SubprocessToolHandler, SimulatorError> {
    let descriptor = simulator_descriptor(tool_id, version)?;
    SubprocessToolHandler::new(descriptor, SIMULATOR_ENTRY_POINT, &["-B", "-s"])
        .map_err(|error| SimulatorError::Value(error.to_string()))
}

/// The descriptor of the pure reference evaluator.
///
/// The evaluator describes the same arithmetic as a pure, side-effect free
/// evaluator. It is the one registration the registry accepts as a trusted
/// in-process evaluator: it is shipped with the kernel, it declares no side
/// effects, and it writes nothing. It is the only way a tool may be invoked
/// inside the kernel process, and only under the explicit trusted mode.
pub fn evaluator_descriptor(
    tool_id: &str,
    version: &str,
) -> Result<ToolDescriptor, SimulatorError> {
    descriptor_value(
        tool_id,
        version,
        "Mechanical reference evaluator",
        "Deterministic closed-form arithmetic with no filesystem, network, or \
         process authority. It is a reference calculation, not a validation.",
        "none",
        &EVALUATOR_CAPABILITIES,
        10.0,
    )
}

/// Build a tool descriptor.
fn descriptor_value(
    tool_id: &str,
    version: &str,
    name: &str,
    description: &str,
    side_effect_level: &str,
    required_capabilities: &[&str],
    timeout_seconds: f64,
) -> Result<ToolDescriptor, SimulatorError> {
    let mut entries = CanonicalObject::new();
    entries.insert(JsonText::new("tool_id"), CanonicalValue::text(tool_id));
    entries.insert(JsonText::new("version"), CanonicalValue::text(version));
    entries.insert(JsonText::new("name"), CanonicalValue::text(name));
    entries.insert(
        JsonText::new("description"),
        CanonicalValue::text(description),
    );
    entries.insert(
        JsonText::new("input_schema_digest"),
        CanonicalValue::text(digest_of(&input_schema())?),
    );
    entries.insert(
        JsonText::new("output_schema_digest"),
        CanonicalValue::text(digest_of(&output_schema())?),
    );
    entries.insert(
        JsonText::new("side_effect_level"),
        CanonicalValue::text(side_effect_level),
    );
    entries.insert(
        JsonText::new("required_capabilities"),
        CanonicalValue::array(
            required_capabilities
                .iter()
                .map(|capability| CanonicalValue::text(*capability)),
        ),
    );
    entries.insert(
        JsonText::new("timeout_seconds"),
        CanonicalValue::Float(timeout_seconds),
    );
    entries.insert(JsonText::new("destructive"), CanonicalValue::Bool(false));
    entries.insert(
        JsonText::new("supports_structured_output"),
        CanonicalValue::Bool(true),
    );
    entries.insert(JsonText::new("idempotent"), CanonicalValue::Bool(true));
    ToolDescriptor::from_value(&CanonicalValue::Object(entries))
        .map_err(|error| SimulatorError::contract(&error))
}

/// The canonical digest of a JSON value.
fn digest_of(value: &CanonicalValue) -> Result<String, SimulatorError> {
    canonical_json_digest(value).map_err(|error| SimulatorError::Json(error.to_string()))
}

/// The handler of the simulator tool, as a value.
#[derive(Clone, Debug)]
pub struct SimulatorToolHandler {
    descriptor: ToolDescriptor,
}

impl SimulatorToolHandler {
    /// Build the handler of the simulator tool.
    pub fn new(tool_id: &str, version: &str) -> Result<Self, SimulatorError> {
        Ok(Self {
            descriptor: simulator_descriptor(tool_id, version)?,
        })
    }
}

impl ToolHandler for SimulatorToolHandler {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn invoke(
        &self,
        request: &ToolRequestEnvelope,
    ) -> Result<ToolResponseEnvelope, ToolProtocolError> {
        simulate(request)
    }
}

impl RegistryHandler for SimulatorToolHandler {
    fn defining_package(&self) -> &'static str {
        KERNEL_PACKAGE_DIRECTORY
    }
}

/// The handler of the pure reference evaluator, as a value.
#[derive(Clone, Debug)]
pub struct ReferenceEvaluatorHandler {
    descriptor: ToolDescriptor,
}

impl ReferenceEvaluatorHandler {
    /// Build the handler of the pure reference evaluator.
    pub fn new(tool_id: &str, version: &str) -> Result<Self, SimulatorError> {
        Ok(Self {
            descriptor: evaluator_descriptor(tool_id, version)?,
        })
    }
}

impl ToolHandler for ReferenceEvaluatorHandler {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn invoke(
        &self,
        request: &ToolRequestEnvelope,
    ) -> Result<ToolResponseEnvelope, ToolProtocolError> {
        evaluate(request)
    }
}

impl RegistryHandler for ReferenceEvaluatorHandler {
    fn defining_package(&self) -> &'static str {
        KERNEL_PACKAGE_DIRECTORY
    }
}

/// Run one simulator call.
pub fn invoke(request: &ToolRequestEnvelope) -> Result<ToolResponseEnvelope, ToolProtocolError> {
    simulate(request)
}

/// Run one simulator call.
fn simulate(request: &ToolRequestEnvelope) -> Result<ToolResponseEnvelope, ToolProtocolError> {
    let arguments = CanonicalValue::Object(request.arguments().clone());
    let parameters = match parse_parameters(&arguments) {
        Ok(parameters) => parameters,
        Err(error) => {
            return failed(
                request,
                ToolErrorCode::InvalidArguments,
                &truncate(&error.to_string()),
            );
        }
    };
    let metrics = compute_metrics(&parameters);
    let mut artifacts: Vec<ArtifactRef> = Vec::new();
    let mut report_path: Option<String> = None;
    if parameters.write_report() {
        match write_report(
            Path::new(request.workspace()),
            &parameters,
            &metrics,
            request.call_id(),
        ) {
            Ok((artifact, relative)) => {
                artifacts.push(artifact);
                report_path = Some(relative);
            }
            Err(error) => {
                return failed(
                    request,
                    ToolErrorCode::WorkspaceViolation,
                    &truncate(&format!("{}: {}", error.python_class_name(), error)),
                );
            }
        }
    }
    let mut result = CanonicalObject::new();
    result.insert(
        JsonText::new("operation"),
        CanonicalValue::text(parameters.operation()),
    );
    result.insert(
        JsonText::new("report_path"),
        match &report_path {
            None => CanonicalValue::Null,
            Some(path) => CanonicalValue::text(path),
        },
    );
    if let CanonicalValue::Object(metrics) = &metrics {
        for (key, value) in metrics {
            result.insert(key.clone(), value.clone());
        }
    }
    ToolResponseEnvelope::succeeded_envelope(
        request.call_id(),
        request.tool_id(),
        request.tool_version(),
        &result,
        &artifacts,
        0.0,
    )
}

/// Run one evaluator call.
fn evaluate(request: &ToolRequestEnvelope) -> Result<ToolResponseEnvelope, ToolProtocolError> {
    let arguments = request.arguments();
    if arguments.get(&JsonText::new("write_report")) == Some(&CanonicalValue::Bool(true)) {
        return failed(
            request,
            ToolErrorCode::WorkspaceViolation,
            "the evaluator cannot write a report",
        );
    }
    let parameters = match parse_parameters(&CanonicalValue::Object(arguments.clone())) {
        Ok(parameters) => parameters,
        Err(error) => {
            return failed(
                request,
                ToolErrorCode::InvalidArguments,
                &truncate(&error.to_string()),
            );
        }
    };
    let metrics = compute_metrics(&parameters);
    let mut result = CanonicalObject::new();
    result.insert(
        JsonText::new("operation"),
        CanonicalValue::text(parameters.operation()),
    );
    result.insert(JsonText::new("report_path"), CanonicalValue::Null);
    if let CanonicalValue::Object(metrics) = &metrics {
        for (key, value) in metrics {
            result.insert(key.clone(), value.clone());
        }
    }
    ToolResponseEnvelope::succeeded_envelope(
        request.call_id(),
        request.tool_id(),
        request.tool_version(),
        &result,
        &[],
        0.0,
    )
}

/// Build a failed response envelope.
fn failed(
    request: &ToolRequestEnvelope,
    error_code: ToolErrorCode,
    error_message: &str,
) -> Result<ToolResponseEnvelope, ToolProtocolError> {
    ToolResponseEnvelope::failed_envelope_with_code(
        request.call_id(),
        request.tool_id(),
        request.tool_version(),
        ToolRunStatus::Failed,
        error_code,
        Some(error_message),
        0.0,
    )
}

/// Truncate an error message to the length the protocol accepts.
fn truncate(message: &str) -> String {
    message.chars().take(MAX_ERROR_MESSAGE_LENGTH).collect()
}

/// Sleep for the misbehaviour delay.
///
/// The unit tests shorten the delay so the suite stays fast.
fn misbehavior_sleep() {
    #[cfg(test)]
    {
        std::thread::sleep(std::time::Duration::from_millis(
            crate::simulator::test_sleep_millis(),
        ));
    }
    #[cfg(not(test))]
    {
        std::thread::sleep(std::time::Duration::from_secs_f64(
            MISBEHAVIOR_SLEEP_SECONDS,
        ));
    }
}

/// A misbehaviour tool that never answers.
pub fn hang_invoke(
    request: &ToolRequestEnvelope,
) -> Result<ToolResponseEnvelope, ToolProtocolError> {
    misbehavior_sleep();
    simulate(request)
}

/// A misbehaviour tool that crashes its worker.
pub fn crash_invoke(_request: &ToolRequestEnvelope) -> ! {
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    std::process::exit(MISBEHAVIOR_EXIT_CODE);
}

/// A misbehaviour tool that floods the output channel.
pub fn flood_invoke(
    request: &ToolRequestEnvelope,
) -> Result<ToolResponseEnvelope, ToolProtocolError> {
    use std::io::Write as _;
    let chunk = vec![b'x'; MISBEHAVIOR_FLOOD_CHUNK_BYTES];
    let mut stdout = std::io::stdout().lock();
    for _ in 0..MISBEHAVIOR_FLOOD_CHUNKS {
        let _ = stdout.write_all(&chunk);
    }
    let _ = stdout.flush();
    simulate(request)
}

/// A misbehaviour tool that reports a result its schema does not allow.
pub fn liar_invoke(
    request: &ToolRequestEnvelope,
) -> Result<ToolResponseEnvelope, ToolProtocolError> {
    let mut result = CanonicalObject::new();
    result.insert(
        JsonText::new("within_limits"),
        CanonicalValue::text("not-a-boolean"),
    );
    ToolResponseEnvelope::succeeded_envelope(
        request.call_id(),
        request.tool_id(),
        request.tool_version(),
        &result,
        &[],
        0.0,
    )
}

/// A misbehaviour tool that reports a failure with an unknown error code.
pub fn unrecognized_code_invoke(
    request: &ToolRequestEnvelope,
) -> Result<ToolResponseEnvelope, ToolProtocolError> {
    ToolResponseEnvelope::failed_envelope(
        request.call_id(),
        request.tool_id(),
        request.tool_version(),
        ToolRunStatus::Failed,
        Some("totally_fine_code"),
        Some("reported under a code the kernel does not recognise"),
        0.0,
    )
}

/// A misbehaviour tool that leaves a grandchild behind that writes a side
/// effect long after the call.
///
/// The grandchild stays in the worker's own process group, so a supervisor
/// that only kills its direct child, or that leaves the group running, lets
/// the side effect land after the call has already been reported as timed
/// out.
pub fn stray_child_invoke(
    request: &ToolRequestEnvelope,
) -> Result<ToolResponseEnvelope, ToolProtocolError> {
    spawn_stray_child(Path::new(request.workspace()));
    misbehavior_sleep();
    simulate(request)
}

/// A misbehaviour tool that echoes the arguments at once while a grandchild
/// in the worker's group keeps running.
pub fn lingering_child_invoke(
    request: &ToolRequestEnvelope,
) -> Result<ToolResponseEnvelope, ToolProtocolError> {
    spawn_stray_child(Path::new(request.workspace()));
    ToolResponseEnvelope::succeeded_envelope(
        request.call_id(),
        request.tool_id(),
        request.tool_version(),
        request.arguments(),
        &[],
        0.0,
    )
}

/// A misbehaviour tool that writes outside its workspace.
pub fn escape_invoke(
    request: &ToolRequestEnvelope,
) -> Result<ToolResponseEnvelope, ToolProtocolError> {
    let workspace = Path::new(request.workspace());
    let target = match workspace.parent() {
        Some(parent) => parent.join(MISBEHAVIOR_ESCAPE_NAME),
        None => Path::new(MISBEHAVIOR_ESCAPE_NAME).to_path_buf(),
    };
    let payload = b"{\"escaped\": true}\n";
    let _ = std::fs::write(&target, payload);
    let uri = format!("file://{}", target.to_string_lossy().replace('\\', "/"));
    let artifact = match ArtifactRef::from_value(&CanonicalValue::Object({
        let mut entries = CanonicalObject::new();
        entries.insert(
            JsonText::new("artifact_id"),
            CanonicalValue::text(format!("{}-escape", request.call_id())),
        );
        entries.insert(JsonText::new("uri"), CanonicalValue::text(uri));
        entries.insert(
            JsonText::new("digest"),
            CanonicalValue::text(crate::protocol::sha256_hex(payload)),
        );
        entries.insert(
            JsonText::new("media_type"),
            CanonicalValue::text("application/json"),
        );
        entries.insert(
            JsonText::new("size_bytes"),
            CanonicalValue::integer(payload.len() as i64),
        );
        entries.insert(
            JsonText::new("classification"),
            CanonicalValue::text(ArtifactClassification::DerivedData.as_str()),
        );
        entries
    })) {
        Ok(artifact) => artifact,
        Err(error) => {
            return failed(
                request,
                ToolErrorCode::InternalError,
                &truncate(&error.to_string()),
            );
        }
    };
    let mut result = CanonicalObject::new();
    result.insert(
        JsonText::new("operation"),
        CanonicalValue::text("beam_volume"),
    );
    result.insert(
        JsonText::new("report_path"),
        CanonicalValue::text(format!("../{MISBEHAVIOR_ESCAPE_NAME}")),
    );
    result.insert(JsonText::new("mass_kg"), CanonicalValue::Float(0.0));
    result.insert(
        JsonText::new("second_moment_mm4"),
        CanonicalValue::Float(0.0),
    );
    result.insert(
        JsonText::new("slenderness_ratio"),
        CanonicalValue::Float(0.0),
    );
    result.insert(
        JsonText::new("surface_area_mm2"),
        CanonicalValue::Float(0.0),
    );
    result.insert(JsonText::new("volume_mm3"), CanonicalValue::Float(0.0));
    result.insert(JsonText::new("within_limits"), CanonicalValue::Bool(true));
    let artifacts = vec![artifact];
    ToolResponseEnvelope::succeeded_envelope(
        request.call_id(),
        request.tool_id(),
        request.tool_version(),
        &result,
        &artifacts,
        0.0,
    )
}

/// Leave a grandchild behind that writes a side effect later.
///
/// The Python implementation spawns `sys.executable` with a small script.
/// The Rust port spawns a child of the host shell, because the crate never
/// depends on an interpreter.
fn spawn_stray_child(workspace: &Path) {
    use std::process::{Command, Stdio};
    let target = workspace.join(MISBEHAVIOR_STRAY_CHILD_NAME);
    #[cfg(unix)]
    {
        let script = format!(
            "sleep {}; printf 'late side effect' > {}",
            shell_delay(MISBEHAVIOR_STRAY_CHILD_DELAY_SECONDS),
            shell_quote(&target.to_string_lossy())
        );
        let _ = Command::new("sh")
            .arg("-c")
            .arg(script)
            .current_dir(workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    #[cfg(windows)]
    {
        let script = format!(
            "ping -n 2 127.0.0.1 > nul & echo late side effect>\"{}\"",
            target.display()
        );
        let _ = Command::new("cmd")
            .arg("/C")
            .arg(script)
            .current_dir(workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

/// Quote a value for a POSIX shell.
#[cfg(unix)]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Render a delay in seconds for a shell that sleeps in whole seconds.
#[cfg(unix)]
fn shell_delay(seconds: f64) -> String {
    if seconds.fract() == 0.0 {
        format!("{}", seconds as i64)
    } else {
        format!("{seconds:.3}")
    }
}

/// Register the simulator in a registry.
///
/// The failure is a registry failure, exactly like the Python implementation.
pub fn register_simulator(
    registry: &ToolRegistry,
    tool_id: &str,
    version: &str,
) -> Result<ToolDescriptor, SimulatorError> {
    let descriptor = simulator_descriptor(tool_id, version)?;
    let handler = simulator_handler(tool_id, version)?;
    registry.register(
        descriptor.clone(),
        Arc::new(handler),
        input_schema(),
        output_schema(),
    )?;
    Ok(descriptor)
}

/// Register the pure evaluator in a registry.
///
/// The failure is a registry failure, exactly like the Python implementation.
pub fn register_reference_evaluator(
    registry: &ToolRegistry,
    tool_id: &str,
    version: &str,
) -> Result<ToolDescriptor, SimulatorError> {
    let descriptor = evaluator_descriptor(tool_id, version)?;
    let handler = ReferenceEvaluatorHandler::new(tool_id, version)?;
    registry.register_trusted_evaluator(
        descriptor.clone(),
        Arc::new(handler),
        input_schema(),
        output_schema(),
    )?;
    Ok(descriptor)
}

/// The delay the misbehaviour tools sleep, in milliseconds.
///
/// The unit tests shorten the delay so the suite stays fast.
#[cfg(test)]
static TEST_SLEEP_MILLIS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(u64::MAX);

/// The delay the misbehaviour tools sleep, in milliseconds.
#[cfg(test)]
pub(crate) fn test_sleep_millis() -> u64 {
    use std::sync::atomic::Ordering;
    let value = TEST_SLEEP_MILLIS.load(Ordering::Relaxed);
    if value == u64::MAX {
        (MISBEHAVIOR_SLEEP_SECONDS * 1_000.0) as u64
    } else {
        value
    }
}

/// Shorten the delay the misbehaviour tools sleep.
///
/// The unit tests call this so the suite stays fast.
#[cfg(test)]
pub(crate) fn set_test_sleep_millis(millis: u64) {
    use std::sync::atomic::Ordering;
    TEST_SLEEP_MILLIS.store(millis, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use kala_json::{CanonicalObject, CanonicalValue, JsonText};

    use super::*;

    /// A request of the unit tests.
    fn request(workspace: &Path) -> ToolRequestEnvelope {
        ToolRequestEnvelope::new(
            crate::protocol::PROTOCOL_VERSION,
            "call-1",
            SIMULATOR_TOOL_ID,
            SIMULATOR_TOOL_VERSION,
            &CanonicalObject::new(),
            &workspace.to_string_lossy(),
            30.0,
            &num_bigint::BigInt::from(1_000_000),
            &num_bigint::BigInt::from(1_000_000),
            &[],
        )
        .expect("request")
    }

    #[test]
    fn the_hang_entry_point_answers_after_the_delay() {
        set_test_sleep_millis(1);
        let workspace = std::env::temp_dir().join("kala-tools-hang");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let response = hang_invoke(&request(&workspace)).expect("response");
        assert!(response.succeeded());
    }

    #[test]
    fn the_evaluator_never_writes() {
        let workspace = std::env::temp_dir().join("kala-tools-evaluator");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let handler = ReferenceEvaluatorHandler::new(EVALUATOR_TOOL_ID, EVALUATOR_TOOL_VERSION)
            .expect("handler");
        let response =
            crate::protocol::ToolHandler::invoke(&handler, &request(&workspace)).expect("response");
        assert!(response.succeeded());
        assert!(response.artifacts().is_empty());
        assert!(!workspace.join(SIMULATOR_REPORT_DIRECTORY).exists());
    }

    #[test]
    fn the_schemas_are_admitted_object_schemas() {
        crate::protocol::check_object_schema(&input_schema(), "input_schema")
            .expect("input schema");
        crate::protocol::check_object_schema(&output_schema(), "output_schema")
            .expect("output schema");
        let digest = canonical_json_digest(&input_schema()).expect("digest");
        assert_eq!(digest.len(), 64);
    }

    #[test]
    fn the_evaluator_descriptor_declares_no_capability() {
        let descriptor =
            evaluator_descriptor(EVALUATOR_TOOL_ID, EVALUATOR_TOOL_VERSION).expect("descriptor");
        assert!(descriptor.required_capabilities().is_empty());
        assert_eq!(descriptor.side_effect_level().as_str(), "none");
    }

    #[test]
    fn the_simulator_descriptor_declares_its_capabilities() {
        let descriptor =
            simulator_descriptor(SIMULATOR_TOOL_ID, SIMULATOR_TOOL_VERSION).expect("descriptor");
        assert_eq!(descriptor.required_capabilities().len(), 4);
        assert_eq!(descriptor.side_effect_level().as_str(), "local_write");
        assert_eq!(
            descriptor.input_schema_digest().to_hex(),
            canonical_json_digest(&input_schema()).expect("digest")
        );
    }

    #[test]
    fn the_input_schema_declares_the_operations() {
        let schema = input_schema();
        let CanonicalValue::Object(ref schema) = schema else {
            panic!("the schema is an object");
        };
        let properties = schema
            .get(&JsonText::new("properties"))
            .expect("properties");
        let CanonicalValue::Object(properties) = properties else {
            panic!("the properties are an object");
        };
        let operation = properties
            .get(&JsonText::new("operation"))
            .expect("operation");
        let CanonicalValue::Object(operation) = operation else {
            panic!("the operation is an object");
        };
        assert_eq!(
            operation.get(&JsonText::new("enum")),
            Some(&CanonicalValue::array([
                CanonicalValue::text("beam_volume"),
                CanonicalValue::text("bar_summary"),
            ]))
        );
    }
}
