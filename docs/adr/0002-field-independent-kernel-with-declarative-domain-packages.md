# ADR-0002: Field-Independent Kernel with Declarative Domain Packages

| Field | Value |
|---|---|
| Status | **Proposed** — awaiting stakeholder confirmation. Implementation evidence does not convert a Proposed decision into accepted policy. |
| Date | 2026-09-25 |
| Last reviewed | 2026-09-26 |
| Scope | Package boundaries, extensibility model |
| Requirements addressed | FR-02 field-independent kernel, FR-03 mechanical/wing/building scope, FR-04 simple mechanical component as first reference workflow, FR-12 shared kernel with declarative domain packages |
| Related ADRs | ADR-0001, ADR-0003, ADR-0004, ADR-0005, ADR-0006 |
| Implementation status | **Partially implemented, with a material deviation.** The field-independent kernel is real and mechanically enforced: `tests/unit/test_kernel.py:268` AST-walks every `kernel/*.py` and rejects any import of `domains`, `runtime`, or `tools`. All three packages exist — `mechanical.bracket`, `wing.parametric-tapered`, `building.storey-envelope` — with 10 catalog steps, and `tests/unit/test_domains.py:729` runs all three through one `DomainEvaluator` interface. The mechanical package is the end-to-end reference and its loop runs to an approved, committed revision. **Deviation (D2) — unchanged by this review:** packages are not data-only. Their *declarations* are data (typed `ParameterSpec` tables, `CatalogStep` descriptors, `WorkflowDefinition`, schema digests, a content digest), but their *evaluators* are trusted first-party Python classes imported by the composition root and run in the kernel process. There is no non-Turing-complete expression language, no sandboxed expression evaluator, and no data-only loader; a third-party package cannot be loaded at all. **What was added since the last review closes the substitution gap, not the D2 gap.** Every evaluator bound to a catalog step is now identified by `evaluator_implementation` and given an `EvaluatorImplementation` digest committing to its qualified name, the SHA-256 of its defining module's **source file on disk**, its step id, and its declared capabilities. That digest is folded into `DomainPackage.digest()` and published through `manifest_schema_digests`, so it is pinned on every committed revision; substituting an evaluator changes the digest **even when the substitute computes exactly the same numbers**, and an evaluator with no identifiable source implementation is **refused** rather than digested blind. A catalog step and its evaluator must declare the same capability set, and a run's request must match the declared set **exactly**, checked **before any evaluation executes**. **A digest identifies code; it does not sandbox it.** D2 stands. |
| Verification | 1,227 tests pass on CPython 3.11.16 and CPython 3.14.7; branch coverage 94.76% / 94.77% against a configured `fail_under = 90`; the mechanical reference loop is exercised end to end through both adapters in `tests/integration/test_mechanical_loop.py` (12 tests); `tests/security/test_domain_integrity.py` (7 tests) covers evaluator identity and capability enforcement. See `docs/verification-report.md` §2. |

## Context

The product spans three engineering domains that share almost nothing technically:

- **Mechanical components** — dimensions, tolerances, material properties, fits, fastener
  patterns, static loads.
- **Parametric wings** — spanwise stations, airfoil sections, sweep and taper, incidence, lift and
  moment coefficients, volume and balance integration.
- **Buildings** — envelopes, grids, storey heights, areas and volumes, load paths, code-driven
  constraints.

Yet they need exactly the same engine around them: parameterised design state, immutable revisions,
a deterministic workflow, artifact storage, policy checkpoints, tool isolation, memory, and
provenance. The obvious alternative to writing that engine three times is to build it once and
parameterise the differences.

The risk of a "generic engineering kernel" is that "generic" quietly becomes "the union of
everything", and the kernel accumulates knowledge of airfoils and fastener torques alike. The result
is a system where no domain can be removed, every change risks all three, and the field-independent
claim is false. Conversely, three separate applications would triple the work on the hard,
domain-independent parts — which is exactly where correctness risk lives (journal integrity, tool
isolation, capability enforcement, rollback).

There is also a sequencing constraint. The first end-to-end reference workflow is a *simple
mechanical component*. If the kernel's shape is only validated against mechanical work, we will
discover architectural problems when the wing and building packages arrive. If the kernel is
designed for hypothetical generality up front, we will over-engineer before any user need is
validated. The decision must therefore produce a kernel that is general by *constraint* and
populated by *evidence*.

Finally, engineering knowledge changes slowly and is best expressed as data, while engine
behaviour changes quickly and is best expressed as code. Conflating the two produces the usual
outcome: domain logic hard-coded in the kernel, or behaviour smeared across YAML.

## Decision

**D1 — One kernel, three field-independent responsibilities.** The kernel provides exactly the
capabilities that are domain-neutral: unit and quantity arithmetic, the typed design graph,
immutable revisions, the event journal, the deterministic workflow engine, policy checkpoints,
memory, the capability broker, budgets, and provenance. The kernel contains no domain nouns, no
domain vocabularies, and no domain-specific defaults. This is enforced mechanically, not by
convention (D5).

**D2 — Domains as declarative packages, not plugins with code.** A domain package is a versioned,
inspected, content-addressed bundle of *data*: JSON Schema definitions for its entity types, unit
declarations, parameter templates, a catalog of named workflow steps with typed inputs and outputs,
derived-quantity rules expressed as a restricted, non-Turing-complete expression language, example
cases, tool bindings by capability, and a policy overlay. A domain package contains no executable
code in the MVP. Domain-specific computation that is genuinely needed is added to the kernel only
as a *general* operation, or is delegated to a tool.

**D3 — Bounded expression language for derived quantities.** Derived quantities are declarative
rules over typed inputs, evaluated by a restricted evaluator in `kernel` with a fixed operator set,
no user-defined functions, no I/O, and a step limit. This keeps derived values deterministic and
inspectable while allowing genuine domain content to live in data. A domain package that needs
open-ended computation must route it through a tool, where isolation and capability checks apply.

**D4 — Shared engine, per-domain catalogs.** The workflow engine, revision model, and checkpoint
protocol are identical across domains. What differs is the step catalog, the entity schema, the
parameter templates, and the policy overlay. Domain authors get workflow steps, lint rules, and
UI-free views for free by writing only declarations.

**D5 — Boundary enforced by import rules and a domain-vocabulary audit.** Three mechanisms:
1. `kernel`, `contracts`, and `persistence` may not import `domains`, `tools`, `runtime`, `api`, or
   `cli`. `domains` may import `contracts` and the documented public kernel API, and nothing else.
2. A test scans `kernel`/`contracts`/`persistence` source for a maintained deny-list of domain
   terms — currently `airfoil`, `sweep`, `taper`, `incidence`, `fastener`, `torque`, `tolerance`,
   `storey`, `envelope`, `bearing`, `weld`, `chord`, `span` — and fails on any match outside comments
   and docstrings. The deny-list is part of the test, so extending the vocabulary is a deliberate,
   reviewed act.
3. `contracts` imports nothing from the rest of the system. It is the shared vocabulary, so it must
   be a leaf.

**D6 — Validate generality with a second and third domain before the kernel is declared stable.**
The kernel is provisionally complete when a second domain package (`wing`) and a third
(`building`) load and run a workflow on the *unmodified* kernel. Any kernel change required
specifically for the wing or building package is a design defect to be resolved by generalising,
by pushing the specialisation into the package, or by amending this ADR. Phase 5 in the delivery
plan is blocked on this evidence, not on a date.

**D7 — Domain packages are versioned, and revisions bind to package versions.** Every revision
records the exact digest and version of each domain package that contributed to it. A workflow
replay is only valid against the package versions recorded in the original run, and a package
upgrade is an explicit, checkpointed operation that produces a new revision rather than mutating
history.

**D8 — The mechanical reference workflow is the acceptance vehicle, not the template.** The simple
mechanical component is the first end-to-end exercise of the kernel because it is the smallest
case that still exercises parameters, constraints, derived quantities, a tool call, an artifact, a
checkpoint, and a revision. Domain content derived from it is not generalised to other domains;
the kernel guarantees are, and are checked by the second and third packages.

## Consequences

### Positive

- The hard, high-risk parts of the system are written once and tested once: journal integrity,
  tool isolation, capability enforcement, rollback, and checkpointing.
- Adding a fourth domain is a data-authoring task, not a software project, which makes the
  architecture genuinely extensible rather than nominally so.
- The field-independence claim is mechanically checkable, so it cannot silently erode.
- Declarative packages are inspectable, diffable, and reviewable as data — a material advantage for
  engineering content that domain experts must sign off on.
- The bounded expression language keeps derived-quantity evaluation deterministic and free of the
  arbitrary-code risk that a full expression language would introduce into package data.
- Version binding makes old revisions reproducible even after the domain package evolves, which is
  the property engineering users need for long-lived projects.
- Domain packages can be authored and reviewed without reading kernel code.

### Negative

- The restricted expression language will not cover every real domain rule. Complex domains will
  push work into tools, which means more tool contracts to write and more indirection to debug.
- Data-only packages cannot encapsulate behaviour, so some legitimate domain logic is either
  awkward as data or must be reimplemented in a tool per domain.
- The domain-vocabulary deny-list is an ongoing maintenance cost and will need extending as
  genuinely generic engineering concepts (tolerance, units, safety factors) become kernel
  concepts. Each addition is a small, reviewable decision — which is the intent.
- Deferring kernel stabilisation to the third domain lengthens the period during which the kernel's
  shape may still change, which is uncomfortable for early adopters.
- A data-driven model limits how far a domain package can customise workflow structure; genuinely
  novel topologies must be expressed in the shared engine.
- Two enforcement mechanisms (import rules plus vocabulary audit) must both be maintained in CI.

### Neutral

- A domain package is a distinct artifact with its own digest, version, and trust record, so
  packaging, signing, and upgrade policy apply to domains as well as to tools.
- Domain packages being data means they can be authored in any editor; the format is unopinionated
  but the schema is strict.
- Version pinning means users will occasionally need a package upgrade workflow, which is extra
  surface but not extra risk.
- The kernel's public API is a stability commitment; changing it requires checking all three
  packages.

## Alternatives Considered

**Three independent applications, one per domain.**
- Rejected: triplicates the engine work, which is where the correctness risk is concentrated, and
  produces three incompatible revision and provenance stories for users who work across domains.
  Genuinely appealing for maximal domain freedom; rejected because the shared engine is the
  product's main asset.

**A single monolithic application with domain branches inside the kernel.**
- Rejected: the field-independence requirement is explicit, and this is the specific failure mode
  the requirement exists to prevent. It also makes the wing and building work depend on mechanical
  changes and vice versa.

**Plugin architecture where domain packages contain arbitrary Python code.**
- Rejected for the MVP: it makes every domain package a full-trust code dependency, defeating
  package trust and sandboxing (ADR-0006) and giving a data-only review story nowhere to stand. If
  a domain needs code, that code should be a tool, which is already capability-checked and isolated.
  Reconsider only if declarative packages prove insufficient, and then as an explicitly
  code-signed, sandbox-executed extension type rather than an in-process import.

**A general-purpose entity/attribute modelling system driven entirely by user-authored JSON Schema,
with no domain catalog concept at all.**
- Rejected: it satisfies field-independence trivially but provides no workflow step vocabulary, so
  every domain would re-implement the workflow logic in configuration. Retained as a subset —
  schemas are how domain packages declare entities — while the step catalog is the part that makes
  packages useful.

**Rule-engine or expert-system framing (e.g. a production-rule engine).**
- Rejected: attractive for buildings, where code compliance is naturally rule-shaped, but rule
  engines carry their own runtime semantics, weak typing, and poor provenance. The bounded
  expression language plus typed entity schemas covers the same need with better typing and
  deterministic evaluation.

**A general-purpose CAD/CAE-neutral "engineering knowledge graph" as the core abstraction.**
- Rejected: a knowledge-graph core would either absorb domain semantics into the kernel (violating
  D1) or require every domain to be modelled as a graph, adding a translation layer for
  parametric data that is naturally hierarchical and typed.

**Compile domain packages to code at install time.**
- Rejected: reintroduces arbitrary code from data, defeats review, and makes reproducibility depend
  on a generator toolchain.

## Trade-offs

- We accept reduced expressive power per domain in exchange for real field independence, a
  reviewable data artifact, and one engine to trust.
- We accept more indirection for complex rules — rule in data, or tool call — in exchange for
  determinism and isolation on every path.
- We accept a longer kernel-stabilisation horizon, gated on three domains rather than a date, in
  exchange for not freezing the architecture against a single domain's evidence.
- We accept the cost of maintaining an explicit vocabulary deny-list in exchange for a
  field-independence guarantee that cannot rot unnoticed.
- We accept a less pleasant authoring experience for domain experts than a code SDK would give, in
  exchange for a package format that can be signed, diffed, and reviewed without running it.

## Revisit Triggers

- A domain requirement cannot be expressed within the declarative model and the bounded expression
  language after a genuine attempt at generalisation — this is the primary trigger for reconsidering
  code-bearing packages.
- A fourth domain arrives with a workflow topology the shared engine cannot express.
- A requirement for real, kernel-backed geometry (ADR-0006 trust model applies to native libraries)
  turns out to need domain-specific geometry operators.

## Verification

Status as of 2026-09-26:

1. Import-rule test fails the build on any forbidden dependency edge, including transitive ones.
   **Partially met.** Two hand-written AST tests replace an import-linter contract:
   `tests/unit/test_kernel.py:268` rejects `domains`, `runtime`, and `tools` imports from every
   `kernel/*.py`; `tests/unit/test_domains.py:975` rejects anything outside
   `kala_agent.contracts` and `kala_agent.domains` from every `domains/**/*.py`, and asserts the
   module count so a new file cannot be added unnoticed. Neither walks the **transitive** closure, and
   no `import-linter` contract is configured.
2. The domain-vocabulary deny-list test passes. **Not met.** No deny-list test exists. Field
   independence is enforced structurally, by rules 1 and 3 plus review, rather than by a vocabulary
   scan. The test named in this ADR is still worth writing.
3. `wing` and `building` packages load and complete a workflow each on the unmodified kernel.
   **Met for the declarative surface.** All three packages run through the same `DomainEvaluator`
   interface (`tests/unit/test_domains.py:729`) and all three evaluate through
   `kala-agent evaluate --domain <name>` and `POST /api/v1/domains/{domain}/evaluations`. Only the
   mechanical package has a runnable committed-revision workflow; wing and building reach an
   evaluation and its report, not a `PRE_COMMIT` commit.
4. A domain package can be added by writing only data files, with no kernel or tool source change, and
   the fixture workflow passes. **Not met, and not currently possible.** There is no package loader,
   no fixture repository, and no data-only format. Adding a package today means adding Python
   modules. This is the concrete consequence of the D2 deviation, and it is unchanged.
5. A test asserts that swapping an evaluator behind an unchanged catalog changes the pinned package
   digest, even when the substitute computes the same numbers. **Met.**
   `evaluator_implementation` digests the evaluator's qualified name, the SHA-256 of its module's
   source file, its step id, and its declared capabilities, under
   `IMPLEMENTATION_DIGEST_VERSION = "evaluator-implementation-digest-v1"`. The digest is part of
   `package_payload`, so it is part of `DomainPackage.digest()`, and it is published in the manifest
   through `manifest_schema_digests` and therefore pinned on every committed revision.
   (`tests/security/test_domain_integrity.py:261`, `tests/unit/test_domains.py:834`.)
6. A test asserts that a digest is stable within a process and across processes, and that rewriting
   an evaluator module on disk changes it. **Met.** `module_source_digest` digests the bytes that
   *would be imported* rather than the loaded module object, and caches per file identity
   `(path, st_dev, st_ino, st_size, st_mtime_ns)`. A rewritten file therefore yields a new digest
   while an unchanged one stays stable. (`tests/security/test_domain_integrity.py:320,333`.)
7. A test asserts that an evaluator with no identifiable implementation is refused rather than
   digested blind. **Met.** A module that is not imported, has no `__file__`, or whose file cannot be
   read raises `DomainPackageError`. The honest failure mode: a digest computed from nothing reports
   a number that means nothing. (`tests/security/test_domain_integrity.py:366`.)
8. A test asserts that an evaluator cannot carry a capability its catalog step did not declare, and
   that a run's declared capabilities must match the package's **exactly**, with the check running
   before any evaluation. **Met.** `_identify_evaluators` requires step and evaluator capability
   equality, and `enforce_capability_request` refuses a request that is empty, that omits a declared
   capability, or that carries an undeclared one.
   (`tests/security/test_domain_integrity.py:302,407`, `tests/unit/test_domains.py:872`.)
9. A test asserts that a step declaring a checkpoint kind its workflow transition does not carry
   rejects the package. **Met.** `_validate_checkpoints` requires the declared checkpoint to be
   carried by the transition that runs the step, with matching kinds.
   (`tests/security/test_domain_integrity.py:475`.)

**What items 5 through 9 do not establish.** They make substitution and capability drift
*detectable*. They do not make an evaluator *inert*: the evaluator is still a Python class running in
the kernel process, and a first-party package is trusted to compute only arithmetic. Items 5–9 are a
real improvement on the previous state and a partial substitute for D4, not a fulfilment of it. See
`docs/architecture-report.md` §3.1 and residual risk **RR-16** in `docs/threat-model.md`, which exists
specifically to stop these controls being read as closing RR-5.

## Implementation notes (2026-09-26)

| Decision | Implementation | Reference |
|---|---|---|
| D1 field-independent kernel | `kernel/budget.py`, `kernel/policy.py`, `kernel/planning.py`, `kernel/workflow.py`, `kernel/memory.py`; import rule enforced by AST test | `tests/unit/test_kernel.py:268` |
| D2 declarative packages | Declarations are data: `ParameterSpec`, `CatalogStep`, `WorkflowDefinition`, `parameter_schema_digest`. **Evaluators are trusted first-party Python**, not a restricted expression language. | `domains/__init__.py`, `domains/*/evaluation.py` |
| D3 one shared engine | `WorkflowEngine` and `run_reference_workflow`; the three packages declare the same shapes | `kernel/workflow.py`, `domains/__init__.py:461` |
| D4 three packages | `mechanical.bracket` (4 steps, one declaring `PRE_COMMIT`), `wing.parametric-tapered` (3), `building.storey-envelope` (3) | `domains/*/workflow.py` |
| D5 mechanical enforcement | Two AST import tests; no vocabulary deny-list | as above |
| D6 mechanical first | The mechanical loop is the only one that reaches a committed revision | `tests/integration/test_mechanical_loop.py` |
| D7 package identity | `DomainPackage.digest()` is the SHA-256 of the canonical package payload; `manifest()` produces the contract `DomainPackageManifest` that each revision pins | `domains/__init__.py:390,394` |
| D8 non-authoritative output | Every evaluation carries `NON_AUTHORITATIVE_FINDING`, and each package adds a stricter disclaimer. The building package marks its code version `non-authoritative-placeholder`; mechanical material constants are labelled `illustrative-constant-not-certified-material-data`. | `domains/__init__.py:45` |

The deviation on D2 is stated plainly because it changes what "declarative" means in this codebase.
`DomainPackage.__post_init__` still enforces the consistency this ADR wanted — unique catalog step ids
matching `WorkflowDefinition.available_step_ids` exactly, one evaluator per step, a validating
workflow, published schema digests, and a required description and disclaimer. What it does not
enforce is inertness: a package evaluator is ordinary Python running in the kernel process. The
import boundary limits which subsystems it can reach; it does not limit what its code may do.

## References

- `docs/architecture-report.md` — as-built architecture, package boundaries and their enforcement
  (§8), the domain packages (§13), and the deviations in §3.1 and §3.3.
- `docs/threat-model.md` — TB-4, the package boundary: T4.1–T4.5 and residual risk RR-5.
- `docs/verification-report.md` — measurements, the import-boundary test references, and residual
  risks.
- ADR-0003 — Deterministic Workflow Engine with Bounded LLM Planning and Policy Checkpoints.
- ADR-0004 — Single-Writer Immutable Revisions over an Event Journal and Content-Addressed Artifact Store.
- ADR-0005 — Capability-Based Typed Tool Contracts Executed in Isolated Processes.
- ADR-0006 — Package Trust, Sandboxing, Provenance, Rollback, and Agent Budgets.
