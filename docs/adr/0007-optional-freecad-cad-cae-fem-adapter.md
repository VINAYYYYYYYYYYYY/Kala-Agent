# ADR-0007: Optional FreeCAD CAD/CAE/FEM Adapter as a Separate Distribution

| Field | Value |
|---|---|
| Status | **Proposed** — awaiting stakeholder confirmation. Implementation evidence does not convert a Proposed decision into accepted policy. |
| Date | 2026-09-26 |
| Last reviewed | 2026-09-26 |
| Scope | An optional external engineering tool adapter, its seam to the kernel, its dependency model, and its honesty obligations |
| Requirements addressed | A new FR-17 (an optional, separately distributed CAD/CAE tool adapter) and NFR-19 (an optional system dependency must not become a core dependency); it does not satisfy any of the MVP's existing acceptance criteria on its own |
| Related ADRs | ADR-0001 (local-first, no egress), ADR-0002 (field-independent kernel, declarative packages), ADR-0004 (immutable revisions, content-addressed artifacts), ADR-0005 (capability-based tool contracts, isolated processes), ADR-0006 (package trust, sandboxing, provenance) |
| Implementation status | **Implemented as a separate distribution, and explicitly not part of the product composition root.** `adapters/freecad` builds the distribution `kala-agent-freecad` (package `kala_agent_freecad`), which publishes two frozen tool contracts — `cad.freecad-geometry` 1.0.0 and `cad.freecad-fem` 1.0.0 — behind `SubprocessToolHandler`, an availability probe, a CAD sandbox profile, and two worker entry points. **Not implemented: automatic core wiring (KalaApplication does not discover, register, or dispatch these tools), any certified engineering result, and a bundled mesher or solver.** Verified on a host with FreeCAD 1.1.3, and verified absent-safe on a host without it. |
| Verification | **301 adapter tests pass with 100% branch coverage** (1 152 statements, 306 branches, 0 missed, 0 partial), `ruff check`, `ruff format --check`, and strict `mypy` clean over 30 files. Root suite unchanged at **1 227 tests**, `ruff` and strict `mypy` clean. 17 of the 301 tests are marked `freecad` and exercise a real installation; 284 run without one. See `docs/verification-report.md` §2.7 and §4.14. |

## Context

ADR-0005 decided that tools run as isolated subprocesses behind typed contracts, and ADR-0006 decided
that executable tools are separately versioned packages rather than part of a domain package. Both
left one question open rather than answered: **what does the first real engineering integration look
like, and what does it cost the kernel?**

The previous answer in this repository was "none". `docs/architecture-report.md` §3.2 recorded that
there was no native executable, no vendor SDK, no geometry kernel, no mesher, and no solver anywhere in
the tree, and the only tool was `mechanical.reference-simulator`, a closed-form arithmetic stand-in.
That was honest, and it left a real gap: the tool plane was complete, tested, and entirely hypothetical
with respect to the kind of software an engineering user actually has on their machine.

FreeCAD is the natural first subject for three reasons.

1. It is the most widely deployed free parametric CAD/CAE system on Linux, and it is **already a
   system dependency on an ordinary engineering workstation** — there is nothing to install, vendor-
   contract, or containerize.
2. It is **LGPL-2.0-only**, dynamically linked, and importable as a plain Python extension module from
   a directory on the filesystem. That makes it a test of ADR-0005's import-root seam: a worker
   interpreter started with `python -I` inherits no host path, so an adapter that wants FreeCAD must
   be handed its import root explicitly and must be able to say so honestly.
3. It is **not** a Python wheel and never will be a supported one. Anything that claims otherwise is
   either wrong or is vendoring a build nobody maintains.

The failure modes this creates are specific and predictable, and they are the reason a decision was
needed rather than a module.

- **Import order is a safety property, not a style preference.** FreeCAD's workbench modules bind to
  the application library that only `FreeCAD` itself initialises. Importing `ObjectsFem` or
  `femtools.ccxtools` first can abort the process *inside* the native library. No Python-level
  `try`/`except` recovers from that, so a mis-ordered import is a process abort rather than a
  handled error.
- **The native library writes to file descriptor 1**, bypassing `sys.stdout`. A FreeCAD banner printed
  there would corrupt the tool protocol channel and turn a successful call into a malformed one.
- **A CAD/CAE tool wants a much larger artifact and time budget** than the shipped default allows —
  a 256 MB FCStd export is not a pathological input — which means an adapter that silently reuses the
  default profile will fail on legitimate work.
- **A solver is an external executable found by name on `PATH`.** Running one that a caller supplied
  would be arbitrary code execution by configuration, and *not* running one while reporting numbers
  would be fabricating engineering results. Both directions need an explicit policy.
- **Optional dependencies make import-time side effects poisonous.** An adapter that imports FreeCAD at
  module scope breaks every host that does not have it, and the failure appears in the *core* if the
  adapter is imported from the composition root.

That last point is the load-bearing one. The question is not "how do we call FreeCAD" but **"where does
the seam live, so that a host without FreeCAD is unaffected and a host with it is not silently
granted new authority."**

## Decision

**D1 — FreeCAD support ships as a separate, optional distribution. It is not a core module and the
core never imports it.**

`adapters/freecad` is an independent `pyproject.toml` with its own distribution name
(`kala-agent-freecad`), its own `src` root, its own tests, its own lint/type/coverage configuration,
and one declared dependency on `kala-agent`. The core `pyproject.toml` gains nothing: no dependency, no
extra, no optional group, and no import. The core's declared runtime dependencies remain exactly
`fastapi` and `pydantic`.

The boundary is enforced statically, not by convention. `tests/unit/test_adapter_boundaries.py` walks
every file under `src/kala_agent/` and asserts that **no core module imports `FreeCAD`, `femtools`, or
`kala_agent_freecad`**, and it walks every file under the adapter and asserts that **no adapter module
imports `kala_agent.application`, `kala_agent.api`, `kala_agent.cli`, `kala_agent.domains`,
`kala_agent.kernel`, `kala_agent.persistence`, or `kala_agent.runtime`**. The adapter reaches the
kernel only through `kala_agent.contracts`, `kala_agent.tools.protocol`, `kala_agent.tools.registry`,
`kala_agent.tools.executor`, and `kala_agent.tools.sandbox`.

**D2 — FreeCAD is a discovered system dependency. There is no FreeCAD PyPI wheel, and the adapter
never installs one.**

`discover()` resolves availability from the host filesystem alone, and it is the *only* function that
decides whether the adapter has anything to offer:

- It locates `freecadcmd` then `freecad` on `PATH` (both must resolve to a regular executable file).
- It derives candidate import roots from the interpreter's own parents and from a fixed prefix list
  (`/usr`, `/usr/local`, `/opt/freecad`, `/opt/FreeCAD`, `/app`, `/snap/freecad/current`) crossed with a
  fixed relative-layout table (`lib/freecad/lib`, `lib/freecad-python3/lib`, `lib64/freecad/lib`,
  `lib/freecad/lib64`, `freecad/lib`, `lib/freecad`, …). **A candidate is kept only if it is a directory
  that actually contains `FreeCAD.so`, `FreeCAD.dylib`, or `FreeCAD.py`** — the probe tests for the
  module, not for a plausible path.
- It reads the version in an **isolated probe process** (`python -I -B -c …`) with a constructed
  environment, a fresh temporary `HOME`, `cwd` inside that temporary directory, a fixed
  `PATH=/usr/local/bin:/usr/bin:/bin`, no inherited `PYTHONPATH`, and a 30-second timeout. The
  discovered import roots travel in one dedicated variable, `KALA_FREECAD_PROBE_ROOTS`, because an
  isolated interpreter ignores `PYTHONPATH` on purpose. The FreeCAD banner (`freecadcmd -v`) is a
  fallback, not the primary path.
- The result is a frozen `FreeCADAvailability` record with bounded string and tuple fields, carrying
  `available`, `interpreter_path`, `executable_path`, `import_roots`, `version`, `version_parts`,
  `calculix_path`, and a bounded `detail`.

Nothing in discovery reaches the network, and nothing in discovery loads FreeCAD into the calling
process.

**D3 — The seam is exactly three kernel types plus one value.**

The adapter does not reimplement dispatch, isolation, authorization, or artifact verification. It uses:

| Seam | Kernel type | What the adapter supplies |
|---|---|---|
| Execution | `SubprocessToolHandler` | `entry_point = "kala_agent_freecad.worker:invoke_geometry"` / `…:invoke_fem"`, with default `interpreter_args = ("-B", "-s")`. `invoke()` raises by design: only the executor dispatches a subprocess handler. |
| Admission | `ToolRegistry.register(descriptor, handler, input_schema=…, output_schema=…)` | Frozen `ToolDescriptor` records with pre-computed canonical schema digests; the registry deep-freezes both schemas, recomputes the digests, and raises on mismatch. |
| Loader visibility | `ToolExecutor(..., import_roots=(…))` | The FreeCAD import root plus the adapter's own package root. `_normalize_import_roots` refuses a root that is not an existing directory, and `_child_import_roots` appends the kernel package root and the interpreter's `stdlib`/`platstdlib`/`purelib`/`platlib`. |
| Budget and isolation | `cad_profile()` → `SandboxProfile` | Larger artifact/time/worker budgets, `profile_name = "freecad-cad"`, and `isolation_profile = IsolationProfile.STRICT_LINUX` by default. |

**D4 — No module in the adapter imports FreeCAD at module scope. Every import happens at call time
inside `freecad_session`, in a fixed order that must not change.**

`FREECAD_IMPORT_ORDER = ("FreeCAD", "Part", "ObjectsFem", "femtools.ccxtools")`. The first three are
required; `femtools.ccxtools` is optional. The order exists because the workbench modules bind to the
application library that only `FreeCAD` initialises, and importing one first can abort the process
inside the native library where no Python-level `try`/`except` can help. `femtools.ccxtools` failing is
recorded as a bounded `solver_bridge_detail` and yields `solver_bridge_available == False`; it is
raised only when the caller explicitly asked for a solve it cannot perform.

`freecad_session` additionally:

- refuses to start when no FreeCAD import root is reachable, with `FreeCADDependencyError`, which the
  worker reports as `sandbox_unavailable`;
- inserts only the roots it introduced into `sys.path` and removes exactly those on exit;
- redirects native file descriptor 1 to stderr for the duration of every FreeCAD call
  (`native_output_to_stderr`), because FreeCAD writes to fd 1 from native code and would otherwise
  corrupt the response envelope;
- closes every document the call opened, on both the success and the failure path.

**D5 — The FEM tool always builds a real analysis document, and the solver runs only behind a safely
discovered CalculiX executable.**

`cad.freecad-fem` constructs, in one FreeCAD document, a `Part::Feature` solid, a
`ObjectsFem.makeAnalysis` analysis container, a solid material card via
`makeMaterialSolid`, a fixed constraint via `makeConstraintFixed` referencing the requested face, a
Gmsh mesh object via `makeMeshGmsh`, and a CalculiX solver object via `makeSolverCalculiXCcxTools` —
then recomputes the document. That work is unconditional and is what the tool publishes.

The solve itself is behind three gates, in order:

1. `run_solver` must be `true`; otherwise the status is `not_requested`.
2. `is_safe_calculix(calculix_path)` must pass: the path was found by `resolve_calculix` on the fixed
   name list `("ccx", "CalculiX", "ccx_2.22", "ccx_2.21", "ccx_2.20", "ccx_2.19")`; the basename must
   match a bounded pattern with no path separator; it must be a regular, non-empty file of at most
   64 MiB carrying an execute bit. **A caller can never supply the path** — the worker resolves it
   itself.
3. `femtools.ccxtools` must have imported.

If any gate fails, the reported status is `solver_unavailable`, `executed` is `false`, `executable` is
`null` unless a *safe* binary was found, and the `results` block reports `available: false`,
`max_displacement_mm: null`, `max_stress_mpa: null`, `node_count: 0`. **No displacement, no stress, and
no pass/fail verdict is ever synthesised.** The five status values are a closed set:
`not_requested`, `solver_unavailable`, `prerequisites_unmet`, `completed`, `failed`.

**D6 — Every artifact is confined to the worker workspace, digest-verified, and classified
`design_binary`.**

`cad.artifacts` writes through the kernel's own `resolve_workspace_path` and refuses traversal, a
symlinked artifact directory, and a symlinked destination. Each export is then measured against the
caller's `max_artifact_bytes`, read back in 1 MiB chunks, and hashed with SHA-256; the digest and byte
count reported are those of the bytes actually on disk, and a file that changed while it was measured
is an error. A rejected export is unlinked rather than left as a partial file. Every published artifact
carries `ArtifactClassification.DESIGN_BINARY` explicitly.

That classification matters beyond this adapter: the executor's `_derive_classification` is monotone
toward `design_binary`, and it re-derives the verdict from the bytes regardless of the claim, so a
STEP, IGES, FCStd, or FRD file cannot be downgraded to text by anything the adapter does or says.

**D7 — `cad_profile()` defaults to strict Linux isolation, never widens authority, and fails closed
when the probe is unavailable.**

`cad_profile()` returns a **new** `SandboxProfile` derived from the shipped defaults with
`dataclasses.replace`; the shipped profile is read, never mutated. It raises
`max_output_bytes` to 1 MiB, `max_artifact_bytes` to 256 MiB, `default_timeout_seconds` to 900 s,
`max_workers` to 2, and `max_queue_seconds` to 30 s, and it keeps
`allow_network = allow_egress = allow_privileged_capabilities = False` and
`enforce_child_limits = True`. The only authority-shaped field it changes is `isolation_profile`, which
defaults to `IsolationProfile.STRICT_LINUX` and may be set to `PORTABLE` explicitly.

"Strict" is honest, not aspirational: `ToolExecutor` probes the sandbox, `judge_sandbox` refuses when
the six required controls were not verified, and the executor then **denies** a `local_write`,
`network`, or `egress` tool, or any tool declaring `destructive`, with `sandbox_unavailable`. Both CAD
tools are `local_write`, so under a strict profile on a host that cannot build the jail, **both are
refused and nothing runs.** `cad_report()` exists so an operator can print the enforced control set,
the isolation profile, and the budget without guessing.

**D8 — The adapter carries no authority of its own. `PolicyEngine` and the capability model govern
every call, and the first use of each tool class needs an approval.**

Both tools declare `required_capabilities = ("artifact.write", "design.write", "tool.execute")` — a
subset of the kernel's default grant — and `side_effect_level = local_write`, `destructive = false`,
`idempotent = true`, with descriptor timeouts of 180 s (geometry) and 900 s (FEM). The executor
intersects the descriptor's and the call's capabilities with the grant, refuses a call requesting
anything the descriptor does not declare, and refuses a side-effect mismatch. With a `PolicyEngine`
attached, each call becomes a `TOOL_CLASS_CHANGE` request keyed by `(action_digest, scope)`, where
`action_digest` covers the argument digest, the resolved workspace, the effective capabilities, the
tool and version identity, the descriptor digest, and both schema digests. The two CAD tools are two
distinct tool classes and each needs its own explicit first-use approval. **The adapter cannot
self-authorize, cannot grant itself a capability, and cannot mark its own action as human-approved.**

**D9 — Registration is explicit, and the shipped composition root does not do it.**

The public registration surface is small and host-agnostic:

```python
from kala_agent_freecad import (
    cad_profile, discover, register, register_if_available, worker_import_roots,
)
from kala_agent.tools.registry import ToolRegistry
from kala_agent.tools.executor import ToolExecutor

availability = discover()                      # filesystem only; never imports FreeCAD
registry = ToolRegistry()
register_if_available(registry, availability)   # () on a host without FreeCAD

executor = ToolExecutor(
    registry,
    run_directory=run_directory,
    granted_capabilities=("artifact.write", "design.write", "tool.execute"),
    profile=cad_profile(),                     # strict Linux; fails closed if unprobeable
    policy=policy_engine,                      # required for the TOOL_CLASS_CHANGE checkpoint
    import_roots=worker_import_roots(availability),   # FreeCAD lib root + this adapter's src root
)
```

`worker_import_roots` is what makes the call possible at all: the worker is started with `python -I`,
which ignores the host's import settings, so a worker that could not see FreeCAD on the parent
`sys.path` would fail with `sandbox_unavailable`. It returns the discovered FreeCAD roots plus the
adapter's own package root, collapsing non-directories and duplicates.

`register()` refuses a registry that is not a `ToolRegistry`, refuses an availability record of the
wrong type, and returns `()` when the host has no usable FreeCAD. `register_if_available()` is the
same thing without the type checks, for embedding code that wants a no-op on a host without FreeCAD.

**KalaApplication does not perform any of this.** The core composition root does not import
`kala_agent_freecad`, does not call `discover()`, does not register either tool, and does not dispatch
them. `kala-agent init` does not list them under `tools`, no HTTP route exposes them, and no CLI
command invokes them. **An embedder that wants CAD must install the distribution and register it
explicitly.** That is a deliberate property, not an oversight: automatic discovery at startup would
make an optional system dependency change the product's tool catalog, its approval surface, and its
isolation profile based on what happens to be installed on the machine.

**D10 — Honest solver semantics are part of the contract, not a runtime detail.**

The output schema admits exactly five solver statuses, and `cad.freecad-fem`'s own description states
that without a safely discovered CalculiX executable the call reports `solver_unavailable` and no
result is invented. The result block is a `Fem::FemResultObjectPython` lookup: if no such object exists
in the document, `available` is `false` with a message saying so, and displacement and stress maxima
are computed only from a real result object's `DisplacementLengths` and `vonMises` arrays, rounded to
six digits, with non-finite and non-numeric entries rejected rather than coerced.

`cad.freecad-geometry` is a geometry and export tool, not an analysis tool. It builds one primitive
solid (`box` or `cylinder`) with bounded millimetre dimensions, optionally measures it
(`area_mm2`, `volume_mm3`, `bounding_box_mm`, `edge_count`, `face_count`, `vertex_count`, `solid`), and
writes the requested `step`, `iges`, and `fcstd` exports into the workspace. `measure: false` reports
zeros rather than omitting the block, so the output shape never varies with input.

## Consequences

### Positive

- The core gains a real engineering integration without gaining a dependency. `pyproject.toml` still
  declares only `fastapi` and `pydantic`; a host with no FreeCAD is byte-for-byte unaffected, and the
  adapter's own tests pass with FreeCAD uninstalled.
- The seam is proven to be small. Four kernel types are the whole integration surface, and the adapter
  is 13 modules. Everything that matters — dispatch, isolation, capability intersection, the
  `TOOL_CLASS_CHANGE` checkpoint, workspace confinement, artifact digest verification, fail-closed
  classification — is the kernel's existing code, exercised by the kernel's existing 158 security
  tests, not reimplemented per adapter.
- **Import order is a verified, tested property rather than a convention.** A subprocess asserts the
  observed order, and the code states why reordering can abort the process natively.
- A native library that writes to fd 1 can no longer corrupt the protocol channel, and that is tested
  on a real FreeCAD, not simulated.
- The solver is fail-closed in the direction that matters: the adapter will decline to produce a
  result rather than produce a wrong one, and a caller cannot inject the executable path.
- The budget mismatch between "engineering work" and "the shipped default" is resolved by an explicit,
  named profile rather than by a special case inside the executor.
- The adapter suite is at **100% branch coverage** across 301 tests, including 284 that need no
  FreeCAD installation, so the optional-dependency path is as well tested as the present one.

### Negative

- **CAD is not available out of the box.** The single largest cost of D9. An operator must install
  the distribution, construct an executor, register the tools, attach a `PolicyEngine`, grant
  capabilities, and choose an isolation profile. Nothing in the CLI or the HTTP API does this for
  them, and `docs/architecture-report.md` §3.2 and §3.10 record it as a gap rather than hiding it.
- **FreeCAD is a large, heavy, host-specific dependency.** It pulls Qt, VTK, OpenCASCADE, Coin, MPI,
  and PySide6. The adapter's own footprint is trivial; the *system* dependency is not, and it must be
  installed and upgraded by the operator outside this repository's control.
- **The lazy-import discipline is invisible from the outside and easy to break.** One module-scope
  `import FreeCAD` in a future file would break every host without FreeCAD, and the failure would look
  like a core import error. The AST boundary test catches the core direction; the adapter's own
  laziness is covered by tests and by the docstrings, not by a static rule.
- **The FEM tool is honest and therefore disappointing on a machine without CalculiX.** It returns
  `solver_unavailable` with `results.available = false`. A caller that wanted numbers must install a
  solver; the adapter will not approximate them. This is correct and it is also a poor product
  experience compared to inventing plausible figures.
- **`femtools.ccxtools` is not guaranteed importable even when FreeCAD is.** On the verification host
  it is absent because `PySide6` is not importable in the test interpreter, so the bridge reports
  `solver_bridge_available == False` with a detail string. Geometry and analysis construction still
  work; only the solve is affected.
- **Strict isolation is a real obstacle on a developer machine.** Because the CAD profile defaults to
  strict Linux, a host whose probe fails refuses both CAD tools outright. On the verification host the
  probe is unavailable when the executor computes it, so functional CAD tests deliberately run the
  portable profile and the strict profile is verified as *enforced or refused*. This is the fail-closed
  behaviour working, and it is also a rough edge.
- **Two distinct tool classes means two distinct first-use approvals** before either tool can run
  under a `PolicyEngine`. Correct, and an extra interaction.

### Neutral

- The tools are `local_write`, `idempotent`, and non-destructive, and their required capabilities are a
  subset of the kernel's default grant. Whether that is the right authority for a CAD kernel is a
  deployment judgement, not a change to this decision.
- The adapter declares `depends on kala-agent`, so the dependency direction is adapter → kernel. The
  kernel can never reach the adapter, which is what makes D9's "explicit registration" sustainable.
- `cad.freecad-fem` builds a Gmsh *mesh object*. It does not run Gmsh. The object is part of a real
  analysis document; `mesh.generated` is `false` and `node_count`/`element_count` are `0` until a
  mesher actually runs, and this report never claims otherwise.
- Artifact media types are declared constants (`application/x-free-cad`, `model/step`, `model/iges`,
  `text/plain` for FRD), not negotiated or sniffed. The kernel's own monotone classification is the
  authority, not these strings.
- The FreeCAD version and import root are reported in every result. That is diagnostic provenance, not
  a dependency lock; nothing pins a FreeCAD version.
- Timeouts live in both places on purpose: the descriptor carries 180 s / 900 s, and
  `cad_profile()` carries a 900 s default. `SandboxProfile.timeout_for` takes the smaller.

## Alternatives Considered

**Import FreeCAD from the core composition root, with an optional extra.**

- Rejected. It makes the product's tool catalog, approval surface, and isolation profile depend on
  what is installed on the machine, which is the opposite of ADR-0001's local-authority posture and
  ADR-0002's field independence. It also converts a missing optional system dependency into a core
  import error, and puts a 300 MB native dependency behind a `pyproject.toml` that currently declares
  two small pure-Python packages.

**A plugin or entry-point discovery mechanism (`importlib.metadata.entry_points`).**

- Deferred, and this is the natural next step. Entry points would let a host list installable adapters
  without the core naming any of them, which is the right direction. It was not chosen now because
  it makes a third-party package able to change the kernel's tool catalog merely by being installed —
  which is precisely the "installed ≠ trusted" problem ADR-0006 leaves open (D3, staged installation
  and quarantine, unimplemented). Explicit `register_if_available(registry, availability)` keeps the
  decision with the embedding code until signature verification and a quarantine store exist.

**Shell out to `freecadcmd` with a generated script, and parse its output.**

- Rejected as the primary mechanism. It avoids the import-root problem entirely, which is a real
  advantage, and it is a reasonable fallback for an environment where the interpreter cannot import
  the module. It was rejected because stdout is the tool protocol channel and FreeCAD's native output
  goes to fd 1; parsing a mixed native-and-Python stdout is exactly the fragility that
  `native_output_to_stderr` removes. `freecadcmd -v` **is** used, but only as a version-probe fallback
  inside an isolated subprocess with a constructed environment.

**Run FreeCAD in a long-lived daemon or an in-kernel service.**

- Rejected. A persistent FreeCAD process accumulates documents, module state, and native handles
  across calls, which makes per-call cleanup, per-call timeouts, and per-call authority impossible to
  guarantee. ADR-0005's short-lived worker exists for this reason. Every FreeCAD import in the adapter
  is call-scoped, and every document the call opened is closed on the way out.

**Bundle a CalculiX binary or a Gmsh build with the distribution.**

- Rejected for now, and it is a licensing and supply-chain question rather than a technical one.
  Shipping a solver means shipping its licence, its provenance, and its binary verification story —
  all open items in ADR-0006. Discovering a solver on the host and refusing anything unsafe is the
  conservative interim position. It is a Revisit Trigger, not a settled decision.

**Compute plausible FEM results when no solver is present (an analytic stiffness estimate, a
closed-form beam deflection, an interpolation table).**

- **Rejected outright.** This is the same class of error as the surrogate evaluators in §3.1, but worse:
  those results carry `NON_AUTHORITATIVE_FINDING` and are labelled `illustrative-constant-not-certified-material-data`
  everywhere they appear. A number emitted from a FEM tool's `results` block would be read as a
  solver result. `solver_unavailable` with `available: false` and null extrema is the only honest
  answer, and it is what the tool returns.

**Deprecate or pin a FreeCAD PyPI wheel.**

- Rejected as a matter of fact. No such wheel is a supported FreeCAD distribution, the build is not
  reproducible, and pulling one would put LGPL native code into the Python dependency graph where the
  three quality gates and `pip-audit` (not run; see verification §7) would have nothing to inspect.
  FreeCAD is discovered on the host. It is never installed from an index.

**Build a full CAE adapter set (CAD, CAE, BIM, building-code) as one component.**

- Rejected as premature. One narrow integration, honestly bounded, teaches more about the seam than a
  broad one teaches at once, and the seam cost is now known: 13 modules, four kernel types, one named
  profile. The next adapter should be cheaper because of this one.

## Trade-offs

- We accept that CAD is **unavailable without explicit integration work** in exchange for a core that
  has no optional native dependency, no host-dependent tool catalog, and no host-dependent isolation
  profile. The operator's cost is a few lines of embedding code; the kernel's cost is zero.
- We accept **a larger artifact and time budget** (256 MiB, 900 s, 2 workers) in exchange for CAD work
  that is not artificially throttled. The profile still grants no network, no egress, and no privileged
  capability, and the budget is a ceiling, not an entitlement.
- We accept **strict Linux by default and a hard refusal when the probe fails** in exchange for never
  running a CAD tool in a jail the host could not build. This makes CAD unusable on some developer
  machines; the alternative is running it with the host filesystem visible, which is the trade the
  kernel already declines to make.
- We accept **declining to produce a result** in exchange for never producing a fabricated one. The FEM
  tool's most common outcome on a laptop is `solver_unavailable`, and that is the correct outcome.
- We accept **lazy imports plus a call-scoped session** in exchange for a per-call import cost and a
  per-call native-library teardown. Correctness and cleanup beat a warm cache.
- We accept **discovering a solver by name on `PATH` and validating it structurally** in exchange for
  not pinning, verifying, or distributing solver binaries. We accept that this is a weaker guarantee
  than a signature and name it as such.
- We accept **a separate test suite, a separate coverage configuration, and a separate lint/type run**
  in exchange for the adapter not being governed by the core's gates. Two coverage floors means two
  numbers to read; it also means an adapter regression cannot be hidden by core coverage.

## Revisit Triggers

- A second adapter appears (a different CAE kernel, a building-code checker, a mesher). That is the
  point at which `importlib.metadata.entry_points` discovery plus ADR-0006's D3 quarantine becomes
  worth its cost.
- ADR-0006's D3 (staged installation, quarantine) or its D1 signature verification is implemented.
  Until one of them is, automatic discovery of installed adapters is a supply-chain risk, not a
  convenience.
- FreeCAD is installed alongside a real CalculiX and Gmsh on the verification host. That would let the
  `completed`, `prerequisites_unmet`, and `failed` solver branches be verified against a real solve
  rather than only against their refusal and skip paths, and it would make `mesh.generated` and a
  non-null `results` block observable.
- The strict sandbox probe becomes verifiable through the executor's own `python_executable` default on
  the verification host. Today the probe succeeds when it is given the venv interpreter and fails when
  the executor resolves `sys.executable` to the system interpreter, because the probe entry point must
  be able to import `kala_agent` before it isolates. That is worth fixing, and it is a kernel concern
  rather than an adapter one.
- The composition root gains a first-class embedding/extension surface. If it does, the natural place
  for `register_if_available` is that surface, and D9's "explicit" should become "explicit through the
  product" rather than "explicit in the embedder's own code".
- A CAD/CAE tool needs a capability the current model cannot express — a GPU, a specific hardware
  device, or a licensed seat. That is an ADR-0005 revisit, triggered by this adapter.
- The kernel's reference workflow is rewired through `ToolExecutor` (open decision 12 in
  `docs/architecture-report.md` §21). At that point the adapter is no longer the only out-of-process
  caller, and the isolation questions get answered once instead of twice.
- FreeCAD's LGPL obligations become a real obligation rather than a package-manager fact — for example
  if the adapter were ever vendored rather than discovered.

## Verification

Status as of 2026-09-26. Full commands and results are in `docs/verification-report.md` §2.7 and
§4.14. The adapter suite is `adapters/freecad/tests`, run from `adapters/freecad`.

| # | Claim | Evidence | Result |
|---|---|---|---|
| D1 | The adapter is a separate distribution, and the core never imports FreeCAD or the adapter | `tests/unit/test_adapter_boundaries.py` (2 tests): an AST walk of every `src/kala_agent/**/*.py` rejecting `FreeCAD`, `femtools`, `kala_agent_freecad`; and an AST walk of every adapter file rejecting `kala_agent.{application,api,cli,domains,kernel,persistence,runtime}`. `adapters/freecad/pyproject.toml` declares `name = "kala-agent-freecad"` and never lists FreeCAD | **Met.** All 52 core modules import none of the three forbidden roots. The core's runtime dependencies are still `fastapi` and `pydantic` only. |
| D2 | FreeCAD is a discovered system dependency, not a wheel; discovery never imports it and never reaches the network | `test_availability.py` (31), `test_availability_records.py` (28) | **Met.** Discovery is a fixed name list, a fixed prefix list, a fixed relative-layout table, and a module-marker test; the version probe runs `python -I -B -c` in a temporary `HOME` with a constructed environment and a 30 s timeout; `FreeCADAvailability` refuses unbounded fields in `__post_init__`. |
| D3 | The seam is `SubprocessToolHandler` + `ToolRegistry` + `ToolExecutor(import_roots=…)` + `SandboxProfile` | `test_registration.py` (17), `test_registration_roots.py` (2), `test_contracts.py` (44), `test_executor_live.py` (5) | **Met.** Registration refuses a non-`ToolRegistry` and a wrong-typed availability; a non-directory import root is dropped and a duplicate root is collapsed; `import_roots` that is not a directory is refused by the executor. |
| D3 | With those four types, a real CAD call succeeds out of process and the kernel verifies what FreeCAD wrote | `test_executor_live.py::test_both_tools_register_and_dispatch` | **Met on a host with FreeCAD 1.1.3.** The geometry call succeeds with three artifacts; each artifact's bytes re-hash to the reported digest and every one is classified `design_binary`. The FEM call succeeds with `fcstd` and `step` artifacts and `objects.analysis == "KalaAnalysis"`. |
| D4 | FreeCAD is never imported at module scope, and the import order is fixed and observed | `test_cad_live.py::test_freecad_is_imported_before_its_workbenches` runs a real `python -I -B` subprocess that calls `freecad_session()` and writes the observed order; `test_cad_live.py::test_the_session_reports_the_fem_modules_it_loaded`; `test_cad_runtime_loader.py` (16) | **Met.** The observed order begins `FreeCAD`, `Part`, `ObjectsFem`, matching `FREECAD_IMPORT_ORDER[:3]`, and the session reports a real version and the root it loaded from. `FREECAD_IMPORT_ORDER` is published from the package root. |
| D4 | Native fd-1 output cannot corrupt the protocol channel | `test_cad_runtime_loader.py` plus every live call, which run real FreeCAD that writes a native banner | **Met.** `native_output_to_stderr` redirects fd 1 around every FreeCAD call and restores it afterwards; no live test has ever seen a corrupted envelope. |
| D4 | Documents are closed after every call, success or failure | `test_cad_live.py::test_documents_are_closed_after_every_call` | **Met.** After a geometry call and an FEM call in the same process, `FreeCAD.listDocuments()` is empty. |
| D5 | The FEM tool builds a real analysis document: analysis, material, fixed constraint, mesh, solver | `test_cad_live.py::test_fem_builds_an_analysis_without_a_solver` validates the result against `FEM_OUTPUT_SCHEMA` and asserts all five object names are non-empty | **Met on FreeCAD 1.1.3.** `objects` carries `analysis`, `constraint`, `material`, `mesh`, and `solver`; the material card reports the requested `youngs_modulus_mpa`, `poisson_ratio`, and `density_kg_m3`; `analysis_name == "KalaAnalysis"`. |
| D5 | With no solver, the tool returns `solver_unavailable` and no fabricated result | `test_cad_live.py::test_fem_reports_a_missing_solver_without_inventing_results`; `test_cad_fem_fakes.py` (32) for the branch logic | **Met on this host.** `status == "solver_unavailable"`, `executed is False`, `available is False`, `executable is None`, the message says no safe CalculiX executable was discovered, `max_displacement_mm` and `max_stress_mpa` are `null`, `results.node_count == 0`, and `mesh.generated is False`. The test skips itself on a host that does have a solver, so the assertion cannot silently become vacuous. |
| D5 | A caller cannot supply the solver path | `test_cad_fem_fakes.py`; `worker.py:invoke_fem` calls `resolve_calculix()` itself and never reads a path from `request.arguments`; `is_safe_calculix` enforces the bounded name pattern, no separator, regular file, non-empty, ≤ 64 MiB, execute bit | **Met.** No FEM input schema field carries a path. |
| D5 | `run_solver: false` is reported as a clean skip, not as a failure | `test_cad_live.py::test_fem_without_a_solve_reports_a_clean_skip` | **Met.** `status == "not_requested"`, `executed is False`, message `the caller did not ask for a solve`. |
| D6 | Every write is confined to the workspace | `test_cad_live.py::test_geometry_confines_every_write_to_the_workspace`; `test_cad_artifacts_limits.py` (5) | **Met.** A sibling path outside the workspace does not exist after a call, every written path is workspace-relative with no `..` component, and the only files written are the three declared exports. |
| D6 | Every artifact is digest-verified, budget-bounded, and `design_binary` | `test_cad_live.py` re-hashes every artifact and asserts `ArtifactClassification.DESIGN_BINARY`; `test_cad_artifacts_limits.py` drives the size ceiling | **Met.** With a 1 KiB artifact budget the call fails with an output-too-large or artifact-rejected code and publishes nothing. |
| D7 | `cad_profile()` derives from the shipped defaults, grants no authority, and does not mutate the shipped profile | `test_policy.py` (9) | **Met.** `profile_name == "freecad-cad"`, budgets exceed the shipped ones, `allow_network`/`allow_egress`/`allow_privileged_capabilities` are all `False`, `enforce_child_limits is True`, two calls produce equal-but-distinct values, and the shipped `SandboxProfile()` is unchanged (`portable-minimum`, 8 388 608, 30.0, 4, `PORTABLE`). |
| D7 | The strict default is real and fails closed | `test_policy.py::test_strict_profile_refuses_side_effects_when_it_is_unavailable`; `test_executor_live.py::test_strict_profile_is_enforced_or_refused` | **Met as *enforced or refused*, which is the accurate claim.** On the verification host the executor's strict probe is **unavailable**, so the call is `DENIED` with `sandbox_unavailable`; the test asserts exactly that branch, and asserts the `SUCCEEDED` branch plus control containment when a probe is available. A separate observation, run outside the suite with the venv interpreter, reports all six required controls plus five optional ones and no `scratch_tmpfs` — so the host *can* build the jail and the refusal is the executor's interpreter default, not a host limitation. See verification §5.2. |
| D8 | The tools claim a subset of the default capability grant and are refused without it | `test_executor_live.py::test_dispatch_refuses_an_ungranted_capability` | **Met.** With `granted_capabilities=()` the call is `DENIED` with `capability_denied` before dispatch. |
| D8 | Arguments outside the schema are refused | `test_executor_live.py::test_dispatch_refuses_arguments_the_schema_forbids` | **Met.** An extra property is rejected with `invalid_arguments` and no result is returned. |
| D8 | The first use of each tool class requires a `TOOL_CLASS_CHANGE` approval | The kernel's own tests: `tests/security/test_tool_executor.py:504,516,552,618` and `tests/security/test_policy.py:373,406,456` | **Met by the kernel, inherited unchanged.** The adapter supplies no policy code and cannot bypass it; both CAD tools are distinct classes and each re-approves on a changed capability set. |
| D9 | Registration is explicit and no-ops without FreeCAD | `test_registration.py` (17) | **Met.** `register_if_available` returns `()` for an unavailable record; `register` refuses a non-`ToolRegistry` and a wrong-typed availability; `tool_identifiers()` publishes exactly `("cad.freecad-geometry", "1.0.0")` and `("cad.freecad-fem", "1.0.0")`. |
| D9 | The composition root does not load the adapter | `tests/unit/test_adapter_boundaries.py`; absence of any `freecad` reference under `src/`; `kala-agent init` output listing exactly `["mechanical.reference-simulator"]` | **Met.** No HTTP route and no CLI command registers or dispatches either tool. This is stated as a gap in `docs/architecture-report.md` §3.2 and §3.10, not as a feature. |
| D10 | The result block is a real lookup, not a synthesis | `test_cad_fem_fakes.py` (32); `fem.result_summary` | **Met.** A missing `Fem::FemResultObjectPython` object yields `available: false` with `no Fem result object exists in the analysis`; a present one is read for `DisplacementLengths` and `vonMises`, and non-numeric or non-finite entries are rejected rather than coerced. |
| Suite | The adapter is as well tested as the core | `docs/verification-report.md` §2.7 | **Met.** 301 tests, **100.00% branch coverage** (1 152 statements, 306 branches, 0 missed, 0 partial, all 13 files skipped as fully covered), `ruff check` and `ruff format --check` clean, strict `mypy` clean over 30 files. 17 tests are marked `freecad` and 284 run without an installation. |

### What this ADR does **not** claim

- **No certified engineering result.** A successfully executed `cad.freecad-fem` call on this host
  produces a valid FreeCAD analysis document and a `solver_unavailable` status. That is a document, not
  an analysis.
- **No automatic wiring.** The adapter is not on any product path. `docs/architecture-report.md` §3.2
  and the verification report's G3 and G9 record it.
- **No filesystem confinement under the portable profile.** The functional CAD tests run portable, so
  a FreeCAD worker in those tests sees the host filesystem. Under the profile's own strict default they
  would be refused on this host. See verification §5.2 and residual risk RR-19.
- **No mesher or solver is bundled, pinned, or verified.** `makeMeshGmsh` creates a mesh *object*; no
  mesher ran. `makeSolverCalculiXCcxTools` creates a solver *object*; no solver ran.
- **No FreeCAD version lock.** The discovered version is reported, not pinned, and nothing in the
  adapter asserts compatibility with a particular one.

## Implementation notes (2026-09-26)

| Decision | Implementation | Reference |
|---|---|---|
| D1 separate distribution | `adapters/freecad/pyproject.toml`: `name = "kala-agent-freecad"`, `dependencies = ["kala-agent>=0.1", "pydantic>=2.10,<3"]`, and a comment stating FreeCAD is a system dependency discovered on the host and never a wheel. `pythonpath = ["../../src", "src"]` so the suite runs against the uninstalled tree. | `adapters/freecad/pyproject.toml` |
| D1 no core import | The core's runtime dependency list is unchanged. A static AST test enforces the boundary in both directions. | `tests/unit/test_adapter_boundaries.py` |
| D2 discovery | `FREECAD_EXECUTABLE_NAMES`, `CALCULIX_EXECUTABLE_NAMES`, `SEARCH_PREFIXES`, `LIB_ROOT_RELATIVE_CANDIDATES`, `MODULE_MARKERS`, `VERSION_PROBE_TIMEOUT_SECONDS = 30.0`, `PROBE_ROOTS_ENVIRONMENT_KEY = "KALA_FREECAD_PROBE_ROOTS"`, `candidate_lib_roots`, `is_safe_calculix`, `resolve_calculix`, `read_version`, `discover` | `availability.py` |
| D3 execution | `SubprocessToolHandler(descriptor=…, entry_point=…)` for both tools; `ToolRegistry.register(…, input_schema=…, output_schema=…)`; `ToolExecutor(…, import_roots=worker_import_roots(availability))` | `contracts.py:geometry_handler, fem_handler`, `registration.py:register` |
| D3 loader visibility | `ToolExecutor._normalize_import_roots` refuses a non-directory; `_child_import_roots` appends the kernel package root and the interpreter's own `stdlib`/`platstdlib`/`purelib`/`platlib`; `worker_import_roots` prepends the discovered FreeCAD roots and appends `Path(__file__).resolve().parent.parent` | `tools/executor.py:1202,1677`, `registration.py:89` |
| D4 lazy import | `FREECAD_IMPORT_ORDER = ("FreeCAD", "Part", "ObjectsFem", "femtools.ccxtools")`, `REQUIRED_MODULE_ORDER`, `OPTIONAL_MODULE`, `freecad_session`, `_import_solver_bridge`, `native_output_to_stderr`, `close_documents`, `import_roots_from_path` | `cad/runtime.py` |
| D5 honest solver | `parse_request`, `_build_analysis` (`makeAnalysis`, `makeMaterialSolid`, `makeConstraintFixed`, `makeMeshGmsh`, `makeSolverCalculiXCcxTools`), `_run_solver`, `mesh_counts`, `result_summary`, `RESULT_TYPE_ID = "Fem::FemResultObjectPython"`, `SOLVER_STATUSES` | `cad/fem.py` |
| D6 artifact confinement | `resolve_target` (via `resolve_workspace_path`), `prepare_target` (refuses a symlinked directory or destination), `finalize` (stat, budget, digest, re-measure), `digest_artifact` (1 MiB chunks), `discard`, `PublishedArtifact.artifact` with `ArtifactClassification.DESIGN_BINARY` | `cad/artifacts.py` |
| D7 CAD profile | `cad_profile(isolation=IsolationProfile.STRICT_LINUX, probe=None)` via `dataclasses.replace`, keeping `allow_network = allow_egress = allow_privileged_capabilities = False`; `cad_report()` for an operator-facing description | `policy.py` |
| D8 declared authority | `TOOL_CAPABILITIES = ("artifact.write", "design.write", "tool.execute")`, `SideEffectLevel.LOCAL_WRITE`, `GEOMETRY_TIMEOUT_SECONDS = 180.0`, `FEM_TIMEOUT_SECONDS = 900.0`, `CAD_MAX_ARTIFACT_BYTES = 268_435_456`, `CAD_MAX_WORKERS = 2` | `contracts.py` |
| D9 explicit registration | `register`, `register_if_available`, `freecad_import_roots`, `worker_import_roots`, `tool_identifiers`, `REGISTRATIONS` | `registration.py` |
| D10 closed status set | `SOLVER_STATUSES = ("not_requested", "solver_unavailable", "prerequisites_unmet", "completed", "failed")` and the `FEM_OUTPUT_SCHEMA` `solver` and `results` sub-schemas that admit them | `contracts.py` |

Observed on the verification host, quoted verbatim from `discover()` and the package manager:

```text
FreeCAD              1.1.3-2   (pacman; LGPL-2.0-only)
interpreter          /usr/bin/freecadcmd
executable           /usr/bin/freecad
import root          /usr/lib/freecad/lib  (holds FreeCAD.so)
version              1.1.3  parts (1, 1, 3)
detail               "FreeCAD 1.1.3 is available for import"
calculix_path        ""        solver_available = False
worker_import_roots  ('/usr/lib/freecad/lib', '<repo>/adapters/freecad/src')
```

```console
$ python -c "import FreeCAD"
ModuleNotFoundError: No module named 'FreeCAD'          # bare import fails without the discovered root

$ PYTHONPATH=/usr/lib/freecad/lib python -I -B -c "…"
FreeCAD 1.1.3 / Part True / ObjectsFem True
ccxtools unavailable: ModuleNotFoundError No module named 'PySide6'
```

`ccx`, `CalculiX`, and `gmsh` are **not installed** on this host. That is why the FEM tool's observed
behaviour is a real document plus `solver_unavailable`, and why the tests assert the refusal path
rather than a solved result.

## References

- `adapters/freecad/pyproject.toml` — the `kala-agent-freecad` distribution
- `adapters/freecad/src/kala_agent_freecad/availability.py` — discovery, the version probe, and solver safety
- `adapters/freecad/src/kala_agent_freecad/contracts.py` — the two frozen tool contracts, schemas, and digests
- `adapters/freecad/src/kala_agent_freecad/registration.py` — the public registration surface
- `adapters/freecad/src/kala_agent_freecad/policy.py` — `cad_profile` and `cad_report`
- `adapters/freecad/src/kala_agent_freecad/worker.py` — the two entry points the executor dispatches
- `adapters/freecad/src/kala_agent_freecad/cad/runtime.py` — the fixed import order and the call-scoped session
- `adapters/freecad/src/kala_agent_freecad/cad/geometry.py`, `cad/fem.py`, `cad/artifacts.py`, `cad/arguments.py`
- `adapters/freecad/tests/` — 301 tests, 100% branch coverage
- `tests/unit/test_adapter_boundaries.py` — the static no-core-import boundary
- `docs/architecture-report.md` §3.2, §3.6, §3.10, §10.7, §18 — status of the adapter in the product
- `docs/verification-report.md` §2.7, §4.14, §5.2, §6 G3/G9, §7, §8 — commands, measurements, and gaps
- `docs/threat-model.md` §2 A17/A18, §4 TB-7, §5.7, §6.1, §7 — assets, controls, and residual risks
