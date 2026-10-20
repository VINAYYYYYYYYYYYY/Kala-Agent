# Solid Edge implementation wave — Windows only

## Active objective
USER CORRECTION SUPERSEDES ALL EARLIER SPLIT-HOST TASK BRIEFS: target runtime Windows only, Kala and Solid Edge on the same logged-in Windows desktop. No Linux adapter, no SSH tunnel, no remote Windows host workflow. Development and pure tests may run on Linux. User authorized implementation and subsequently said retry; continue.

First release basic parts and positioned assemblies. Threads, fillets, fusion, true compounds, FEA and an in-SE add-in deferred. Actual Solid Edge Student Edition automation/export remain unverified until a real Windows probe passes.

## Locked behavior
- Verified contract: 87 tests, ruff check/format and strict mypy passed after review.
- Canonical planes XY(u,v,0), normal+Z; XZ(u,0,v), normal-Y; YZ(0,u,v), normal+X.
- Cuts along canonical normal independently of pad reversal; no-op cuts must fail eventual native execution.
- Component-local part metrics vs translated world assembly bounds; no compound/assembly equivalence claim.
- Circle revolve lowered to pad, supplied length else diameter; rectangle around component-localY, rejectXZ.
- Bridge independent of Kala core, lazy Windows imports; local loopback only.
- One pumping STA child process, HTTP supervisor responsive, timeout indeterminate, no mutation replay.
- Narrow import fix makes contracts/Studio importable onWindows without Linux executor. Strict sandbox itself remains unported, not silently weakened.

## Current milestone
A. Shared contract complete and verified.
B. Standalone local Windows bridge/probe, job/HTTP foundation, real SDK evidence, test-backend-only offline validation.
C. Windows-local client/compiler/doctor/script-build/status/cancel. No Linux runtime path.
D. Portable import prerequisite and docs/boundary/CI checks.
Native production build readiness only after actual Windows probe+implemented live native handlers, never a fake backend. Shared LLM extraction and Studio CAD-action wiring remain later gates, no false ready UI.

## Ownership
- contract complete: adapters/solidedge-bridge/{pyproject.toml,src/kala_solidedge_bridge/{__init__,protocol,units,errors}.py,tests/{test_protocol,test_units}.py}. No concurrent edits.
- bridge: rest of adapters/solidedge-bridge; may additionally update bridge pyproject only for CLI/test markers/runtime metadata after foundation completion, preserving existing checks.
- portable: src/kala_agent/__init__.py, src/kala_agent/api/__init__.py, src/kala_agent/api/{envelope,schemas,middleware}.py, tests/unit/test_portable_imports.py. Expansion approved after middleware->schemas->application chain verified; extract portable error envelope, retain view-model construction unchanged.
- windows: adapters/solidedge/** only, Windows-local target.
- docs delivered: docs/solidedge.md,docs/construction-semantics.md,docs/adr/0008-solid-edge-windows-bridge.md. Final command-status refresh after code.
- qa later: tests/unit/test_solidedge_boundaries.py,.github/workflows/solidedge.yml.
- api research done: cells/api/REPORT.md, source-derived unverified installed type library.
- conductor coordination artifacts under this wave only, no implementation edits.
- Former linux task cancelled/interrupted before any adapters/solidedge files existed; do not resume it.

## Model policy
Latest explicit instruction: GPT-6.1-Sol remains conductor/reviewer. All new implementation and repair tasks use ONLY OpenCode Go opencode-go/deepseek-v4.1-flash, variant max. No free-model or alternate-provider fallback. Track T3 task IDs and preserve active work; up to three disjoint workers.

## Baseline
User untracked Rust+buid123/,docs/ui-screens-wireframes.md,rust-implementation.md preserved. No AGENTS found. No commit,push,deploy or credential changes.

## Acceptance
Actual pytest/ruff/format/strictmypy checks; import-no-Linux-sandbox tests; Windows CI smoke defined but not claimed run remotely; docs commands match implemented CLI. No real Windows/Student support claim from Linux testing. Conductor inspects code, verifies and requests focused OpenCode repair as needed.

