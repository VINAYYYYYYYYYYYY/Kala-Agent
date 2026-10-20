//! The tool registry.
//!
//! This module is the Rust port of `kala_agent.tools.registry`. It binds a
//! `(tool_id, version)` pair to a descriptor, a handler and the two schemas
//! of a tool, freezes the schemas, recomputes the schema digests, and
//! re-verifies the immutability of a registration on every lookup.
//!
//! The public names match the Python module one for one, so a caller can read
//! the Python implementation and the Rust implementation side by side.
//!
//! # The handler contract
//!
//! Python accepts any object that satisfies the runtime checkable
//! `ToolHandler` protocol, so the registry holds a handler it cannot name and
//! answers two questions about it at runtime:
//!
//! * "handler must implement the tool protocol" — the Python check
//!   `isinstance(self.handler, ToolHandler)`.
//! * "handler must expose a ToolDescriptor" — the Python check
//!   `isinstance(getattr(self.handler, "descriptor", None), ToolDescriptor)`.
//!
//! Rust answers both with the type system: [`RegistryHandler`] extends the
//! protocol's [`ToolHandler`] trait, so a value that is registered always
//! implements the tool protocol and always exposes a descriptor.
//!
//! # The provenance of a handler
//!
//! Python decides whether a handler is first party by inspecting the module
//! that defines it and the file that module was loaded from, and by refusing
//! a handler whose file does not resolve inside the kernel package. Rust
//! cannot inspect the origin of a type at runtime, so a handler reports the
//! package that defines it through
//! [`RegistryHandler::defining_package`] and the registry compares that
//! report with its own package directory. A handler that is defined outside
//! the kernel package therefore reports a different directory and is refused
//! the trusted in-process privilege.
//!
//! # The immutability of a registration
//!
//! Python registers a tool in a frozen dataclass that `object.__setattr__`
//! can still mutate, and it re-verifies the descriptor and the two schemas on
//! every lookup. The Rust registration owns immutable values in private
//! fields, so a mutation is structurally impossible; the re-verification is
//! kept as defence in depth and is exercised by the crate's unit tests.

mod error;
mod version;

use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use kala_contracts::{ContractModel, SideEffectLevel, ToolDescriptor};
use kala_json::CanonicalValue;

use crate::protocol::{
    SubprocessToolHandler, ToolHandler, ToolProtocolError, ToolRequestEnvelope,
    ToolResponseEnvelope, canonical_json_digest, canonical_json_dumps, check_object_schema,
    validate_against_schema,
};

pub use error::ToolRegistryError;
pub use version::{VersionKey, VersionSegment, version_key};

/// The maximum number of tools a registry may hold.
pub const MAX_REGISTERED_TOOLS: usize = 512;

/// The Python package that ships the kernel.
///
/// The Rust port keeps the name for parity with the Python module. The
/// provenance check itself compares the directory of the defining package,
/// because Rust has no runtime module introspection.
pub const FIRST_PARTY_PACKAGE: &str = "kala_agent";

/// The directory of the package that defines this crate.
///
/// A handler is first party when it reports this directory.
pub const KERNEL_PACKAGE_DIRECTORY: &str = env!("CARGO_MANIFEST_DIR");

/// The capabilities a trusted in-process evaluator must not declare.
///
/// A tool that needs one of these has authority the kernel process must not
/// hold, so it can never be marked as a trusted evaluator.
const TRUSTED_EVALUATOR_CAPABILITIES: [&str; 9] = [
    "egress",
    "export",
    "network",
    "network.access",
    "network.egress",
    "network.internet",
    "process.spawn",
    "shell.execute",
    "system.admin",
];

/// The maximum length of a tool identifier, in code points.
const MAX_IDENTIFIER_LENGTH: usize = 128;

/// The maximum length of a tool version, in code points.
const MAX_VERSION_LENGTH: usize = 64;

/// The handler contract of the tool registry.
///
/// The trait extends the protocol's [`ToolHandler`] trait, so a registered
/// handler always implements the tool protocol and always exposes a
/// descriptor.
pub trait RegistryHandler: ToolHandler + Send + Sync {
    /// The descriptor the handler serves right now.
    ///
    /// The Python registry reads `handler.descriptor` again on every
    /// verification, so a handler that changes its descriptor after the
    /// registration is detected. A Rust handler cannot hand out a mutable
    /// reference through a shared reference, so the registry reads a snapshot
    /// of the descriptor instead. The default implementation returns the
    /// descriptor of the protocol trait, which cannot change.
    fn descriptor_snapshot(&self) -> ToolDescriptor {
        self.descriptor().clone()
    }

    /// The directory of the package that defines this handler.
    ///
    /// The default is the empty string, which is never the kernel package, so
    /// a handler is foreign unless it says otherwise.
    fn defining_package(&self) -> &'static str {
        ""
    }

    /// Whether the handler runs its tool in a child process.
    ///
    /// The Python implementation checks `isinstance(handler,
    /// SubprocessToolHandler)`. A subprocess handler can never be marked as a
    /// trusted in-process evaluator.
    fn is_subprocess(&self) -> bool {
        false
    }

    /// The subprocess handler behind this registration, when there is one.
    ///
    /// The executor builds the worker command from the entry point of the
    /// handler, so it needs the handler itself and not only the flag. The
    /// default is `None`, so a handler that is not a subprocess handler never
    /// claims one.
    fn subprocess_handler(&self) -> Option<&SubprocessToolHandler> {
        None
    }
}

impl<T: RegistryHandler + ?Sized> ToolHandler for Arc<T> {
    fn descriptor(&self) -> &ToolDescriptor {
        ToolHandler::descriptor(&**self)
    }

    fn invoke(
        &self,
        request: &ToolRequestEnvelope,
    ) -> Result<ToolResponseEnvelope, ToolProtocolError> {
        ToolHandler::invoke(&**self, request)
    }
}

/// The protocol's subprocess handler is a registry handler as well.
///
/// The Python implementation recognises it with
/// `isinstance(handler, SubprocessToolHandler)`, and a subprocess handler can
/// never run inside the kernel process.
impl RegistryHandler for SubprocessToolHandler {
    fn descriptor_snapshot(&self) -> ToolDescriptor {
        self.descriptor().clone()
    }

    fn is_subprocess(&self) -> bool {
        true
    }

    fn subprocess_handler(&self) -> Option<&SubprocessToolHandler> {
        Some(self)
    }
}

/// The registered description of a tool.
///
/// The registration owns the descriptor, the handler and the two schemas. The
/// schemas are deep frozen and their digests are recomputed, so a schema that
/// does not match the digest the descriptor declares is refused.
#[derive(Clone)]
pub struct RegisteredTool {
    descriptor: ToolDescriptor,
    handler: Arc<dyn RegistryHandler>,
    input_schema: CanonicalValue,
    output_schema: CanonicalValue,
    input_schema_digest: String,
    output_schema_digest: String,
    descriptor_digest: String,
    trusted_in_process: bool,
}

impl std::fmt::Debug for RegisteredTool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RegisteredTool")
            .field("descriptor", &self.descriptor)
            .field("input_schema_digest", &self.input_schema_digest)
            .field("output_schema_digest", &self.output_schema_digest)
            .field("descriptor_digest", &self.descriptor_digest)
            .field("trusted_in_process", &self.trusted_in_process)
            .finish()
    }
}

impl RegisteredTool {
    /// Build a registration, validating every field.
    ///
    /// The checks are the ones of the Python `__post_init__`, in the same
    /// order: the handler must expose the same descriptor as the
    /// registration, the schemas must be JSON objects, the schema digests
    /// must match the schemas and a trusted in-process registration must be a
    /// first-party, pure evaluator.
    pub fn new(
        descriptor: ToolDescriptor,
        handler: Arc<dyn RegistryHandler>,
        input_schema: CanonicalValue,
        output_schema: CanonicalValue,
        input_schema_digest: impl Into<String>,
        output_schema_digest: impl Into<String>,
    ) -> Result<Self, ToolRegistryError> {
        let mut tool = Self {
            descriptor,
            handler,
            input_schema,
            output_schema,
            input_schema_digest: input_schema_digest.into(),
            output_schema_digest: output_schema_digest.into(),
            descriptor_digest: String::new(),
            trusted_in_process: false,
        };
        tool.validate()?;
        Ok(tool)
    }

    /// Validate the registration, exactly like the Python `__post_init__`.
    fn validate(&mut self) -> Result<(), ToolRegistryError> {
        if self.handler.descriptor_snapshot() != self.descriptor {
            return Err(ToolRegistryError::registration(
                "handler descriptor does not match the registered descriptor",
            ));
        }
        if !matches!(self.input_schema, CanonicalValue::Object(_))
            || !matches!(self.output_schema, CanonicalValue::Object(_))
        {
            return Err(ToolRegistryError::registration(
                "schemas must be JSON objects",
            ));
        }
        let expected_input = digest_of(&self.input_schema)?;
        if self.input_schema_digest != expected_input {
            return Err(ToolRegistryError::schema_digest_mismatch(
                "input schema digest does not match the descriptor",
            ));
        }
        let expected_output = digest_of(&self.output_schema)?;
        if self.output_schema_digest != expected_output {
            return Err(ToolRegistryError::schema_digest_mismatch(
                "output schema digest does not match the descriptor",
            ));
        }
        if self.trusted_in_process {
            self.require_trusted_evaluator()?;
        }
        self.descriptor_digest = descriptor_digest(&self.descriptor)?;
        Ok(())
    }

    /// Refuse to mark anything but a first-party pure evaluator as trusted.
    fn require_trusted_evaluator(&self) -> Result<(), ToolRegistryError> {
        if self.handler.is_subprocess() {
            return Err(ToolRegistryError::registration(
                "a subprocess handler cannot run inside the kernel process",
            ));
        }
        if !is_pure_evaluator(&self.descriptor) {
            return Err(ToolRegistryError::registration(
                "a trusted in-process evaluator must be a non-destructive tool with no side \
                 effects and no privileged capability",
            ));
        }
        if !is_first_party(self.handler.as_ref()) {
            return Err(ToolRegistryError::registration(
                "a trusted in-process evaluator must be implemented by the kernel package",
            ));
        }
        Ok(())
    }

    /// Return the registration with the trusted in-process privilege.
    ///
    /// The Python implementation uses `dataclasses.replace`, which rebuilds the
    /// registration and therefore runs the validation again.
    pub fn with_trusted_in_process(
        &self,
        trusted_in_process: bool,
    ) -> Result<Self, ToolRegistryError> {
        let mut tool = self.clone();
        tool.trusted_in_process = trusted_in_process;
        tool.validate()?;
        Ok(tool)
    }

    /// The descriptor of the tool.
    pub fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    /// The handler of the tool.
    pub fn handler(&self) -> &dyn RegistryHandler {
        self.handler.as_ref()
    }

    /// The handler of the tool, as an owned handle.
    ///
    /// The executor dispatches a handler on another thread, so it needs an
    /// owned handle. The registration keeps the handler behind an `Arc`, so the
    /// clone shares the same handler.
    pub fn handler_handle(&self) -> Arc<dyn RegistryHandler> {
        self.handler.clone()
    }

    /// The frozen input schema.
    pub fn input_schema(&self) -> &CanonicalValue {
        &self.input_schema
    }

    /// The frozen output schema.
    pub fn output_schema(&self) -> &CanonicalValue {
        &self.output_schema
    }

    /// The digest of the input schema.
    pub fn input_schema_digest(&self) -> &str {
        &self.input_schema_digest
    }

    /// The digest of the output schema.
    pub fn output_schema_digest(&self) -> &str {
        &self.output_schema_digest
    }

    /// The digest of the descriptor.
    pub fn descriptor_digest(&self) -> &str {
        &self.descriptor_digest
    }

    /// Whether the tool may run inside the kernel process.
    pub fn trusted_in_process(&self) -> bool {
        self.trusted_in_process
    }

    /// The tool identifier.
    pub fn tool_id(&self) -> &str {
        self.descriptor.tool_id()
    }

    /// The tool version.
    pub fn version(&self) -> &str {
        self.descriptor.version().as_str().unwrap_or_default()
    }

    /// The capabilities the tool requires.
    pub fn required_capabilities(&self) -> &[kala_contracts::Identifier] {
        self.descriptor.required_capabilities()
    }

    /// The side effect level of the tool.
    pub fn side_effect_level(&self) -> SideEffectLevel {
        self.descriptor.side_effect_level()
    }

    /// The timeout of the tool, in seconds.
    pub fn timeout_seconds(&self) -> f64 {
        self.descriptor.timeout_seconds()
    }

    /// Validate the arguments of a call and return their canonical text.
    ///
    /// The failure is the `SchemaValidationError` of the protocol layer.
    pub fn validate_arguments(
        &self,
        arguments: &CanonicalValue,
    ) -> Result<String, ToolProtocolError> {
        validate_against_schema(arguments, &self.input_schema, "$", 0)?;
        canonical_json_dumps(arguments, crate::protocol::MAX_JSON_BYTES)
    }

    /// Validate the result of a call and return its canonical text.
    ///
    /// The failure is the `SchemaValidationError` of the protocol layer.
    pub fn validate_result(&self, result: &CanonicalValue) -> Result<String, ToolProtocolError> {
        validate_against_schema(result, &self.output_schema, "$", 0)?;
        canonical_json_dumps(result, crate::protocol::MAX_JSON_BYTES)
    }

    /// Whether the granted capabilities cover the required capabilities.
    pub fn is_authorized_for(&self, granted_capabilities: &[String]) -> bool {
        self.required_capabilities().iter().all(|required| {
            granted_capabilities
                .iter()
                .any(|granted| granted == required.as_str())
        })
    }

    /// Verify that the registration was not modified after it was built.
    ///
    /// The failure is a `ToolRegistryError`.
    pub fn verify_immutable(&self) -> Result<&RegisteredTool, ToolRegistryError> {
        if self.handler.descriptor_snapshot() != self.descriptor {
            return Err(ToolRegistryError::registry(format!(
                "registered descriptor for {} was modified after registration",
                self.tool_id()
            )));
        }
        if digest_of(&self.input_schema)? != self.input_schema_digest {
            return Err(ToolRegistryError::registry(format!(
                "registered input schema for {} was modified",
                self.tool_id()
            )));
        }
        if digest_of(&self.output_schema)? != self.output_schema_digest {
            return Err(ToolRegistryError::registry(format!(
                "registered output schema for {} was modified",
                self.tool_id()
            )));
        }
        if self.trusted_in_process {
            self.require_trusted_evaluator()?;
        }
        Ok(self)
    }

    /// Replace the descriptor of the registration.
    ///
    /// This is the seam the crate's unit tests use to model the Python
    /// `object.__setattr__` mutation of a frozen dataclass. It is not part of
    /// the public API.
    #[cfg(test)]
    pub(crate) fn tamper_descriptor(&mut self, descriptor: ToolDescriptor) {
        self.descriptor = descriptor;
    }

    /// Replace the input schema of the registration.
    ///
    /// This is the seam the crate's unit tests use to model the Python
    /// `object.__setattr__` mutation of a frozen dataclass. It is not part of
    /// the public API.
    #[cfg(test)]
    pub(crate) fn tamper_input_schema(&mut self, schema: CanonicalValue) {
        self.input_schema = schema;
    }

    /// Replace the output schema of the registration.
    ///
    /// This is the seam the crate's unit tests use to model the Python
    /// `object.__setattr__` mutation of a frozen dataclass. It is not part of
    /// the public API.
    #[cfg(test)]
    pub(crate) fn tamper_output_schema(&mut self, schema: CanonicalValue) {
        self.output_schema = schema;
    }
}

/// The registry of the tools a session may call.
///
/// The registry is safe to share between threads, exactly like the Python
/// registry, which guards its table with a re-entrant lock. The registration
/// order is preserved, so a tool that sorts equal to another keeps the order
/// in which it was registered, exactly like the Python dictionary.
pub struct ToolRegistry {
    tools: RwLock<Vec<Registration>>,
    limit: usize,
}

/// One entry of the registry table.
///
/// The Python implementation keys its table with the `(tool_id, version)`
/// pair of the registration and keeps the registration itself as the value.
/// The Rust table keeps the key beside the registration, so a lookup finds
/// the registration even when the descriptor it carries was modified.
struct Registration {
    tool_id: String,
    version: String,
    tool: RegisteredTool,
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ToolRegistry")
            .field("len", &self.len())
            .field("limit", &self.limit)
            .finish()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    /// Build an empty registry.
    pub fn new() -> Self {
        Self::with_limit(MAX_REGISTERED_TOOLS)
    }

    /// Build an empty registry with a smaller capacity.
    ///
    /// The Python implementation reaches the same behaviour by patching the
    /// module constant in a test. The Rust port exposes the capacity so the
    /// refusal can be exercised without registering hundreds of tools.
    pub fn with_limit(limit: usize) -> Self {
        Self {
            tools: RwLock::new(Vec::new()),
            limit,
        }
    }

    /// Build a registry that already holds the given tools.
    ///
    /// A duplicate registration is refused, exactly like the Python
    /// constructor.
    pub fn with_tools(
        tools: impl IntoIterator<Item = RegisteredTool>,
    ) -> Result<Self, ToolRegistryError> {
        let registry = Self::new();
        for tool in tools {
            registry.register_tool(tool)?;
        }
        Ok(registry)
    }

    /// Build a registration without registering it.
    ///
    /// A tool with a side effect must declare the capabilities it needs, the
    /// timeout must be nonnegative and the two schemas must be admitted
    /// object schemas.
    pub fn build(
        &self,
        descriptor: ToolDescriptor,
        handler: Arc<dyn RegistryHandler>,
        input_schema: CanonicalValue,
        output_schema: CanonicalValue,
    ) -> Result<RegisteredTool, ToolRegistryError> {
        if matches!(
            descriptor.side_effect_level(),
            SideEffectLevel::LocalWrite | SideEffectLevel::Network | SideEffectLevel::Egress
        ) && descriptor.required_capabilities().is_empty()
        {
            return Err(ToolRegistryError::registration(
                "a tool with side effects must declare its required capabilities",
            ));
        }
        if descriptor.timeout_seconds() < 0.0 {
            return Err(ToolRegistryError::registration(
                "timeout_seconds must be nonnegative",
            ));
        }
        check_object_schema(&input_schema, "input_schema")
            .map_err(|error| ToolRegistryError::registration(error.to_string()))?;
        check_object_schema(&output_schema, "output_schema")
            .map_err(|error| ToolRegistryError::registration(error.to_string()))?;
        RegisteredTool::new(
            descriptor.clone(),
            handler,
            input_schema,
            output_schema,
            descriptor.input_schema_digest().to_hex(),
            descriptor.output_schema_digest().to_hex(),
        )
    }

    /// Build a registration that may run inside the kernel process.
    ///
    /// The privilege is granted only to a first-party, side-effect free
    /// evaluator. A registration is not a grant: the executor also has to be
    /// configured with the explicit trusted in-process isolation mode.
    pub fn build_trusted_evaluator(
        &self,
        descriptor: ToolDescriptor,
        handler: Arc<dyn RegistryHandler>,
        input_schema: CanonicalValue,
        output_schema: CanonicalValue,
    ) -> Result<RegisteredTool, ToolRegistryError> {
        let tool = self.build(descriptor, handler, input_schema, output_schema)?;
        tool.with_trusted_in_process(true)
    }

    /// Build a registration and register it.
    pub fn register(
        &self,
        descriptor: ToolDescriptor,
        handler: Arc<dyn RegistryHandler>,
        input_schema: CanonicalValue,
        output_schema: CanonicalValue,
    ) -> Result<RegisteredTool, ToolRegistryError> {
        let tool = self.build(descriptor, handler, input_schema, output_schema)?;
        self.register_tool(tool)
    }

    /// Build a trusted in-process registration and register it.
    pub fn register_trusted_evaluator(
        &self,
        descriptor: ToolDescriptor,
        handler: Arc<dyn RegistryHandler>,
        input_schema: CanonicalValue,
        output_schema: CanonicalValue,
    ) -> Result<RegisteredTool, ToolRegistryError> {
        let tool =
            self.build_trusted_evaluator(descriptor, handler, input_schema, output_schema)?;
        self.register_tool(tool)
    }

    /// Register an already built tool.
    ///
    /// A duplicate `(tool_id, version)` pair and a full registry are refused.
    pub fn register_tool(&self, tool: RegisteredTool) -> Result<RegisteredTool, ToolRegistryError> {
        let mut tools = self.write();
        if tools.iter().any(|registered| {
            registered.tool_id == tool.tool_id() && registered.version == tool.version()
        }) {
            return Err(ToolRegistryError::duplicate(format!(
                "tool {} version {} is already registered",
                tool.tool_id(),
                tool.version()
            )));
        }
        if tools.len() >= self.limit {
            return Err(ToolRegistryError::registration("the tool registry is full"));
        }
        tools.push(Registration {
            tool_id: tool.tool_id().to_owned(),
            version: tool.version().to_owned(),
            tool: tool.clone(),
        });
        Ok(tool)
    }

    /// Look a tool up.
    ///
    /// Without a version the latest registered version of the tool is
    /// returned. An unknown tool and an unknown version are refused with
    /// different classes, exactly like the Python implementation.
    pub fn require(
        &self,
        tool_id: &str,
        tool_version: Option<&str>,
    ) -> Result<RegisteredTool, ToolRegistryError> {
        let normalized_id = identifier(tool_id, "tool_id")?;
        let Some(raw_version) = tool_version else {
            return self.latest(&normalized_id);
        };
        let normalized_version = version_value(raw_version)?;
        let tools = self.read();
        let tool = tools
            .iter()
            .find(|registered| {
                registered.tool_id == normalized_id && registered.version == normalized_version
            })
            .map(|registered| registered.tool.clone());
        let known_versions = tools
            .iter()
            .filter(|registered| registered.tool_id == normalized_id)
            .count();
        drop(tools);
        let Some(tool) = tool else {
            if known_versions == 0 {
                return Err(ToolRegistryError::unknown_tool(format!(
                    "tool {normalized_id} is not registered"
                )));
            }
            return Err(ToolRegistryError::version_not_found(format!(
                "tool {normalized_id} has no registered version {normalized_version}"
            )));
        };
        self.verify_unmodified(&tool)?;
        Ok(tool)
    }

    /// Look the latest registered version of a tool up.
    pub fn latest(&self, tool_id: &str) -> Result<RegisteredTool, ToolRegistryError> {
        let normalized_id = identifier(tool_id, "tool_id")?;
        let tools = self.read();
        let candidates: Vec<RegisteredTool> = tools
            .iter()
            .filter(|registered| registered.tool_id == normalized_id)
            .map(|registered| registered.tool.clone())
            .collect();
        drop(tools);
        if candidates.is_empty() {
            return Err(ToolRegistryError::unknown_tool(format!(
                "tool {normalized_id} is not registered"
            )));
        }
        let mut keys = Vec::with_capacity(candidates.len());
        for candidate in &candidates {
            keys.push(version_key(candidate.version())?);
        }
        let mut selected = 0usize;
        for (index, key) in keys.iter().enumerate() {
            if key > &keys[selected] {
                selected = index;
            }
        }
        let tool = candidates[selected].clone();
        self.verify_unmodified(&tool)?;
        Ok(tool)
    }

    /// Every registered tool that requires a capability.
    pub fn tools_requiring(
        &self,
        capability: &str,
    ) -> Result<Vec<RegisteredTool>, ToolRegistryError> {
        let normalized = identifier(capability, "capability")?;
        let tools = self.read();
        let mut selected: Vec<RegisteredTool> = tools
            .iter()
            .filter(|registered| {
                registered
                    .tool
                    .descriptor()
                    .required_capabilities()
                    .iter()
                    .any(|required| required.as_str() == normalized)
            })
            .map(|registered| registered.tool.clone())
            .collect();
        drop(tools);
        let mut keys = Vec::with_capacity(selected.len());
        for tool in &selected {
            keys.push(version_key(tool.version())?);
        }
        selected.sort_by(|left, right| {
            left.tool_id().cmp(right.tool_id()).then_with(|| {
                let left_key = version_key(left.version()).unwrap_or_default();
                let right_key = version_key(right.version()).unwrap_or_default();
                left_key.cmp(&right_key)
            })
        });
        Ok(selected)
    }

    /// Look a tool up and check that every capability it requires is granted.
    ///
    /// The failure is a `ToolCapabilityError` with the missing capabilities.
    pub fn require_capabilities(
        &self,
        tool_id: &str,
        tool_version: Option<&str>,
        granted_capabilities: &[String],
    ) -> Result<RegisteredTool, ToolRegistryError> {
        let tool = self.require(tool_id, tool_version)?;
        let mut granted: Vec<String> = Vec::with_capacity(granted_capabilities.len());
        for value in granted_capabilities {
            granted.push(identifier(value, "granted capability")?);
        }
        let mut missing: Vec<String> = tool
            .required_capabilities()
            .iter()
            .filter(|required| !granted.iter().any(|value| value == required.as_str()))
            .map(|required| required.as_str().to_owned())
            .collect();
        missing.sort();
        missing.dedup();
        if !missing.is_empty() {
            return Err(ToolRegistryError::capability(
                format!(
                    "tool {} requires capabilities that are not granted: {}",
                    tool.tool_id(),
                    missing.join(", ")
                ),
                missing,
            ));
        }
        Ok(tool)
    }

    /// Every registered version of a tool, in version order.
    pub fn versions(&self, tool_id: &str) -> Result<Vec<String>, ToolRegistryError> {
        let normalized_id = identifier(tool_id, "tool_id")?;
        let tools = self.read();
        let mut selected: Vec<String> = tools
            .iter()
            .filter(|registered| registered.tool_id == normalized_id)
            .map(|registered| registered.version.clone())
            .collect();
        drop(tools);
        let mut keys = Vec::with_capacity(selected.len());
        for version in &selected {
            keys.push(version_key(version)?);
        }
        selected.sort_by(|left, right| {
            version_key(left)
                .unwrap_or_default()
                .cmp(&version_key(right).unwrap_or_default())
        });
        Ok(selected)
    }

    /// Every registered tool, in tool and version order.
    pub fn all_tools(&self) -> Result<Vec<RegisteredTool>, ToolRegistryError> {
        let tools = self.read();
        let selected: Vec<RegisteredTool> = tools
            .iter()
            .map(|registered| registered.tool.clone())
            .collect();
        drop(tools);
        let mut keys = Vec::with_capacity(selected.len());
        for tool in &selected {
            keys.push(version_key(tool.version())?);
        }
        let mut order: Vec<usize> = (0..selected.len()).collect();
        order.sort_by(|left, right| {
            selected[*left]
                .tool_id()
                .cmp(selected[*right].tool_id())
                .then_with(|| keys[*left].cmp(&keys[*right]))
        });
        let mut ordered = Vec::with_capacity(selected.len());
        for index in order {
            ordered.push(selected[index].clone());
        }
        Ok(ordered)
    }

    /// Every registered tool identifier, in order.
    pub fn tool_ids(&self) -> Result<Vec<String>, ToolRegistryError> {
        let mut identifiers: Vec<String> = self
            .all_tools()?
            .iter()
            .map(|tool| tool.tool_id().to_owned())
            .collect();
        identifiers.sort();
        identifiers.dedup();
        Ok(identifiers)
    }

    /// Every capability a registered tool requires, in order.
    pub fn required_capabilities(&self) -> Result<Vec<String>, ToolRegistryError> {
        let mut names: Vec<String> = Vec::new();
        for tool in self.all_tools()? {
            for capability in tool.descriptor().required_capabilities() {
                names.push(capability.as_str().to_owned());
            }
        }
        names.sort();
        names.dedup();
        Ok(names)
    }

    /// The number of registered tools.
    pub fn len(&self) -> usize {
        self.read().len()
    }

    /// Whether the registry holds no tool.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every registered tool, in tool and version order.
    pub fn iter(&self) -> Result<std::vec::IntoIter<RegisteredTool>, ToolRegistryError> {
        Ok(self.all_tools()?.into_iter())
    }

    /// Whether a `(tool_id, version)` pair is registered.
    pub fn contains(&self, tool_id: &str, version: &str) -> bool {
        self.read()
            .iter()
            .any(|registered| registered.tool_id == tool_id && registered.version == version)
    }

    /// Verify that a registration was not modified after it was registered.
    fn verify_unmodified(&self, tool: &RegisteredTool) -> Result<(), ToolRegistryError> {
        let current = descriptor_digest(tool.descriptor())?;
        if current != tool.descriptor_digest() {
            return Err(ToolRegistryError::registry(format!(
                "registered descriptor for {} was modified",
                tool.tool_id()
            )));
        }
        tool.verify_immutable()?;
        Ok(())
    }

    /// Read the table, ignoring a poisoned lock.
    fn read(&self) -> RwLockReadGuard<'_, Vec<Registration>> {
        self.tools.read().unwrap_or_else(|error| error.into_inner())
    }

    /// Write the table, ignoring a poisoned lock.
    fn write(&self) -> RwLockWriteGuard<'_, Vec<Registration>> {
        self.tools
            .write()
            .unwrap_or_else(|error| error.into_inner())
    }
}

/// Deep freeze a JSON value.
///
/// The Python implementation converts every mapping into a read-only proxy
/// and every sequence into a tuple, and refuses a mapping with a key that is
/// not a string. The canonical value model of the accepted `kala-json` crate
/// is already immutable and its object keys are always strings, so the freeze
/// is structural: the function walks the value without recursing and returns
/// it unchanged.
pub fn deep_freeze(value: &CanonicalValue) -> Result<CanonicalValue, ToolRegistryError> {
    let mut pending: Vec<&CanonicalValue> = vec![value];
    while let Some(current) = pending.pop() {
        match current {
            CanonicalValue::Object(entries) => {
                for item in entries.values() {
                    pending.push(item);
                }
            }
            CanonicalValue::Array(items) => {
                for item in items {
                    pending.push(item);
                }
            }
            _ => {}
        }
    }
    Ok(value.clone())
}

/// Deep freeze a schema.
///
/// The name mirrors the Python `_frozen_schema`.
pub fn frozen_schema(schema: &CanonicalValue) -> Result<CanonicalValue, ToolRegistryError> {
    deep_freeze(schema)
}

/// Whether a descriptor is a pure evaluator.
///
/// A pure evaluator has no side effect, is not destructive and declares none
/// of the privileged capabilities.
pub fn is_pure_evaluator(descriptor: &ToolDescriptor) -> bool {
    descriptor.side_effect_level() == SideEffectLevel::None
        && !descriptor.destructive()
        && !descriptor
            .required_capabilities()
            .iter()
            .any(|capability| TRUSTED_EVALUATOR_CAPABILITIES.contains(&capability.as_str()))
}

/// Whether a handler is implemented by the shipped kernel package.
pub fn is_first_party(handler: &dyn RegistryHandler) -> bool {
    handler.defining_package() == KERNEL_PACKAGE_DIRECTORY
}

/// The canonical digest of a JSON value.
fn digest_of(value: &CanonicalValue) -> Result<String, ToolRegistryError> {
    canonical_json_digest(value).map_err(|error| ToolRegistryError::Json(error.to_string()))
}

/// The canonical digest of a descriptor.
fn descriptor_digest(descriptor: &ToolDescriptor) -> Result<String, ToolRegistryError> {
    let payload = descriptor
        .to_wire_value()
        .to_canonical_value()
        .map_err(|error| ToolRegistryError::Json(error.to_string()))?;
    digest_of(&payload)
}

/// Validate and normalise a tool identifier.
///
/// The identifier is stripped, it must be nonempty, it must not be longer
/// than 128 code points and it must not contain whitespace or a control
/// character.
pub fn identifier(value: &str, field_name: &str) -> Result<String, ToolRegistryError> {
    let normalized = python_strip(value);
    if normalized.is_empty() || normalized.chars().count() > MAX_IDENTIFIER_LENGTH {
        return Err(ToolRegistryError::registry(format!(
            "{field_name} must be nonempty and bounded"
        )));
    }
    if normalized
        .chars()
        .any(|character| is_python_whitespace(character) || (character as u32) < 32)
    {
        return Err(ToolRegistryError::registry(format!(
            "{field_name} contains unsafe characters"
        )));
    }
    Ok(normalized)
}

/// Validate and normalise a tool version.
///
/// The version is stripped, it must be nonempty, it must not be longer than
/// 64 code points and it must not contain whitespace or a control character.
pub fn version_value(value: &str) -> Result<String, ToolRegistryError> {
    let normalized = python_strip(value);
    if normalized.is_empty() || normalized.chars().count() > MAX_VERSION_LENGTH {
        return Err(ToolRegistryError::registry(
            "tool_version must be nonempty and bounded",
        ));
    }
    if normalized
        .chars()
        .any(|character| is_python_whitespace(character) || (character as u32) < 32)
    {
        return Err(ToolRegistryError::registry(
            "tool_version contains unsafe characters",
        ));
    }
    Ok(normalized)
}

/// Whether a character is whitespace for CPython's `str.strip()` and
/// `str.isspace()`.
///
/// CPython strips every character whose `str.isspace()` is true: the ASCII
/// whitespace, the C1 control characters `0x1c` to `0x1f` and the Unicode
/// space separators.
fn is_python_whitespace(character: char) -> bool {
    matches!(
        u32::from(character),
        0x09..=0x0D
            | 0x1C..=0x1F
            | 0x20
            | 0x85
            | 0xA0
            | 0x1680
            | 0x2000..=0x200A
            | 0x2028
            | 0x2029
            | 0x202F
            | 0x205F
            | 0x3000
    )
}

/// Strip CPython's whitespace from both ends of a text.
fn python_strip(value: &str) -> String {
    value
        .trim_start_matches(is_python_whitespace)
        .trim_end_matches(is_python_whitespace)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kala_contracts::{ContractModel, SideEffectLevel, ToolDescriptor};
    use kala_json::{CanonicalObject, CanonicalValue, JsonText};

    use super::*;
    use crate::protocol::{ToolHandler, ToolRequestEnvelope, ToolResponseEnvelope};

    /// A handler of the unit tests.
    struct UnitHandler {
        descriptor: ToolDescriptor,
    }

    impl ToolHandler for UnitHandler {
        fn descriptor(&self) -> &ToolDescriptor {
            &self.descriptor
        }

        fn invoke(
            &self,
            _request: &ToolRequestEnvelope,
        ) -> Result<ToolResponseEnvelope, ToolProtocolError> {
            Err(ToolProtocolError::protocol("the unit handler does not run"))
        }
    }

    impl RegistryHandler for UnitHandler {
        fn defining_package(&self) -> &'static str {
            KERNEL_PACKAGE_DIRECTORY
        }
    }

    /// The input schema of the unit tests.
    fn input_schema() -> CanonicalValue {
        CanonicalValue::object([
            ("additionalProperties", CanonicalValue::Bool(false)),
            (
                "properties",
                CanonicalValue::object([(
                    "operation",
                    CanonicalValue::object([("type", CanonicalValue::text("string"))]),
                )]),
            ),
            ("type", CanonicalValue::text("object")),
        ])
    }

    /// The output schema of the unit tests.
    fn output_schema() -> CanonicalValue {
        CanonicalValue::object([
            ("additionalProperties", CanonicalValue::Bool(false)),
            (
                "properties",
                CanonicalValue::object([(
                    "ok",
                    CanonicalValue::object([("type", CanonicalValue::text("boolean"))]),
                )]),
            ),
            ("type", CanonicalValue::text("object")),
        ])
    }

    /// A descriptor of the unit tests.
    fn descriptor() -> ToolDescriptor {
        let mut entries = CanonicalObject::new();
        entries.insert(
            JsonText::new("tool_id"),
            CanonicalValue::text("mechanical.reference-evaluator"),
        );
        entries.insert(JsonText::new("version"), CanonicalValue::text("1.0.0"));
        entries.insert(JsonText::new("name"), CanonicalValue::text("Test tool"));
        entries.insert(
            JsonText::new("description"),
            CanonicalValue::text("A test tool."),
        );
        entries.insert(
            JsonText::new("input_schema_digest"),
            CanonicalValue::text(digest_of(&input_schema()).expect("digest")),
        );
        entries.insert(
            JsonText::new("output_schema_digest"),
            CanonicalValue::text(digest_of(&output_schema()).expect("digest")),
        );
        entries.insert(
            JsonText::new("side_effect_level"),
            CanonicalValue::text(SideEffectLevel::None.as_str()),
        );
        entries.insert(
            JsonText::new("required_capabilities"),
            CanonicalValue::array(Vec::new()),
        );
        entries.insert(
            JsonText::new("timeout_seconds"),
            CanonicalValue::Float(10.0),
        );
        entries.insert(JsonText::new("destructive"), CanonicalValue::Bool(false));
        entries.insert(
            JsonText::new("supports_structured_output"),
            CanonicalValue::Bool(true),
        );
        entries.insert(JsonText::new("idempotent"), CanonicalValue::Bool(true));
        ToolDescriptor::from_value(&CanonicalValue::Object(entries)).expect("descriptor")
    }

    /// A registered tool of the unit tests.
    fn registered() -> RegisteredTool {
        let registry = ToolRegistry::new();
        let handler = Arc::new(UnitHandler {
            descriptor: descriptor(),
        });
        registry
            .build(descriptor(), handler, input_schema(), output_schema())
            .expect("registration")
    }

    #[test]
    fn the_verify_detects_a_mutated_input_schema() {
        let mut tool = registered();
        tool.tamper_input_schema(CanonicalValue::object([(
            "tampered",
            CanonicalValue::Bool(true),
        )]));
        let error = tool.verify_immutable().expect_err("modified");
        assert!(error.to_string().contains("input schema"));
    }

    #[test]
    fn the_verify_detects_a_mutated_output_schema() {
        let mut tool = registered();
        tool.tamper_output_schema(CanonicalValue::object([(
            "tampered",
            CanonicalValue::Bool(true),
        )]));
        let error = tool.verify_immutable().expect_err("modified");
        assert!(error.to_string().contains("output schema"));
    }

    #[test]
    fn the_verify_detects_a_mutated_descriptor() {
        let mut tool = registered();
        let mut entries = CanonicalObject::new();
        entries.insert(
            JsonText::new("tool_id"),
            CanonicalValue::text("mechanical.reference-evaluator"),
        );
        entries.insert(JsonText::new("version"), CanonicalValue::text("9.9.9"));
        entries.insert(JsonText::new("name"), CanonicalValue::text("Test tool"));
        entries.insert(
            JsonText::new("description"),
            CanonicalValue::text("A test tool."),
        );
        entries.insert(
            JsonText::new("input_schema_digest"),
            CanonicalValue::text(digest_of(&input_schema()).expect("digest")),
        );
        entries.insert(
            JsonText::new("output_schema_digest"),
            CanonicalValue::text(digest_of(&output_schema()).expect("digest")),
        );
        entries.insert(
            JsonText::new("side_effect_level"),
            CanonicalValue::text(SideEffectLevel::None.as_str()),
        );
        entries.insert(
            JsonText::new("required_capabilities"),
            CanonicalValue::array(Vec::new()),
        );
        entries.insert(
            JsonText::new("timeout_seconds"),
            CanonicalValue::Float(10.0),
        );
        entries.insert(JsonText::new("destructive"), CanonicalValue::Bool(false));
        entries.insert(
            JsonText::new("supports_structured_output"),
            CanonicalValue::Bool(true),
        );
        entries.insert(JsonText::new("idempotent"), CanonicalValue::Bool(true));
        tool.tamper_descriptor(
            ToolDescriptor::from_value(&CanonicalValue::Object(entries)).expect("descriptor"),
        );
        let error = tool.verify_immutable().expect_err("modified");
        assert!(error.to_string().contains("modified after registration"));
    }

    #[test]
    fn the_registry_detects_a_mutated_descriptor() {
        let registry = ToolRegistry::new();
        let handler = Arc::new(UnitHandler {
            descriptor: descriptor(),
        });
        let mut tool = registry
            .build(descriptor(), handler, input_schema(), output_schema())
            .expect("registration");
        tool.tamper_descriptor(other_descriptor());
        registry.register_tool(tool).expect("registered");
        let error = registry
            .require("mechanical.reference-evaluator", Some("1.0.0"))
            .expect_err("modified");
        assert!(error.to_string().contains("was modified"));
    }

    /// A descriptor with another name.
    ///
    /// The identity of the tool is kept, so the lookup still finds the
    /// registration and the digest check is the one that fires.
    fn other_descriptor() -> ToolDescriptor {
        let mut entries = CanonicalObject::new();
        entries.insert(
            JsonText::new("tool_id"),
            CanonicalValue::text("mechanical.reference-evaluator"),
        );
        entries.insert(JsonText::new("version"), CanonicalValue::text("1.0.0"));
        entries.insert(JsonText::new("name"), CanonicalValue::text("Other tool"));
        entries.insert(
            JsonText::new("description"),
            CanonicalValue::text("A test tool."),
        );
        entries.insert(
            JsonText::new("input_schema_digest"),
            CanonicalValue::text(digest_of(&input_schema()).expect("digest")),
        );
        entries.insert(
            JsonText::new("output_schema_digest"),
            CanonicalValue::text(digest_of(&output_schema()).expect("digest")),
        );
        entries.insert(
            JsonText::new("side_effect_level"),
            CanonicalValue::text(SideEffectLevel::None.as_str()),
        );
        entries.insert(
            JsonText::new("required_capabilities"),
            CanonicalValue::array(Vec::new()),
        );
        entries.insert(
            JsonText::new("timeout_seconds"),
            CanonicalValue::Float(10.0),
        );
        entries.insert(JsonText::new("destructive"), CanonicalValue::Bool(false));
        entries.insert(
            JsonText::new("supports_structured_output"),
            CanonicalValue::Bool(true),
        );
        entries.insert(JsonText::new("idempotent"), CanonicalValue::Bool(true));
        ToolDescriptor::from_value(&CanonicalValue::Object(entries)).expect("descriptor")
    }
}
