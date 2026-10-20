# Tools registry cell report — native kala-tools (registry and simulator)

Date: 2026-10-20
Cell: `tools-registry-r1` (wave `rust-backend-20261019`)
Crate: `backend/crates/kala-tools` (the accepted `tools-protocol-r1` crate,
extended)
Implementer: OpenCode Go **MiniMax M3.1 Flash (Free)**
Status: implemented and validated in isolation; ready for independent review.
No commit, push, publish, deploy, delegation, credential read, runtime
cutover, Windows runtime execution, speed claim or readiness claim.

## Scope and ownership

Owned and changed only:

- `backend/crates/kala-tools/**` — the `registry` and `simulator` modules,
  their tests, the test oracle and the crate README
- this cell's `REPORT.md`

Not touched: `backend/Cargo.toml` and the root `backend/Cargo.lock`, any
accepted crate source or manifest (`kala-json`, `kala-contracts`,
`kala-budget`, `kala-process`, `kala-kernel`, `kala-domains`,
`kala-repositories`, …), the Python product or its tests, `adapters/`,
`studio/**`, shared fixtures, the other waves and `Rust+buid123/`.

The crate is a **standalone workspace** (`[workspace]` in its own
`Cargo.toml`, its own `Cargo.lock`). It is not a member of the root
`backend/` workspace and the root manifest and lock were not edited. The
accepted `kala-json` and `kala-contracts` crates are consumed **read-only**
through path dependencies; no public API of a dependency was patched.

**The protocol module was not changed.** `backend/crates/kala-tools/src/protocol/**`
and the four protocol test files are byte-identical to the accepted
`tools-protocol-r1` state (verified by their unchanged modification times
after `cargo fmt`); their 71 tests are still green and were re-run in every
gate. `Cargo.toml` and `Cargo.lock` are unchanged: the cell adds **no**
dependency.

## Delivered

A complete native port of `src/kala_agent/tools/registry.py` (499 lines) and
`src/kala_agent/tools/simulator.py` (714 lines), and of the exports they
contribute to `src/kala_agent/tools/__init__.py`:

| Python member | Rust member |
| --- | --- |
| `MAX_REGISTERED_TOOLS`, `FIRST_PARTY_PACKAGE` | `kala_tools::registry::{MAX_REGISTERED_TOOLS, FIRST_PARTY_PACKAGE, KERNEL_PACKAGE_DIRECTORY}` |
| `ToolRegistryError`, `ToolRegistrationError`, `DuplicateToolRegistrationError`, `SchemaDigestMismatchError`, `UnknownToolError`, `ToolVersionNotFoundError`, `ToolCapabilityError` | `kala_tools::registry::ToolRegistryError` with the variants `Registry`, `Registration`, `Duplicate`, `SchemaDigestMismatch`, `UnknownTool`, `VersionNotFound`, `Capability { message, missing_capabilities }` (plus `Value` for the CPython `ValueError` of a version segment and `Json` for the `JsonEnvelopeError` of a schema that cannot be canonicalised) |
| `RegisteredTool` (all fields, `tool_id`, `version`, `required_capabilities`, `side_effect_level`, `timeout_seconds`, `trusted_in_process`, `descriptor_digest`, `validate_arguments`, `validate_result`, `is_authorized_for`, `verify_immutable`) | `kala_tools::registry::RegisteredTool` with the same members |
| `ToolRegistry` (`build`, `build_trusted_evaluator`, `register`, `register_trusted_evaluator`, `register_tool`, `require`, `latest`, `tools_requiring`, `require_capabilities`, `versions`, `all_tools`, `tool_ids`, `required_capabilities`, `__len__`, `__iter__`, `__contains__`) | `kala_tools::registry::ToolRegistry` with the same members (`len`, `iter`, `contains(tool_id, version)`) |
| `_identifier`, `_version` | `identifier`, `version_value` |
| `_version_key` | `version_key`, `VersionKey`, `VersionSegment` |
| `_is_pure_evaluator` | `is_pure_evaluator` |
| `_is_first_party`, `_handler_module` | `is_first_party` and `RegistryHandler::defining_package` (see the differences) |
| `_deep_freeze`, `_frozen_schema` | `deep_freeze`, `frozen_schema` |
| the runtime checkable `ToolHandler` protocol | the `RegistryHandler` trait, which extends the protocol's `ToolHandler` trait |
| `SIMULATOR_TOOL_ID`, `SIMULATOR_TOOL_VERSION`, `SIMULATOR_ENTRY_POINT`, `SIMULATOR_REPORT_DIRECTORY`, `SIMULATOR_REPORT_NAME`, `SIMULATOR_CAPABILITIES`, `EVALUATOR_TOOL_ID`, `EVALUATOR_TOOL_VERSION`, `EVALUATOR_CAPABILITIES`, `MISBEHAVIOR_ENTRY_POINTS`, `MISBEHAVIOR_SLEEP_SECONDS`, `MISBEHAVIOR_EXIT_CODE`, `MISBEHAVIOR_FLOOD_CHUNKS`, `MISBEHAVIOR_ESCAPE_NAME`, `MISBEHAVIOR_STRAY_CHILD_NAME`, `MISBEHAVIOR_STRAY_CHILD_DELAY_SECONDS`, `STRAY_CHILD_SOURCE` | the same constants in `kala_tools::simulator`, plus `MISBEHAVIOR_FLOOD_CHUNK_BYTES` for the chunk size |
| `INPUT_SCHEMA`, `OUTPUT_SCHEMA` | `input_schema()`, `output_schema()` (functions, because the value is built at run time) |
| `SimulatorInputError` | `SimulatorError::Input` |
| `SimulatorParameters` (`to_payload`) | `SimulatorParameters` with `to_payload` and the field accessors |
| `simulator_descriptor`, `simulator_handler`, `evaluator_descriptor` | the same three functions |
| `parse_parameters`, `compute_metrics`, `_within_limits`, `report_payload` | `parse_parameters`, `compute_metrics`, `within_limits`, `report_payload` |
| `write_report`, `_write_workspace_file` | `write_report`, `write_workspace_file` |
| `SimulatorToolHandler`, `ReferenceEvaluatorHandler` | the same two types |
| `invoke`, `_simulate`, `_evaluate` | `invoke` and the private `simulate`/`evaluate` |
| `hang_invoke`, `crash_invoke`, `flood_invoke`, `liar_invoke`, `unrecognized_code_invoke`, `stray_child_invoke`, `lingering_child_invoke`, `escape_invoke`, `_spawn_stray_child` | the same nine entry points |
| `register_simulator`, `register_reference_evaluator` | the same two functions |

### Changed paths

```
backend/crates/kala-tools/README.md                       (extended)
backend/crates/kala-tools/src/lib.rs                      (extended)
backend/crates/kala-tools/src/registry/mod.rs             (new)
backend/crates/kala-tools/src/registry/error.rs           (new)
backend/crates/kala-tools/src/registry/version.rs         (new)
backend/crates/kala-tools/src/simulator/mod.rs            (new)
backend/crates/kala-tools/src/simulator/error.rs          (new)
backend/crates/kala-tools/src/simulator/parameters.rs     (new)
backend/crates/kala-tools/src/simulator/report.rs         (new)
backend/crates/kala-tools/tests/oracle.py                 (extended: registry and simulator modes)
backend/crates/kala-tools/tests/support/mod.rs            (extended: the test handler and the request helper)
backend/crates/kala-tools/tests/registry_branches.rs      (new)
backend/crates/kala-tools/tests/registry_differential.rs  (new)
backend/crates/kala-tools/tests/registry_security.rs      (new)
backend/crates/kala-tools/tests/simulator_branches.rs     (new)
backend/crates/kala-tools/tests/simulator_differential.rs (new)
backend/crates/kala-tools/tests/simulator_security.rs     (new)
.mece/waves/rust-backend-20261019/cells/tools-registry-r1/REPORT.md (new)
```

**Not changed:** `backend/crates/kala-tools/Cargo.toml`,
`backend/crates/kala-tools/Cargo.lock`,
`backend/crates/kala-tools/src/protocol/**` and the four protocol test files.

The public API is listed exhaustively in
`backend/crates/kala-tools/README.md`.

## Compatibility and security evidence

### The accepted crates are used, not re-implemented

* The canonical bytes, the sorted code-point keys, the ASCII escaping, the
  arbitrary-precision integers and the bounded limits come from the accepted
  `kala-json` crate. The registry and the simulator only map its failures to
  the Python message.
* The typed tool descriptor, artifact reference, run status and result models
  come from the accepted `kala-contracts` crate. The descriptor digest of a
  registration is the canonical digest of the descriptor's wire value, which
  the differential compares with the Python `model_dump(mode="json")` digest
  for every registration of the corpus.

### The registration validation

The order of the checks, the messages and the classes are the Python ones:

| Check | Message | Class |
| --- | --- | --- |
| the handler exposes the registered descriptor | `handler descriptor does not match the registered descriptor` | `ToolRegistrationError` |
| the schemas are JSON objects | `schemas must be JSON objects` | `ToolRegistrationError` |
| the input schema digest matches | `input schema digest does not match the descriptor` | `SchemaDigestMismatchError` |
| the output schema digest matches | `output schema digest does not match the descriptor` | `SchemaDigestMismatchError` |
| a trusted registration is not a subprocess handler | `a subprocess handler cannot run inside the kernel process` | `ToolRegistrationError` |
| a trusted registration is pure | `a trusted in-process evaluator must be a non-destructive tool with no side effects and no privileged capability` | `ToolRegistrationError` |
| a trusted registration is first party | `a trusted in-process evaluator must be implemented by the kernel package` | `ToolRegistrationError` |
| a side effecting tool declares its capabilities | `a tool with side effects must declare its required capabilities` | `ToolRegistrationError` |
| the schemas are admitted object schemas | the `SchemaValidationError` text of the protocol layer | `ToolRegistrationError` |
| a duplicate pair | `tool {id} version {version} is already registered` | `DuplicateToolRegistrationError` |
| a full registry | `the tool registry is full` | `ToolRegistrationError` |
| an unknown tool | `tool {id} is not registered` | `UnknownToolError` |
| an unknown version | `tool {id} has no registered version {version}` | `ToolVersionNotFoundError` |
| a missing capability | `tool {id} requires capabilities that are not granted: {names}` | `ToolCapabilityError` with `missing_capabilities` |
| an unsafe identifier or version | `{field} must be a string` / `must be nonempty and bounded` / `contains unsafe characters` | `ToolRegistryError` |
| a modified registration | `registered descriptor for {id} was modified` / `... was modified after registration` / `registered input schema for {id} was modified` / `registered output schema for {id} was modified` | `ToolRegistryError` |

### The trust and the provenance rules (ADR-0005 D12, ADR-0006 D5)

* A registration may be marked as a trusted in-process evaluator only when
  the handler is **not** a subprocess handler, the descriptor declares **no**
  side effect, is **not** destructive and declares **none** of the nine
  privileged capabilities (`egress`, `export`, `network`, `network.access`,
  `network.egress`, `network.internet`, `process.spawn`, `shell.execute`,
  `system.admin`), and the handler reports the **kernel package**.
  `tests/registry_security.rs` exercises all four refusals and the nine
  privileged capabilities.
* The Python implementation decides the provenance from the module that
  defines the handler and the file that module was loaded from. Rust cannot
  inspect the origin of a type at runtime, so the handler reports the package
  that defines it and the registry compares that report with its own package
  directory. The default is "foreign", and only the crate's own handlers
  (`SimulatorToolHandler`, `ReferenceEvaluatorHandler`) report the kernel
  package.
* The capability closure is exact: a tool that declares a capability the
  grant does not contain is refused with the sorted list of the missing
  capabilities, and the granted capabilities are themselves validated with
  the same identifier rules as the tool identifiers.

### The simulator determinism and its bounds (ADR-0005 D12)

* The calculation is the closed form reference of the Python module, in the
  same order of operations, so the binary64 results are bit identical. The
  `height ** 3` of the Python module is `powf(3.0)` in Rust, so the same
  libm rounding applies.
* The rounding is the CPython `round(value, 6)`: the exact value of the
  double is rounded to six decimals with a tie going to the even choice, which
  is what the Rust formatter does. The unit test covers the two exact ties
  (`0.0078125` and `0.0234375`) and the differential covers 386 runs.
* The bounds are the Python ones: the dimensions are bounded, the reported
  volume, mass and slenderness are bounded and the `within_limits` flag is
  computed from the **unrounded** metrics, exactly like the Python module.
* The report is published atomically: the parent directory is created with a
  private mode, re-checked with a single `lstat` so a symlinked directory is
  refused, the payload is written to a fresh exclusive file and renamed into
  place, and a failed publication leaves no temporary file behind.
  `tests/simulator_security.rs` proves the confinement, the refusal of a
  symlinked directory, the refusal of an uncreatable directory, the absence
  of a leftover partial file and the digest and size of the published
  artifact.

### Live Python differential

The differential runs the **repository's own** `kala_agent.tools.registry` and
`kala_agent.tools.simulator` through
`/home/v9i/Projects/Kala-Agent/.venv/bin/python` (Python 3.14.8) with `-I`
(isolated mode) and `PYTHONPATH=src`; the oracle script adds the repository
`src` to `sys.path` itself. A missing interpreter fails the test rather than
skipping it. The override is `KALA_TOOLS_PYTHON`.

Every comparison is byte-for-byte on the canonical JSON of the whole result:
the success value, or the class of the exception **and** its message. There is
no normalisation of digests, messages or ordering.

| Corpus | Cases | Compared against |
| --- | --- | --- |
| registry operations | 146 cases (26 curated + 120 randomised), 678 operations, of which 366 are refusals | `ToolRegistry.build`, `build_trusted_evaluator`, `register`, `register_trusted_evaluator`, `register_tool`, `require`, `latest`, `tools_requiring`, `require_capabilities`, `versions`, `all_tools`, `tool_ids`, `required_capabilities`, `len`, `contains`, `verify_immutable` |
| simulator operations | 386 cases (86 curated + 300 randomised), of which 36 are refusals | `parse_parameters`, `compute_metrics`, `report_payload`, `write_report`, `invoke`, `ReferenceEvaluatorHandler.invoke`, `simulator_descriptor`, `evaluator_descriptor`, `simulator_handler`, `register_simulator`, `register_reference_evaluator`, `liar_invoke`, `unrecognized_code_invoke`, `escape_invoke` |

The registry corpus covers the valid registration and lookup, the trusted
evaluator (accepted and refused for a foreign handler, an impure descriptor
and a privileged capability), the side effecting tool without capabilities,
the two schema digest mismatches, the two malformed schemas, the duplicate
registration, the unknown tool and the unknown version, the identifier and
version validation, the trimming of the identifiers, the capability closure,
the tool filter, the numeric version ordering, the handler that changes its
descriptor after the registration, and a randomised mix of the same
operations over randomised identifiers, versions, capabilities and side effect
levels.

The simulator corpus covers the parameter defaults, the integer dimensions,
the non numeric, the non finite and the out of bounds dimensions, the unknown
operation, the non boolean report flag, the report payload, the published
artifact, the two handlers, the refusal of the evaluator to write a report,
the misbehaviour entry points and a randomised mix over randomised dimensions
inside the bounds.

The corpus sizes and the number of refusals are asserted by a test, so this
report cannot drift away from the corpus that runs.

**Result: 1,064 comparisons, zero unexplained mismatches.**

The corpus that carries a non finite number travels through the document as
a marker, because a canonical JSON document cannot carry one; both sides
decode the marker into the same double.

### The ported tests

* `tests/registry_branches.rs` (38 tests) is the Rust port of
  `tests/unit/test_registry_branches.py`: the version ordering, the first
  party detection, the purity rule, the deep freeze, the exposed descriptor
  fields, the matching handler descriptor, the schema digests, the three
  trusted refusals, the argument and result validation, the capability
  authorization, the mutated handler descriptor, the malformed descriptors and
  schemas, the duplicate, the capacity, the unknown tool and version, the
  latest version, the tool filter, the capability closure, the version list,
  the enumeration, the membership, the identifier and version validation, the
  trimming and the two registrations.
* `tests/registry_security.rs` (9 tests) is the Rust port of the security
  parts of `tests/security/test_schema_redos.py` and
  `tests/security/test_tool_executor.py`: the refusal of a schema whose
  pattern can be split two ways, the four refusals of the trusted privilege,
  the nine privileged capabilities, the side effect rule, the duplicate, the
  capacity, the identifier bounds, the exact capability closure and the error
  hierarchy.
* `tests/simulator_branches.rs` (40 tests) is the Rust port of
  `tests/unit/test_simulator_branches.py`.
* `tests/simulator_security.rs` (10 tests) covers the confinement of the
  report, the atomic publication, the symlinked directory, the uncreatable
  directory, the artifact digest and classification, the escaping entry
  point, the stray child and the refusal of a report outside the workspace.
* The crate's unit tests (18) cover the version key with the Unicode digit
  tables, the CPython integer conversion limit, the six decimal rounding, the
  tamper detection of the descriptor and of the two schemas through the
  crate-internal tamper seam, the hang entry point and the evaluator.

### ADR coverage

* **ADR-0005 D1** — the registry binds `(tool_id, version)` to a descriptor, a
  handler and the two schemas and recomputes the schema digests; a mismatch is
  a `SchemaDigestMismatchError`.
* **ADR-0005 D3** — the capability closure is exact and the granted
  capabilities are validated; a tool cannot grant itself a token.
* **ADR-0005 D5** — a side effecting tool must declare its capabilities and
  the descriptor is refused otherwise.
* **ADR-0005 D12** — the trusted in-process privilege is granted only to a
  first-party, side effect free, non-destructive, non-privileged evaluator and
  is re-checked on every lookup.
* **ADR-0006 D5** — the report is confined to the workspace, the publication
  is atomic and a symlinked directory is refused.
* **ADR-0006 D12** — the schema admission, the digest pinning, the least
  privilege and the atomic artifact write are all present.

## Commands and results actually run

From `backend/crates/kala-tools/` with `cargo 1.99.1` / `rustc 1.99.1`:

```
cargo fmt --check                                              -> clean
cargo clippy --locked --all-targets --all-features -- -D warnings -> clean
cargo test --locked                                            -> 192 passed, 0 failed
  (18 unit + 45 protocol branch + 13 protocol differential + 3 pattern differential
   + 10 protocol security + 38 registry branch + 3 registry differential
   + 9 registry security + 40 simulator branch + 3 simulator differential
   + 10 simulator security + 0 doc)
cargo test --locked --release                                  -> 192 passed, 0 failed
cargo build --locked --release                                 -> success
cargo check --locked --target x86_64-pc-windows-gnu             -> success
cargo check --locked --target x86_64-pc-windows-gnu --all-targets -> success
cargo clippy --locked --target x86_64-pc-windows-gnu --all-targets -- -D warnings -> clean
```

The `x86_64-pc-windows-gnu` target **is installed** on this machine, so the
Windows cross-check was run, including the test targets. It is a compile
check only: no Windows runtime execution is claimed and no Windows behaviour
is claimed.

Debug and release totals are identical. No ignored test, no benchmark, no
performance number and no readiness statement is claimed. The 71 accepted
protocol tests are included in the 192 and are green.

## Intentional differences (complete list)

1. **Exception classes.** Python raises `TypeError` for a value of the wrong
   type in a few places where Rust has no equivalent class. The Rust port
   reports the same message text and reports the failure through the error
   enum of the module. The live differential compares the message and the
   failure and compares the class only where Rust has a class.
2. **The handler contract.** Python accepts any object that satisfies the
   runtime checkable `ToolHandler` protocol and answers two questions about it
   at runtime: "handler must implement the tool protocol" and "handler must
   expose a ToolDescriptor". Rust answers both with the type system, so those
   two messages exist in the port but cannot be reached.
3. **The provenance of a handler.** Python decides the first party status
   from the module that defines the handler and the file that module was
   loaded from, and refuses a handler whose file does not resolve inside the
   kernel package. Rust cannot inspect the origin of a type at runtime, so a
   handler reports the package that defines it through
   `RegistryHandler::defining_package` and the registry compares that report
   with its own package directory. The default is "foreign"; only the crate's
   own handlers report the kernel package. A third-party implementation of the
   trait could declare the kernel package, which is a deliberate weakening of
   a file system location check to a compile time declaration.
4. **The immutability of a registration.** Python registers a tool in a
   frozen dataclass that `object.__setattr__` can still mutate, and the
   registry re-verifies the descriptor and the two schemas on every lookup.
   The Rust registration owns immutable values in private fields, so a
   mutation is structurally impossible. The re-verification is kept as defence
   in depth and is exercised by the crate's unit tests through a
   crate-internal tamper seam.
5. **The registry holds a copy.** Python's registry table holds the
   registration objects themselves, so a mutation of a registration is visible
   to the registry. The Rust table holds a copy beside the `(tool_id, version)`
   key, so the key of a registration never follows a descriptor that was
   modified and a mutation of the caller's copy cannot reach the registry.
6. **The capacity.** Python reaches a smaller registry capacity by patching
   the module constant in a test. The Rust port exposes the capacity as
   `ToolRegistry::with_limit`.
7. **The version ordering.** Python builds the version key with `int(segment)`
   for a segment that `str.isdigit()` accepts, so a superscript digit raises a
   `ValueError` and a segment of more than 4,300 digits raises the CPython
   integer conversion limit. The Rust port reproduces both, including the
   Unicode digit tables (the 71 `Nd` ranges and the 20 further ranges of the
   Unicode "digit" property) and the CPython messages. The corpus covers the
   ASCII ordering; the exotic digits are covered by the crate's unit tests.
8. **The entry point of a worker.** The Python entry point names a Python
   module. The Rust port keeps the name so the descriptor, the constants and
   the differential stay comparable; the Rust executor maps the entry point
   to its own worker.
9. **The stray child.** The Python implementation spawns `sys.executable` with
   a small Python script. The Rust port spawns a child of the host shell
   (`sh -c` on Unix, `cmd /C` on Windows), because the crate never depends on
   an interpreter. The side effect lands at the same moment; the bytes are
   the same on Unix and gain a trailing newline on Windows.
10. **The modes of a directory.** The Python module creates the report
    directory with the mode `0o700` and its ancestors with the default mode.
    The Rust port does the same on a host with POSIX modes; a host without
    them leaves the privacy of the directory to the sandbox profile.
11. **`strerror`.** Python reports `error.strerror`, the message of the C
    library without the error number. The Rust message carries the same text
    with the error number in a suffix, which the port removes. The two texts
    agree on Linux and differ on a host with a different message catalogue.
12. **`INPUT_SCHEMA` and `OUTPUT_SCHEMA`.** Python exposes them as module
    constants. The Rust port exposes them as `input_schema()` and
    `output_schema()`, because the value is built at run time. The canonical
    digests are identical, which the differential proves for every descriptor
    of the corpus.
13. **`os.makedirs` and the leaf mode.** Python creates the ancestors of the
    report directory with the default mode and only the leaf with `0o700`.
    The Rust port does the same, and the "already exists" case is resolved the
    same way, so the message of an uncreatable directory agrees.
14. **The exception hierarchy.** The seven Python classes of the registry and
    the three of the simulator are the variants of one Rust enum each, so a
    caller that catches `DuplicateToolRegistrationError` in Python catches the
    `Duplicate` variant in Rust; there is no shared `ValueError` base.

## Remaining gaps and limits

1. **Not a runtime cutover.** Nothing in the product calls this crate yet.
   The sandbox and the executor cells still own their own modules and are not
   touched by this cell.
2. **The trusted privilege is a declaration.** The provenance of a handler is
   a declaration by the handler (see the difference 3), not a file system
   check. The executor cell should keep the mode itself the second gate: a
   registration is not a grant, and the executor also has to be configured
   with the explicit trusted in-process isolation mode.
3. **The misbehaviour entry points are not the worker.** The Python entry
   points are the module attributes of a Python worker. The Rust port
   reproduces their *behaviour*, not the worker: the `crash` entry point
   exits the process, the `flood` entry point writes to the standard output,
   the `hang` entry point sleeps and the stray child leaves a grandchild
   behind. The executor cell owns the worker that dispatches them.
4. **`os._exit` and the standard output.** The Python flood entry point writes
   to `sys.__stdout__`, which is the original standard output while the worker
   bootstrap has redirected `sys.stdout` to the standard error. The Rust port
   writes to the standard output directly; the redirection is the business of
   the Rust worker of the executor cell.
5. **The `strerror` text and the mode bits are platform dependent.** The
   differential ran on Linux only; the Windows behaviour is a compile check
   only.
6. **The exotic Unicode digits of a version are covered by unit tests, not by
   the differential.** The corpus of the differential uses ASCII versions,
   because the CPython `int()` behaviour of the other digits is a corner of
   the CPython integer conversion, not of the registry.
7. **No performance claim.** The differential and the test suite are
   regression guards, not throughput measurements.
8. **The crate root re-exports only the protocol API.** The registry and the
   simulator are reached through `kala_tools::registry` and
   `kala_tools::simulator`, which mirror the Python module paths. The crate
   root is left to the executor cell.

## Reproduction

```bash
cd backend/crates/kala-tools
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --locked --release
cargo build --locked --release
cargo check --locked --target x86_64-pc-windows-gnu
cargo check --locked --target x86_64-pc-windows-gnu --all-targets
```

The differential needs the repository virtual environment
(`/home/v9i/Projects/Kala-Agent/.venv/bin/python`); `KALA_TOOLS_PYTHON`
overrides it.
