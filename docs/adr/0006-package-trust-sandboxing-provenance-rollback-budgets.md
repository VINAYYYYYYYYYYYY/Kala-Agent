# ADR-0006: Package Trust, Sandboxing, Provenance, Rollback, and Agent Budgets

| Field | Value |
|---|---|
| Status | **Proposed** — awaiting stakeholder confirmation. Implementation evidence does not convert a Proposed decision into accepted policy. |
| Date | 2026-09-25 |
| Last reviewed | 2026-09-26 |
| Scope | Extension trust, execution security, lineage, recovery, resource governance |
| Requirements addressed | FR-12 declarative domain packages, FR-13 fail-safe memory, FR-14 package trust, sandboxing, provenance, rollback, and agent budgets |
| Related ADRs | ADR-0001, ADR-0002, ADR-0003, ADR-0004, ADR-0005 |
| Implementation status | **Partially implemented, and the least complete of the six ADRs.** Implemented: package content identity and manifest pinning (`DomainPackage.digest()`, `DomainPackageManifest`, every revision pins its package digests); budget accounting (`kernel/budget.py`, 7 dimensions, reservation/commit/release, versioned state); fail-safe memory rules (`kernel/memory.py`); layered sandboxing with a probed strict Linux profile and fail-closed behavior; recovery of policy and budget state at startup with refusal to reset unverifiable state; secret-safe audit records. **Not implemented: D2 trust tiers beyond `user-local`, D3 staged installation (there is no installer, no quarantine store, and no signature verifier — `DomainPackageManifest` validates that `signer_id` and `signature_digest` are supplied together, but nothing checks a signature), D4 data-only packages (evaluators are trusted first-party Python, see ADR-0002), D8 design rollback, and D10 hierarchical budget scope.** Packages are first-party and reviewed; "trusted" here means reviewed, not verified. **What was added since the last review:** every domain evaluator is now identified by a digest of its own source file and its declared capabilities, published in the manifest and pinned on every committed revision, and a run's declared capabilities must match the package's **exactly** before any evaluation runs. That closes the *substitution* half of D1's identity requirement and makes capability drift detectable. **It does not close D4**: a digest identifies code, it does not sandbox it, and the evaluator is still a Python class in the kernel process. See `docs/architecture-report.md` §3.1 and residual risk **RR-16**, which exists to stop these controls being read as closing D4. |
| Verification | **1,227 tests pass on CPython 3.11.16 and CPython 3.14.7**; branch coverage 94.76% / 94.77% against a configured `fail_under = 90`; `kernel/budget.py` 86.00%, `kernel/memory.py` 98.68%, `tools/sandbox.py` 99.13%. Evaluator identity and capability enforcement are verified by `tests/security/test_domain_integrity.py` (7 tests). Provenance recording is verified end to end in `tests/integration/test_mechanical_loop.py`; rollback is not implemented and therefore not verified. See `docs/verification-report.md` §2. |

## Context

A field-independent kernel is useful only if domain content and tools can be added without granting every
extension the same authority as the user’s design data. A package can be syntactically valid and still be
wrong: it may request broad filesystem access, depend on an unverified native solver, contain a workflow
that silently exports data, or produce an artifact that cannot be reproduced. The system therefore needs
a trust model that distinguishes data, policy, and executable authority rather than treating “installed”
as “trusted.”

The same concerns apply to the agent itself. A model can be helpful while proposing too many calls,
requesting expensive resources, or repeatedly retrying a failing solver. A local process can also be
interrupted by a crash, a full disk, a missing package, or a changed dependency. A safe MVP must stop
before these conditions create an unreviewed design revision or an unbounded bill.

Engineering users need a way to answer four questions after the fact:

1. Which package and tool versions contributed to this revision?
2. Which inputs, policies, model transcript, and transformations produced each artifact?
3. Which user decisions authorized consequential actions?
4. If a package or run is defective, can the user return to a known-good state without destroying
   the evidence that explains the failure?

These questions require an explicit provenance graph, immutable history (ADR-0004), package-version
pinning, and a rollback operation. They also require clear budget accounting so that a run fails
safe at a known boundary rather than consuming arbitrary resources.

The MVP is single-user and local, but “single-user” does not mean “trusted by default.” A package may
come from a local directory, a removable medium, or a future registry; its bytes may be untrusted even
when the download channel is convenient. Trust must therefore be based on content, declared
capabilities, user approval, and execution isolation, not on filename or origin alone.

## Decision

We adopt the following decisions.

**D1 — Packages are versioned content, and revisions pin their manifests.** A domain package is a
content-addressed bundle containing a manifest, JSON Schema definitions, unit declarations, parameter
templates, workflow definitions, step declarations, policy overlay, examples, and capability
requirements. In the MVP, domain packages contain data only. Tool adapters and native executables are
separate executable packages. Every revision and workflow run records the exact package ID, version,
manifest digest, schema digests, and tool descriptors used. A package upgrade is a new reviewed
selection and never mutates an existing revision.

**D2 — The trust model has explicit tiers and is deny-by-default.** The initial categories are:

- **Core:** code and schemas shipped with the kernel, covered by the application’s own source and test
  review.
- **User-local:** a package explicitly selected by the local user, with a recorded digest and an
  explicit install decision.
- **Reviewed-signed:** a package whose manifest and content digest validate against a configured local
  trust root and whose requested capabilities pass review.
- **Quarantined/untrusted:** a package that fails digest, signature, schema, compatibility, capability,
  or policy checks, or that has not received the required user approval.

A tier changes the installation and execution workflow, not the truth of a package’s claims. Missing
signature infrastructure does not make a package trusted; it makes it eligible only under an explicit
user-local policy. Signature fields in the current `DomainPackageManifest` are declarative contract
fields, not evidence that verification has been implemented.

**D3 — Installation is a staged, inspectable operation.** Installation first reads a package from a
local source into a quarantine area, canonicalizes and hashes its files, validates the manifest and
schemas, checks compatibility and required capabilities, and presents a human-readable diff and
capability summary. It then requires the applicable approval checkpoint. Only after approval is the
package atomically registered by digest. Installation performs no arbitrary package code, no implicit
network fetch, and no modification of existing revisions. A failed validation leaves the quarantine
record and error journaled, not an active package.

**D4 — Data-only domain packages have no import authority.** Domain packages contribute validated
data and catalog references only. The kernel does not import a package as Python, evaluate arbitrary
expressions, or allow a package to register hidden steps. Derived quantities use the bounded expression
language in ADR-0002. If a domain needs executable behavior, it must use a separately versioned tool
adapter with a typed contract and a sandbox profile; the domain package cannot smuggle code through
schema, policy, or expression fields.

**D5 — Executable tools are isolated and least-privileged.** A tool package declares its executable
identity, version, input/output schemas, side-effect class, required capabilities, resource envelope,
dependencies, and sandbox profile. The tool supervisor (ADR-0005) verifies the package and executable
digest, starts a separate process with a minimal environment and scoped filesystem, denies network by
default, enforces time and size limits, and records the result. A tool cannot access the event journal,
credentials, or arbitrary host paths merely because it was installed.

**D6 — Package provenance is content-addressed and transitive.** The manifest and every referenced
schema, workflow, expression, policy overlay, tool descriptor, and executable receive a digest. A
provenance record links subjects (requirements, specifications, revisions, or workflows) to source
artifacts, input states, tool calls, model transcript digests, package manifests, transformations,
seed, policy decisions, and output artifacts. Provenance is append-only and queryable from the journal;
a missing link is an integrity finding, not an invitation to infer a value.

**D7 — Every consequential decision is attributable.** The journal records the local actor, package
and tool identities, action digest, state digest, selected option, typed modification digest, and
timestamp for checkpoints and exports. A package cannot impersonate the user or mark its own action as
human-approved. If a future multi-user system adds stronger identities, this record becomes the
migration point rather than a reason to rewrite history.

**D8 — Rollback is additive and scoped.** A user may roll back a design to a prior immutable revision,
a package selection to a prior digest, or a tool configuration to a prior reviewed descriptor. Each
rollback first verifies that the target and all referenced content remain available and compatible,
then presents a checkpoint and writes a new rollback event and, when design state changes, a new
revision. It never deletes the failed revision, artifact, transcript, or package. The current head,
previous head, target, and reason are all recorded.

**D9 — Budgets are first-class, pre-authorized limits.** A run budget includes maximum model calls,
tool calls, total tokens, artifact bytes, duration, estimated cost, and replanning attempts. The
`RunBudget` contract provides these fields and `kernel/budget.py` implements the accounting service:
before an operation the ledger reserves the maximum charge it can reasonably bound, and after
completion it commits actual usage. A call that cannot fit within the remaining budget is denied
before dispatch. Budget exhaustion is a safe terminal or checkpoint state, never an invitation to
bypass the limit.

**D10 — Budget scope is hierarchical and visible.** A user/session budget is divided among workflows,
and a workflow may allocate a step budget. Package requirements can lower, but never raise, an
authorized ceiling. Actual usage, reservations, remaining budget, and the reason for a denial are
journaled. A workflow that needs a larger budget must create a new explicit policy decision, not
silently mutate the current budget.

**D11 — Memory is fail-safe and non-authoritative by default.** Durable events and revisions are the
system of record. Working memory is bounded, typed, provenance-labelled, and disposable; summaries and
model-generated facts are labelled derived and cannot authorize a tool, satisfy a checkpoint, or replace
a design parameter without a typed transition. Contradictory, stale, oversized, or untrusted memory is
quarantined and surfaced. The memory subsystem must fail by returning less context, not by inventing
context or escalating capability.

**D12 — Security controls are layered and conservative.** The minimum control set is strict schema and
canonical JSON validation, digest and size checks, least-privilege capability grants, process isolation,
no implicit egress, append-only journaling, atomic artifact writes, package quarantine, human
checkpoints, and resource budgets. Where the operating system offers namespace, seccomp, or equivalent
controls, they are enabled as a stronger profile. If a control cannot be established, the operation
fails closed. The design does not claim protection from an attacker who already controls the local
user account or can rewrite all local files and backups.

**D13 — Recovery is a first-class workflow state.** On startup, the system verifies the journal
sequence, active package manifests, pending checkpoints, unfinished tool calls, and artifact digests.
It can resume only from a state whose required inputs are present and whose policy decisions are still
valid. Otherwise it presents a recovery checkpoint or marks the run failed. Recovery never repeats a
non-idempotent tool call automatically.

**D14 — Security decisions are auditable without leaking secrets.** The journal records capability
names, package digests, policy outcomes, hashes, sizes, destinations when egress is approved, and
error codes. It does not record API keys, credentials, unrestricted environment dumps, or raw secret
material. A user can export an audit manifest locally after reviewing its contents.

## Consequences

### Positive

- Package installation and execution have a visible trust boundary rather than relying on filenames,
  directories, or the user’s ability to inspect arbitrary code before it runs.
- Data-only domain packages are reviewable and cannot directly execute code in the kernel.
- Transitive provenance makes package versions, inputs, tools, model transcripts, and outputs
  reconstructable from the journal.
- Rollback preserves the failed history, which is essential for diagnosing a bad package or tool rather
  than merely hiding the symptom.
- Budgets make runaway model/tool behavior a controlled failure and support predictable local resource
  use.
- Fail-safe memory prevents a plausible but unsupported remembered fact from becoming an engineering
  input or authorization.
- Layered controls provide useful protection on a normal workstation while leaving stronger OS
  sandboxing as a selectable profile.

### Negative

- Package installation and execution require more user decisions and more diagnostic information than
  an unrestricted script.
- Signature and trust-root management adds operational work; unsigned local packages remain useful but
  receive a lower trust posture.
- Data-only packages cannot express every domain behavior, so complex requirements need tool adapters
  and more contract design.
- Strong resource limits and isolation may require adapting native solvers or third-party tools.
- Additive rollback means the current state can move backward without deleting history, which requires
  careful user-interface language.
- Full provenance and transcript retention increases storage and privacy review obligations.

### Neutral

- A package can be perfectly non-malicious and still be rejected because its declared capabilities are
  broader than the current policy; this is a governance result, not a bug verdict.
- Budgets use estimated cost for hosted providers and zero or hardware-cost accounting for local
  models, but the same counters are retained for uniform audit.
- Trust tiers are policy metadata; they do not make a package’s engineering results correct.
- Sandboxing and provenance are independent controls: a sandbox limits effects, while provenance
  explains them.
- A local single-user deployment has no automatic organization-wide identity or key-management system;
  the user remains the final approval authority.

## Alternatives Considered

**Trust any package that can be read from a local directory.**

- Rejected: local origin is not a content or capability check, and it makes a copied or tampered package
  indistinguishable from a reviewed one.

**Require cryptographic signatures for every package before installation.**

- Deferred as the only path: it is a strong option for distribution, but it creates a key-management
  and trust-root problem for a local-first MVP and can prevent a user from using a private package.
  User-local digest pinning is supported, with signatures becoming a stronger policy option.

**Allow domain packages to contain arbitrary Python plugins.**

- Rejected: importing them into the kernel defeats data-only review, capability enforcement, and
  field-independence. Code-bearing behavior belongs in a separately trusted and sandboxed tool adapter.

**Run every tool as the logged-in user with no additional isolation.**

- Rejected: a faulty or malicious tool could read all user files, inherit secrets, access the network,
  or damage the journal. Process isolation and fail-closed capability checks are the minimum.

**Use containers or virtual machines as the only sandbox.**

- Deferred for universal MVP use because of startup, packaging, hardware, and cross-platform costs.
  The contract can support a container profile when a real CAD/CAE integration needs it.

**Make provenance a sidecar log or generated report rather than an event-journal concern.**

- Rejected: sidecars can be lost, reordered, or edited independently of the action they describe.
  Provenance facts must be committed with the relevant durable events.

**Delete the bad revision and restore the previous file.**

- Rejected: destructive undo destroys evidence and can break references from later work. Additive
  rollback is slower but preserves the complete causal chain.

**Use a single total token or wall-clock budget.**

- Rejected: a single limit does not identify which resource was exhausted or protect local storage and
  tool concurrency. Separate counters make denial actionable and auditable.

**Allow a workflow to request a budget increase automatically.**

- Rejected: it defeats the purpose of a budget and makes a model/provider failure potentially
  unbounded. An increase is a new explicit user/policy decision.

**Treat model-generated memory as authoritative design state.**

- Rejected: generated summaries can be stale, wrong, or prompt-injected. They are derived context only;
  typed design state and journaled decisions remain authoritative.

## Trade-offs

- We accept more installation friction and lower convenience in exchange for explicit trust, capability
  review, and a smaller execution blast radius.
- We accept a data-only domain model in exchange for field independence, inspectability, and a clear
  path to sandboxed code when it is genuinely needed.
- We accept storage and query work for full provenance in exchange for defensible replay, review, and
  rollback.
- We accept budget-related interruptions in exchange for bounded local resource consumption and
  predictable provider cost.
- We accept that local integrity controls are not equivalent to protection against a compromised OS
  or user account; the MVP prioritizes accidental failure prevention and auditable intent.
- We accept conservative recovery—stop and ask rather than retry—when a target, package, artifact, or
  checkpoint cannot be verified.

## Revisit Triggers

- A real CAD/CAE integration requires native binaries, GPU access, hardware tokens, or a vendor
  license service that the portable process profile cannot support.
- The product becomes multi-user or hosted and needs organizational identity, key distribution, and
  stronger attestation.
- A package marketplace or remote registry makes signature lifecycle and revocation a first-class
  concern.
- Provenance storage or transcript retention exceeds local capacity and requires encrypted archival.
- A user needs collaborative merge semantics that cannot be represented by a single writer.

## Verification

Status as of 2026-09-26:

1. Package fixtures with tampered manifests, unknown schemas, invalid signatures, and excessive
   capabilities are rejected and remain quarantined. **Partially met, and only for first-party
   content.** A package with an inconsistent declaration is refused by `DomainPackage.__post_init__`
   — unique catalog step ids matching `WorkflowDefinition.available_step_ids` exactly, one evaluator
   per step, a workflow that satisfies the contract, published schema digests, a required description
   and disclaimer, and every declared checkpoint carried by the transition that runs its step with
   matching kinds (`tests/unit/test_domains.py:754`, `tests/security/test_domain_integrity.py:475`).
   An invalid *signature* is not detected, because no signature is ever verified. **There is no
   quarantine store**, so there is nothing for a rejected package to be quarantined into.
   **Excessive capabilities are now enforced rather than merely published:** a catalog step and its
   evaluator must declare the same capability set, and a run's request must match the package's
   declared set **exactly** — not empty, not missing a declaration, not carrying an extra — with the
   check running **before any evaluation executes** (`tests/security/test_domain_integrity.py:302,407`,
   `tests/unit/test_domains.py:872`). A substituted evaluator therefore cannot smuggle in a wider
   capability set.
2. A data-only package fixture loads without executing Python, shell commands, network access, or
   arbitrary expressions. **Not met, and not met more strongly than before.** This is the ADR-0002
   deviation: evaluators are trusted first-party Python classes imported by the composition root and
   run in the kernel process. There is no data-only format and no loader. What exists instead is an
   import boundary — `tests/unit/test_domains.py:975` proves a package can reach only
   `kala_agent.contracts` and `kala_agent.domains` — plus, new in this review, **evaluator identity**:
   every evaluator is identified by `evaluator_implementation` and given a digest committing to its
   qualified name, the SHA-256 of its defining module's source file on disk, its step id, and its
   declared capabilities, published in the manifest through `manifest_schema_digests` and therefore
   pinned on every committed revision (`tests/security/test_domain_integrity.py:261,320,333`). That
   makes substitution **detectable**: a different class, a different module, a rewritten source file,
   or a different capability set all change the digest, and the digest changes **even when the
   substitute computes exactly the same numbers**. An evaluator whose implementation cannot be
   identified from a source file is **refused** rather than digested blind
   (`test_domain_integrity.py:366`).
   **None of this makes the package inert.** Identity is not confinement: the evaluator is still
   arbitrary Python running with kernel-process authority, and a first-party package is trusted to
   compute only arithmetic. This item stays **Not met**, and `docs/threat-model.md` **RR-16** records
   the specific risk that these controls get read as closing it.
3. Sandbox tests demonstrate denial of undeclared file, network, environment-secret, and journal
   access. **Partially met.** Network and egress are denied by capability name unless the profile
   opts in. The child environment is built from a fixed key set rather than inherited, and
   executor-owned keys cannot be overridden. Workspace traversal, out-of-workspace artifacts, and
   symlinked artifacts are refused. A child holds no database handle, no event-journal path, and no
   revision-coordinator reference. **Undeclared *file* access is not denied** under the shipped
   portable profile, which confines a process and not a filesystem; the strict profile does confine
   it, is probed before use, and fails closed, but is opt-in. See ADR-0005 and residual risk RR-1.
4. Provenance queries reconstruct the package, tool, input, transformation, transcript, policy, and
   output chain for a committed revision. **Partially met.** The chain is **recorded** on every
   committed revision and read back correctly: subject and design identifiers, domain package digests,
   source and output artifact ids, tool call ids, parent provenance, transformation ids, an optional
   model transcript digest, and an optional seed. What is missing is the **query** — there is no
   provenance graph and no traversal API, so reconstruction means reading the journal rather than
   running a query. `model_transcript_digest` is `null` on the reference path because the offline
   planner never calls the gateway.
5. Rollback tests verify that a new revision/head is created and all prior evidence remains readable.
   **Not met; rollback is not implemented.** The adjacent property *is* enforced: a commit that does
   not extend the current head is refused, so a design history cannot be forked
   (`tests/security/test_revision_commit.py:268`). The only `rollback` in the codebase is the SQL
   transaction rollback in `persistence/database.py`.
6. Budget tests exhaust each counter independently and prove that the next operation is denied and
   journaled without a side effect. **Met for the ledger.** All 7 dimensions — `model_calls`,
   `tool_calls`, `total_tokens`, `artifact_bytes`, `duration_seconds`, `cost`, `replanning_attempts`
   — raise a per-dimension `BudgetExhaustedError` carrying the dimension, the request, the remaining
   amount, and the limit, and the executor reserves before dispatch and commits actual usage
   afterwards, releasing on every failure path
   (`tests/security/test_tool_executor.py:1602`, `tests/unit/test_kernel.py`). Budget state is
   recorded at the end of every operation and restored at startup, with strict import validation
   (`tests/security/test_revision_commit.py:515`). **Not implemented:** D10's hierarchical
   user/session-to-workflow division, and any journaling of a budget denial as a first-class event.
7. Recovery tests kill the process during installation, tool execution, checkpoint, and commit and
   verify that startup does not repeat a non-idempotent action. **Partially met.** A clean restart
   restores usage, grants, and approvals, a revoked capability stays revoked, and a tampered or
   non-validating kernel-state record **stops the launch** rather than resetting to defaults
   (`tests/security/test_revision_commit.py:515,563,581,608`). Restore is fail-closed in both
   directions that matter: **missing** kernel state in an already-used directory also stops the launch
   (`test_revision_commit.py:794`), while a genuinely fresh directory restores defaults with no record
   at all (`test_revision_commit.py:842`). **No `SIGKILL` fault-injection test exists**, and there is
   no installation step to interrupt.
8. A test asserts that a user-supplied identifier cannot name the reserved aggregate that holds
   durable kernel state. **Met, and this closes a real collision.** Without the namespace, a user
   could name a workflow `kernel-state:kernel-policy-state` and write into the aggregate holding the
   policy snapshot. `kernel_state_aggregate` now prefixes every stored aggregate, and
   `_is_reserved_identifier` refuses both the bare legacy names and the whole `kernel-state:` prefix
   in a user-supplied `workflow_id`. The bare names stay reserved so records written by the earlier
   version still read back. (`tests/security/test_revision_commit.py:676,709,735`,
   `tests/unit/test_api.py:780`.)
9. A test asserts that the composition root reports whether a hosted egress path is actually wired,
   rather than whether one is configured. **Met.** `egress_configured` is derived from the client the
   application actually holds, and `provider_endpoint_declared` is reported alongside it, so `/health`
   and `kala-agent init` cannot overstate what the process can reach.
   (`tests/unit/test_api.py:76,97,113,125,133,142`.)

## Implementation notes (2026-09-26)

| Decision | Implementation | Reference |
|---|---|---|
| D1 versioned content, pinned manifests | `DomainPackage.digest()` over the canonical package payload, which now includes each evaluator's `EvaluatorImplementation` record — qualified name, module source digest, step id, declared capabilities. `manifest_schema_digests` publishes those digests alongside the schema digests, and every committed revision pins the union. An evaluator with no identifiable source is **refused** | `domains/__init__.py:383,463,570,623` |
| D2 explicit deny-by-default trust tiers | **Not implemented** beyond `user-local`. `DomainPackageManifest` carries `signer_id` and `signature_digest` and validates that they are supplied together; **no verifier reads them.** | `contracts/models.py:422` |
| D3 staged, inspectable installation | **Not implemented.** No installer, no staged directory, no quarantine store. Packages are first-party and reviewed in the repository. | — |
| D4 data-only packages | **Deviation, unchanged.** Import-constrained trusted Python, not data. The new implementation digests and exact capability enforcement make substitution and capability drift *detectable*; they do not make an evaluator *inert*. | `docs/architecture-report.md` §3.1 |
| D5 isolated, least-privileged tools | Registry, executor, and the probed sandbox; see ADR-0005 | `tools/` |
| D6 content-addressed transitive provenance | `Provenance` on every revision, with package digests, artifact ids, tool call ids, parents, and transformations | `contracts/models.py:778` |
| D7 attributable decisions | Every workflow transition, checkpoint, revision phase, and domain evaluation is journalled; the reference run produced 20 events | `application.py` |
| D8 additive, scoped rollback | **Not implemented** | — |
| D9 first-class pre-authorized budgets | `BudgetLedger` with 7 dimensions, two-phase reservation, and versioned state | `kernel/budget.py:426` |
| D10 hierarchical budget scope | **Not implemented.** A single per-run ledger. | — |
| D11 fail-safe, non-authoritative memory | `MemoryManager` conflict detection, expiry, bounded working set, optional provenance-on-promotion. **Not wired into the composition root**, so no product path stores or reads a memory. | `kernel/memory.py:108` |
| D12 layered, conservative controls | Strict schemas, closed catalogs, **exact** capabilities both for tools and for domain evaluator runs, deny-by-default egress and privileged names, process isolation, bounded resources, strict regex admission, fail-closed artifact classification | across `contracts/`, `kernel/`, `tools/`, `domains/` |
| D13 recovery as a first-class state | Policy and budget state restored at startup under the reserved `kernel-state:` namespace; unverifiable state **and** missing state in an already-used directory both stop the launch, while a fresh directory restores defaults | `application.py` `_restore_kernel_state`, `application.py:671,675,678` |
| D14 auditable without leaking secrets | `redact_secrets`, bounded transcripts and redaction records, workspace-path scrubbing in diagnostics, and a CLI that reports provider and model identity but never endpoints or keys | `runtime/model_gateway.py`, `tools/protocol.py`, `cli/main.py` |

Two of these deserve a sentence each, because they are the ones most likely to be assumed working.

**Trust is reviewed, not verified.** A package digest is recorded on every revision, so a later run
can detect that the code behind a revision changed. Nothing verifies a *signature*, so a substitution
that produced a self-consistent digest would be recorded rather than refused. Making "trusted" a
verifiable property needs a trust-root format, a signer lifecycle, and a quarantine store — none of
which exists. Residual risk RR-5.

**Fail-safe memory is real and unreachable.** `MemoryManager` implements exactly the rules this ADR
asked for: a same-source contradiction raises `MemoryConflictError` rather than overwriting, expired
entries are evicted before the bound is enforced, and `require_provenance_on_promotion` can refuse a
durable write without provenance. It is exported from `kernel/__init__.py` and covered by tests, and
`KalaApplication` never constructs it. There is no embedding and no retrieval of any kind. Residual
risk RR-10.

## References

- `docs/architecture-report.md` — budgets (§11.1), policy and checkpoints (§11.2), the workflow engine
  (§11.4), memory (§11.5), and the deviations for memory, rollback, trust, and retention
  (§3.1, §3.4, §3.7, §3.9).
- `docs/verification-report.md` — per-file coverage for `kernel/budget.py`, `kernel/memory.py`, and
  `tools/sandbox.py`, the security-control tables, defects R1–R4, and residual risks RR-1, RR-5,
  RR-6, and RR-10.
- `docs/threat-model.md` — TB-4 (T4.1–T4.5) and TB-5 (T5.4, T5.8, T5.9), plus residual risks RR-5,
  RR-6, and RR-10.
- ADR-0001 — Local-First, No-Egress-By-Default Operation with a Provider-Neutral LLM Gateway.
- ADR-0002 — Field-Independent Kernel with Declarative Domain Packages.
- ADR-0003 — Deterministic Workflow Engine with Bounded LLM Planning and Policy Checkpoints.
- ADR-0004 — Single-Writer Immutable Revisions over an Event Journal and Content-Addressed Artifacts.
- ADR-0005 — Capability-Based Typed Tool Contracts Executed in Isolated Processes.
