# Backend cell — REPORT-AGENT-BACKEND

## Task

Enrich the Studio offline `local` chat provider into a deterministic
Mechanical Engineering Agent (parametric sizing, analytic geometry
estimation, OpenCascade `STEP`/`STL` handoff & build guidance) while
preserving every existing security, length, offline, and test invariant.

## Files changed

```
src/kala_agent/studio/providers/local.py
src/kala_agent/studio/providers/dispatch.py
tests/unit/test_studio_providers.py
```

No files outside the allowed set were touched. `REPORT.md`, other cells,
the core API, the FreeCAD adapter, and packaging were left untouched.

## What changed

### `src/kala_agent/studio/providers/local.py`

- **Invariants preserved**: `MAX_PROMPT_LENGTH = 4096`,
  `MAX_RESPONSE_LENGTH = 1024`; non-string and `> 4096` prompts still raise
  `ProviderProtocolError`; empty prompts still return a graceful reply;
  `outcome.offline is True`; response still contains `Local helper` and
  `Reasoning trace id`; stdlib-only (`hashlib`, `math`, `re`), deterministic,
  zero network, zero fabricated solver numbers.
- **Default model** renamed to `DEFAULT_LOCAL_MODEL =
  "deterministic-paraphrase-v1"`, as requested by the task. A caller-supplied
  `model` still wins (dispatch passes the requested id through unchanged).
- **Deterministic engineering guidance** assembled in a fixed order so the
  required substrings and the OpenCascade reminder survive truncation:
  1. `_reasoning_hint` (contains `Local helper` + `Reasoning trace id`).
  2. `_estimate` — nominal analytic geometry estimate, emitted only when
     dimensions are detected.
  3. `_opencascade_note` — the unified OpenCascade workflow reminder.
  4. `_first_sentences` paraphrase of the prompt.
  The joined body is sliced to `MAX_RESPONSE_LENGTH`.
- **Dimension extraction** (all deterministic, regex based):
  - explicit triples separated by `x`, `×` (matched via `\u00d7`), `*`, or
    `by` — e.g. `20x30x40`, `20 by 30 by 40`;
  - labelled values: `size_x/size_y/size_z`, `x/y/z`, `radius/r`,
    `height/h`, accepting `=`, `:` or whitespace;
  - unit suffixes `mm`, `cm`, `m` converted to millimetres (`cm` ×10,
    `m` ×1000), so `2x3x4 cm` is normalised to `20x30x40 mm`;
  - primitive keywords: `box/block/bracket/plate`, `cylinder/shaft/pin/rod/pipe`,
    `sphere/ball/bearing`.
- **Analytic estimates** (geometry only; no solver quantities):
  - box: `V = x·y·z`, `A = 2(xy+yz+zx)`;
  - cylinder: `V = πr²h`, `A = 2πr(r+h)`;
  - sphere: `V = 4/3·πr³`, `A = 4πr²`.
  Volume is printed in `mm^3` and `cm^3`; surface area in `mm^2`. Numbers are
  formatted deterministically with scientific fallback for extreme
  magnitudes so the response stays bounded.
- **OpenCascade note** always states the OCP workbench is active by default
  for quick geometry/envelope estimates, neutral `STEP`/`STL` handoff to
  external CAD/CAE, and complete standalone `STEP`/`STL` builds in Kala
  Studio. If a named suite appears (`SOLIDWORKS`, `Solid Edge`, `AutoCAD`,
  `Fusion 360`, `FreeCAD`, `OpenSCAD`, `ANSYS Mechanical`, `Abaqus`) it adds
  a neutral-import reminder and states the studio does not drive a native
  vendor API. The copy uses no tier wording — it never mentions "hobbyist"
  or "pro"/"professional".

### `src/kala_agent/studio/providers/dispatch.py`

- The `local` provider `notes` now describe the deterministic offline
  mechanical-engineering helper: dimension extraction, nominal analytic
  volume/surface-area estimates, and the unified OpenCascade (`STEP`/`STL`)
  estimate, neutral CAD/CAE handoff, and standalone build workflow; plus
  "opens no network connection and runs no solver". Row id/name and
  `configured=True` are unchanged.

### `tests/unit/test_studio_providers.py`

Added 10 tests (29 total, up from 19); all pre-existing tests are unchanged
and still passing:

- default model is `deterministic-paraphrase-v1`;
- box estimate is analytic, exact and deterministic (`24000 mm^3`,
  `24 cm^3`, `5200 mm^2`);
- cylinder estimate uses `radius`/`height` (`9424.778 mm^3`, `2513.274 mm^2`);
- sphere estimate uses `radius` (`4188.79 mm^3`, `1256.637 mm^2`);
- centimetre dimensions convert to millimetres;
- no estimate is invented when the prompt carries no dimensions;
- unified OpenCascade `STEP`/`STL` handoff + standalone guidance is present
  for box/cylinder/sphere prompts;
- no "hobbyist"/"pros"/"professional" separation wording;
- length `<= MAX_RESPONSE_LENGTH` on a long prompt, with `Local helper`,
  `Reasoning trace id`, and the OpenCascade note still present;
- the `local` provider record notes describe the engineering helper.

## Verification

**Note on the task's exact command.** The literal command
`.venv-studio/bin/pytest tests/unit/test_studio_providers.py
tests/unit/test_studio_server.py tests/unit/test_studio_geometry.py
tests/unit/test_studio_security.py` could not run as written: `.venv-studio`
does not exist in this checkout, and the security suite lives at
`tests/security/test_studio_security.py`, not `tests/unit/`. These files are
outside the permitted edit set, so no venv/symlink/test copy was created.
The equivalent command against the project's actual venv and suite path was
run instead.

```
.venv/bin/pytest tests/unit/test_studio_providers.py \
  tests/unit/test_studio_server.py tests/unit/test_studio_geometry.py \
  tests/security/test_studio_security.py -q --no-header
```

Result: **107 passed, 1 warning** (exit 0). Log:
`/tmp/kala-agent-backend-verify.log`. The single warning is FastAPI's
`starlette.testclient` httpx deprecation notice emitted by the framework,
not by studio code. Baseline before this change was 97 passed, so all
+10 tests are new and green.

Broader Studio regression run (all `test_studio*.py` unit files plus
`tests/security/test_studio_security.py`):

```
.venv/bin/pytest tests/unit/test_studio_providers.py \
  tests/unit/test_studio_providers_http.py tests/unit/test_studio_server.py \
  tests/unit/test_studio_geometry.py tests/unit/test_studio_registry.py \
  tests/unit/test_studio_step.py tests/unit/test_studio_errors.py \
  tests/unit/test_studio_static.py tests/security/test_studio_security.py \
  -q --no-header
```

Result: **178 passed, 1 warning**.

Lint / format / types on the touched files:

```
.venv/bin/ruff check src/kala_agent/studio/providers/local.py \
  src/kala_agent/studio/providers/dispatch.py tests/unit/test_studio_providers.py
.venv/bin/ruff format --check <same files>
.venv/bin/mypy src/kala_agent/studio --follow-imports=silent
```

Result: `All checks passed!`, `3 files already formatted`, and
`Success: no issues found in 23 source files`.

## Invariants honoured

- Offline / deterministic / stdlib-only: verified by the network-blocking
  `_forbid_network` fixture in `test_studio_providers.py` and by the absence
  of any non-stdlib import.
- No fabricated FEA/CFD/stress/thermal numbers: the helper emits only
  analytic geometry (volume/surface area) and explicitly states solver
  numbers are never fabricated.
- `Local helper` and `Reasoning trace id` remain mandatory substrings; they
  are emitted first so truncation can never drop them.
- `MAX_PROMPT_LENGTH = 4096`, `MAX_RESPONSE_LENGTH = 1024`, non-string and
  oversize rejection, offline `ChatOutcome`, provider id `local` — all
  preserved and covered.
- No "hobbyist vs pro" separation in any copy.

## Limitations

- The estimate is a nominal analytic envelope only; it does not model
  fillets, bores, thread, tolerance stack-ups, or mass (mass would require a
  material density, which is not assumed).
- `bearing` maps to the sphere estimate group and `bracket`/`plate` to the
  box group, per the task's grouping; these are sizing envelopes, not true
  part topology.
- The literal verification command in the task cannot execute in this
  checkout (missing `.venv-studio`, security suite path under `tests/security/`);
  the equivalent run above is the evidence.
