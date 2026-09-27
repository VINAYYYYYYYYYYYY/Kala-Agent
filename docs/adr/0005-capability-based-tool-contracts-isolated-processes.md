# ADR-0005: Capability-Based Typed Tool Contracts Executed in Isolated Processes

| Field | Value |
|---|---|
| Status | **Proposed** — awaiting stakeholder confirmation. Implementation evidence does not convert a Proposed decision into accepted policy. |
| Date | 2026-09-25 |
| Last reviewed | 2026-09-26 |
| Scope | Tool boundary, authorization, execution isolation, worker model |
| Requirements addressed | FR-10 process-isolated tool execution, FR-11 capability-based typed tool contracts, FR-06 policy checkpoints, FR-14 least privilege and provenance |
| Related ADRs | ADR-0001, ADR-0002, ADR-0003, ADR-0004, ADR-0006 |
| Implementation status | **Implemented, with one material limitation on isolation strength.** `tools/protocol.py` defines a versioned wire protocol with strict JSON envelopes and hard bounds. `tools/registry.py` binds `(tool_id, version)` to schema digests, deep-freezes schemas, and re-verifies immutability on every lookup. `tools/executor.py` dispatches out of process, enforces capabilities, policy, budgets, workspace confinement, and artifact verification, and records a `ToolExecutionReport`. `tools/sandbox.py` probes a strict Linux jail in a real child process. **Hardening since the last review:** schema `pattern` admission is now strict — a bounded `maxLength` is mandatory, the pattern length is bounded, a fixed table of unsafe constructs is searched, nested repetition is rejected, **ambiguous repetition is rejected**, and over-budget repetition is refused, all decided from the pattern's own text **before anything is compiled and before the pattern is ever run against a value**, so a refused pattern is never compiled and never executed. Artifact classification is **monotone** toward `design_binary`, and an unknown format defaults to `design_binary` rather than falling through to a permissive case. `TOOL_CLASS_CHANGE` is genuinely enforced on every non-egress call. **Limitation, unchanged:** the shipped application uses `IsolationProfile.PORTABLE`, which confines a **process, not the filesystem** — `judge_sandbox` returns that reason verbatim with an empty control tuple, and `enforced_controls` never claims a filesystem control under it. The strict profile is implemented, probed from inside a real child, and fails closed for side-effecting tools, but it is opt-in and is not the product default, so **the fail-closed path is never reached by the shipped product**. There is no vendor CAD/CAE tool; the only registered tool is the first-party `mechanical.reference-simulator`. |
| Verification | **1,227 tests pass on CPython 3.11.16 and CPython 3.14.7**; branch coverage 94.76% / 94.77% against a configured `fail_under = 90`; `tests/security/test_tool_executor.py` (44 tests) and `tests/security/test_schema_redos.py` (9 tests) cover this ADR directly; `tools/protocol.py` 97.58%, `tools/sandbox.py` 99.13%, `tools/executor.py` 93.91%. See `docs/verification-report.md` §2. |

## Context

The agent needs to call useful operations—evaluate a derived quantity, inspect a parameter set, or later
invoke a CAD/CAE program—but an engineering design agent must not grant a model or a third-party package
ambient authority over the user’s device. A model-generated shell string, a path supplied by a package,
or a tool that silently opens a network connection can turn a planning request into data loss or
confidentiality loss.

There are several distinct requirements often conflated as “tool calling”:

- A **contract** describes what a tool accepts, returns, costs, and is allowed to do.
- A **capability** is a narrowly scoped permission to perform a particular kind of action.
- A **broker** decides whether a particular call satisfies the current session grant and policy.
- An **executor** runs the tool with the granted authority and resource limits.
- A **provenance record** explains which descriptor, package, input digest, and result produced an
  output.

A direct in-process function call cannot reliably enforce these boundaries. A subprocess without a
capability broker is still too broad, because the child may inherit the parent’s environment,
filesystem, network namespace, and credentials. Conversely, a full virtual machine for every small
calculation would make the first MVP unnecessarily expensive and difficult to operate. The boundary
therefore needs to be process-isolated by default and capable of stronger operating-system sandboxing
where the host provides it.

The system also needs bounded concurrency. A single-user local process should not create an unbounded
number of solvers or workers, and SQLite’s one-writer rule (ADR-0004) must not be bypassed by tools
writing directly to the database. Tool execution is a side effect of a workflow, not an authority to
mutate the design store.

## Decision

We adopt the following decisions.

**D1 — Every tool has a versioned, typed descriptor.** A tool is identified by `tool_id` and a
non-empty semantic version. Its descriptor declares:

- input and output JSON Schema digests;
- a human-readable name and description that cannot grant authority;
- a side-effect class: `NONE`, `READ`, `LOCAL_WRITE`, `NETWORK`, or `EGRESS`;
- required capabilities and optional resource limits;
- timeout, idempotency, destructiveness, and structured-output support flags.

The schema digests must resolve to schemas registered in the local tool registry. A model cannot
invent a schema, path, executable name, or capability. A descriptor change creates a new version;
an existing call retains the descriptor digest and version used at invocation time.

**D2 — Calls and results are typed protocol messages.** A `ToolCall` contains a stable call ID,
tool ID and version, canonical JSON arguments, requested capabilities, declared side-effect level,
timeout, and issue timestamp. A `ToolResult` contains the matching call ID and tool ID, a status,
canonical JSON result, stable error code/message, output `ArtifactRef` values, and timing metadata.
Arguments and results are parsed with strict settings: no unknown fields, no duplicate JSON keys,
no non-finite numbers, bounded strings and collections, and a bounded serialized size. Failure results
must not smuggle a partial result in the success payload.

**D3 — Capability grants are explicit, narrow, and short-lived.** A session receives a set of
capability tokens from the user, policy overlay, and package manifest. The broker computes the exact
capability closure required by one call and requires every token before dispatch. Capabilities are
not inferred from a tool’s usefulness, domain membership, or a previous successful call. A token is
scoped to a session or step, records the decision that granted it, and is revoked when the workflow
ends, the policy changes, or a checkpoint invalidates the action. A tool cannot grant itself a token.

**D4 — Least privilege is the default.** The baseline session policy allows local reads of explicitly
referenced design state and bounded computation, but does not allow network access, egress, package
installation, arbitrary filesystem access, or destructive writes. Each operation must request only
the capabilities named by its descriptor. A domain package’s `required_capabilities` is a request
for review, not an authorization. A new tool class or a materially broader capability requires a
`TOOL_CLASS_CHANGE` checkpoint.

**D5 — Side effects are classified and separately authorized.** `NONE` and read-only tools may run
inside a pre-authorized budget. `LOCAL_WRITE` tools receive a scoped writable directory or artifact
sink, not the journal or arbitrary user paths. `NETWORK` is denied unless a future policy explicitly
grants a non-egress network capability and records the destination. `EGRESS` is denied by default
and additionally requires the egress policy and checkpoint defined by ADR-0001. A descriptor cannot
downgrade its actual side effects; runtime audit and resource controls treat declared and observed
behavior as a trust boundary.

**D6 — Tool code never runs in the kernel process.** The kernel sends a validated call to a tool
supervisor, which starts or reuses a worker process from a bounded pool. The worker receives a
minimal message envelope over a private pipe or local IPC channel, not a Python object graph, shared
mutable memory, or a shell command. The worker returns one result envelope. The supervisor validates
the envelope, verifies output artifact digests, records the call/result events, and only then exposes
the result to the workflow. A tool cannot import kernel internals, mutate the event journal, or call
another tool directly.

**D7 — Sandboxing is layered and fail-closed.** The minimum profile is a separate process with a
non-privileged user identity where available, a dedicated temporary working directory, an allowlisted
read-only input mount, a scoped output directory, a minimal environment, no inherited API keys, no
network namespace unless a capability grants it, CPU/memory/output/time limits, and a process-group
kill on timeout or cancellation. On Linux, namespace, seccomp, and filesystem controls should be an
optional stronger profile; the portable fallback still keeps process isolation and denies ambient
authority. If a required sandbox control cannot be established, the call is denied and the reason is
journaled rather than downgraded to an in-process call.

**D8 — Resource bounds are part of the contract.** A call has a finite timeout, maximum input and
output bytes, declared concurrency class, and optional CPU/memory limits. The supervisor enforces
limits outside the tool so a malicious or faulty tool cannot opt out. Timeouts, cancellation, and
child-process termination are observable states. Retries are allowed only for an idempotent tool
and only under a fixed, budgeted policy; a non-idempotent or destructive call is never retried
automatically.

**D9 — Tool results are provisional until verified.** The supervisor checks result schema, call ID,
tool version, status consistency, artifact references, digest, size, classification, and provenance
requirements before the workflow can consume the result. Output files are promoted to the
content-addressed store through the atomic path in ADR-0004. A result that fails validation is a
failed tool event, even if the child process exited with status zero.

**D10 — Worker concurrency is bounded and writer-independent.** The MVP permits at most eight tool
workers by configuration. `Settings.worker_count` validates a range of one to eight, and
`ToolExecutor` enforces it with a `BoundedSemaphore` whose acquisition has a queue timeout, so
saturation denies (`worker_capacity_exhausted`) rather than queueing without bound. A worker only
ever returns a `ToolResult`; it holds no database handle, no event-journal path, and no
revision-coordinator reference, so it can neither open a write transaction against the authoritative
journal nor commit a revision. What does not exist is a general parallel *worker pool*: the parallel
dispatch that does exist lives in `runtime/agent.py`'s `AgentCoordinator`, which is not used by the
HTTP or CLI surfaces.

**D11 — Tool provenance is mandatory and append-only.** A successful call records the tool ID and
version, descriptor and schema digests, package manifest digest, argument digest, input artifact
digests, requested and granted capabilities, side-effect class, sandbox profile, process identifier
for diagnosis, result digest, output artifact digests, timing, and error code if any. Logs must not
contain credentials or raw secrets. The provenance graph is assembled by ADR-0006; this ADR defines
the execution facts it consumes.

**D12 — A small trusted built-in tool set is not an authorization bypass.** Pure parameter
validation, canonical JSON operations, and the simple mechanical reference calculation may be
implemented as kernel-level general operations where they are deterministic and side-effect free.
They still use the same typed result and budget accounting. Any operation that needs domain-specific
logic, native code, file access, or a solver belongs behind a tool descriptor and sandbox.

## Consequences

### Positive

- The model can request useful work without receiving unrestricted process, filesystem, or network
  authority.
- Typed schemas and digest-pinned descriptors make tool behavior reviewable, versioned, and
  testable before execution.
- Process isolation limits the blast radius of crashes, infinite loops, accidental writes, and
  dependency defects.
- Explicit capability grants make least privilege visible in the journal and straightforward to
  audit.
- Centralized supervision permits consistent timeouts, cancellation, resource limits, retries, and
  result validation.
- A bounded worker pool keeps a single-user process predictable and preserves the one-writer rule.
- Tool provenance connects every output to an exact descriptor, package, input, policy decision,
  and execution result.

### Negative

- Process startup and IPC add latency compared with an in-process Python function.
- Sandboxing, resource limits, and native solver integration are platform-sensitive and require
  careful testing, especially on Linux, macOS, and Windows.
- Strict schemas reject useful but poorly specified tools; every integration must define input,
  output, error, and artifact contracts.
- Capability administration creates more interruption points and requires clear user-facing names.
- Strong isolation can break legitimate tools that rely on shared libraries, temporary files, or
  hardware resources; adapters need documented profiles.
- A separate supervisor and worker lifecycle is more code to test than a direct function call.

### Neutral

- Tool calls are local workflow events, not remote RPC services, even when their implementation later
  wraps a remote vendor service; any network use still requires the appropriate policy.
- A tool may be computationally expensive without being untrusted; trust, side-effect class, and
  budget are separate axes.
- Process IDs and timing are diagnostic metadata and are not part of deterministic design state.
- The eight-worker limit is a maximum configuration, not a promise that every tool benefits from
  parallel execution.

## Alternatives Considered

**Direct in-process Python function calls with a capability check before invocation.**

- Rejected: a capability check cannot constrain what imported code does after the call, and a crash,
  infinite loop, or accidental import can affect the kernel. It is acceptable only for a narrowly
  defined, general, side-effect-free operation explicitly treated as part of the kernel.

**A model-native tool-calling protocol where the model emits executable calls directly.**

- Rejected: it makes the provider schema and model output part of the execution authority boundary,
  and it bypasses a stable, provider-neutral catalog. The model may propose a catalog step, but the
  kernel and broker issue the actual typed call.

**A container or virtual machine for every tool invocation.**

- Deferred as the universal MVP mechanism: strong isolation is valuable but image distribution,
  startup cost, hardware passthrough, and platform support are disproportionate for the first local
  reference workflow. The contract remains compatible with a stronger profile later.

**A single unrestricted long-running tool server process.**

- Rejected: it creates ambient credentials, shared mutable state, unclear cleanup, and a single
  failure domain. Short-lived or explicitly pooled workers with a narrow IPC protocol are safer.

**Let tools write directly to SQLite and artifact paths for efficiency.**

- Rejected: it bypasses the single writer, event ordering, classification, digest verification, and
  provenance. Tools return bytes or scoped outputs to the supervisor, which performs the only
  authoritative write.

**Allow arbitrary shell commands supplied by a domain package or user prompt.**

- Rejected: shell parsing expands the attack surface and makes capability intent ambiguous. Commands
  must be represented as a registered tool ID plus typed arguments.

**Use OS file permissions alone for sandboxing.**

- Rejected as a complete control: permissions do not constrain network, CPU, memory, or output size,
  and they do not provide a portable per-call policy. They remain a useful layer in the sandbox.

## Trade-offs

- We accept additional latency and implementation complexity in exchange for a clear isolation and
  least-privilege boundary around tools.
- We accept a conservative capability model in exchange for preventing a model or package from
  silently acquiring network, filesystem, or destructive authority.
- We accept a portable process-only baseline with optional OS hardening in exchange for keeping the
  MVP operable on ordinary local machines; stronger profiles can be added without changing the
  tool contract.
- We accept strict catalog and schema enforcement in exchange for refusing improvisation that cannot
  be validated or reviewed.
- We accept serialized authoritative writes behind parallel workers in exchange for preserving total
  event order and crash recovery.

## Revisit Triggers

- A required CAD/CAE tool cannot run under the portable process profile and needs a signed,
  containerized, or hardware-specific adapter.
- The local workload demonstrates that eight workers are insufficient or wasteful.
- A tool requires a capability model that cannot be expressed with the current side-effect classes.
- A future hosted deployment requires authenticated remote execution or distributed workers.
- Native integrations need a reproducible dependency and binary-signing policy beyond the MVP.

## Verification

Status as of 2026-09-26. All of these live in `tests/security/test_tool_executor.py` unless noted.

1. Contract tests validate every descriptor and reject unknown fields, invalid schemas, unsafe limits,
   inconsistent side-effect declarations, and malformed results. **Met.** `ToolDescriptor` refuses
   `destructive` combined with `side_effect_level = none`; `ToolRegistry.build` requires a
   side-effecting tool to declare at least one capability; schema digests are recomputed at
   registration and raise `SchemaDigestMismatchError` on mismatch; a mutated registration is detected
   on the next `require()`; and schema `pattern` values are length-, input-, and repeat-bounded with
   nested quantifiers rejected **before** the pattern reaches `re`. (`:1440, :1467, :1557, :1209`.)
2. Capability tests deny a call that omits one required token and grant it only when the exact token
   set and checkpoint decision are present. **Met.** Effective capabilities are the union of the
   descriptor's and the call's, intersected with the grant; a call requesting a capability the
   descriptor does not declare is denied; registration refuses a tool claiming ungranted authority;
   and with a `PolicyEngine` attached every call becomes a `TOOL_CLASS_CHANGE` (or `EGRESS`) policy
   request, so first use of a class needs approval and repeats inside the class do not. An approval is
   bound to the argument digest, the resolved workspace, the effective capabilities, and the tool and
   descriptor digests, so it cannot be replayed. (`:393, :404, :496, :504, :516, :552, :618`.)
3. Isolation tests run a tool that attempts network access, arbitrary file access, environment-secret
   access, and parent-process interaction, and verify that each attempt is denied or isolated.
   **Partially met, and the gap is stated rather than papered over.** Network and egress capabilities
   are denied by name unless the profile opts in (`:434`). The child environment is built from a
   fixed key set rather than inherited, and executor-owned keys cannot be overridden (`:1364, :1399`).
   Workspace traversal, out-of-workspace artifacts, and symlinked artifacts are refused (`:992, :1024,
   :1061`). A stray child process is killed with its process group and reaped (`:724`). **But** under
   the portable profile a tool can read the host filesystem and the parent process's environment,
   because the profile confines a process and not a filesystem. That is residual risk RR-1, and the
   strict profile that does confine it is opt-in.
4. Resource tests cover timeout, cancellation, output-size limit, memory/CPU limit, child cleanup,
   and retry behavior for idempotent and non-idempotent tools. **Met for the bounds that exist.**
   Wall-clock deadline, process-group `SIGKILL` and reap, `RLIMIT_CPU`, `RLIMIT_FSIZE`, `RLIMIT_NOFILE
   = 256`, `RLIMIT_CORE = 0`, `umask 0o077`, bounded stdout and stderr readers, bounded arguments, a
   bounded worker semaphore with a queue timeout, and a budget reservation committed after the fact.
   (`:627, :724, :871, :905, :940, :982, :757, :790, :1602`.) **Not covered:** retry policy for a
   non-idempotent tool after a crash — the executor does not retry at all, which is the safe default,
   but no test asserts that a retry must not occur.
5. Concurrency tests demonstrate that workers cannot commit revisions and that eight is the enforced
   maximum. **Met.** A worker only ever returns a `ToolResult`; it holds no database handle and no
   revision-coordinator authority. `MAX_WORKERS = 8`, `SandboxProfile.max_workers` is bounded 1..8, and
   `Settings.worker_count` is bounded 1..8. (`:757`.)
6. Provenance tests reconstruct the exact tool descriptor, package digest, arguments, inputs, outputs,
   and policy decision from journal records. **Partially met.** Every call produces a
   `ToolExecutionReport` recording the tool and version, the isolation mode, the sandbox profile, the
   process id, the granted capabilities, the artifact bytes, the enforced controls, and the envelope
   digest, and the approval binding is recomputable from the request. What is missing is the *query*:
   there is no provenance graph traversal and no audit-query builder, so reconstruction is a matter of
   reading the journal and the in-memory report rather than of running a query. See
   `docs/architecture-report.md` §3.8.
7. A test asserts that a hostile schema `pattern` cannot be registered to make the validator run
   long, and that a refused pattern is never compiled. **Met.** `_bounded_pattern`
   (`tools/protocol.py:459`) decides admission from the pattern's own text, before anything is
   compiled and before the pattern is ever run against a value: a bounded `maxLength` is mandatory,
   the pattern length is bounded, a fixed table of unsafe constructs is searched (backreferences,
   lookaround, conditional groups, inline flags, atomic/possessive groups, wildcard repetition),
   nested repetition is rejected, **ambiguous repetition is rejected**, and over-budget repetition is
   refused. `tests/security/test_schema_redos.py` proves this from four directions — the compiler is
   monkeypatched to show a refused pattern is never reached (`:145`), the admitted subset still matches
   and still binds (`:162`), refusing a hostile pattern is itself bounded work (`:171`), and both the
   validator and the registry refuse an ambiguous schema (`:130,181,193`). The construct table is
   enumerated rather than derived, and no fuzzing was run, so an unlisted ReDoS shape is not caught by
   construction.
8. A test asserts that a tool cannot downgrade a `design_binary` verdict, and that an unknown format
   is classified `design_binary` by default. **Met.** `_derive_classification` (`tools/executor.py:441`)
   is monotone: a recognised signature, non-decodable bytes, an unrecognised extension, or the tool's
   own claim all yield `design_binary`, and no weaker claim moves content back to `TEXT`. An unknown
   format defaults to the strict case rather than falling through to a permissive one.
   (`:1124`.)
9. A test asserts that `enforced_controls` never lists a control the host does not actually enforce.
   **Met, and the portable verdict is deliberately shaped to say so.** The portable verdict is allowed
   with a non-empty `reason` — the literal string `the portable profile confines a process, not the
   filesystem` — and an **empty** `controls` tuple. The strict verdict is allowed with an empty
   `reason` and the full observed control set, every one verified from inside a real child process.
   On the verification host the probe reported all six required controls plus `device_nodes_minimal`,
   `ipc_namespace`, `pid_namespace`, `procfs`, and `uts_namespace`, and correctly did **not** report
   `scratch_tmpfs`, which that host does not provide. (`:568`, `docs/verification-report.md` §5.)

## Implementation notes (2026-09-26)

| Decision | Implementation | Reference |
|---|---|---|
| D1 versioned typed descriptor | `ToolDescriptor`; registry keyed by `(tool_id, version)`, `MAX_REGISTERED_TOOLS = 512`, version-keyed `latest()` | `tools/registry.py:245` |
| D2 typed protocol messages | `PROTOCOL_VERSION = "1.0"`; one `ToolRequestEnvelope` on stdin, one `ToolResponseEnvelope` on stdout, with stdout redirected to stderr for the duration so tool prints cannot corrupt the channel. **Schema `pattern` values go through strict admission** — mandatory bounded `maxLength`, bounded pattern length, a fixed unsafe-construct table, rejected nested repetition, rejected **ambiguous** repetition, and a backtracking budget — decided from the pattern text before anything is compiled or run | `tools/protocol.py:459` |
| D3 explicit, narrow, short-lived grants | `CapabilityGrant(capability, scope, expires_at)`; wildcard `"*"` refused at construction; an expired grant denies | `kernel/policy.py:259` |
| D4 least privilege by default | Default grant is `artifact.read`, `artifact.write`, `design.read`, `tool.execute`, `design.write`; network, egress, and privileged names are denied unless the profile opts in | `application.py`, `tools/executor.py:1041` |
| D5 side effects classified and separately authorized | `SideEffectLevel`; a call whose declared side effect differs from the descriptor is denied; side-effecting and destructive tools need an approval | `tools/executor.py:1095` |
| D6 tool code never runs in the kernel | `IsolationMode.AUTO` refuses an in-kernel handler rather than invoking it; in-kernel execution needs the explicit `TRUSTED_IN_PROCESS_EVALUATOR` mode plus a registry-marked first-party, side-effect-free, non-destructive, non-privileged evaluator | `tools/executor.py:1152` |
| D7 layered, fail-closed sandbox | `probe_sandbox` builds the exact jail in a throwaway interpreter and verifies from the inside; only observed controls are reported; when the strict profile is selected and the probe fails, side-effecting and destructive tools are denied and read-only tools still run. **The shipped default is the portable profile**, whose verdict carries the literal reason `the portable profile confines a process, not the filesystem` and an **empty** control tuple — so the fail-closed branch above is real, tested, and not reached by the product | `tools/sandbox.py:690,803` |
| D8 resource bounds in the contract | Timeout on the descriptor and the call, `max_output_bytes`, `max_argument_bytes`, `max_artifact_bytes`, `max_workers`, `max_queue_seconds` | `tools/executor.py:214` |
| D9 results provisional until verified | Identity, terminal status, result schema, artifact confinement, artifact size, and artifact digest are all re-checked before the result reaches the state machine. Classification is **monotone** toward `design_binary`, and an unknown format defaults to `design_binary` | `tools/executor.py:441,1514` |
| D10 bounded, writer-independent workers | A `BoundedSemaphore` sized to the profile; a worker never commits anything | `tools/executor.py:1182` |
| D11 mandatory append-only tool provenance | `tool_calls` and `tool_results` rows with a restrictive foreign key, plus the in-memory `ToolExecutionReport` | `persistence/database.py:174` |
| D12 a small trusted built-in set is not a bypass | The trusted-evaluator privilege is granted only through `build_trusted_evaluator`, which re-checks first-party provenance, purity, and non-privilege on every registration and every lookup | `tools/registry.py:286` |

One thing this ADR asked for that is worth calling out explicitly, because it is easy to assume: **a
child exiting zero is not evidence of success.** A response that violates its schema, digest, status,
identity, or provenance contract is a failed call, and a tool that returns an unrecognised error code
has it normalized to `internal_error` so it cannot mint an authoritative-looking status.

The enforced control list, which is what the system will actually *claim* on the default profile:

```text
artifact_classification_kernel_verified   output_size_limit
artifact_digest_verification              process_group_kill
cwd_confined                              process_isolation
environment_minimal                       wall_clock_timeout
file_descriptor_close                     worker_concurrency_cap
[resource_limits]
```

Notably absent: anything about the filesystem. That absence is the honest state of portable mode, and
`enforced_controls` will add the probed controls when the strict profile is selected.

## References

- `docs/architecture-report.md` — the as-built tool plane (§10), the two isolation profiles and honest
  control reporting (§10.5), the reference tool (§10.6), and the sandbox deviation (§3.6).
- `docs/verification-report.md` — the 44-row control table for tool execution, the static sandbox
  probe result (§5), and residual risks RR-1, RR-4, and RR-7.
- `docs/threat-model.md` — TB-3, the tool process boundary: T3.1–T3.15.
- ADR-0001 — Local-First, No-Egress-By-Default Operation with a Provider-Neutral LLM Gateway.
- ADR-0002 — Field-Independent Kernel with Declarative Domain Packages.
- ADR-0003 — Deterministic Workflow Engine with Bounded LLM Planning and Policy Checkpoints.
- ADR-0004 — Single-Writer Immutable Revisions over an Event Journal and Content-Addressed Artifacts.
- ADR-0006 — Package Trust, Sandboxing, Provenance, Rollback, and Agent Budgets.
