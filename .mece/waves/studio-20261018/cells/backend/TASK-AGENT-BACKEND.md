# Cell Backend Task: Enrich Local Mechanical Engineering Agent Provider & Tests

## Objective

Enhance the Studio backend local provider (`src/kala_agent/studio/providers/local.py` and `src/kala_agent/studio/providers/dispatch.py`) and its unit test suite (`tests/unit/test_studio_providers.py` and `tests/unit/test_studio_server.py`) so that the built-in offline `local` chat provider acts as a genuinely helpful, deterministic **Mechanical Engineering Agent** for parametric sizing, analytical geometry estimation, and OpenCascade (`STEP` / `STL`) build & CAD/CAE handoff guidance—while preserving every existing security, length, offline, and test invariant.

## Core Requirements

1. **Preserve All Existing `local_chat` Invariants (`src/kala_agent/studio/providers/local.py`)**:
   - `MAX_PROMPT_LENGTH = 4096` and `MAX_RESPONSE_LENGTH = 1024`.
   - Reject non-string, empty, or `> 4096` character prompts with `ProviderProtocolError`.
   - Return `ProviderOutcome(text=..., provider="local", model="deterministic-paraphrase-v1", offline=True)`.
   - **CRITICAL TEST INVARIANT**: `outcome.text` MUST still include `"Local helper"` and `"Reasoning trace id"` (tested by `tests/unit/test_studio_providers.py`: `assert "Local helper" in outcome.text`).
   - Remain 100% deterministic, stdlib-only, and offline (`offline=True`, zero network calls, zero fabricated FEA/CFD/solver numbers).

2. **Add Deterministic Mechanical Engineering & OpenCascade Estimation Guidance in `local_chat`**:
   - Inspect the user prompt deterministically:
     - Extract any numeric dimensions mentioned in the prompt (e.g., `mm`, `cm`, `m`, `radius`, `height`, `size_x`, `x`, `by`, etc.) or detect primitive keywords (`box`, `block`, `bracket`, `plate`, `cylinder`, `shaft`, `pin`, `rod`, `pipe`, `sphere`, `ball`, `bearing`, `step`, `stl`, `estimate`, `volume`, `mass`, `cad`, `solidworks`, `freecad`, `ansys`, `abaqus`).
     - If dimensions are detected (for example `20x30x40` or `radius 10 height 30`), compute deterministic nominal analytical estimates (volume in $\text{mm}^3$ / $\text{cm}^3$ and surface area in $\text{mm}^2$) right in the response so the user gets immediate engineering estimation feedback!
     - Always remind the user how the unified OpenCascade workflow works:
       - The built-in **OpenCascade (OCP)** workbench is active by default for both **quick geometry/envelope estimates & neutral `STEP`/`STL` handoff** to external CAD/CAE suites AND **complete standalone `STEP`/`STL` builds** directly in Kala Studio.
       - Never separate or discriminate between "hobbyists" and "pros" in any copy or notes.
       - Never fabricate FEA/stress/thermal solver numbers.
   - Update the `local` provider `notes` in `src/kala_agent/studio/providers/dispatch.py` so it clearly describes the deterministic offline mechanical engineering & OpenCascade estimation helper.

3. **Unit Tests (`tests/unit/test_studio_providers.py`)**:
   - Keep all existing tests passing.
   - Add unit tests verifying:
     - Deterministic dimension extraction / geometry estimate guidance when a prompt asks to size or estimate a box, cylinder, or sphere.
     - Guidance mentioning OpenCascade (`STEP` / `STL`) for both quick estimation/external CAD handoff and direct builds without any "hobbyist vs pro" separation.
     - Output length remains strictly `<= MAX_RESPONSE_LENGTH` (`1024`) even on long prompts.

## Verification Commands

Run from `/home/v9i/Projects/Kala-Agent`:
1. `.venv-studio/bin/pytest tests/unit/test_studio_providers.py tests/unit/test_studio_server.py tests/unit/test_studio_geometry.py tests/unit/test_studio_security.py`

Write a concise completion summary to `.mece/waves/studio-20261018/cells/backend/REPORT-AGENT-BACKEND.md` when finished.
