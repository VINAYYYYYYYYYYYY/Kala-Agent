# Understanding and building Kala Agent

## A guide for a mechanical engineering student learning computer science

Written from the working tree on 15 October 2026. This includes the current, uncommitted FreeCAD work; it describes the source that exists here, rather than assuming the older architecture report describes every newer feature. This is a learning guide, not a fresh security audit or engineering validation report.

## 1. What you are actually building

Kala is an engineering design system with several distinct jobs:

1. Represent a design and its requirements in structured records.
2. Evaluate parameters using engineering domain logic.
3. Control which steps may happen next.
4. Record results, decisions, and immutable revisions locally.
5. Run tools under explicit contracts and resource limits.
6. Optionally use language models to propose structured designs or plans.
7. In the separate FreeCAD package, turn structured construction instructions into native CAD operations.

The most useful initial distinction is between **the engineering calculation** and **the system that makes the calculation reproducible, controllable, and traceable**. A stress formula may occupy ten lines. Validating its inputs, tracking its version, preserving its outputs, handling failures, and preventing an invalid commit take far more code.

Think of a controlled engineering workshop. Domain logic is the engineering calculation sheet. Contracts are the drawing and inspection formats. The kernel is the production procedure and interlocks. Persistence is the controlled document archive. The tool executor is the interface to machinery. A model is a design assistant whose proposals still need inspection. `KalaApplication` assembles these systems into a usable process.

These analogies explain responsibilities, but have limits: software validation cannot establish physical correctness, and recording a result does not make it a correct result.

### Two paths you must keep separate

**Core reference path:** the CLI or HTTP API calls `KalaApplication`, which runs deterministic arithmetic evaluators, manages checkpoints, and writes revisions. The default model client is offline and deterministic. The mechanical reference workflow does not send its evaluation through the subprocess tool executor.

**FreeCAD design path:** the FreeCAD dock accepts a prompt, the adapter calls remote models, parses a construction script, compiles that script into tool calls, checks their procedure, and executes CAD operations. This path has its own GUI, orchestration, and run logging. The core composition root does not automatically register this package.

In particular, do not assume that the FreeCAD GUI inherits every policy or network control in the core model gateway. Its `cad/models.py` contains direct OpenRouter HTTP calls. That is an architectural distinction visible in the current source.

### What the results mean today

The mechanical, wing, and building domain simulations are closed-form surrogates. The bracket material constants are explicitly illustrative. The FreeCAD adapter can manipulate real CAD objects, but valid geometry is not proof of adequate structural performance. A compiler accepting a construction script, a workflow completing, a solver running, and an engineering design satisfying its intended duty are four different outcomes.

## 2. A map of the repository

Start from the project root, `/home/v9i/Projects/Kala-Agent`.

- `pyproject.toml`: package metadata, dependencies, executable entry point, and lint/test/type-check configuration.
- `src/kala_agent/contracts/`: shared data records and construction-script vocabulary.
- `src/kala_agent/domains/`: mechanical, wing, and building parameters, calculations, evaluations, and workflow declarations.
- `src/kala_agent/kernel/`: workflow execution, plan validation, policy, resource accounting, and memory support.
- `src/kala_agent/persistence/`: SQLite records, event history, revision repositories, snapshots, and artifact bytes.
- `src/kala_agent/tools/`: registry, wire protocol, executor, sandbox support, and reference simulator tool.
- `src/kala_agent/runtime/`: planners, agent coordination, and model clients/gateway.
- `src/kala_agent/application.py`: construction and coordination of the core services.
- `src/kala_agent/api/`: HTTP presentation and request protection.
- `src/kala_agent/cli/`: terminal commands and JSON output.
- `tests/unit/`: component behavior.
- `tests/integration/`: behavior across multiple components and presentation layers.
- `tests/security/`: adversarial inputs and security properties.
- `adapters/freecad/`: independently packaged FreeCAD integration and its tests.
- `adapters/freecad/mod/Kala/`: FreeCAD startup and dock UI code.
- `docs/adr/`: architectural decision records explaining intended choices. Their status matters; proposed decisions are not automatically implemented guarantees.

Ignore caches, `.coverage`, and generated `__pycache__` directories when learning. `.venv` is the local Python environment, not project source.

Read [the existing reading map](how-to-read.md) alongside this guide. Use [the architecture report](architecture-report.md) for deeper historical context, checking current source when the two differ.

## 3. Python concepts through code you already care about

Read `domains/mechanical/simulation.py` first. You already understand the quantities, so unfamiliar Python syntax is easier to isolate.

### Variables, values, and units

```python
load_n = parameters.load_kn * 1_000.0
```

`parameters.load_kn` reads a field of an object. `*` multiplies two values. `=` binds the resulting value to the name `load_n`. Underscores in `1_000.0` improve readability; the value is 1000.0.

A Python `float` does not remember whether it represents N, mm, or MPa. Names such as `load_n`, explicit conversion constants, and unit metadata are important here. They are conventions and validation aids, not a full dimensional-analysis type system.

### Functions and pure computation

`simulate_bracket(parameters)` accepts a parameter record and returns a `BracketSimulation`. It does not open a database, call a model, or write a file. This separation makes it possible to reason about the calculation independently.

A pure function is conceptually `output = f(input)` with no externally observable side effects. Determinism means the same inputs produce the same outputs under the same execution assumptions. These are related properties, not synonyms: a function can deterministically write a file and therefore still have a side effect.

This calculation is a good first debugger exercise: inspect `parameters`, step through each assignment, and inspect the returned record.

### Classes and records

`BracketParameters` uses Pydantic, while `BracketSimulation` is a dataclass. Both collect related fields, but their purposes differ.

Pydantic validates external or constructed data against declared field rules. A dataclass generates useful record machinery such as an initializer; its ordinary type annotations do not provide the same runtime validation.

`@dataclass(frozen=True, slots=True)` means the record disallows ordinary field reassignment and limits instance attribute storage. It is useful for stable result records. Neither a frozen dataclass nor a frozen Pydantic model should be treated as a universal guarantee that every nested mutable object is immutable.

### Type annotations

```python
def simulate_bracket(parameters: BracketParameters) -> BracketSimulation:
```

The colon describes the expected input type; the arrow describes the return type. Type annotations help readers and static tools such as mypy. Python itself does not automatically enforce every function annotation at runtime.

`str | None` means a string or no value. `tuple[str, ...]` means a tuple containing zero or more strings. `Mapping[str, object]` describes a dictionary-like interface. `Final` expresses an intention to static checking tools that a binding should not be reassigned; it does not magically freeze its contents.

### Exceptions and context managers

An exception reports that normal execution cannot continue. Catch it at a layer that can meaningfully translate or handle it. A failed parameter validation should not be silently converted into a valid-looking calculation.

A `with` block controls a resource's lifetime. `with KalaApplication(...) as app:` ensures application cleanup. Database transaction context managers group operations so a failure can roll back the database changes.

### Imports and packages

A module is usually one `.py` file. A package organizes modules under an import namespace. `src/` contains the package source; it is not normally part of the import name. Import `kala_agent.domains.mechanical`, not `src.kala_agent.domains.mechanical`.

The core and adapter are separate distributions with separate `pyproject.toml` files. Installing core Python dependencies does not install the native FreeCAD runtime.

## 4. Follow the bracket calculation by hand

Open these in order:

1. `src/kala_agent/domains/mechanical/parameters.py`
2. `src/kala_agent/domains/mechanical/simulation.py`
3. `src/kala_agent/domains/mechanical/evaluation.py`
4. `src/kala_agent/domains/mechanical/workflow.py`

The default plate is 120 × 80 × 8 mm with two 6 mm holes, a 1 kN load, and two load introduction points. The illustrative aluminium density is 2700 kg/m³ and yield strength is 276 MPa.

The implementation computes:

```text
Gross plan area        = L × W
Removed hole area      = (π/4) × hole_count × diameter²
Net plan area          = gross area − removed hole area
Volume                 = net plan area × thickness
Mass                   = volume × 10⁻⁹ × density
Force                  = load_kN × 1000
Force per point        = force / load_points
Lever arm              = L/2
Bending moment         = force × lever arm
Section modulus        = W × thickness² / 6
Bending stress         = moment / section modulus
Shear stress           = 0.5 × force / (W × thickness)
Combined surrogate     = sqrt(bending_stress² + 3 × shear_stress²)
Utilisation            = combined surrogate / illustrative yield strength
Safety factor          = illustrative yield strength / combined surrogate
```

For zero stress the implementation uses a named finite sentinel, rather than emitting mathematical infinity into its result data.

Running the current code gives approximately:

- Mass: **0.206139 kg**.
- Bending moment: **60,000 N·mm**.
- Section modulus: **853.333 mm³**.
- Bending stress: **70.3125 MPa**.
- Combined surrogate stress: **70.32552 MPa**.
- Safety factor: **3.92461**.
- Score: **0.923559**.

Notice that hole removal changes the mass calculation but the bending section modulus still uses the gross rectangular section. The edge-margin expression also uses a simplified linear hole pattern. These are assumptions in the surrogate, not a general treatment of perforated brackets or local stress concentrations.

### Calculation versus evaluation

The simulation produces quantities. Evaluators decide what those quantities mean against declared limits and wrap them in `EvaluationResult` records.

The geometry evaluator checks minimum thickness. The material evaluator reports constants and checks edge margin. The load evaluator checks the per-point bound. The structural evaluator checks stress and thickness and reports the safety-factor margin.

A target safety-factor warning is distinct from a hard constraint failure. In the current structural evaluator, a low target safety margin adds a finding and affects the score; `passed` is determined by the stress and thickness checks there. Other evaluators own their own hard checks.

This teaches an important software modeling skill: distinguish **computed value**, **acceptance rule**, **warning**, and **ranking score**. They should not be collapsed into one boolean.

## 5. Contracts: the software equivalent of controlled interfaces

Read `contracts/models.py`, beginning with `DesignSpec`, `EvaluationMetric`, `EvaluationResult`, `WorkflowDefinition`, `ApprovalRequest`, and `DesignRevision`.

A contract answers: what fields are required, which values are permitted, and which combinations are invalid?

The shared base model forbids undeclared fields, freezes ordinary model assignment, and validates defaults. Additional validators enforce relationships such as unique identifiers and valid workflow references. Do not assume every scalar is globally strict merely because the model has these settings; look at its actual field type and validators.

An **invariant** is a condition that must always remain true. Examples include a revision not being its own parent and a workflow transition naming an existing state. In mechanics, you might enforce compatible dimensions before solving an equation. Here, you enforce compatible records before running a process.

### Serialization and canonicalization

Serialization turns an in-memory record into bytes or text, often JSON. Deserialization reconstructs data from that representation. Validation is a separate question: can the reconstructed data be accepted?

Canonicalization selects one stable representation. These objects can mean the same thing:

```json
{"width":80,"length":120}
```

```json
{"length":120,"width":80}
```

But their literal bytes differ. Stable hashing requires a defined representation, including key order and number handling. The contract helpers address duplicate keys and non-finite values as well as formatting. JSON parsing alone is not sufficient validation.

Exercise: find a validator in this file, write an input that violates it, and explain what downstream bug its rejection prevents.

## 6. Workflow engines, state machines, and graphs

The mechanical workflow declares:

```text
specified
  → sectioned
  → material-graded
  → load-cased
  → assessed
```

Each transition names a catalog step. The final assessment transition declares a `PRE_COMMIT` checkpoint.

A finite state machine consists of states, permitted transitions, and rules for taking a transition. This is closely related to an industrial controller: a machine should not start a cutting cycle before its required setup is complete.

`kernel/workflow.py` implements the shared machinery. The domain declares its own workflow without importing that engine. This lets mechanical, wing, and building packages describe different processes without teaching the engine their formulas.

Read `KalaApplication.run_mechanical_reference_workflow` after understanding the small domain files. It starts or restores workflow state, obtains the next step, evaluates it, records the result, advances or records failure, saves snapshots, and registers a pending checkpoint. It bounds the step loop. It returns at the checkpoint rather than quietly committing a revision.

### Plans are dependency graphs

A plan can express dependencies between steps. A graph has vertices and edges; here, a vertex is a task and an edge represents a dependency. A cycle such as A needing B while B needs A prevents either from becoming ready.

`kernel/planning.py` checks unknown steps, missing dependencies, self-dependencies, cycles, attempts, capabilities, and plan limits. This is a concrete reason to learn graph traversal and topological ordering.

Start with these data structures: lists for ordered actions, dictionaries for lookup by ID, sets for membership and duplicate detection, queues for ready work, and graphs for dependencies. Learn their behavior and complexity before trying advanced algorithms. Typical dictionary/set lookup is average O(1); scanning a list is O(n). Graph traversal can be O(V + E), depending on representation and implementation.

## 7. Persistence: what survives after Python exits

There are three different kinds of state:

- In-memory objects: convenient while the process runs.
- SQLite records: durable structured history and current records.
- Artifact files: durable result bytes, addressed by content digest.

Read `persistence/database.py`, `journal.py`, `artifacts.py`, and then the relevant repository in `repositories.py`.

### SQL and transactions

SQLite is an embedded relational database. A table stores rows with defined columns. A key identifies a row; indexes speed selected lookups; constraints and triggers enforce some rules.

A transaction groups changes into a commit or rollback. For a revision operation, you do not want a revision record written while its required associated database records are missing.

The connection wrapper and writer lock serialize access to the shared connection. The lock covers logical operations so another thread cannot interleave statements halfway through a transaction. This is about correctness under concurrency, not just speed.

Database transactions do not automatically roll back external files. The artifact store has its own publication mechanism, so reason separately about file publication and database commitment.

### Journal, snapshots, revisions

An event journal records occurrences in order. A snapshot stores a serialized state used for restoration. A revision is an immutable engineering record with a parent relationship and provenance. These solve related but different problems.

The journal has database triggers refusing normal updates/deletes and a hash chain for integrity verification. Hash chaining makes changes detectable when checked against trustworthy expectations; it is not a cryptographic signature from an external authority. Someone who controls the entire store may have more power than someone merely using repository APIs.

### Content-addressed storage

The artifact store uses SHA-256 of the bytes as an address. An artifact URI looks like `artifact://sha256/<digest>`. Identical bytes yield the same digest, enabling deduplication. Reading verifies the digest; publication uses an atomic mechanism to avoid exposing partial artifacts.

A digest identifies content. It does not establish who produced it or whether the content is physically correct. Provenance, trust, and engineering validation are separate concerns.

### Why revision preparation and commitment are separate

Read `prepare_design_revision`, `decide_revision_commit`, and `commit_design_revision` in `application.py`.

The process derives a proposed revision and the digests to which approval applies. A later decision checks that the approved state is still the relevant state. Head comparison prevents stale work from silently overwriting newer work.

Example: two operations prepare against revision R0. One commits R1. The other must not pretend R0 is still the current head. This is optimistic concurrency control, commonly implemented as compare-and-swap: update only if the expected previous value still matches.

An approval is therefore a decision about specified content and context, not an unlimited permission to commit anything later.

## 8. Tools, operating systems, and resource limits

Read `tools/protocol.py`, `registry.py`, `executor.py`, then selected parts of `sandbox.py`.

A tool descriptor declares identity, inputs, outputs, capabilities, and side effects. The registry maps approved tool identities to implementations. The executor validates and dispatches a call and handles results and failure conditions.

### A process is not a thread

A process has its own address space and operating-system identity. A thread shares its process's memory. A child process can crash without necessarily taking down the parent, but that does not mean it cannot read the parent's files or reach the network.

The tool plane includes subprocess execution and sandbox profiles. The default composition uses the portable profile; do not equate it with the strict Linux filesystem-confining profile. The strict profile uses Linux isolation features and probes their enforcement.

This is why the operating-system topics matter: process creation, stdin/stdout pipes, exit codes, signals, permissions, timeouts, mounts, and namespaces determine what an external tool can actually do.

### Capabilities and budgets

A capability is authority to perform a particular category of action. A schema-valid call can still be unauthorized. Policy decides whether the requested authority is allowed or needs a checkpoint.

The budget ledger accounts for model calls, tool calls, tokens, artifact bytes, duration, cost, and replanning. Reservations avoid allowing several pending operations to each spend the same remaining budget. Actual use and reserved use must both be considered when calculating availability.

This resembles capacity planning for shared machinery: allocating every job against total capacity without subtracting already reserved capacity overbooks the machine.

## 9. Language models and agent control

Read `runtime/model_gateway.py` and `runtime/agent.py`.

A language model generates proposed text or structured data. It can produce malformed, inconsistent, or unsuitable output. A planner converts a request into steps; a validator checks those steps; a coordinator drives the process. A model is one possible implementation behind these interfaces.

The core's `LocalModelClient` is a deterministic offline client. Its existence does not imply a local neural-network inference engine is running. The default `KalaApplication` also constructs an `OfflinePlanner`.

The core gateway's remote-client path includes explicit egress controls and request manifests. Study the protocol/interface between a client and gateway: callers should depend on the behavior they need, rather than a particular provider implementation. This is dependency inversion.

Dependency injection means supplying collaborators such as a clock, model client, or database when constructing an object. `KalaApplication.__init__` exposes many such seams. Tests can inject a fixed clock or fake collaborator to reproduce behavior without relying on live services.

An agent loop needs stopping conditions: step bounds, retry bounds, replanning bounds, time limits, and budgets. Repeatedly asking a model to fix its own output without these limits is not a reliable workflow.

## 10. The FreeCAD path is also a compiler project

Read the adapter in this order:

1. `src/kala_agent/contracts/construction.py`
2. `adapters/freecad/src/kala_agent_freecad/cad/compile.py`
3. `cad/procedure.py`
4. `cad/design.py`
5. `cad/accept.py`
6. `cad/compose.py` and `cad/models.py`
7. `adapters/freecad/mod/Kala/kala_gui.py`

### From intention to executable operations

```text
User prompt
  → remote model proposal
  → validated ConstructionScript
  → acceptance checks and bounded correction in composition
  → compiled CAD tool calls
  → procedure validation
  → native FreeCAD execution
  → measurements, output artifacts, and run history
```

The exact entry point matters: the GUI runs work in the open FreeCAD session; the adapter also has worker-oriented tools. Do not assume every CAD operation takes the same dispatch route.

A construction script is a small domain-specific language: a deliberately limited vocabulary for engineering actions. Components contain steps such as sketch, extrusion, pocket, revolve, and helix. Assembly instructions refer to components. Analysis and revision records have their own vocabulary.

The script is an **intermediate representation**, or IR. It separates what the design says from how a particular backend realizes it. In a conventional compiler, source code becomes an IR and then machine instructions. Here, a construction description becomes toolbar-style operations and then FreeCAD API calls.

`compile_construction` walks components in order, emits each component's calls, adds assembly operations, and optionally appends analysis operations. `apply_edits` and `rewrite_component` transform scripts. These are useful introductions to compiler lowering and structured program transformations.

### Different kinds of validity

- JSON validity: can the text be parsed?
- Schema validity: are fields and values accepted?
- Reference validity: do named components exist?
- Procedure validity: is this operation legal at this stage?
- Geometric validity: did the native kernel produce an acceptable shape?
- Duty satisfaction: does the proposal address the requested function?
- Physical adequacy: does the design meet verified engineering requirements?

Passing an earlier stage does not imply passing later ones.

`procedure.py` tracks stages such as start, body, sketch, profile, and solid. It rejects operations before prerequisites exist and rejects a procedure that ends with an open sketch or without a solid. Full geometric and numeric correctness still depends on deeper validation and execution.

`accept.py` adds prompt-sensitive checks. For example, a requested hole needs a pocket, and a thread needs a helix. Screw-jack checks examine a base, screw, cup, tommy bar, dimensions, pitch, and alignment. These are specific heuristics over the script, not a general natural-language theorem prover or a complete machine-design analysis.

### GUI concurrency

`kala_gui.py` defines a `QThread` planner and Qt signals for logs, ready results, and failures. Network planning can take time; performing it directly in the UI event loop would freeze interaction. Signals communicate the result back to the UI.

This teaches event-driven programming: the program responds to button presses, thread completion, and timers. CAD document and UI access must respect the host application's threading requirements. Putting an operation on a background thread is not automatically safe.

### Dependencies and native code

The adapter lazily loads FreeCAD and reports availability. Native CAD modules depend on more than ordinary Python packages: interpreter compatibility, shared libraries, import paths, and application state can matter.

Fakes let unit tests exercise orchestration without a real CAD installation. Real-runtime tests check integration with the native application. A fake passing does not establish that a real FreeCAD build or solver is installed and working.

## 11. CLI, HTTP, and the application boundary

The CLI translates command arguments into application calls and emits JSON. HTTP handlers translate requests into those same application operations. They should not each reimplement engineering calculations or commitment rules.

HTTP adds transport concerns: methods, paths, request bodies, status codes, authentication, origins, and size limits. The API protects loopback access with a launch bearer token and request-boundary checks. A service on localhost can still be targeted by other local processes or browser-mediated requests.

`127.0.0.1` is the loopback address. A port selects a listening service on that address. A bearer token is a credential carried in a request; it is not encryption. An API schema describes a message; it is not authorization.

Read `api/app.py` for routes, then `boundary.py`, `middleware.py`, and `errors.py` for boundary behavior. Learn HTTP after you understand a direct application call; otherwise framework details can hide the real operation.

## 12. Hands-on lab: run the core without a model or CAD

These commands use the existing environment and a separate temporary data directory. Run them from the repository root. They do not need provider credentials.

```bash
.venv/bin/python -m kala_agent.cli --help
.venv/bin/python -m kala_agent.cli --data-dir /tmp/kala-student-lab init
.venv/bin/python -m kala_agent.cli --data-dir /tmp/kala-student-lab domains
.venv/bin/python -m kala_agent.cli --data-dir /tmp/kala-student-lab evaluate --domain mechanical
.venv/bin/python -m kala_agent.cli --data-dir /tmp/kala-student-lab evaluate --domain mechanical --param thickness_mm=4.0
```

Predict the thinner plate's effect before reading the output. In this surrogate, halving thickness reduces section modulus by a factor of four and increases bending stress by a factor of four. At 4 mm the stress exceeds the default 150 MPa limit. The CLI distinguishes an evaluation with a failed outcome from a command that could not execute.

Next, run a workflow with fresh identifiers:

```bash
.venv/bin/python -m kala_agent.cli --data-dir /tmp/kala-student-lab run-mechanical --design-id student-bracket --workflow-id student-workflow
.venv/bin/python -m kala_agent.cli --data-dir /tmp/kala-student-lab workflow-status --workflow-id student-workflow
```

Read the checkpoint and evaluation output. To exercise the explicit approval path for this lab:

```bash
.venv/bin/python -m kala_agent.cli --data-dir /tmp/kala-student-lab approve --workflow-id student-workflow
.venv/bin/python -m kala_agent.cli --data-dir /tmp/kala-student-lab workflow-status --workflow-id student-workflow
.venv/bin/python -m kala_agent.cli --data-dir /tmp/kala-student-lab events
```

After completion, use a new workflow ID for another run; terminal workflows are not silently restarted. Approval here commits the reference result, not a certified engineering design. Parameter overrides belong to the evaluation and revision-preparation interfaces; the reference workflow command does not accept arbitrary parameter overrides.

To inspect the pure calculation independently:

```bash
.venv/bin/python - <<'PY'
from kala_agent.domains.mechanical.parameters import BracketParameters
from kala_agent.domains.mechanical.simulation import simulate_bracket

parameters = BracketParameters()
result = simulate_bracket(parameters)
print(result)
print(result.mass_kg, result.stress_mpa, result.safety_factor)
PY
```

## 13. How to read and use tests

Start with `tests/integration/test_mechanical_loop.py`. It demonstrates complete operations and checks properties such as immutable revision chains, artifact hashes, approval-bound commits, provenance, and checkpoint behavior.

Then pair each source file with a focused test. For the adapter, use `test_compile.py`, `test_procedure.py`, `test_accept.py`, and the corresponding fake-runtime tests.

A test usually has three parts: arrange inputs and collaborators; act by calling the behavior; assert the observable outcome. A pytest fixture supplies repeatable setup and cleanup. `tmp_path` creates an isolated test directory. `monkeypatch` temporarily replaces behavior such as network access.

Learn to distinguish:

- Unit tests: isolate behavior and error branches.
- Integration tests: verify components agree at their interfaces.
- Security tests: attempt actions the system must reject.
- Live tests: exercise the actual native runtime, when available.

Useful commands from the project root:

```bash
.venv/bin/python -m pytest tests/integration/test_mechanical_loop.py -q
.venv/bin/python -m pytest tests/unit/test_domains.py -q
.venv/bin/python -m pytest tests/security/test_revision_commit.py -q
.venv/bin/python -m ruff check src tests
.venv/bin/python -m mypy src tests
```

For adapter tests, change to its directory so its own pytest configuration is selected:

```bash
cd adapters/freecad
../../.venv/bin/python -m pytest tests/test_compile.py tests/test_procedure.py tests/test_accept.py -q
```

Pytest checks examples of behavior. Mypy checks static type consistency. Ruff checks configured style and coding rules. Coverage measures exercised code paths. None of these alone proves that a structural model is physically correct.

## 14. A practical learning sequence

Treat these as stages with exit criteria, not a race through files.

### Stage 1: Python and numerical code

Learn values, functions, conditionals, loops, records, exceptions, modules, and basic floating-point behavior. Study the mechanical simulation. Recompute the default result by hand and write a small script varying thickness.

**Ready to advance when:** you can explain every expression in `simulate_bracket`, including unit conversions and the zero-load branch.

### Stage 2: Modeling data and invariants

Learn type annotations, Pydantic validation, enums, JSON, immutability, and cross-field constraints. Study parameters and shared contracts. Try valid and invalid inputs and explain why each is accepted or refused.

**Ready to advance when:** you can separate syntactic validity, parameter admissibility, and engineering acceptance.

### Stage 3: Architecture and interfaces

Learn modules, dependency direction, protocols, composition, and dependency injection. Trace one CLI evaluation into `KalaApplication` and then into the domain package. Draw the imports separately from the sequence of calls; they are different diagrams.

**Ready to advance when:** you can explain where a new calculation belongs and why it should not import the API or database.

### Stage 4: Data structures and workflows

Learn sets, dictionaries, queues, graphs, finite state machines, and cycle detection. Trace a mechanical workflow through its checkpoint. Read the plan rejection codes and construct one failing example for several of them.

**Ready to advance when:** you can explain how invalid ordering, retries, and missing approval stop progress.

### Stage 5: Databases and integrity

Learn SQL tables, keys, indexes, transactions, locks, optimistic concurrency, hashing, and canonical serialization. Trace a revision from preparation to commitment and an artifact from publication to verified read.

**Ready to advance when:** you can explain why approval can become stale and why a database transaction is not a filesystem transaction.

### Stage 6: Operating systems and transport

Learn processes versus threads, files and permissions, pipes, timeouts, HTTP, and authentication. Study the executor and API boundaries. Use tests to inspect failures before attempting native runtime changes.

**Ready to advance when:** you can explain what process isolation does and which additional controls filesystem confinement needs.

### Stage 7: Compilers and CAD execution

Learn parsing, IRs, semantic checks, lowering, stateful execution, and event loops. Follow a construction script through compile, procedure validation, and execution. Read the fake-runtime tests before relying on the live host.

**Ready to advance when:** you can identify whether a failed design is a proposal, schema, procedure, geometry, solver, or engineering-duty problem.

### Stage 8: Model-assisted orchestration

Learn bounded generation, retries, structured output validation, tool authority, provider errors, and reproducible evaluation. Compare the core model gateway with the adapter's direct model calls.

**Ready to advance when:** you can describe the controls of each actual call path without assuming they share enforcement.

## 15. Exercises that build toward contributing

1. **Sensitivity script:** sweep admissible thickness values and print mass, surrogate stress, and safety factor. Predict trends first. Keep this outside the domain implementation until you decide it is a product feature.
2. **Validation investigation:** find an individually in-range parameter combination rejected by cross-field validation. Explain the invariant rather than just the exception text.
3. **Failure trace:** create a thin-plate evaluation, follow the failed metric into the evaluator and CLI result, and distinguish execution success from design acceptance.
4. **Graph exercise:** draw a five-task plan with a dependency cycle. Explain how a traversal detects it and why retrying cannot repair the unchanged graph.
5. **Revision experiment:** use a temporary store to prepare competing revisions, commit one, then observe stale-head rejection for the other. Follow the existing tests so the expected digests are used correctly.
6. **Compiler trace:** take a construction script from `adapters/freecad/tests/test_compile.py`, list its emitted calls, and annotate the procedure stage after each call.
7. **Acceptance investigation:** read a passing and failing screw-jack example in `test_accept.py`. Identify which heuristics are checked and which physical duties remain outside them.
8. **Small domain extension:** add a new clearly named derived metric, its unit, evaluator output, and a meaningful expectation for known inputs. Avoid introducing persistence or UI dependencies into the formula.
9. **Larger design exercise:** draft the contracts and failure states for a shaft domain before implementing it. Explain parameter validation, evaluator steps, provenance, and how a future solver adapter would differ from the surrogate.

Do not begin by rewriting `application.py`, weakening validators, or adding an unrestricted model-to-Python execution path. Begin with one small, observable behavior whose responsibilities you understand.

## 16. A checklist for making a change

Before editing, describe the behavior in one sentence and identify its owner. Read the closest existing test. Check the working tree so you can distinguish your edits from existing work.

For a calculation change, write down the assumptions and units, derive a known result, and check relevant boundaries. For a contract change, consider stored-record compatibility and every producer/consumer. For a tool change, consider authority, timeouts, output publication, and failure reporting. For a workflow change, consider restart, rejection, retries, and checkpoint semantics.

After editing, run focused tests and relevant static checks. Add tests for meaningful new behavior or risks, rather than tests that merely repeat each implementation line. Read the diff and verify that unrelated files did not change.

A useful completion note says what changed, why, what was checked, and what remains unverified.

## 17. Questions you should eventually be able to answer

- Why is an evaluator different from a simulator?
- Why are validation, authorization, and physical correctness separate?
- Why does the kernel not need to understand bracket equations?
- What persists when the process stops?
- What is the difference between a snapshot, event, artifact, and revision?
- Why do stable hashes need stable serialization?
- What prevents two operations from committing incompatible heads?
- What does a frozen record protect, and what can it fail to protect?
- Why is a subprocess not automatically a filesystem sandbox?
- How do budget reservations prevent overspending?
- Why compile a construction script instead of executing model-generated Python?
- What can procedure validation establish before FreeCAD runs?
- Why can a passing fake-runtime test miss a native integration failure?
- Which model calls use the core gateway, and which use the adapter's direct client?
- Which evidence would be needed before a generated part could be treated as an adequately verified engineering design?

If you can answer these using actual functions and tests in this repository, you understand much more than its file layout: you understand the engineering of the software itself.

## 18. Verification behind this guide

The guide was grounded in the project configuration, module inventory and responsibilities, core contracts, mechanical parameter/simulation/evaluation/workflow code, composition and workflow-driving methods, persistence and plan-validation code, tool isolation definitions, CLI parser, integration tests, and current FreeCAD construction/compilation/procedure/acceptance/model/GUI code.

The default simulation values above were executed with this repository's `.venv/bin/python`. CLI syntax was checked against the current parser and CLI help. The lab uses a separate temporary data directory. Existing verification counts in older reports are historical; they are not presented here as fresh measurements. See the accompanying delivery message for checks actually run in this session.

Session checks: all nine CLI lab invocations were exercised in an isolated temporary directory with the expected exit codes and parseable JSON. The focused adapter compilation, procedure, and acceptance suite passed: 31 tests. The core integration suite passed its first two tests, then made no further progress during observation and was interrupted; a complete integration-suite pass is not claimed. The full suite, live CAD execution, and remote providers were not exercised for this documentation task.
