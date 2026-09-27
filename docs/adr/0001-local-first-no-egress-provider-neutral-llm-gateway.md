# ADR-0001: Local-First, No-Egress-By-Default Operation with a Provider-Neutral LLM Gateway

| Field | Value |
|---|---|
| Status | **Proposed** — awaiting stakeholder confirmation. Implementation evidence does not convert a Proposed decision into accepted policy. |
| Date | 2026-09-25 |
| Last reviewed | 2026-09-26 |
| Scope | Kernel-wide (execution model, data residency, model integration) |
| Requirements addressed | FR-01 local-first single-user MVP, FR-05 provider-neutral LLMs, FR-07 no design binaries leave device by default |
| Related ADRs | ADR-0002, ADR-0003, ADR-0004, ADR-0005, ADR-0006 |
| Implementation status | **Substantially implemented.** `runtime/model_gateway.py` provides the `LLMClient` protocol, `ModelGateway`, the deterministic `LocalModelClient`, and the opt-in `OpenAICompatibleClient`. `application.py` builds the local client and grants no egress capability. Hosted egress is **not enabled by default** and is off in the shipped composition root. Deviations: no kernel-owned prompt-assembly layer with typed domain context blocks (D5) — artifacts are sanitized and redacted but domain packages do not contribute typed context blocks; no journal write of transcripts (D6) — transcripts and manifests are held by the gateway and passed to callbacks, and the reference path produces `model_transcript_digest = null`. **Egress hardening since the last review:** authorization is evaluated on **every** call rather than cached against a client-level grant, an `EGRESS` policy request is built fresh each time with `requested_capabilities = ("model.egress",)`, and both the `state_digest` and the `action_digest` are recomputed at call time. An `ModelEgressManifest` must be recorded **before** dispatch; a missing or failing callback raises rather than proceeding unrecorded. **An empty allow-list denies:** `allowed_endpoints` defaults to the empty tuple and the request's own normalized destination must be a member of it, so the default is a denial and `consent=True` alone is never sufficient. **Truncation is byte-safe:** text is encoded to UTF-8, sliced at the byte bound, and decoded with `errors="ignore"`, so a character straddling the cut is dropped whole. **Health reports egress truth:** `egress_configured` is derived from the client the application actually holds, never from a setting or an intent, and `provider_endpoint_declared` reports whether an endpoint was ever declared. None of this changes the two deviations above. |
| Verification | **1,227 tests pass on CPython 3.11.16 and CPython 3.14.7**; branch coverage 94.76% (3.11.16) / 94.77% (3.14.7) against a configured `fail_under = 90`; `tests/security/test_model_residency.py` (14 tests) covers every egress control. See `docs/verification-report.md` §2 for the exact commands. |

## Context

Kala Agent is a local-first, single-user agent for engineering design work across three domains
(mechanical components, parametric wings, buildings). The design data it will hold is
confidential intellectual property: geometry, dimensions, materials, load cases, and derived
engineering results.

Three forces pull against each other:

1. **Data residency.** Engineering design data has a low tolerance for leaving the device it was
   authored on. A user may have contractual or legal obligations (NDAs, export-control
   classifications, employer policy) that forbid sending a bracket, an airfoil section, or a
   building envelope to a third-party service.
2. **Model access.** Useful planning and interpretation require a capable language model. Hosted
   frontier models give the best quality but require network egress and create a vendor
   dependency. Locally hosted models preserve residency but vary widely in quality, tool-calling
   support, and operational burden.
3. **Greenfield timing.** The repository was empty when this decision was taken. Residency and
   provider coupling are both *architectural* properties: retrofitting "no implicit egress" into a
   system that already has scattered network calls, or removing a vendor SDK from the kernel's core
   loop, is far more expensive than deciding it then.

A related tension exists between *no egress at all* and *useful assistance*. A purely offline
system is trivially safe and much less capable. The design therefore needs an explicit, auditable
boundary rather than an absolute prohibition: egress is a governed capability, denied by default,
and it must be the single, narrowest possible choke point in the system.

Finally, the kernel must remain field-independent (ADR-0002). That means the kernel may not know
what a wing is, and therefore may not know what a wing "sensitive" prompt looks like. Residency
policy has to be expressed in kernel-level terms — artifact classification and capability checks —
rather than in domain-level terms.

## Decision

We adopt the following decisions.

**D1 — Local-first system of record.** The event journal and content-addressed artifact store
(ADR-0004) are the only authoritative state. Every capability of the system — including running
workflows, producing revisions, verifying provenance, and rolling back — works with the network
disconnected. Remote services are optional accelerators, never sources of truth. A workflow must be
able to reach a terminal, checkpointed state with all providers unreachable.

**D2 — Single egress choke point with deny-by-default policy.** The kernel opens no outbound
network sockets. All outbound traffic passes through one component, the `EgressGate`, which is
reachable only from the LLM gateway. Policy default is `deny`; a named allowlist of provider
endpoints must be configured by the user. There is no implicit discovery, no telemetry, no
crash-report upload, no update check, and no asset fetch.

**D3 — Artifact classification with design binaries blocked by default.** Every artifact carries a
classification computed by the kernel at write time, not chosen by the producing tool:
`design_binary` (native geometry, CAD/CAE exchange files, meshes, solver decks), `derived_data`
(computed tables, plots, reports), `text` (human-readable summaries), and `metadata` (digests,
sizes, provenance references). Artifacts classified `design_binary` cannot be attached to a model
request, exported, or referenced by a remote URL unless **all** of the following hold: (a) the user
grants a durable, named egress grant for that artifact set; (b) an `EGRESS` policy checkpoint is
approved (ADR-0003); and (c) a provenance and export manifest entry is written to the journal
(ADR-0006). The classification is derived from a kernel-side registry of file-signature rules, not
from file extensions alone, so renaming a file does not change its treatment.

**D4 — Provider-neutral LLM gateway.** The kernel depends on an internal `LLMClient` protocol only:
a request carries typed messages, a schema for the expected response, declared capabilities
(structured output, tool calling), a seed, and a budget ceiling; a response carries typed content,
usage counters, and a provider request identifier. Adapters implement this protocol for hosted
providers and for OpenAI-compatible local endpoints. No provider SDK type, error class, or
identifier appears in `kernel`, `contracts`, or any domain package. Provider selection is a
per-session policy value, not a code path.

**D5 — Kernel-owned prompt assembly.** Prompt construction is a kernel responsibility. Domain
packages contribute *typed context blocks* — each with a declared schema, a classification, and a
size bound — which the kernel renders into the provider format. A domain package cannot inject
raw prompt text, override system instructions, or suppress citations. Attachments travel by
reference: digest, classification, metadata, provenance id, and a size-bounded redacted textual
preview. Raw binary payloads are never placed in a request body.

**D6 — Verbatim, local model transcript retention.** Every model interaction is written to the
journal before its result is used: provider, model id, request digest, full request and response
bodies, token usage, cost estimate, and the digests of any referenced artifacts. Transcripts are
stored locally and are themselves classified. This makes runs replayable and auditable
(ADR-0003) and makes provider substitution detectable.

**D7 — Local models are a first-class, recommended configuration.** A local OpenAI-compatible
endpoint is the recommended default for work on confidential designs. The system must be fully
functional, and fully useful for the reference workflow, against a local endpoint alone.

**D8 — No silent provider substitution.** If a provider becomes unavailable, the workflow does not
quietly switch to a different model. It degrades to deterministic-only steps and raises a
checkpoint (`PROVIDER_DEGRADED`) for the user to choose between waiting, switching provider as an
explicit recorded action, or proceeding without model assistance.

## Consequences

### Positive

- Confidential design data has a mechanically enforced residency guarantee rather than a policy
  promise: the kernel has no network capability to leak through.
- The system is fully usable air-gapped, which also removes network access as a failure mode and
  a source of nondeterminism.
- Provider neutrality is testable: a static import check can assert that no provider SDK is
  referenced outside the adapter modules.
- Verbatim transcripts make provider-dependent behaviour auditable and replayable, and let a
  later run reproduce a decision exactly even if the model has since changed.
- Local model support is not a second-class path, so the confidential-by-default configuration is
  also the simplest and cheapest one.
- Because egress is a capability, a future multi-user or hosted Kala deployment can add egress
  without changing the kernel's core model.

### Negative

- Local models are weaker than frontier models for some planning and interpretation tasks, so the
  achievable quality ceiling on a fully offline configuration is lower.
- Recording full transcripts inflates the local database, and transcripts are the most
  storage-hungry class of record.
- The artifact-classification registry must be maintained as new formats appear; an unrecognised
  format falls back to the most restrictive classification, which is safe but can block legitimate
  work and generate support burden.
- D8 converts what users may expect to be automatic resilience into an explicit interruption.
- A single choke point is a single point of failure: an `EgressGate` defect is a residency defect.
  This is accepted and mitigated by keeping the gate tiny, dependency-free, and exhaustively
  tested.
- A strict kernel-owned prompt-assembly model constrains domain package authors, who may want
  domain-specific prompting behaviour.

### Neutral

- Model quality becomes a configuration and hardware question rather than a code question; this
  does not change any architecture component.
- Provider-specific features (vendor caching, vendor-specific tool schemas) are unavailable unless
  the internal protocol is extended; unused protocol fields must be tolerated.
- The system is deliberately harder to operate as a hosted multi-tenant service; that is a
  consequence of the single-user local-first goal, not a defect.
- Cost tracking is present even for local models, where cost is zero, because the accounting is
  uniform.

## Alternatives Considered

**Cloud-first agent with a hosted model as the default path.**
- Rejected: makes the vendor's availability, pricing, and terms load-bearing for the core product,
  and makes the confidentiality posture depend on configuration rather than construction. Also
  forecloses the air-gapped use case entirely.

**Single-vendor model SDK used directly throughout the kernel.**
- Rejected: cheapest to write, most expensive to remove. Provider identifiers, SDK exception types,
  and retry semantics leak into kernel control flow, which conflicts with the field-independent
  kernel and with determinism guarantees.

**Local models only, no hosted provider support at all.**
- Rejected: over-constrains the product. Users will want frontier quality for some tasks. The
  neutral gateway supports both at comparable cost, and the residency guarantee is provided by D2
  and D3 rather than by banning providers.

**No egress at all, enforced by an air-gap assumption.**
- Rejected: it is unenforceable once a user installs a local proxy or attaches a network interface;
  it also prevents legitimate, explicitly approved collaboration. A governed capability is both
  enforceable and useful.

**Adopt an existing agent framework (for example LangGraph, AutoGen, CrewAI, or a cloud agent
SDK) for the loop and provider integration.**
- Rejected for the MVP: these optimise for multi-tenant, network-bound, model-centric workloads and
  make the event journal, the deterministic step runner, and the capability broker their own
  abstractions rather than the foundation. Revisit only if a framework can be hosted in-process,
  keep the journal authoritative, and accept our capability contracts; the `LLMClient` boundary
  keeps that option open.

**Send design context to a hosted model but redact it (hashing, coarsening, describing without
values).**
- Rejected as the primary mechanism: redaction of engineering values is unreliable and destroys
  the utility that makes model assistance valuable. Retained only as an additional, user-invoked
  transformation at an approved checkpoint — never as a substitute for D3.

**Egress via an operating-system-level firewall or a VPN egress proxy only.**
- Rejected as the only control: correct as defence in depth, insufficient as the primary control,
  because it cannot express per-artifact, per-session intent, and cannot produce an audit record.
  Retained as a recommended hardening layer.

## Trade-offs

- We accept a lower quality ceiling on fully offline configurations in exchange for a residency
  guarantee that holds by construction. The bounded-planning design of ADR-0003 reduces the
  dependence on model quality by keeping the load-bearing logic deterministic.
- We accept larger local storage and more audit machinery in exchange for replayability and
  provider-substitution detection.
- We accept a modest reduction in domain-package flexibility (D5) in exchange for keeping prompt
  assembly auditable and the kernel free of domain concepts.
- We accept explicit user interruption on provider failure (D8) in exchange for never having
  results silently produced by a model the user did not choose.
- We accept the cost of maintaining artifact classification in exchange for the ability to state a
  precise, testable residency property.

## Revisit Triggers

- Multi-user or hosted deployment becomes a stated requirement (revisit D1 and D2 scope).
- A CAD/CAE integration requires a vendor cloud service to function; classify whether it is a
  `design_binary` egress and whether the checkpoint model can accommodate it.
- A domain package legitimately requires raw prompt templating.
- Provider response formats diverge enough that the internal protocol needs a versioned extension
  mechanism.

## Verification

How we will know this decision is working. Status as of 2026-09-26:

1. A static test asserts that no module outside the LLM adapter package imports a provider SDK or
   constructs a network client. **Partially met.** `httpx` is imported lazily inside
   `OpenAICompatibleClient.complete`, and `tests/security/test_tool_executor.py:1647` asserts that the
   `tools` package imports no network module. There is no whole-repository import-linter contract, but
   no provider SDK is a declared dependency at all: `pyproject.toml` declares only `fastapi` and
   `pydantic`.
2. A test executes the full reference workflow with the network namespace disabled and asserts
   completion with a local endpoint. **Not met as specified.** The workflow completes with no socket
   open, proven structurally rather than by a network-namespace test. See
   `docs/verification-report.md` §7, limitation 7.
3. A test asserts that a `design_binary` artifact is rejected at the `EgressGate` with no grant,
   accepted with a grant plus an approved `EGRESS` checkpoint, and that both outcomes are journaled.
   **Partially met.** Egress with no grant, with a stale approval, or with a missing manifest is denied
   and tested (`tests/security/test_model_residency.py:114,158,185,300`). Artifact payloads are
   refused outright by the hosted client. **No test exercises the accept-with-egress path**, because
   the shipped composition root cannot reach it: `model.egress` is not in the default grant set, and
   there is no real design binary to attach.
4. A test constructs the hosted client with consent but **no** allow-list entries, and asserts the
   destination is denied. **Met.** `allowed_endpoints` defaults to the empty tuple and
   `_authorize_destination` requires the request's own normalized destination to be a member, so an
   empty allow-list is a denial rather than an absence of restriction. Confirmed directly on
   CPython 3.11.16: constructing the client with `consent=True` and no entries raises
   `EgressDeniedError: the hosted provider endpoint is not in the egress allowlist`.
   (`tests/security/test_model_residency.py:472`.)
5. A test asserts that an approval granted for one request cannot be reused for a different request
   to the same destination. **Met.** The `action_digest` covers the request digest in addition to
   destination, provider, model, capability list, and a digest of the redactions, and it is recomputed
   at call time, so changing any facet of the request invalidates the approval.
   (`tests/security/test_model_residency.py:185,208`.)
6. A test asserts that transcript truncation never splits a multi-byte character. **Met.**
   Truncation operates on encoded bytes and decodes with `errors="ignore"`; every size check is on
   the encoded form. (`tests/security/test_model_residency.py:340`.)
7. A test asserts that `/health` cannot report a hosted path that is not wired. **Met.**
   `egress_configured` is derived from the client the application actually holds, so it is `False` for
   the default local composition root and `True` only for a supplied gateway bound to a hosted client.
   `provider_endpoint_declared` is reported alongside it.
   (`tests/unit/test_api.py:76,97,113,125,133,142`.)
4. An audit query over the journal reconstructs the full set of outbound destinations contacted by a
   given session. **Not met.** `ModelEgressManifest` records destination, request digest, digests,
   capabilities, and redactions, and the gateway refuses to dispatch without recording one, but
   manifests are delivered to a callback rather than written to the journal, and no audit query
   exists. See `docs/architecture-report.md` §3.8.

## Implementation notes (2026-09-26)

How each decision maps to code:

| Decision | Implementation | Reference |
|---|---|---|
| D1 local-first system of record | `KalaApplication` builds `LocalModelClient`; `/health` reports `offline: true`; no socket is opened | `application.py`, `api/app.py:153` |
| D2 single egress choke point, deny by default | `ModelGateway._authorize_egress` builds a fresh `PolicyRequest` on **every** call; `EGRESS_CAPABILITIES = ("model.egress",)`, which is absent from `_default_capabilities()`. `OpenAICompatibleClient._authorize_destination` requires the destination to be in `allowed_endpoints`, which defaults to empty — so an empty allow-list denies | `runtime/model_gateway.py:980,1269,1339`, `application.py` |
| D3 kernel-owned artifact classification | `_derive_classification` is **monotone**: a recognised binary signature, non-decodable bytes, an unrecognised extension, or the tool's own claim all yield `design_binary`, and an **unknown** format defaults to `design_binary` rather than falling through to a permissive case. A tool can never downgrade the verdict | `tools/executor.py:441` |
| D4 provider-neutral `LLMClient` protocol | `LLMClient` `Protocol` with `provider_id`, `model_id`, `requires_egress`, `supports`, `complete`; two implementations | `runtime/model_gateway.py` |
| D5 kernel-owned prompt assembly | **Deviation.** Messages, artifact metadata, and preview text are sanitized and redacted, but there is no typed domain context-block contract and no domain-contributed prompt content. | `runtime/model_gateway.py` |
| D6 verbatim local transcript retention | Bounded transcripts and redaction records are produced and exposed on the gateway; truncation is **byte-safe** (encode, slice at the byte bound, decode with `errors="ignore"`). No journal write, and the reference path never calls the gateway | `runtime/model_gateway.py:236,564,566,1017,1541,1550` |
| D7 local models first class | `LocalModelClient` is the default and only wired client. It is a deterministic **test double**, not a language model, and it is documented as such. | `runtime/model_gateway.py`, `docs/architecture-report.md` §12.3 |
| D8 no silent substitution | `ProviderUnavailableError` becomes `ProviderDegradedError`; `DegradingPlanner` records degradations and never degrades on `NEVER_DEGRADED_ERRORS`, which includes every policy denial | `runtime/agent.py`, `tests/security/test_model_residency.py:526,585` |

Two properties are worth stating plainly because they are easy to overclaim. First, **the
deterministic local client does not make a hosted provider necessary or reachable** — it is a stand-in
that makes the offline path fully exercised. Second, **no `design_binary` exists in this repository**,
so the residency guarantee is verified against synthesized bytes and a signature table, not against a
real CAD file.

## References

- `docs/architecture-report.md` — as-built architecture, requirements status, exact API endpoints and
  CLI commands, and the deviations in §3.
- `docs/verification-report.md` — environment, exact commands, measurements, defects R1–R4, and
  residual risks.
- `docs/threat-model.md` — assets, actors, trust boundaries, per-boundary threats and controls,
  residual risk register.
- ADR-0003 — Deterministic Workflow Engine with Bounded LLM Planning and Policy Checkpoints.
- ADR-0004 — Single-Writer Immutable Revisions over an Event Journal and Content-Addressed Artifact Store.
- ADR-0006 — Package Trust, Sandboxing, Provenance, Rollback, and Agent Budgets.
