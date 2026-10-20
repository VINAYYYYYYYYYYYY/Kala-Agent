# kala-runtime

The native Kala runtime: the provider-neutral model gateway and the agent
runtime that plans and dispatches the tools.

The crate is a complete port of `kala_agent.runtime.model_gateway` (the
`gateway` module) and `kala_agent.runtime.agent` (the `agent` module).

The gateway is the single egress choke point of the Kala kernel: a hosted
provider call needs an explicit consent, a per-request policy approval and a
recorded egress manifest, and a design binary never leaves the machine
(ADR-0001).

The agent runtime is the deterministic substrate of ADR-0003: a plan is *data*
that is validated before it runs, a tool call goes only through the tool
executor, and the budget is reserved and settled.

## Boundaries

The crate owns:

* provider request shaping (messages, capabilities, seed, response schema),
* endpoint parsing, normalization and the allow-list rules,
* egress authorization through the accepted `kala-kernel` policy engine,
* budget reservation before the call and settlement after it,
* provider response validation (identity, strict JSON, schema),
* artifact classification and the design-binary refusal,
* credential redaction of every prompt, artifact and transcript body,
* bounded, byte-safe transcripts and egress manifests,
* the bounded step catalog, the task graph and the dependency layers,
* the offline, model and degrading planners,
* the agent coordinator: the budget admission check, the policy checkpoint, the
  dispatch through the executor and the merge of the answers.

The crate does not own the workflow engine, the tool protocol, the persistence
layer or the domain packages, and it never reaches the network except through
the provider client.

## Dependencies

The accepted crates are consumed read-only as path dependencies:

| Crate | What it owns here |
|---|---|
| `kala-json` | the canonical JSON bytes and the SHA-256 digests |
| `kala-contracts` | the artifact, approval, plan-step and side-effect enums |
| `kala-kernel` | the policy engine and the aware timestamp |
| `kala-budget` | the budget ledger and the CPython-compatible decimal |
| `kala-tools` | the protocol, the registry, the executor and the simulator |

The HTTP client is the same stack the accepted `kala-studio` uses: `reqwest`
over `rustls` with the platform trust store, no redirects and no environment
proxy.

## Usage

### The gateway

```rust
use std::sync::Arc;
use kala_runtime::gateway::{
    LocalModelClient, ModelCapability, ModelGateway, ModelMessage, ModelRequest, ModelRole,
};

let client = Arc::new(LocalModelClient::new(
    "local-deterministic",
    "local",
    vec![ModelCapability::StructuredOutput, ModelCapability::Seed],
)?);
let gateway = ModelGateway::new(client);
let request = ModelRequest::new(
    "request-1",
    vec![ModelMessage::new(ModelRole::User, "summarize the run")?],
)?;
let response = gateway.complete(&request).await?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

A hosted provider is built the same way, with an explicit consent, an
allow-list and a policy engine:

```rust,ignore
let client = OpenAICompatibleClient::new(
    "https://api.example.com/v1/chat/completions",
    "example-model",
    vec!["https://api.example.com:443/v1/chat/completions".to_owned()],
    std::env::var("KALA_PROVIDER_KEY").ok(),
    true,
    OPENAI_COMPATIBLE_PROVIDER_ID,
    30.0,
    1_048_576,
    Arc::new(ReqwestTransport::new()?),
    Decimal::zero(),
    Decimal::zero(),
)?;
let gateway = ModelGateway::new(Arc::new(client))
    .with_policy(Arc::new(policy))
    .with_egress_manifest(Arc::new(|manifest| journal.record(manifest)));
```

### The agent runtime

```rust,ignore
let catalog = StepCatalog::new(vec![CatalogStep::from_parts(
    "simulate",
    "mechanical.reference-evaluator",
    &arguments,
    Some("1.0.0"),
    &[],
    &["beam".to_owned(), "volume".to_owned()],
    "Simulate the beam.",
)?])?;
let coordinator = AgentCoordinator::new(Arc::new(executor))
    .with_catalog(Arc::new(catalog))
    .with_planner(Arc::new(OfflinePlanner::new(catalog, None)?));
let result = coordinator.run(&tasks, "run-1")?;
```

The order of the decisions is the security property of the module and is
reproduced exactly:

1. the task graph is validated and ordered;
2. the budget admits the layer, or the layer is denied as a whole;
3. a task whose dependency did not succeed is cancelled, never dispatched;
4. the tool is resolved, and a task that requests a capability the tool does
   not declare is denied;
5. the policy engine decides, and a tool that needs a decision without a
   policy engine is a hard failure, never a silent dispatch;
6. the tool runs through the executor, which is the only path to a tool;
7. the answers are merged and every merged result is validated against the
   output schema of its tool.

## Tests

The crate has seven test binaries:

* `tests/gateway_semantics.rs` — the behaviour of every public function of the
  gateway, mirroring the repository's Python unit tests.
* `tests/gateway_security.rs` — the residency controls: egress denial,
  per-request approval, manifest ordering, byte-safe truncation and the
  credential redaction.
* `tests/python_differential.rs` and `tests/corpus_differential.rs` — the
  live differential against the repository's own Python gateway and against a
  randomized deterministic corpus. The mock provider records every request it
  received, so the wire traffic of the two implementations is compared too.
* `tests/agent_semantics.rs` — the behaviour of every public function of the
  agent runtime, mirroring `tests/unit/test_agent_branches.py`.
* `tests/agent_security.rs` — the agent controls: a bounded and validated
  plan, the policy checkpoint, the budget admission and the settlement, and
  the absence of an approval replay.
* `tests/agent_differential.rs` — the live differential against the
  repository's own Python agent runtime: the same scripted provider, the same
  simulator tools, the same budget, and a byte-for-byte comparison of the
  decisions, the events, the tool calls, the budget ledger and the results.

The differential tests need the repository virtual environment
(`<repo>/.venv/bin/python`, overridable with `KALA_PYTHON`). Python is a test
oracle only: the crate never invokes it at runtime.
