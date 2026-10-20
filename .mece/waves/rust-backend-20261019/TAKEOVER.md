# Conductor takeover: Opus (T3 thread "Build an AI With Stitch Elements"), 2026-10-19

Why: the user asked for a handover. The previous conductor thread (444779f8…, "Integrate Solid Edge with Windows") fails to start any run on the antigravity provider (runs 71–73 failed before start).

## Binding user decisions (supersede WAVE.md where they conflict)
- Rust = connections and internal APIs ONLY. Geometry and STEP are done by the CAD kernel (OpenCascade via the existing Python/OCP code as a supervised geometry worker; Solid Edge via the ADR-0008 bridge on Windows). Spec: `docs/adr/0010-rust-control-plane-cad-kernel-geometry-worker.md`.
- cad-r1 (OCCT C++ FFI port) was INTERRUPTED by Opus. `backend/crates/kala-cad` is kept as-is (parked). `kala-geometry` and `kala-step` are parked: kept, not on a default path. No deletions without an explicit user request.
- The delivery rule "no Python/OCP in the native distribution" is replaced: the package ships native kala-studio, a frozen geometry worker, and studio_dist.
- Workers: OpenCode via T3 only (`opencode-go/deepseek-v4.1-flash` max and `opencode-go/minimax-m3.1-flash-free` max), at most 3 per model across both waves. Opus conducts and reviews. No commit, push, or deploy.

## State inherited (from SYNTHESIS.md)
Accepted: kala-json, contracts, persistence, repositories, budget, artifacts, process, domains, studio (original 9-route slice), workspace root. These are all in scope under ADR-0010.
- kernel-r1: running (policy/workflow/planning). Stays in scope. Its completion notifies the dead parent, so Opus watches its REPORT and thread directly.
- delivery-r3: finished, unreviewed (REVIEW-2 objections: unmanaged destination deletion during staging; the `--dir` then `--dir=false` guard bypass). Folded into r3-delivery-r4.

## New plan
1. r1-worker-mode (Python): `--worker` mode plus `/internal/v1/*` routes per ADR-0010.
2. r2-studio-proxy (Rust, after r1): kala-studio supervises the worker via kala-process, proxies the geometry/design/journal routes, adds chat-with-design, and closes the 30-mismatch live differential.
3. r3-delivery-r4 (after r2): package native + frozen worker; fix the delivery REVIEW-2 objections.
4. r4-solidedge-client (Rust): bridge protocol v1 client plus ConstructionScript lowering, contract-fixture tested; live Windows proof pending a host.
Later: memory, tools executor, application/core API/CLI composition over the accepted crates.

## Update: user directive "if rust will be good I want everything in rust" (2026-10-19)
- ALL of Kala's own backend code moves to Rust (conventions in `cells/CONVENTIONS.md`). Port queue in dependency order: memory-r1 and tools-protocol-r1 (running) → tools registry/sandbox/simulator/executor → runtime model_gateway/agent → application, core API (`api/*`), CLI, config → Studio design logic (templates, intent, service, session journal, local parser).
- Geometry: phase 1 is the Python/OCP worker behind a Rust `GeometryBackend` trait (r1/r2). Phase 2 (PENDING the user's explicit OK, because they earlier objected to "OpenCascade in Rust") is a narrow C++ adapter calling OpenCascade from Rust; it does NOT rewrite OCCT. The parked kala-cad crate is the starting point. It must hit the same exact acceptance numbers, and after that the Python worker and Python runtime can be retired.
- r3 packaging of the frozen Python worker is DEFERRED (avoid wasted work if phase 2 is approved). Dev mode uses the Python worker.
- Python is deleted only at the very end, after every Rust part passes its differential, and only with the user's explicit approval.

## Update: ADR-0011 provider model (user, 2026-10-19)
External CAD providers (Solid Edge, FreeCAD) build the models. OpenCascade only VIEWS the imported STEP, except when the user explicitly selects provider=opencascade. r1 was relaunched (round 2) to add `/internal/v1/import` (view-only). The Rust mirror (r2) must implement provider routing: Solid Edge via kala-solidedge-client (r4), FreeCAD via a supervised FreeCADCmd subprocess, and opencascade/import via the geometry backend.
