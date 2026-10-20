# application-r1 — `kala-application`, the composition root

Cell: `application-r1` of the `rust-backend-20261019` wave. Owner: the Rust
worker `application-r1`. Status: **complete** — the composition root is ported,
the live differential against the repository's Python `KalaApplication` is green
with **zero unexplained mismatches**, the cross-reader runs in both directions,
and every Rust gate passes except the Windows cross-check, which cannot compile
because the mingw C toolchain is not installed here (evidence in §7).

No commit, no push, no delegation, no credential read, and no edit outside
`backend/crates/kala-application/` and this file.

---

## 1. What was built

`backend/crates/kala-application` is a NEW standalone crate (its own
`[workspace]` and `Cargo.lock`) that ports
`src/kala_agent/application.py` (3,010 lines) into the native composition root.
It composes the accepted crates **read-only**, by path:

| Accepted crate | What the root uses |
|---|---|
| `kala-json` | canonical bytes, digests, `CanonicalValue`, `Limits` |
| `kala-contracts` | `DesignSpec`, `DesignRevision`, `Provenance`, `ArtifactRef`, `ApprovalRequest`, `DomainPackageManifest`, `EvaluationResult`, `Timestamp`, `Text`, `WireValue` |
| `kala-persistence` | `Database`, `EventJournal`, `SnapshotStore`, `EventFilter`, `parse_json_object` |
| `kala-repositories` | `DesignSpecRepository`, `DesignRevisionRepository`, `ApprovalRequestRepository` |
| `kala-budget` | `BudgetLedger`, `RunBudget`, `Counter` |
| `kala-artifacts` | `ContentAddressedArtifactStore` |
| `kala-kernel` | `PolicyEngine`, `PolicyRequest`, `PolicyDecision*`, `WorkflowEngine`, `WorkflowState`, `WorkflowApproval` |
| `kala-domains` | the three packages, parameter models, reference workflow runner, capability enforcement |
| `kala-tools` | `ToolRegistry`, `register_simulator`, `ToolExecutor`, `SandboxProfile` |
| `kala-runtime` | `ModelGateway`, `LocalModelClient`, `OfflinePlanner`, `StepCatalog`, `CatalogStep` |
| `kala-api` | `config::Settings`, `settings_from_values`, `settings_from_env` |

The root owns composition and lifecycle only. It builds a deterministic local
model client, grants only the capabilities the declared packages need, never
opens a socket, journals every consequential step, and writes revisions in the
two-phase pre-commit flow. Policy and budget state live in reserved snapshots
and are restored at startup; a missing, unverifiable or invalid record stops the
launch.

### Changed paths (the only ones I touched)

| Path | What |
|---|---|
| `backend/crates/kala-application/Cargo.toml` | standalone workspace, path dependencies, `Cargo.lock` |
| `backend/crates/kala-application/src/lib.rs` | crate surface, re-exports, identity/limitations docs |
| `backend/crates/kala-application/src/error.rs` | `ApplicationError`, `ErrorKind`, exact codes/class names |
| `backend/crates/kala-application/src/value.rs` | the Python validators, timestamp conversion, canonical JSON helpers, `View` support |
| `backend/crates/kala-application/src/dto.rs` | every `_ViewModel` DTO, wire/canonical forms |
| `backend/crates/kala-application/src/domains.rs` | `DomainBinding`, parameter parsing, reference evaluation |
| `backend/crates/kala-application/src/application.rs` | `KalaApplication`, constants, all operations and lifecycle |
| `backend/crates/kala-application/tests/oracle_application.py` | test-only live Python oracle (main, corpus, wire, seed, inspect) |
| `backend/crates/kala-application/tests/differential.rs` | live differential: main, 52-case corpus, wire JSON |
| `backend/crates/kala-application/tests/cross_read.rs` | Python↔Rust data-directory cross-reading |
| `backend/crates/kala-application/tests/security.rs` | 23 security/durability cases |
| `backend/crates/kala-application/tests/public_api.rs` | 5 public-surface cases |

Nothing else was modified: every accepted crate, the root `backend/Cargo.toml`
and lock, the Python source/tests, shared fixtures, adapters and `studio/**` are
untouched. Other cells were writing `backend/crates/kala-api` concurrently; its
transient compile states were waited out, never edited.

---

## 2. Public API for the later `api-routes` and `cli` cells

Everything below is `pub` from `kala_application`. The API is synchronous and
`KalaApplication` is `Send + Sync`, so it can be shared behind an `Arc` (the
concurrency test uses two threads).

### 2.1 Construction

```rust
use kala_application::{KalaApplication, Components, Clock, DomainBinding};

// Defaults, or explicit kala-api Settings (already validated/normalized).
// `open_default()` builds the Python `Settings()` defaults (it does not read
// the process environment); the CLI cell owns environment-derived settings.
let app = KalaApplication::open_default()?;
let app = KalaApplication::open(settings)?;

// Test/embedding injection. A supplied component belongs to the caller and is
// never closed by `KalaApplication::close`.
let app = KalaApplication::with_components(
    settings,
    Components {
        clock: Some(clock),                 // Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>
        domains: Some(DomainBinding::defaults()),
        ..Components::default()
    },
)?;
```

`Components` fields: `database: Option<Arc<Database>>`,
`journal_database: Option<Arc<Database>>`, `artifact_store`,
`budget: Option<Arc<BudgetLedger>>`, `policy: Option<Arc<PolicyEngine>>`,
`tool_registry: Option<Arc<ToolRegistry>>`,
`tool_executor: Option<Arc<ToolExecutor>>`,
`model_gateway: Option<Arc<ModelGateway>>`, `planner: Option<OfflinePlanner>`,
`domains: Option<Vec<DomainBinding>>`, `clock: Option<Clock>`.

The Python constructor accepts an `EventJournal` object; the native equivalent
is `journal_database` (the database the journal reads/writes), because a
repository/journal instance in the accepted crates is a borrow of `Database`.

### 2.2 Facts and components

```rust
app.settings() -> &kala_api::config::Settings
app.database() -> &Arc<Database>
app.journal() -> ApplicationResult<EventJournal<'_>>
app.verify_chain() -> ApplicationResult<kala_persistence::ChainVerification>
app.artifact_store() -> &ContentAddressedArtifactStore
app.snapshots() -> SnapshotStore<'_>
app.design_specs() -> DesignSpecRepository<'_>
app.design_revisions() -> DesignRevisionRepository<'_>
app.approval_requests() -> ApprovalRequestRepository<'_>
app.budget() -> &Arc<BudgetLedger>
app.policy() -> &Arc<PolicyEngine>
app.tool_registry() -> &Arc<ToolRegistry>
app.tool_executor() -> &Arc<ToolExecutor>
app.model_gateway() -> &Arc<ModelGateway>
app.planner() -> &OfflinePlanner
app.step_catalog() -> &Arc<StepCatalog>
app.domains() -> Vec<&'static str>            // sorted: building, mechanical, wing
app.closed() -> bool
app.offline() -> bool
app.egress_configured() -> bool
app.provider_endpoint_declared() -> bool
app.model_gateway_supplied() -> bool
app.provider_id() -> &str
app.model_id() -> &str
```

`offline()` is a fact about the gateway actually held (the built local client is
offline; a caller-supplied egress gateway is not); `provider_endpoint_declared()`
reports a setting only and is never claimed as egress.

### 2.3 Domain declarations

```rust
app.list_domain_packages() -> Vec<DomainPackageView>
app.list_workflow_definitions() -> Vec<WorkflowDefinitionView>
app.list_catalog_steps(domain: &str) -> ApplicationResult<Vec<CatalogStepView>>
app.describe_domain(domain: &str) -> ApplicationResult<DomainDetail>
```

`DomainBinding::new(DomainName)`, `DomainBinding::defaults()`,
`with_reference_parameters`, `with_reference_step_ids`, `with_subject_id`,
`domain()`, `package()`, `reference_parameters()`, `reference_step_ids()`,
`subject_id()`, `parameters(overrides)`.

### 2.4 Evaluation

```rust
app.evaluate_domain_design(
    domain: &str,
    parameters: Option<&CanonicalValue>,   // JSON object overrides or None
    design_id: Option<&str>,               // subject or the declared reference
) -> ApplicationResult<DomainEvaluationReport>
```

### 2.5 Revisions (two-phase commits)

```rust
app.prepare_design_revision(
    design_id: &str, domain: &str,
    summary: Option<&str>,
    parameters: Option<&CanonicalValue>,
    parent_revision_id: Option<&str>,
    parent_provenance_ids: &[String],
    transformation_ids: &[String],
    workflow_id: Option<&str>,
) -> ApplicationResult<RevisionCommitView>

app.decide_revision_commit(
    request_id: &str, decision: &str,          // "approve" | "reject"
    actor_id: &str,
    state_digest: Option<&str>,                // binds the decision to a review
    action_digest: Option<&str>,
    reason: Option<&str>,
    workflow_id: Option<&str>,                 // authorizing completed workflow
) -> ApplicationResult<RevisionCommitView>

app.commit_design_revision(
    request_id: &str, actor_id: &str,
    state_digest: Option<&str>, action_digest: Option<&str>,
    reason: Option<&str>, workflow_id: Option<&str>,
) -> ApplicationResult<RevisionRecord>       // approve + read back

app.create_design_revision(
    design_id: &str, domain: &str, summary: Option<&str>,
    parameters: Option<&CanonicalValue>, parent_revision_id: Option<&str>,
    parent_provenance_ids: &[String], transformation_ids: &[String],
    workflow_id: Option<&str>, actor_id: &str,
    state_digest: Option<&str>, action_digest: Option<&str>,
) -> ApplicationResult<RevisionRecord>       // prepare + commit

app.get_revision_commit(request_id: &str) -> ApplicationResult<RevisionCommitView>
app.list_design_revisions(design_id: &str, limit: i64) -> ApplicationResult<Vec<RevisionRecord>>
app.get_design_revision(revision_id: &str) -> ApplicationResult<RevisionRecord>
```

There are no default arguments in Rust: pass `LOCAL_ACTOR_ID`, `None`, `&[]`
explicitly where the Python method has a default. `list_design_revisions`
defaults to `MAX_REVISION_LIMIT` (1,000) in Python, so an API route should pass
`MAX_REVISION_LIMIT` when the caller did not supply a limit.

### 2.6 The mechanical reference workflow

```rust
app.run_mechanical_reference_workflow(
    workflow_id: Option<&str>,      // default "workflow-<design_id>"
    design_id: &str,                // default DEFAULT_MECHANICAL_DESIGN_ID
    actor_id: &str,
) -> ApplicationResult<MechanicalRunResult>

app.get_workflow_state(workflow_id: &str) -> ApplicationResult<WorkflowStateView>
app.list_workflow_checkpoints(workflow_id: &str) -> ApplicationResult<Vec<ApprovalRequest>>
app.approve_workflow_checkpoint(workflow_id: &str, actor_id: &str,
    state_digest: Option<&str>, action_digest: Option<&str>, reason: Option<&str>)
    -> ApplicationResult<MechanicalRunResult>
app.reject_workflow_checkpoint(...same...) -> ApplicationResult<MechanicalRunResult>
app.decide_workflow_checkpoint(workflow_id: &str, decision: &str, actor_id: &str,
    state_digest: Option<&str>, action_digest: Option<&str>, reason: Option<&str>)
    -> ApplicationResult<MechanicalRunResult>
```

### 2.7 Journal and lifecycle

```rust
app.list_events(
    after_sequence: i64, limit: i64,
    workflow_id: Option<&str>, event_type: Option<&str>, aggregate_id: Option<&str>,
) -> ApplicationResult<EventPage>
app.close() -> ApplicationResult<()>
```

`after_sequence`/`limit` have Python defaults `0`/`50`; pass them explicitly.
`close()` is idempotent, marks the root closed even when the final kernel-state
flush fails (mirroring the Python `finally`), and closes only an owned database.
There is deliberately no `Drop` flush: a constructor that fails while restoring
corrupt kernel state must not write a fresh snapshot over the evidence, so the
lifecycle is explicit exactly as the Python `with` block is.

### 2.8 DTOs

`kala_application::View` gives every read model three forms:

```rust
view.to_wire_value() -> kala_contracts::WireValue   // model_dump order
view.to_wire_json() -> ApplicationResult<String>    // FastAPI/Starlette bytes
view.to_canonical_value() -> ApplicationResult<CanonicalValue>
view.to_canonical_json() -> ApplicationResult<String>
```

The DTOs are `DomainPackageView`, `CatalogStepView`, `WorkflowTransitionView`,
`WorkflowDefinitionView`, `DomainDetail`, `DomainEvaluationReport`,
`StepAttemptView`, `WorkflowTransitionRecordView`, `CheckpointView`,
`WorkflowStateView`, `BudgetUsageView`, `RevisionRecord`, `RevisionCommitView`,
`MechanicalRunResult`, `EventView`, `EventPage`. They mirror the Python
`_ViewModel` field order and types; `to_wire_json()` is byte-identical to
`json.dumps(model.model_dump(mode="json"), ensure_ascii=False,
separators=(",", ":"))` (the Starlette transport form), which is what the API
should send.

### 2.9 Errors

```rust
ApplicationError { kind() -> ErrorKind, code() -> &'static str, message() -> &str }
ErrorKind::python_class_name() -> &'static str
ErrorKind::is_lookup() / is_value()
```

Codes and class families are the Python ones (`unknown_domain`,
`invalid_identifier`, `reserved_identifier`, `invalid_parameters`,
`unknown_revision`, `unknown_workflow`, `provenance_unavailable`,
`checkpoint_not_pending`, `checkpoint_denied`, `checkpoint_digest_mismatch`,
`revision_commit_rejected`, `revision_fork_refused`, `revision_head_moved`,
`unknown_prepared_revision`, `corrupt_kernel_state`, `workflow_finished`,
`application_closed`, `application_error`). The `ErrorKind` mapping lets the
transport reproduce the Python status table without an `isinstance` chain:
lookup ⇒ 404; the checkpoint/revision/close conflicts ⇒ 409; corrupt/closed ⇒
503; invalid identifiers/parameters ⇒ 422; the rest ⇒ 400.

### 2.10 Helpers and constants

`artifact_digest(payload)`, `kernel_state_aggregate(aggregate)`,
`mechanical_step_catalog()`, `reserved_workflow_ids()`, and every Python
constant (`LOCAL_ACTOR_ID`, `DOMAIN_NAMES`, `DEFAULT_MECHANICAL_DESIGN_ID`,
`PRE_COMMIT_CAPABILITIES`, `APPROVAL_OPTIONS`, `CHECKPOINT_DECISIONS`,
`REVISION_DECISIONS`, limits, event names, aggregates, transforms, …).

---

## 3. Fidelity

The whole module is ported: every public method, the DTOs, validation rules and
messages, error precedence, canonical JSON/digest bytes, aware-timestamp
semantics, Decimal cost rendering, limits and the two-phase commit. The
constraints ADR-0002/0003/0004 impose on the root are preserved: a
field-independent kernel composed with declarative packages, a deterministic
workflow with digest-bound checkpoints, immutable single-writer revisions over
a verified journal and content-addressed artifacts, the reserved `kernel-state:`
namespace, and fail-closed restore. Notable
details preserved:

* identifiers: trim, `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$`, then the per-field
  bound; reserved `kernel-*` workflow names are refused before any work;
* text: Python `str.strip()` (including U+001C..U+001F), emptiness, bounds;
* limits: `1..=1000`, booleans refused where Python refuses them;
* parameter summaries: `"<field>: <Pydantic message>"`, including extra-field,
  type, bound, enum and model-level messages;
* revision identities: `rev-<design>-r<sequence>`, `prov-…`,
  `commit-<design>-<state[:16]>-<action[:16]>`;
* the state digest covers head/parent/spec/package/artifact; the action digest
  covers the authorized write; both are canonical JSON SHA-256;
* the report artifact is the canonical report JSON plus `"\n"`, stored
  content-addressed, and charged once to the budget;
* journal timestamps are the journal's own UTC now (truncated to microseconds);
  the injected clock is consulted exactly where Python consults it;
* policy and budget snapshots are written at the end of every unit of work and
  on close, under the reserved `kernel-state:` namespace, with the bare-name
  fallback for legacy records;
* a failed unit restores the policy engine to its pre-unit state (including on
  panic), while artifact bytes and budget charges already spent stay spent;
* closed roots refuse further work with `application_closed`.

---

## 4. The live differential

The oracle is `tests/oracle_application.py`, run as
`<repo>/.venv/bin/python -I tests/oracle_application.py <scenario> <data-dir>`
with the repository `src` inserted by the script itself (so `-I` isolation is
real). Python is a test oracle only; the crate never invokes it.

One shared operation script is driven through the real Python
`KalaApplication` and through the Rust root, each in its own temporary data
directory. The trace compares:

1. every returned DTO as canonical JSON;
2. every SQLite row: schema SQL, ids, counts, all columns, and decoded JSON
   columns (`design_specs`, `design_revisions`, `approval_requests`, `events`,
   `workflow_snapshots`, `tool_calls`, `tool_results`);
3. the complete journal chain: sequence, type, aggregate, workflow, payload,
   digest;
4. the artifact store: layout parts, size, SHA-256 and parsed report;
5. every error class, code and message;
6. the injected-clock call count and every deterministic (2031) timestamp;
7. the restart: grants, budget, revisions, commits, checkpoints, workflow state.

Wall-clock timestamps (journal events and the workflow engine's internal
checkpoint decision time) and 64-hex digest values are normalized. The
normalization is exactly the declared identity boundary (§8): the accepted
`kala-domains` publishes native evaluator/package digests, so every digest
computed over a package digest differs. The test then proves the digest
*algorithm* separately:

* it substitutes the Python package digest into the native artifact/spec and
  recomputes the Python report bytes, artifact digest, spec digest, state
  digest, action digest and request id — all byte-identical;
* for every “clean” journal event (one whose payload carries no package-derived
  digest), it asserts the raw event digest and payload are identical between
  Python and Rust. Only `workflow.approval_resolved`/`invalidated` are exempt
  from the digest equality, because the workflow engine (in Python and in Rust)
  stamps them with the real wall clock rather than the root's injected clock.

### 4.1 Results

| Scenario | Scope | Result |
|---|---|---|
| main | 71 recorded operations, 6 revisions, 2 workflows (approve + reject), revoked capability, stale digests, forks, paging/filters, reserved ids, restart; 59 events, 64 snapshots, 11 approvals, 6 artifacts; 88 clock calls before close, 90 total | **equal** after the declared normalization; clock counts exact; clean event digests exact; identity substitution exact |
| corpus | 52 deterministic cases: 16 mechanical evaluations, 8 building, 8 wing, 12 invalid Pydantic cases (byte-identical error messages), 4 commits + 4 reads; 45 events, 84 snapshots, 4 revisions, 4 artifacts | **equal** |
| wire | 11 DTOs compared as exact transport bytes (domain detail, evaluation report, prepared/decided commit views, revision record, run result, workflow state, approval request, package view, workflow definition, catalog step) | **byte-identical** after only the package/commit identity substitution; field order, CPython float formatting and UTF-8 encoding match |
| cross-read A | Python writes a dir (2 revisions, workflow approval, revoked grant, pending commit, artifacts) → Rust opens, verifies, restores the revoked grant and pending approval, reads the artifact bytes, extends → Python opens, re-grants, reads Rust's row and artifact, extends → Rust reopens and reads Python's writes | **pass** |
| cross-read B | Rust writes the same shape → Python opens, restores Rust's native policy/budget state, reads Rust's row/artifact, extends → Rust reopens and reads Python's writes | **pass** |

Zero unexplained mismatches. Every observed difference is one of the declared
identity-boundary or wall-clock normalizations in §8.

---

## 5. Compatibility and security evidence

* **Same SQLite schema and rows.** The differential compares the full
  `sqlite_master` SQL and every row of every table. The cross-reader further
  proves both implementations can open, verify and *extend* each other's data
  directory, including on a foreign head.
* **Same journal chain.** Rust appends to a Python chain and Python appends to
  a Rust chain; each side's `verify_chain()` then verifies the whole chain. Clean
  events are byte-identical; identity-bearing events differ only in the
  declared digest inputs.
* **Same artifact store.** Both read each other's `objects/<ab>/<digest>` bytes
  and confirm SHA-256; new reports from either side are content-addressed in the
  same layout (`objects/<ab>/<digest>`, trailing `"\n"`).
* **Kernel state compatibility.** Python restores Rust's exported policy state
  (grants, approvals, tracked requests) and budget state; Rust restores
  Python's. The cross tests use a revoked capability and a pending prepared
  commit, so a silent reset to defaults would fail.
* **Security cases** (23 tests): a prepared commit is not a revision; a stale
  digest, rejection, fork, revoked capability, lost head race and concurrent
  double commit all write nothing; a workflow revision requires its own single
  approved pre-commit checkpoint; a workflow cannot authorize another design;
  the journal is append-only (delete trigger) and a payload edit is detected at
  the next launch; tampered/invalid/missing kernel state stops the launch;
  legacy bare-name snapshots still restore; reserved identifiers are refused
  everywhere before work; closed roots refuse work; injected components survive
  close; two threads produce exactly one revision; identifier/limit/parameter
  errors keep code and message.

---

## 6. Tests

From `backend/crates/kala-application`:

| Suite | Tests |
|---|---:|
| `tests/cross_read.rs` | 2 |
| `tests/differential.rs` | 3 |
| `tests/public_api.rs` | 5 |
| `tests/security.rs` | 23 |
| unit tests in `src/lib.rs` | 0 |
| **total** | **33** |

All 33 pass in debug and release, 0 failed, 0 ignored. The live differential
needs `/home/v9i/Projects/Kala-Agent/.venv/bin/python` (override with
`KALA_PYTHON`); the Rust library itself has no Python dependency.

---

## 7. Gates

Run in `backend/crates/kala-application`:

| Gate | Result |
|---|---|
| `cargo fmt --check` | pass |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | pass |
| `cargo test --locked` | 33 passed, 0 failed, 0 ignored |
| `cargo test --locked --release` | 33 passed, 0 failed, 0 ignored |
| `cargo build --locked --release` | pass |
| `cargo check --locked --target x86_64-pc-windows-gnu` | **cannot run**: the target is installed, but the mingw C compiler is not (`libsqlite3-sys`/`ring` build scripts fail with `failed to find tool "x86_64-w64-mingw32-gcc"`). This is a missing cross toolchain, not a code error; no Windows runtime claim is made. |

Two consecutive debug runs and two release runs produced the same result.

---

## 8. Intentional differences

1. **Domain package identity (the one declared identity boundary).** The
   accepted `kala-domains` crate computes **native Rust** evaluator and package
   digests from its source tree; it deliberately does not reproduce the
   historical Python source-file digests. Consequently these differ from
   Python: `DomainPackageView.digest`, the package digest in the evaluation
   report and spec attributes, the report artifact digest/name, `spec_digest`,
   the revision `state_digest` and `action_digest`, the commit `request_id`,
   the `DomainPackageManifest` digest and manifest `schema_digests`, and every
   journal event digest whose payload contains one of those. Parameter-schema
   digests and `parameters_digest` are byte-identical and are asserted exactly.
   Stored Python revisions remain readable and immutable; a Python-prepared
   commit is not silently re-approved under native identity (its re-derivation
   changes identity), matching the domains crate's published provenance
   boundary.
2. **Constructor error typing.** Operation-level errors keep their Python
   class/code/message. Constructor failures from `Database`/journal/artifact
   layers are returned as `ApplicationError` (code `application_error`) rather
   than the concrete `JournalIntegrityError`/SQLite exception; the message is
   preserved and the process still fails closed.
3. **Lone surrogates.** Rust `&str` inputs cannot express a lone surrogate, so
   a caller cannot pass one where Python could. A foreign stored string with a
   lone surrogate is rendered with U+FFFD in diagnostics; the root's own writes
   are always valid UTF-8.
4. **Injected journal.** Python accepts an `EventJournal` object; the native
   constructor accepts the journal's database (`Components::journal_database`).
   The default binds the journal to the application database, as Python does.
5. **Custom domain packages.** Python accepts arbitrary `DomainBinding`
   objects; the native binding names one of the three accepted packages
   (`building`, `mechanical`, `wing`) and may override its reference
   parameters, step order and subject. A fourth package would need to be
   accepted into `kala-domains` first.
6. **Non-application failures.** A budget exhaustion, planner failure or tool
   error that Python would raise as its own exception class is returned as an
   `ApplicationError` with the same message, because the public native surface
   is one error type. No such failure occurs on the reference path.
7. **`list_design_revisions`/`list_events` defaults.** Rust has no default
   arguments; callers pass the Python defaults explicitly (documented above).
8. **Snapshot nonce.** `uuid4().hex[:12]` is replaced by 12 hex characters from
   the OS random source (both are 48 random bits); kernel-state snapshot ids
   remain `<aggregate>-<12 hex>`.
9. **Event payload omission.** The Python `payload_omitted` branch is
   unreachable because `canonical_json_dumps` already raises above the bound;
   the native port keeps the same behaviour (`false`) and the same error
   precedence.

## 9. Remaining gaps

* No runtime cutover, packaging, Windows execution, CLI or HTTP layer is
  claimed. Those are the `api-routes` and `cli` cells; this crate provides the
  synchronous root they call.
* The Windows cross-check could not compile because the mingw C toolchain is
  absent; `libsqlite3-sys` and `ring` need it. The crate itself contains no
  target-specific code.
* The reference flow never exercises a tool call or a model completion; the
  executor/gateway are composed and exposed but not driven by the application
  script. The accepted `kala-runtime`/`kala-tools` cells own that evidence.
* `kala-api` was under concurrent editing by another cell during this run; the
  final dependency state compiles and all gates above were re-run against it.
