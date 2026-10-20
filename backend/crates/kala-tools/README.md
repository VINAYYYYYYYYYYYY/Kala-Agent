# kala-tools

Native tool plane for the Kala Agent backend: bounded canonical JSON, JSON
schema validation, request and response envelopes, workspace path rules, the
worker bootstrap, the child environment, the tool registry, the mechanical
reference simulator and the tool executor.

This crate is a standalone workspace port of the `kala_agent.tools` package.
It consumes the accepted `kala-json` canonical JSON implementation and the
accepted `kala-contracts` typed models read-only. It never invokes Python in
production; the live Python differential is test-only.

The module layout is kept shallow, so the crate root re-exports only what the
protocol layer owns.

## Public API

### `kala_tools::protocol`

`kala_tools::protocol` mirrors `kala_agent.tools.protocol` name for name.

- **Constants** — `PROTOCOL_VERSION`, `MAX_JSON_BYTES`, `MAX_ARGUMENT_BYTES`,
  `MAX_STRING_LENGTH`, `MAX_COLLECTION_ITEMS`, `MAX_KEY_LENGTH`,
  `MAX_JSON_DEPTH`, `MAX_SCHEMA_DEPTH`, `MAX_PATH_SEGMENTS`,
  `MAX_PATH_LENGTH`, `MAX_PATTERN_LENGTH`, `MAX_PATTERN_INPUT_LENGTH`,
  `MAX_PATTERN_REPEAT`, `MAX_PATTERN_UNBOUNDED_REPEATS`,
  `MAX_PATTERN_BACKTRACK_STATES`, `MAX_SANITIZED_DETAIL_LENGTH`.
- **Errors** — `ToolProtocolError` with the variants `Protocol`,
  `JsonEnvelope`, `Envelope`, `Workspace` and `Schema { reason, path }`. The
  variants are the five Python classes
  (`ToolProtocolError`, `JsonEnvelopeError`, `ToolEnvelopeError`,
  `ToolWorkspaceError`, `SchemaValidationError`); `python_class_name()`
  reports the class the Python implementation would have raised and `Display`
  reproduces `str(error)`, including the `"{path}: {reason}"` form of a schema
  failure.
- **Enums** — `ToolEnvelopeKind`, `ToolErrorCode`, `is_failure_status`.
- **Canonical JSON** — `canonical_json_dumps`, `strict_json_loads`,
  `canonical_json_digest`, `sha256_hex`, `sha256_hex_text`.
- **Schema** — `validate_against_schema`, `check_object_schema`.
- **Envelopes** — `ToolRequestEnvelope`, `ToolResponseEnvelope` with the
  payload, canonical text, argument and result digests and `to_result`.
- **Workspace** — `resolve_workspace_path`.
- **Handlers** — `ToolHandler`, `SubprocessToolHandler`, `WORKER_BOOTSTRAP`,
  `build_worker_command`, `build_child_environment`,
  `PROTECTED_CHILD_ENVIRONMENT_KEYS`, `read_request_stream`,
  `write_response_stream`.
- **Diagnostics** — `summarize_error`, `sanitize_child_stderr`.

### `kala_tools::registry`

`kala_tools::registry` mirrors `kala_agent.tools.registry`.

- **Constants** — `MAX_REGISTERED_TOOLS`, `FIRST_PARTY_PACKAGE`,
  `KERNEL_PACKAGE_DIRECTORY`.
- **Errors** — `ToolRegistryError` with the variants `Registry`,
  `Registration`, `Duplicate`, `SchemaDigestMismatch`, `UnknownTool`,
  `VersionNotFound`, `Capability { message, missing_capabilities }`, `Value`
  and `Json`. The variants are the seven Python classes
  (`ToolRegistryError`, `ToolRegistrationError`,
  `DuplicateToolRegistrationError`, `SchemaDigestMismatchError`,
  `UnknownToolError`, `ToolVersionNotFoundError`, `ToolCapabilityError`) plus
  the `ValueError` of a version segment CPython's `int()` cannot parse and
  the `JsonEnvelopeError` of a schema that cannot be canonicalised.
- **Registration** — `RegisteredTool` with the descriptor, the handler, the
  two schemas, the schema digests, the descriptor digest and the
  trusted-in-process flag, with `validate_arguments`, `validate_result`,
  `is_authorized_for`, `verify_immutable` and `with_trusted_in_process`.
- **Registry** — `ToolRegistry` with `build`, `build_trusted_evaluator`,
  `register`, `register_trusted_evaluator`, `register_tool`, `require`,
  `latest`, `tools_requiring`, `require_capabilities`, `versions`,
  `all_tools`, `tool_ids`, `required_capabilities`, `len`, `is_empty`,
  `iter` and `contains`.
- **Handlers** — `RegistryHandler`, the trait a registered handler
  implements. It extends the protocol's `ToolHandler` trait and adds the
  provenance of the handler.
- **Helpers** — `deep_freeze`, `frozen_schema`, `is_pure_evaluator`,
  `is_first_party`, `identifier`, `version_value`, `version_key`,
  `VersionKey`, `VersionSegment`.

### `kala_tools::simulator`

`kala_tools::simulator` mirrors `kala_agent.tools.simulator`.

- **Constants** — `SIMULATOR_TOOL_ID`, `SIMULATOR_TOOL_VERSION`,
  `SIMULATOR_ENTRY_POINT`, `SIMULATOR_REPORT_DIRECTORY`,
  `SIMULATOR_REPORT_NAME`, `SIMULATOR_CAPABILITIES`, `EVALUATOR_TOOL_ID`,
  `EVALUATOR_TOOL_VERSION`, `EVALUATOR_CAPABILITIES`,
  `MISBEHAVIOR_ENTRY_POINTS`, `MISBEHAVIOR_SLEEP_SECONDS`,
  `MISBEHAVIOR_EXIT_CODE`, `MISBEHAVIOR_FLOOD_CHUNKS`,
  `MISBEHAVIOR_FLOOD_CHUNK_BYTES`, `MISBEHAVIOR_ESCAPE_NAME`,
  `MISBEHAVIOR_STRAY_CHILD_NAME`, `MISBEHAVIOR_STRAY_CHILD_DELAY_SECONDS`,
  `STRAY_CHILD_SOURCE`, `OPERATIONS`, `MAX_REPORTED_VOLUME_MM3`,
  `MAX_REPORTED_MASS_KG`, `MAX_REPORTED_SLENDERNESS`.
- **Errors** — `SimulatorError` with the variants `Input`, `Workspace`,
  `Value`, `Overflow`, `Contract`, `Json` and `Registry`.
- **Parameters** — `SimulatorParameters`, `parse_parameters`,
  `compute_metrics`, `within_limits`, `report_payload`.
- **Schemas** — `input_schema`, `output_schema`.
- **Descriptors and handlers** — `simulator_descriptor`, `simulator_handler`,
  `evaluator_descriptor`, `SimulatorToolHandler`, `ReferenceEvaluatorHandler`.
- **Entry points** — `invoke`, `hang_invoke`, `crash_invoke`, `flood_invoke`,
  `liar_invoke`, `unrecognized_code_invoke`, `stray_child_invoke`,
  `lingering_child_invoke`, `escape_invoke`.
- **Report** — `write_report`, `write_workspace_file`.
- **Registration** — `register_simulator`, `register_reference_evaluator`.

### `kala_tools::executor`

`kala_tools::executor` mirrors `kala_agent.tools.executor`.

- **Constants** — `MAX_WORKERS`, `MAX_DEFAULT_TIMEOUT_SECONDS`,
  `MINIMUM_TIMEOUT_SECONDS`, `MAX_ERROR_MESSAGE_LENGTH`, `MAX_QUEUE_SECONDS`,
  `MINIMUM_QUEUE_SECONDS`, `APPROVAL_BINDING_VERSION`, `EXTRA_ENVIRONMENT_KEYS`,
  `NETWORK_CAPABILITIES`, `EGRESS_CAPABILITIES`, `FORBIDDEN_CAPABILITIES`,
  `PORTABLE_CONTROLS`, `TEXTUAL_MEDIA_SUFFIXES`, `BINARY_SIGNATURES`,
  `CLASSIFICATION_HEADER_BYTES`.
- **Errors** — `ToolExecutorError` with the variants `Executor`, `Authority`,
  `Isolation` and `Workspace`. The variants are the three Python classes of
  the module (`ToolExecutorError`, `ToolAuthorityError`, `ToolIsolationError`)
  plus the protocol layer's `ToolWorkspaceError`; `python_class_name()`
  reports the class the Python implementation would have raised.
- **Isolation** — `IsolationMode` with `Auto`, `Subprocess` and
  `TrustedInProcessEvaluator`.
- **Profile** — `SandboxProfile` with the profile name, the output, argument
  and artifact bounds, the default deadline, the worker cap, the queueing
  wait, the network, egress and privileged capability switches, the child
  limit switch, the isolation profile, the extra environment and the probe,
  with `validated`, `enforced_controls` and `timeout_for`.
- **Executor** — `ToolExecutor` with `registry`, `run_directory`,
  `granted_capabilities`, `profile`, `isolation`, `sandbox_probe`,
  `enforced_controls`, `reports`, `register`, `require_authority`,
  `grant_capability`, `workspace_path`, `workspace_for`,
  `resolve_output_path`, `policy_request`, `approval_digests`, `execute`,
  `verify_response` and `verify_artifacts`.
- **Configuration** — `ExecutorConfig` and `ExecutionOptions`, the
  constructor arguments of the executor and the options of one call.
- **Report** — `ToolExecutionReport` with the twelve members of the Python
  payload and `to_payload`.
- **Classification** — `classify_header`, `derive_classification`.

The crate also ships two thin launchers over the accepted `kala-sandbox`
helpers, `kala-tools-sandbox-probe` and `kala-tools-sandbox-init`. They own no
jail logic: the strict jail, the PID-1 transition and the control
verification stay in the accepted crate.

## Compatibility

* Canonical bytes and digests come from the accepted `kala-json` crate, so the
  sorted code-point keys, the ASCII escaping, the arbitrary-precision integers
  and the bounded limits are exactly the Python ones.
* The error classes, the message texts and the error precedence are the
  Python ones, including the order in which an envelope validates its members.
* The schema layer keeps the Python keyword whitelist, the type, `const`,
  `enum`, `allOf`, `anyOf` and `oneOf` rules and the object, array, string and
  number validators.
* The pattern layer keeps the Python admission analysis and adds a matcher with
  CPython `re` semantics, so a schema pattern matches exactly the strings it
  matches in Python: CPython's `str` based `\w`, `\d` and `\s`, `$` before a
  final newline, `\Z` at the very end, leftmost-first alternation and the
  CPython escape, class and repetition rules.
* The matcher and the schema validator are iterative. A hostile pattern cannot
  overflow the thread stack.
* The registry keeps the validation order, the messages, the capability
  closure, the trust rules and the duplicate and capacity refusals of the
  Python module.
* The simulator keeps the deterministic reference calculation, the six
  decimal rounding of CPython, the bounds, the published report, the atomic
  publication and the refusal of a symlinked report directory.
* The executor keeps the decision order, the refusal messages, the error
  precedence, the approval binding digests, the artifact confinement, the
  classification rules and the provenance record of the Python module.

### Intentional differences

The differences of the protocol layer are listed in the section below; the
ones of the registry and the simulator are listed here.

* **Exception classes.** Python raises `TypeError` for a value of the wrong
  type in a few places where Rust has no equivalent class. The Rust port
  reports the same message text and reports the failure through the error
  enum of the module; the live differential compares the message and the
  failure, not the class, in exactly those cases.
* **The handler contract.** Python accepts any object that satisfies the
  runtime checkable `ToolHandler` protocol and answers two questions about it
  at runtime. Rust answers both with the type system: `RegistryHandler`
  extends the protocol's `ToolHandler` trait, so a registered handler always
  implements the tool protocol and always exposes a descriptor. The
  "handler must implement the tool protocol" and "handler must expose a
  ToolDescriptor" messages therefore exist in Rust but cannot be reached.
* **The provenance of a handler.** Python decides whether a handler is first
  party by inspecting the module that defines it and the file that module was
  loaded from. Rust cannot inspect the origin of a type at runtime, so a
  handler reports the package that defines it through
  `RegistryHandler::defining_package` and the registry compares that report
  with its own package directory. A handler defined outside the kernel
  package reports a different directory and is refused the trusted
  in-process privilege.
* **The immutability of a registration.** Python registers a tool in a frozen
  dataclass that `object.__setattr__` can still mutate. The Rust
  registration owns immutable values in private fields, so a mutation is
  structurally impossible; the re-verification is kept as defence in depth
  and is exercised by the crate's unit tests through a crate-internal tamper
  seam.
* **The registry holds a copy.** Python's registry table holds the
  registration objects themselves. The Rust table holds a copy beside the
  `(tool_id, version)` key, so the key of a registration never follows a
  descriptor that was modified.
* **The capacity.** Python reaches a smaller registry capacity by patching
  the module constant in a test. The Rust port exposes the capacity as
  `ToolRegistry::with_limit`.
* **The version ordering.** Python builds the version key with `int(segment)`
  for a segment that `str.isdigit()` accepts, so a superscript digit raises a
  `ValueError` and a segment of more than 4,300 digits raises the CPython
  integer conversion limit. The Rust port reproduces both, including the
  Unicode digit tables and the CPython message.
* **The entry point of a worker.** The Python entry point names a Python
  module. The Rust port keeps the name so the descriptor and the differential
  stay comparable; the Rust executor maps the entry point to its own worker.
* **The stray child.** The Python implementation spawns `sys.executable` with
  a small script. The Rust port spawns a child of the host shell, because the
  crate never depends on an interpreter.
* **A report outside the workspace.** `write_report` resolves the report
  path with the workspace rules of the protocol layer and refuses a path that
  leaves the workspace, exactly like the Python module.
* **The modes of a directory.** The Python module creates the report
  directory with the mode `0o700` and its ancestors with the default mode.
  The Rust port does the same on a host with POSIX modes; a host without them
  leaves the privacy of the directory to the sandbox profile.
* **`strerror`.** Python reports `error.strerror`, the message of the C
  library without the error number. The Rust message carries the same text
  with the error number in a suffix, which the port removes. The two texts
  agree on Linux and differ on a host with a different message catalogue.
* **The executor and the interpreter.** Python's executor starts a worker with
  `sys.executable`, which the language provides. Rust has no `sys.executable`,
  so the interpreter of the worker is a configured value
  (`ExecutorConfig::python_executable`). A subprocess dispatch without one is
  refused with an isolation error instead of running an interpreter the
  executor does not know.
* **The executor and the type system.** Python's `SandboxProfile` validates
  the *types* of its members ("max_workers must be an integer", "allow_network
  must be a boolean"). Rust has no variant of those values, so those four
  messages exist in the port but cannot be reached. Every other validation
  message and its order is reproduced.
* **The executor and the deadline override.** Python's `execute` accepts a
  `timeout_seconds` override and never reads it: the deadline of a call is
  the smaller of the deadline the call declares and the deadline of the
  descriptor, capped by the profile. The port keeps the parameter and the
  behaviour.
* **The executor and the internal failure.** Python's `execute` lets a
  `JsonEnvelopeError` of the approval binding propagate to its caller. The
  port reports it as a failed run with the internal error code, because a
  `ToolResult` is the only value the port returns.
* **The executor and the run directory.** Python expands a leading `~` in the
  run directory. The port takes the path as it is; a caller that wants the
  home directory has to expand it.
* **The executor and the process group.** Python starts the worker with
  `start_new_session=True`, so the worker leads a new session and a new
  process group. The accepted `kala-process` supervisor starts the worker
  with `setpgid(0, 0)`, so the worker leads a new process group inside the
  session of the supervisor. The containment boundary the executor relies on
  — the process group, which is killed on every outcome — is the same; the
  session of the worker is not.
* **The executor and the sandbox jail.** Python builds the strict jail in the
  forked child, before the worker is executed. The port runs the worker
  through the accepted native init helper, which performs the PID-1
  transition the accepted launcher does not have, so the worker cannot see
  the host process table and the namespace dies with the jail. The control
  set is the one the preflight probe verified; a control the host could not
  establish is never requested and never claimed.

## Tests

From `backend/crates/kala-tools`:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --locked --release
cargo build --locked --release
cargo check --locked --target x86_64-pc-windows-gnu
```

The suite covers the ported unit tests, the ported security tests and a live
differential against the repository Python. The differential compares strict
decoding, canonical encoding, schema validation, object schema admission,
request and response envelopes, workspace path resolution, child diagnostics
and error summaries over a curated and a randomised corpus, the pattern
admission and the pattern matching over 417 patterns, the registry operations
over a curated and a randomised corpus of registrations, the simulator
operations over a curated and a randomised corpus of arguments, and the tool
executor over a corpus of twenty-three cases that covers every misbehaviour
entry point of the reference simulator, the decision order, the refusals, the
policy approval, the approval digests, the provenance records and the strict
profile. Python is a test oracle only; the library has no Python or subprocess
calls.

`tools/generate_unicode_tables.py` regenerates the Unicode class tables the
matcher uses, and `tools/probe_python_regex.py` documents the CPython regular
expression behaviour the matcher reproduces. Neither is used at runtime.
